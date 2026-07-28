// SPDX-License-Identifier: GPL-3.0-only

//! Numeric hardware id -> human name, out of the system `hwdata` database.
//!
//! `pci.ids` alone is 1.6 MB / 43k lines and `usb.ids` another 730 KB. A full parse
//! would build ~68k `String`s to answer the ~40 questions a single machine actually
//! raises, so [`HwIds::load`] takes a [`Wanted`] set gathered by the pci/usb probes and
//! allocates only for ids that are in it. The class tables of both files and all of
//! `pnp.ids` are parsed in full instead - together they are under 500 lines and every
//! device asks about its class.

use std::collections::{HashMap, HashSet};

use super::util::read_bytes;

/// Where the hwdata files live, most likely first.
///
/// `/usr/share/hwdata` is the modern location (and the only populated one on Arch);
/// `/usr/share/misc` is Debian's and is present-but-empty elsewhere, which is why the
/// chain probes contents rather than existence.
const PCI_PATHS: &[&str] = &[
    "/usr/share/hwdata/pci.ids",
    "/usr/share/misc/pci.ids",
    "/usr/share/pci.ids",
];

const USB_PATHS: &[&str] = &[
    "/usr/share/hwdata/usb.ids",
    "/usr/share/misc/usb.ids",
    "/var/lib/usbutils/usb.ids",
    "/usr/share/usb.ids",
];

const PNP_PATHS: &[&str] = &[
    "/usr/share/hwdata/pnp.ids",
    "/usr/share/misc/pnp.ids",
    "/usr/share/pnp.ids",
];

/// The ids a given machine needs resolved. Filled in by `pci::collect_wanted` and
/// `usb::collect_wanted` before the hwdata files are opened.
#[derive(Debug, Default)]
pub struct Wanted {
    pub pci_vendors: HashSet<u16>,
    pub pci_devices: HashSet<(u16, u16)>,
    pub pci_subsystems: HashSet<(u16, u16, u16, u16)>,
    pub usb_vendors: HashSet<u16>,
    pub usb_devices: HashSet<(u16, u16)>,
}

#[derive(Debug, Default)]
pub struct HwIds {
    /// At least one hwdata file was readable. Everything else can legitimately be empty.
    loaded_any: bool,

    pci_vendors: HashMap<u16, String>,
    pci_devices: HashMap<(u16, u16), String>,
    pci_subsystems: HashMap<(u16, u16, u16, u16), String>,
    pci_classes: HashMap<u8, String>,
    pci_subclasses: HashMap<(u8, u8), String>,
    pci_progifs: HashMap<(u8, u8, u8), String>,

    usb_vendors: HashMap<u16, String>,
    usb_devices: HashMap<(u16, u16), String>,
    usb_classes: HashMap<u8, String>,
    usb_subclasses: HashMap<(u8, u8), String>,
    usb_protocols: HashMap<(u8, u8, u8), String>,

    pnp_vendors: HashMap<String, String>,
}

impl HwIds {
    pub fn load(wanted: &Wanted) -> Self {
        let mut ids = Self::default();

        match read_first(PCI_PATHS) {
            Some(data) => {
                ids.parse_pci(&data, wanted);
                ids.loaded_any = true;
            }
            None => log::warn!("no readable pci.ids; PCI devices will show raw ids"),
        }
        match read_first(USB_PATHS) {
            Some(data) => {
                ids.parse_usb(&data, wanted);
                ids.loaded_any = true;
            }
            None => log::warn!("no readable usb.ids; USB devices will show raw ids"),
        }
        match read_first(PNP_PATHS) {
            Some(data) => {
                ids.parse_pnp(&data);
                ids.loaded_any = true;
            }
            None => log::debug!("no readable pnp.ids; monitors will show their EDID code"),
        }

        ids
    }

    /// False when no hwdata file could be read at all, so the UI can say device names
    /// are unavailable and only raw ids are shown.
    pub fn available(&self) -> bool {
        self.loaded_any
    }

    pub fn pci_vendor(&self, vendor: u16) -> Option<&str> {
        self.pci_vendors.get(&vendor).map(String::as_str)
    }

    pub fn pci_device(&self, vendor: u16, device: u16) -> Option<&str> {
        self.pci_devices.get(&(vendor, device)).map(String::as_str)
    }

