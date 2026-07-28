// SPDX-License-Identifier: GPL-3.0-only

//! Graphics adapters and attached displays.
//!
//! GPUs are a filtered view of the PCI list that `pci.rs` already resolved, so no device
//! is scanned or named twice. Displays come from `/sys/class/drm/<card>-<connector>/`,
//! including the raw EDID blob, which is parsed here by hand.

use std::path::Path;

use super::util::{read_bytes, read_dir_sorted, read_trim};
use super::{Display, Gpu, Graphics, PciDevice, hwids::HwIds};

const DRM_CLASS: &str = "/sys/class/drm";
const MODULE_DIR: &str = "/sys/module";

/// PCI class 0x03: display controller.
const CLASS_DISPLAY: u8 = 0x03;

const EDID_HEADER: [u8; 8] = [0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00];
const EDID_BASE_LEN: usize = 128;

pub fn probe(pci: &[PciDevice], ids: &HwIds) -> Graphics {
    Graphics {
        gpus: gpus(pci),
        displays: displays(ids),
    }
}

// ---------------------------------------------------------------- gpus

fn gpus(pci: &[PciDevice]) -> Vec<Gpu> {
    pci.iter()
        .filter(|dev| dev.raw.class_base == CLASS_DISPLAY)
        .map(|dev| Gpu {
            slot: dev.raw.slot.clone(),
            vendor_name: dev.vendor_name.clone(),
            device_name: dev.device_name.clone(),
            subsystem_name: dev.subsystem_name.clone(),
            id_string: dev.id_string(),
            revision: dev.raw.revision,
            driver: dev.raw.driver.clone(),
            driver_version: dev.raw.driver.as_deref().and_then(module_version),
            drm_nodes: dev.raw.drm_nodes.clone(),
        })
        .collect()
}

/// `/sys/module/<driver>/version`, when the module publishes one.
///
/// Out-of-tree modules export it (`nvidia` reports e.g. `610.43.03`); in-tree DRM
/// drivers such as `i915` and `amdgpu` do not have the attribute at all, so `None` is
/// the ordinary result rather than an error.
fn module_version(driver: &str) -> Option<String> {
    if let Some(version) = read_trim(Path::new(MODULE_DIR).join(driver).join("version")) {
        return Some(version);
    }
    // A driver name may carry dashes where the module directory uses underscores.
    let alt = driver.replace('-', "_");
    if alt == driver {
        return None;
    }
    read_trim(Path::new(MODULE_DIR).join(alt).join("version"))
}

// ---------------------------------------------------------------- displays

fn displays(ids: &HwIds) -> Vec<Display> {
    let mut out = Vec::new();
    for path in read_dir_sorted(DRM_CLASS) {
        let Some(entry) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(connector) = connector_name(entry) else {
            continue;
        };

        // Only `status` is trusted here. Under the proprietary NVIDIA driver every
        // connector reports `enabled=disabled` and `dpms=Off` even while it is driving a
        // monitor, so reading those attributes would hide every working display.
        let connected = read_trim(path.join("status")).as_deref() == Some("connected");

        let (preferred_mode, mode_count) = modes(path.join("modes"));

        let mut display = Display {
            connector: connector.to_owned(),
            connected,
            preferred_mode,
            mode_count,
            ..Display::default()
        };

        // `edid` is a sysfs binary attribute: stat() claims size 0 while the file really
        // holds 128 bytes, so it must be read to end (util::read_bytes) and never sized
        // from metadata.
        if let Some(blob) = read_bytes(path.join("edid")) {
            match parse_edid(&blob) {
                Some(edid) => apply_edid(&mut display, edid, ids),
                None => log::debug!("ignoring unparseable EDID on connector {connector}"),
            }
        }

        out.push(display);
    }
    out
}

/// Splits `card1-DP-1` into `DP-1`, rejecting `card1`, `renderD128` and `version`.
fn connector_name(entry: &str) -> Option<&str> {
    let rest = entry.strip_prefix("card")?;
    let dash = rest.find('-')?;
    // Everything between `card` and the first dash is the card index; anything else
    // (`controlD65`, `renderD128`) is not a connector.
    if dash == 0 || !rest[..dash].bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let name = &rest[dash + 1..];
    (!name.is_empty()).then_some(name)
}

/// Reads the `modes` attribute: the first line is the preferred mode, the remaining
/// lines are only counted.
fn modes(path: impl AsRef<Path>) -> (Option<String>, usize) {
    let Some(text) = read_trim(path) else {
        return (None, 0);
    };
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    let preferred = lines.next().map(str::to_owned);
    (preferred, lines.count())
}

fn apply_edid(display: &mut Display, edid: Edid, ids: &HwIds) {
    display.make = edid.manufacturer.map(|code| {
        ids.pnp_vendor(&code)
            .map(|name| name.to_string())
            .unwrap_or(code)
    });
    display.model = edid.model_name;
    // The 0xFF descriptor string is what the vendor prints on the label; the numeric
    // serial in bytes 12-15 is only a fallback.
    display.serial = edid
        .serial_string
        .or_else(|| edid.serial_number.map(|n| n.to_string()));
    display.year = edid.year;
    display.physical_mm = edid.physical_mm;
}

// ---------------------------------------------------------------- edid

/// The fields we take from an EDID 1.x base block. Extension blocks (CTA-861 and
/// friends) are ignored: they carry audio and HDMI capabilities we do not display.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Edid {
    /// Three-letter PNP manufacturer code, e.g. `DEL`.
    manufacturer: Option<String>,
    model_name: Option<String>,
    serial_string: Option<String>,
    serial_number: Option<u32>,
    year: Option<u32>,
    physical_mm: Option<(u32, u32)>,
}

fn parse_edid(blob: &[u8]) -> Option<Edid> {
    let base = blob.get(..EDID_BASE_LEN)?;
    if base[..8] != EDID_HEADER {
        return None;
    }
    // The whole base block, checksum byte included, must sum to zero mod 256. A monitor
    // behind a flaky cable or a bad KVM hands out corrupted blobs; reporting garbage
    // vendor names from them is worse than reporting nothing.
    if base.iter().fold(0u8, |acc, &b| acc.wrapping_add(b)) != 0 {
        return None;
    }

    let mut edid = Edid {
        manufacturer: manufacturer_code(u16::from_be_bytes([base[8], base[9]])),
        ..Edid::default()
    };

    let serial = u32::from_le_bytes([base[12], base[13], base[14], base[15]]);
    if serial != 0 {
        edid.serial_number = Some(serial);
    }

    // Byte 17 holds the manufacture year offset from 1990; 0 means "not stated".
    if base[17] != 0 {
        edid.year = Some(1990 + u32::from(base[17]));
    }

    // Bytes 21-22 are the image size in whole centimetres. Both zero means the size is
    // undefined (projectors, and some capture dongles), not a 0 mm panel.
    let (w_cm, h_cm) = (u32::from(base[21]), u32::from(base[22]));
    if w_cm != 0 && h_cm != 0 {
        edid.physical_mm = Some((w_cm * 10, h_cm * 10));
    }

    for offset in [54, 72, 90, 108] {
        let descriptor = &base[offset..offset + 18];
        // A zero pixel clock in bytes 0-2 marks a display descriptor rather than a
        // detailed timing block; byte 3 then selects which kind it is.
        if descriptor[..3] != [0, 0, 0] {
            continue;
        }
        match descriptor[3] {
            0xFC => edid.model_name = descriptor_text(descriptor),
            0xFF => edid.serial_string = descriptor_text(descriptor),
            _ => {}
        }
    }

    Some(edid)
}

/// Decodes the packed manufacturer id: three 5-bit letters, big-endian, `A` == 1.
fn manufacturer_code(raw: u16) -> Option<String> {
    let mut code = String::with_capacity(3);
    for shift in [10, 5, 0] {
        let letter = ((raw >> shift) & 0x1F) as u8;
        if !(1..=26).contains(&letter) {
            return None;
        }
        code.push((b'A' + letter - 1) as char);
    }
    Some(code)
}