    pub fn pci_subsystem(
        &self,
        vendor: u16,
        device: u16,
        sub_vendor: u16,
        sub_device: u16,
    ) -> Option<&str> {
        self.pci_subsystems
            .get(&(vendor, device, sub_vendor, sub_device))
            .map(String::as_str)
    }

    /// Degrades: exact (base, sub, progif) -> (base, sub) -> (base).
    ///
    /// The subclass name is the useful label here - pci.ids writes it self-contained
    /// ("SATA controller", "VGA compatible controller") - so it leads and the base class
    /// is only the fallback. Most subclasses have no prog-if children at all, so an
    /// exact-triple-only lookup would return `None` for the majority of devices.
    pub fn pci_class(&self, base: u8, sub: u8, progif: u8) -> Option<String> {
        let base_name = self.pci_classes.get(&base).map(String::as_str);
        let sub_name = self.pci_subclasses.get(&(base, sub)).map(String::as_str);
        let progif_name = self
            .pci_progifs
            .get(&(base, sub, progif))
            .map(String::as_str);

        let parts: Vec<&str> = [sub_name.or(base_name), progif_name]
            .into_iter()
            .flatten()
            .collect();
        compose(&parts)
    }

    pub fn usb_vendor(&self, vendor: u16) -> Option<&str> {
        self.usb_vendors.get(&vendor).map(String::as_str)
    }

    pub fn usb_device(&self, vendor: u16, device: u16) -> Option<&str> {
        self.usb_devices.get(&(vendor, device)).map(String::as_str)
    }

    /// Degrades the same way. Used for USB device and interface classes.
    ///
    /// Unlike PCI, usb.ids subclass names are not self-contained ("Boot Interface
    /// Subclass", "SCSI"), so here the *base* class leads and the deeper names are
    /// appended as detail.
    pub fn usb_class(&self, base: u8, sub: u8, protocol: u8) -> Option<String> {
        let base_name = self.usb_classes.get(&base).map(String::as_str);
        let sub_name = self.usb_subclasses.get(&(base, sub)).map(String::as_str);
        let proto_name = self
            .usb_protocols
            .get(&(base, sub, protocol))
            .map(String::as_str);

        let parts: Vec<&str> = [base_name, sub_name, proto_name]
            .into_iter()
            .flatten()
            .collect();
        compose(&parts)
    }

    /// EDID three-letter manufacturer code, e.g. `"SAM"` -> `"Samsung Electric Company"`.
    pub fn pnp_vendor(&self, code: &str) -> Option<&str> {
        self.pnp_vendors
            .get(&code.trim().to_ascii_uppercase())
            .map(String::as_str)
    }

    // ------------------------------------------------------------ parsing

    fn parse_pci(&mut self, data: &[u8], wanted: &Wanted) {
        let mut section = Section::Vendors;
        let mut vendor: Option<u16> = None;
        let mut device: Option<u16> = None;
        let mut class: Option<u8> = None;
        let mut subclass: Option<u8> = None;

        for line in lines(data) {
            let Some(&first) = line.first() else { continue };
            if first == b'#' {
                continue;
            }

            if first.is_ascii_uppercase() {
                let (marker, rest) = split_marker(line);
                section = if marker == b"C".as_slice() {
                    Section::Classes
                } else {
                    Section::Other
                };
                vendor = None;
                device = None;
                class = None;
                subclass = None;
                if section == Section::Classes
                    && let Some((id, name)) = entry8(rest)
                {
                    class = Some(id);
                    self.pci_classes.insert(id, lossy(name));
                }
                continue;
            }

            let depth = line.iter().take_while(|&&b| b == b'\t').count();
            let rest = &line[depth..];

            match (section, depth) {
                (Section::Vendors, 0) => {
                    device = None;
                    vendor = None;
                    if let Some((id, name)) = entry16(rest) {
                        vendor = Some(id);
                        if wanted.pci_vendors.contains(&id) {
                            self.pci_vendors.insert(id, lossy(name));
                        }
                    }
                }
                (Section::Vendors, 1) => {
                    device = None;
                    if let (Some(v), Some((id, name))) = (vendor, entry16(rest)) {
                        device = Some(id);
                        if wanted.pci_devices.contains(&(v, id)) {
                            self.pci_devices.insert((v, id), lossy(name));
                        }
                    }
                }
                (Section::Vendors, 2) => {
                    // "\t\tssss ssss  Subsystem Name": two ids sharing one entry.
                    if let (Some(v), Some(d), Some((id, name))) =
                        (vendor, device, split_entry(rest))
                        && let Some((sv, sd)) = sub_ids(id)
                        && wanted.pci_subsystems.contains(&(v, d, sv, sd))
                    {
                        self.pci_subsystems.insert((v, d, sv, sd), lossy(name));
                    }
                }
                (Section::Classes, 1) => {
                    subclass = None;
                    if let (Some(c), Some((id, name))) = (class, entry8(rest)) {
                        subclass = Some(id);
                        self.pci_subclasses.insert((c, id), lossy(name));
                    }
                }
                (Section::Classes, 2) => {
                    if let (Some(c), Some(s), Some((id, name))) = (class, subclass, entry8(rest)) {
                        self.pci_progifs.insert((c, s, id), lossy(name));
                    }
                }
                _ => {}
            }
        }
    }

    fn parse_usb(&mut self, data: &[u8], wanted: &Wanted) {
        let mut section = Section::Vendors;
        let mut vendor: Option<u16> = None;
        let mut class: Option<u8> = None;
        let mut subclass: Option<u8> = None;

        for line in lines(data) {
            let Some(&first) = line.first() else { continue };
            if first == b'#' {
                continue;
            }

            if first.is_ascii_uppercase() {
                // usb.ids has TEN top-level sections, not one: AT BIAS C HCC HID HUT L
                // PHY R VT. A parser that only leaves the vendor tree on "C " reads
                // `L 0001  Arabic` as vendor 0x0001, so any leading capital switches
                // sections and everything but "C" is skipped wholesale.
                let (marker, rest) = split_marker(line);
                section = if marker == b"C".as_slice() {
                    Section::Classes
                } else {
                    Section::Other
                };
                vendor = None;
                class = None;
                subclass = None;
                if section == Section::Classes
                    && let Some((id, name)) = entry8(rest)
                {
                    class = Some(id);
                    self.usb_classes.insert(id, lossy(name));
                }
                continue;
            }

            let depth = line.iter().take_while(|&&b| b == b'\t').count();
            let rest = &line[depth..];

            match (section, depth) {
                (Section::Vendors, 0) => {
                    vendor = None;
                    if let Some((id, name)) = entry16(rest) {
                        vendor = Some(id);
                        if wanted.usb_vendors.contains(&id) {
                            self.usb_vendors.insert(id, lossy(name));
                        }
                    }
                }
                (Section::Vendors, 1) => {
                    if let (Some(v), Some((id, name))) = (vendor, entry16(rest))
                        && wanted.usb_devices.contains(&(v, id))
                    {
                        self.usb_devices.insert((v, id), lossy(name));
                    }
                }
                // Depth 2 in the usb.ids vendor tree is per-device interface names,
                // which sysfs never gives us an id for. Nothing to store.
                (Section::Classes, 1) => {
                    subclass = None;
                    if let (Some(c), Some((id, name))) = (class, entry8(rest)) {
                        subclass = Some(id);
                        self.usb_subclasses.insert((c, id), lossy(name));
                    }
                }
                (Section::Classes, 2) => {
                    if let (Some(c), Some(s), Some((id, name))) = (class, subclass, entry8(rest)) {
                        self.usb_protocols.insert((c, s, id), lossy(name));
                    }
                }
                _ => {}
            }
        }
    }

    /// pnp.ids is flat and tab separated: `SAM\tSamsung Electric Company`.
    fn parse_pnp(&mut self, data: &[u8]) {
        for line in lines(data) {
            match line.first() {
                None | Some(&b'#') => continue,
                _ => {}
            }
            let text = lossy(line);
            let Some((code, name)) = text.split_once('\t') else {
                continue;
            };
            let (code, name) = (code.trim(), name.trim());
            if code.is_empty() || name.is_empty() {
                continue;
            }
            // EDID stores the code in upper case; a handful of pnp.ids rows are not.
            self.pnp_vendors
                .insert(code.to_ascii_uppercase(), name.to_owned());
        }
    }
}

// ---------------------------------------------------------------- helpers

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Vendors,
    Classes,
    Other,
}

fn read_first(paths: &[&str]) -> Option<Vec<u8>> {
    paths.iter().find_map(read_bytes)
}