/// Payload of a text descriptor: 13 bytes at offset 5, terminated by `0x0A` and padded
/// with spaces.
fn descriptor_text(descriptor: &[u8]) -> Option<String> {
    let payload = descriptor.get(5..18)?;
    let end = payload
        .iter()
        .position(|&b| b == 0x0A)
        .unwrap_or(payload.len());
    let text: String = payload[..end]
        .iter()
        // The spec says printable ASCII; blobs in the wild carry latin-1 or stray NULs,
        // which become spaces and are then trimmed away.
        .map(|&b| {
            if (0x20..0x7F).contains(&b) {
                b as char
            } else {
                ' '
            }
        })
        .collect();
    let text = text.trim().to_owned();
    (!text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a valid 128-byte base block for a Dell-branded monitor.
    fn synth_edid() -> [u8; EDID_BASE_LEN] {
        let mut edid = [0u8; EDID_BASE_LEN];
        edid[..8].copy_from_slice(&EDID_HEADER);
        // 0x10AC == "DEL".
        edid[8] = 0x10;
        edid[9] = 0xAC;
        edid[12..16].copy_from_slice(&0x3132_4D4Cu32.to_le_bytes());
        edid[17] = 23; // 1990 + 23
        edid[21] = 52; // 520 mm
        edid[22] = 32; // 320 mm

        write_text_descriptor(&mut edid, 54, 0xFC, b"DELL U2412M");
        write_text_descriptor(&mut edid, 72, 0xFF, b"0FFXD3AE12ML");

        let sum = edid[..EDID_BASE_LEN - 1]
            .iter()
            .fold(0u8, |acc, &b| acc.wrapping_add(b));
        edid[EDID_BASE_LEN - 1] = sum.wrapping_neg();
        edid
    }

    fn write_text_descriptor(edid: &mut [u8; EDID_BASE_LEN], at: usize, tag: u8, text: &[u8]) {
        edid[at + 3] = tag;
        edid[at + 5..at + 5 + text.len()].copy_from_slice(text);
        edid[at + 5 + text.len()] = 0x0A;
        for byte in &mut edid[at + 6 + text.len()..at + 18] {
            *byte = b' ';
        }
    }

    #[test]
    fn parses_a_well_formed_base_block() {
        let parsed = parse_edid(&synth_edid()).expect("valid block");
        assert_eq!(parsed.manufacturer.as_deref(), Some("DEL"));
        assert_eq!(parsed.model_name.as_deref(), Some("DELL U2412M"));
        assert_eq!(parsed.serial_string.as_deref(), Some("0FFXD3AE12ML"));
        assert_eq!(parsed.serial_number, Some(0x3132_4D4C));
        assert_eq!(parsed.year, Some(2013));
        assert_eq!(parsed.physical_mm, Some((520, 320)));
    }

    #[test]
    fn rejects_bad_header_checksum_and_short_blobs() {
        let good = synth_edid();

        let mut wrong_header = good;
        wrong_header[1] = 0x00;
        assert_eq!(parse_edid(&wrong_header), None);

        let mut wrong_checksum = good;
        wrong_checksum[EDID_BASE_LEN - 1] = wrong_checksum[EDID_BASE_LEN - 1].wrapping_add(1);
        assert_eq!(parse_edid(&wrong_checksum), None);

        assert_eq!(parse_edid(&good[..127]), None);
        assert_eq!(parse_edid(&[]), None);
    }

    #[test]
    fn absent_optional_fields_stay_none() {
        let mut edid = [0u8; EDID_BASE_LEN];
        edid[..8].copy_from_slice(&EDID_HEADER);
        edid[8] = 0x10;
        edid[9] = 0xAC;
        // No serial, no year, no physical size, no descriptors.
        let sum = edid[..EDID_BASE_LEN - 1]
            .iter()
            .fold(0u8, |acc, &b| acc.wrapping_add(b));
        edid[EDID_BASE_LEN - 1] = sum.wrapping_neg();

        let parsed = parse_edid(&edid).expect("valid block");
        assert_eq!(parsed.serial_number, None);
        assert_eq!(parsed.serial_string, None);
        assert_eq!(parsed.model_name, None);
        assert_eq!(parsed.year, None);
        assert_eq!(parsed.physical_mm, None);
    }

    #[test]
    fn manufacturer_bits_decode_to_three_letters() {
        assert_eq!(manufacturer_code(0x10AC).as_deref(), Some("DEL"));
        // A=1, B=2, C=3
        assert_eq!(
            manufacturer_code((1 << 10) | (2 << 5) | 3).as_deref(),
            Some("ABC")
        );
        assert_eq!(manufacturer_code(0x220E).as_deref(), Some("HPN"));
        // Zero letters are out of the 1..=26 range: an unprogrammed or corrupt id.
        assert_eq!(manufacturer_code(0x0000), None);
        assert_eq!(manufacturer_code(0xFFFF), None);
    }

    #[test]
    fn descriptor_text_stops_at_the_terminator() {
        let mut descriptor = [b'X'; 18];
        descriptor[..3].copy_from_slice(&[0, 0, 0]);
        descriptor[3] = 0xFC;
        descriptor[4] = 0;
        descriptor[5..].copy_from_slice(b"OMEN 27k\n    ");
        assert_eq!(descriptor_text(&descriptor).as_deref(), Some("OMEN 27k"));

        // Padding only: nothing worth showing.
        let mut blank = [0u8; 18];
        blank[3] = 0xFF;
        blank[5..].copy_from_slice(b"             ");
        assert_eq!(descriptor_text(&blank), None);
    }

    #[test]
    fn connector_names_drop_the_card_prefix() {
        assert_eq!(connector_name("card1-DP-1"), Some("DP-1"));
        assert_eq!(connector_name("card0-HDMI-A-1"), Some("HDMI-A-1"));
        assert_eq!(connector_name("card1"), None);
        assert_eq!(connector_name("renderD128"), None);
        assert_eq!(connector_name("controlD64"), None);
        assert_eq!(connector_name("version"), None);
        assert_eq!(connector_name("card1-"), None);
        assert_eq!(connector_name("card-DP-1"), None);
    }
}