/// Splits into lines without decoding.
///
/// These files are not guaranteed valid UTF-8 across releases (historical Latin-1 vendor
/// names), and `BufReader::lines` answers `Err(InvalidData)` on the first such line,
/// which truncates the whole parse. Bytes in, lossy decoding only for names we keep.
fn lines(data: &[u8]) -> impl Iterator<Item = &[u8]> {
    data.split(|&b| b == b'\n').map(|line| {
        if line.last() == Some(&b'\r') {
            &line[..line.len() - 1]
        } else {
            line
        }
    })
}

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Splits a section line such as `C 03  Display controller` into `C` and the remainder.
fn split_marker(line: &[u8]) -> (&[u8], &[u8]) {
    match line.iter().position(|&b| b == b' ') {
        Some(sep) => (&line[..sep], &line[sep + 1..]),
        None => (line, &[]),
    }
}

/// Splits `1234  Some Name` at the mandatory two-space separator.
fn split_entry(rest: &[u8]) -> Option<(&[u8], &[u8])> {
    let sep = rest.windows(2).position(|w| w[0] == b' ' && w[1] == b' ')?;
    let id = &rest[..sep];
    let name = rest[sep + 2..].trim_ascii();
    if id.is_empty() || name.is_empty() {
        None
    } else {
        Some((id, name))
    }
}

fn entry8(rest: &[u8]) -> Option<(u8, &[u8])> {
    let (id, name) = split_entry(rest)?;
    Some((u8::try_from(hex(id)?).ok()?, name))
}

fn entry16(rest: &[u8]) -> Option<(u16, &[u8])> {
    let (id, name) = split_entry(rest)?;
    Some((u16::try_from(hex(id)?).ok()?, name))
}

/// Splits the `ssss ssss` id pair of a pci.ids subsystem line.
fn sub_ids(id: &[u8]) -> Option<(u16, u16)> {
    let sep = id.iter().position(|&b| b == b' ')?;
    let sv = u16::try_from(hex(&id[..sep])?).ok()?;
    let sd = u16::try_from(hex(&id[sep + 1..])?).ok()?;
    Some((sv, sd))
}

fn hex(bytes: &[u8]) -> Option<u32> {
    if bytes.is_empty() || bytes.len() > 8 {
        return None;
    }
    let mut value: u32 = 0;
    for &byte in bytes {
        value = value * 16 + char::from(byte).to_digit(16)?;
    }
    Some(value)
}

/// Joins class names from general to specific with `" - "`, keeping only the ones that
/// contribute something.
///
/// Two things are dropped. Filler names, because `C 03 / 00 No Subclass / 01 Keyboard`
/// should read "Human Interface Device - Keyboard". And names whose every word is
/// already present, because pci.ids has `C 03 / 00 VGA compatible controller / 00 VGA
/// controller` and "VGA compatible controller - VGA controller" is noise.
fn compose(parts: &[&str]) -> Option<String> {
    let (head, extras) = parts.split_first()?;
    let mut seen = words(head);
    let mut out = (*head).to_owned();
    for extra in extras {
        if is_filler(extra) {
            continue;
        }
        let extra_words = words(extra);
        if extra_words.is_empty() || extra_words.iter().all(|w| seen.contains(w)) {
            continue;
        }
        seen.extend(extra_words);
        out.push_str(" - ");
        out.push_str(extra);
    }
    Some(out)
}

fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Placeholder names the class tables use for "nothing special here".
fn is_filler(name: &str) -> bool {
    const FILLER: &[&str] = &[
        "no subclass",
        "unused",
        "undefined",
        "unspecified",
        "none",
        "reserved",
        "generic",
        "other",
        "vendor specific",
        "vendor specific subclass",
        "vendor specific protocol",
    ];
    FILLER.contains(&name.trim().to_ascii_lowercase().as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PCI_FIXTURE: &[u8] = b"\
#\tList of PCI IDs
#\tVersion: 2026.07.01

10de  NVIDIA Corporation
\t1c03  GP106 [GeForce GTX 1060 6GB]
\t\t1043 8562  GTX1060-DC2O6G
\t\t1458 3739  GeForce GTX 1060 6GB
\t1c04  GP106 [GeForce GTX 1060 5GB]
8086  Intel Corporation
\t9a49  TigerLake-LP GT2 [Iris Xe Graphics]

# Device classes
C 01  Mass storage controller
\t06  SATA controller
\t\t00  Vendor specific
\t\t01  AHCI 1.0
\t08  Non-Volatile memory controller
\t\t02  NVM Express
C 03  Display controller
\t00  VGA compatible controller
\t\t00  VGA controller
\t02  3D controller
";

    const USB_FIXTURE: &[u8] = b"\
# usb.ids
046d  Logitech, Inc.
\tc52b  Unifying Receiver
\t\t00  Keyboard interface
1d6b  Linux Foundation
\t0002  2.0 root hub
C 03  Human Interface Device
\t00  No Subclass
\t\t01  Keyboard
\t01  Boot Interface Subclass
\t\t02  Mouse
C 09  Hub
\t00  Unused
\t\t00  Full speed (or root) hub
AT 00  Reserved
HID 01  Monitor
L 0001  Arabic
\t01  Saudi Arabia
\t02  Iraq
VT 0100  Vendor
";

    fn pci_wanted() -> Wanted {
        let mut wanted = Wanted::default();
        wanted.pci_vendors.insert(0x10de);
        wanted.pci_devices.insert((0x10de, 0x1c03));
        wanted
            .pci_subsystems
            .insert((0x10de, 0x1c03, 0x1043, 0x8562));
        wanted
    }

    fn usb_wanted() -> Wanted {
        let mut wanted = Wanted::default();
        wanted.usb_vendors.insert(0x046d);
        wanted.usb_vendors.insert(0x0001);
        wanted.usb_devices.insert((0x046d, 0xc52b));
        wanted
    }

    fn loaded_pci() -> HwIds {
        let mut ids = HwIds::default();
        ids.parse_pci(PCI_FIXTURE, &pci_wanted());
        ids
    }

    fn loaded_usb() -> HwIds {
        let mut ids = HwIds::default();
        ids.parse_usb(USB_FIXTURE, &usb_wanted());
        ids
    }

    #[test]
    fn pci_nesting_resolves_vendor_device_and_subsystem() {
        let ids = loaded_pci();
        assert_eq!(ids.pci_vendor(0x10de), Some("NVIDIA Corporation"));
        assert_eq!(
            ids.pci_device(0x10de, 0x1c03),
            Some("GP106 [GeForce GTX 1060 6GB]")
        );
        assert_eq!(
            ids.pci_subsystem(0x10de, 0x1c03, 0x1043, 0x8562),
            Some("GTX1060-DC2O6G")
        );
    }

    #[test]
    fn pci_ids_outside_the_wanted_set_are_not_allocated() {
        let ids = loaded_pci();
        assert_eq!(ids.pci_vendor(0x8086), None);
        assert_eq!(ids.pci_device(0x10de, 0x1c04), None);
        // Same device, a subsystem nobody asked about.
        assert_eq!(ids.pci_subsystem(0x10de, 0x1c03, 0x1458, 0x3739), None);
        assert_eq!(ids.pci_vendors.len(), 1);
        assert_eq!(ids.pci_devices.len(), 1);
        assert_eq!(ids.pci_subsystems.len(), 1);
    }

    #[test]
    fn pci_class_degrades_and_drops_redundant_progif() {
        let ids = loaded_pci();
        // Exact triple, prog-if adds information.
        assert_eq!(
            ids.pci_class(0x01, 0x06, 0x01).as_deref(),
            Some("SATA controller - AHCI 1.0")
        );
        // Prog-if present in sysfs but not in the table: fall back to the subclass.
        assert_eq!(
            ids.pci_class(0x01, 0x06, 0x77).as_deref(),
            Some("SATA controller")
        );
        // Filler prog-if name is dropped.
        assert_eq!(
            ids.pci_class(0x01, 0x06, 0x00).as_deref(),
            Some("SATA controller")
        );
        // Subclass with no prog-if children at all.
        assert_eq!(
            ids.pci_class(0x03, 0x02, 0x00).as_deref(),
            Some("3D controller")
        );
        // Unknown subclass: fall back to the base class.
        assert_eq!(
            ids.pci_class(0x03, 0x77, 0x00).as_deref(),
            Some("Display controller")
        );
        // "VGA compatible controller - VGA controller" would be noise.
        assert_eq!(
            ids.pci_class(0x03, 0x00, 0x00).as_deref(),
            Some("VGA compatible controller")
        );
        assert_eq!(ids.pci_class(0xff, 0x00, 0x00), None);
    }

    #[test]
    fn usb_vendor_tree_and_wanted_filter() {
        let ids = loaded_usb();
        assert_eq!(ids.usb_vendor(0x046d), Some("Logitech, Inc."));
        assert_eq!(ids.usb_device(0x046d, 0xc52b), Some("Unifying Receiver"));
        assert_eq!(ids.usb_vendor(0x1d6b), None);
    }

    #[test]
    fn usb_non_class_sections_are_not_read_as_vendors() {
        let ids = loaded_usb();
        // The trap: `L 0001  Arabic` must not become vendor 0x0001, and its indented
        // country rows must not become that vendor's devices.
        assert_eq!(ids.usb_vendor(0x0001), None);
        assert_eq!(ids.usb_device(0x0001, 0x01), None);
        assert_eq!(ids.usb_vendors.len(), 1);
        assert_eq!(ids.usb_devices.len(), 1);
        // The `HID`/`AT`/`VT` markers must not land in the class tables either.
        assert_eq!(ids.usb_classes.len(), 2);
    }

    #[test]
    fn usb_class_leads_with_the_base_class() {
        let ids = loaded_usb();
        assert_eq!(
            ids.usb_class(0x03, 0x00, 0x01).as_deref(),
            Some("Human Interface Device - Keyboard")
        );
        assert_eq!(
            ids.usb_class(0x03, 0x01, 0x02).as_deref(),
            Some("Human Interface Device - Boot Interface Subclass - Mouse")
        );
        // Unknown subclass/protocol: base class only.
        assert_eq!(
            ids.usb_class(0x03, 0x77, 0x77).as_deref(),
            Some("Human Interface Device")
        );
        // "Unused" is filler; "hub" is already in the head, the rest is not.
        assert_eq!(
            ids.usb_class(0x09, 0x00, 0x00).as_deref(),
            Some("Hub - Full speed (or root) hub")
        );
        assert_eq!(ids.usb_class(0x08, 0x06, 0x50), None);
    }

    #[test]
    fn pnp_lookup_is_case_insensitive() {
        let mut ids = HwIds::default();
        ids.parse_pnp(
            b"SAM\tSamsung Electric Company\nACR\tAcer Technologies\ninu\tInovatec S.p.A.\n",
        );
        assert_eq!(ids.pnp_vendor("SAM"), Some("Samsung Electric Company"));
        assert_eq!(ids.pnp_vendor("acr"), Some("Acer Technologies"));
        assert_eq!(ids.pnp_vendor("INU"), Some("Inovatec S.p.A."));
        assert_eq!(ids.pnp_vendor("ZZZ"), None);
    }

    #[test]
    fn latin1_bytes_do_not_truncate_the_parse() {
        // 0xe9 is a bare Latin-1 'e' with acute: invalid UTF-8, and historically present
        // in these files. It must degrade to U+FFFD without stopping the parser.
        let mut data = Vec::from(&b"1234  Caf"[..]);
        data.push(0xe9);
        data.extend_from_slice(b" SA\n5678  Later Vendor\n");

        let mut wanted = Wanted::default();
        wanted.pci_vendors.insert(0x1234);
        wanted.pci_vendors.insert(0x5678);
        let mut ids = HwIds::default();
        ids.parse_pci(&data, &wanted);

        assert_eq!(ids.pci_vendor(0x1234), Some("Caf\u{fffd} SA"));
        assert_eq!(ids.pci_vendor(0x5678), Some("Later Vendor"));
    }

    #[test]
    fn empty_load_is_not_available() {
        let ids = HwIds::default();
        assert!(!ids.available());
        assert_eq!(ids.pci_vendor(0x8086), None);
        assert_eq!(ids.pci_class(0x03, 0x00, 0x00), None);
        assert_eq!(ids.usb_class(0x09, 0x00, 0x00), None);
        assert_eq!(ids.pnp_vendor("SAM"), None);
    }

    #[test]
    fn malformed_lines_are_skipped() {
        let mut wanted = Wanted::default();
        wanted.pci_vendors.insert(0x8086);
        let mut ids = HwIds::default();
        // Single-space separator, no separator, empty name, non-hex id.
        ids.parse_pci(
            b"8086 Intel\nzzzz  Bogus\n8086  \n\n8086  Intel Corporation\n",
            &wanted,
        );
        assert_eq!(ids.pci_vendor(0x8086), Some("Intel Corporation"));
        assert_eq!(ids.pci_vendors.len(), 1);
    }
}
