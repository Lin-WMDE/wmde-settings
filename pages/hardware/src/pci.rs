// SPDX-License-Identifier: GPL-3.0-only

//! PCI enumeration from `/sys/bus/pci/devices`.
//!
//! Everything comes from the per-device sysfs attributes, which are world readable. The
//! `config` file is deliberately never opened: every value we want has a dedicated
//! attribute, and anything past the first 64 bytes of config space is root-only anyway.
//!
//! The one thing sysfs does not expose is the list of *candidate* kernel modules for a
//! device (it only tells us which driver is currently bound), so `lspci -nnk` is run once
//! as optional enrichment. A missing or broken `lspci` never fails the scan.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use super::hwids::{HwIds, Wanted};
use super::util::{
    link_basename, read_dir_sorted, read_hex, read_hex8, read_hex16, read_i64, read_u32,
};
use super::{PciDevice, PciRaw};

const PCI_DEVICES: &str = "/sys/bus/pci/devices";

/// Absolute paths instead of `which`: this runs in a desktop session whose `PATH` we do
/// not control, and we do not want to execute whatever a hostile `PATH` points at.
const LSPCI_BINARIES: &[&str] = &["/usr/bin/lspci", "/bin/lspci"];

const LSPCI_TIMEOUT: Duration = Duration::from_secs(5);

/// Enumerates every PCI device, sorted by slot.
pub fn scan() -> Vec<PciRaw> {
    // Directory names are the full `domain:bus:device.function` slot, so sorting the
    // paths sorts by slot. Nothing here assumes a canonical slot exists: `0000:00:02.0`
    // (integrated graphics) is simply absent on a CPU without an iGPU.
    let mut devices: Vec<PciRaw> = read_dir_sorted(PCI_DEVICES)
        .into_iter()
        .filter_map(|path| read_device(&path))
        .collect();
    enrich_modules(&mut devices);
    devices
}

fn read_device(path: &Path) -> Option<PciRaw> {
    let slot = path.file_name()?.to_str()?.to_owned();

    // Vendor and device are the only mandatory attributes: an entry without them is not
    // something we can identify or name, so it is skipped entirely.
    let vendor_id = read_hex16(path.join("vendor"))?;
    let device_id = read_hex16(path.join("device"))?;

    let (class_base, class_sub, class_progif) =
        split_class(read_hex(path.join("class")).unwrap_or(0));

    Some(PciRaw {
        parent: parent_slot(path),
        slot,
        class_base,
        class_sub,
        class_progif,
        vendor_id,
        device_id,
        subsystem_vendor_id: read_hex16(path.join("subsystem_vendor")),
        subsystem_device_id: read_hex16(path.join("subsystem_device")),
        revision: read_hex8(path.join("revision")).unwrap_or(0),
        driver: link_basename(path.join("driver")),
        modules: Vec::new(),
        // IRQ 0 means "no interrupt assigned", not "interrupt line 0"; host bridges and
        // other passive devices all report it.
        irq: read_u32(path.join("irq")).filter(|&irq| irq != 0),
        // -1 is the kernel's "no NUMA affinity", which is what every single-node desktop
        // reports, hence the signed read.
        numa_node: read_i64(path.join("numa_node"))
            .filter(|&node| node >= 0)
            .and_then(|node| i32::try_from(node).ok()),
        drm_nodes: drm_nodes(path),
    })
}

/// Slot of the bridge a device sits behind, or `None` at the root complex.
///
/// Every entry in `/sys/bus/pci/devices` is a symlink into `/sys/devices`, and that path
/// *is* the topology: `0000:07:00.0 -> .../pci0000:00/0000:00:1c.2/0000:07:00.0` says the
/// device hangs off bridge `00:1c.2`. There is no `parent` attribute to read instead.
fn parent_slot(path: &Path) -> Option<String> {
    parent_from_link(&std::fs::read_link(path).ok()?)
}

/// Pure half of [`parent_slot`], split out so it can be tested without sysfs.
fn parent_from_link(target: &Path) -> Option<String> {
    let mut components: Vec<&str> = target
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect();
    // The last component is the device itself; the one before it is the parent, but only
    // when it is a slot. Under the root complex it is `pci0000:00`, which is not.
    components.pop()?;
    let candidate = components.pop()?;
    is_slot(candidate).then(|| candidate.to_owned())
}

/// Splits the sysfs `class` attribute into base class, subclass and prog-if.
///
/// The attribute is 24-bit (`0x030000` = base 03, subclass 00, prog-if 00), not the
/// 16-bit `[0300]` code lspci prints. Parsing it as four hex digits drops the base class
/// and turns every device into class 00.
fn split_class(class: u32) -> (u8, u8, u8) {
    (
        ((class >> 16) & 0xff) as u8,
        ((class >> 8) & 0xff) as u8,
        (class & 0xff) as u8,
    )
}

/// DRM nodes bound to this device, e.g. `["card1", "controlD65", "renderD128"]`.
///
/// Reading `<device>/drm/` is the only correct GPU -> DRM node association. Node numbers
/// come from probe order, so a machine with exactly one GPU can have `card1` and no
/// `card0` at all; guessing `card0` is wrong on any machine with a disabled iGPU.
fn drm_nodes(path: &Path) -> Vec<String> {
    read_dir_sorted(path.join("drm"))
        .into_iter()
        .filter_map(|node| node.file_name()?.to_str().map(str::to_owned))
        .collect()
}

/// Records the hwdata lookups this device list needs, for the single-pass `pci.ids` read.
pub fn collect_wanted(raw: &[PciRaw], wanted: &mut Wanted) {
    for device in raw {
        wanted.pci_vendors.insert(device.vendor_id);
        wanted
            .pci_devices
            .insert((device.vendor_id, device.device_id));
        if let (Some(sub_vendor), Some(sub_device)) =
            (device.subsystem_vendor_id, device.subsystem_device_id)
        {
            // pci.ids nests subsystem names under the device, not under the subsystem
            // vendor, so the lookup key is the whole quad.
            wanted.pci_subsystems.insert((
                device.vendor_id,
                device.device_id,
                sub_vendor,
                sub_device,
            ));
        }
    }
}

/// Attaches hwdata names to the scanned devices.
pub fn resolve(raw: Vec<PciRaw>, ids: &HwIds) -> Vec<PciDevice> {
    raw.into_iter()
        .map(|raw| {
            let subsystem_name = match (raw.subsystem_vendor_id, raw.subsystem_device_id) {
                (Some(sub_vendor), Some(sub_device)) => ids
                    .pci_subsystem(raw.vendor_id, raw.device_id, sub_vendor, sub_device)
                    .map(|name| name.to_string()),
                _ => None,
            };
            PciDevice {
                vendor_name: ids.pci_vendor(raw.vendor_id).map(|name| name.to_string()),
                device_name: ids
                    .pci_device(raw.vendor_id, raw.device_id)
                    .map(|name| name.to_string()),
                subsystem_name,
                class_name: ids
                    .pci_class(raw.class_base, raw.class_sub, raw.class_progif)
                    .map(|name| name.to_string()),
                raw,
            }
        })
        .collect()
}

// ---------------------------------------------------------------- lspci enrichment

/// Fills in `PciRaw::modules` from `lspci -nnk`.
///
/// Candidate modules are the single piece of information `lspci` has that sysfs does not,
/// so this is the only place the app shells out for PCI data. Any failure is logged and
/// ignored: the device list is already complete without it.
fn enrich_modules(devices: &mut [PciRaw]) {
    let Some(binary) = LSPCI_BINARIES
        .iter()
        .copied()
        .find(|path| Path::new(path).is_file())
    else {
        log::debug!("lspci not installed; PCI kernel module candidates unavailable");
        return;
    };
    let Some(output) = run_lspci(binary) else {
        return;
    };

    for (slot, modules) in parse_lspci(&output) {
        let matched = devices
            .iter_mut()
            .find(|device| device.slot == slot || short_slot(&device.slot) == short_slot(&slot));
        if let Some(device) = matched {
            device.modules = modules;
        }
    }
}

fn run_lspci(binary: &str) -> Option<String> {
    let mut child = Command::new(binary)
        .arg("-nnk")
        // Without this the block labels we parse ("Kernel modules:") come back localised.
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| log::debug!("lspci failed to start: {err}"))
        .ok()?;

    let mut stdout = child.stdout.take()?;
    let (tx, rx) = mpsc::channel();
    // `Command` has no timeout, and lspci can wedge on a config-space read of a
    // misbehaving device. Read on a helper thread so a stuck child cannot hang the scan.
    std::thread::spawn(move || {
        let mut buffer = String::new();
        let _ = stdout.read_to_string(&mut buffer);
        let _ = tx.send(buffer);
    });

    match rx.recv_timeout(LSPCI_TIMEOUT) {
        Ok(text) => {
            let _ = child.wait();
            Some(text)
        }
        Err(err) => {
            log::debug!("lspci produced no output ({err}); module candidates unavailable");
            let _ = child.kill();
            let _ = child.wait();
            None
        }
    }
}

/// Parses the block format of `lspci -nnk` into `(slot, modules)` pairs.
///
/// A device block starts at column 0 with its slot; every following indented line belongs
/// to it. Only `Kernel modules:` is of interest - the rest of the block duplicates sysfs.
fn parse_lspci(text: &str) -> Vec<(String, Vec<String>)> {
    let mut devices: Vec<(String, Vec<String>)> = Vec::new();

    for line in text.lines() {
        if line.starts_with(' ') || line.starts_with('\t') {
            let Some(rest) = line.trim_start().strip_prefix("Kernel modules:") else {
                continue;
            };
            // Ignore a stray continuation line before any device header.
            let Some((_, modules)) = devices.last_mut() else {
                continue;
            };
            *modules = rest
                .split(',')
                .map(str::trim)
                .filter(|module| !module.is_empty())
                .map(str::to_owned)
                .collect();
        } else if let Some(slot) = line.split_whitespace().next()
            && is_slot(slot)
        {
            devices.push((slot.to_owned(), Vec::new()));
        }
    }

    devices
}

/// True for `bb:dd.f` and for the `dddd:bb:dd.f` form lspci prints with `-D`.
fn is_slot(token: &str) -> bool {
    let Some((head, function)) = token.rsplit_once('.') else {
        return false;
    };
    !function.is_empty()
        && function.chars().all(|c| c.is_ascii_hexdigit())
        && head.contains(':')
        && head.chars().all(|c| c.is_ascii_hexdigit() || c == ':')
}

/// Drops the PCI domain, because lspci omits `0000:` unless `-D` is passed.
fn short_slot(slot: &str) -> &str {
    if slot.matches(':').count() == 2 {
        slot.split_once(':').map_or(slot, |(_, rest)| rest)
    } else {
        slot
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_is_split_as_24_bit() {
        // VGA controller: base 03, subclass 00, prog-if 00.
        assert_eq!(split_class(0x03_00_00), (0x03, 0x00, 0x00));
        // xHCI USB controller: the prog-if 0x30 is what makes it "USB 3".
        assert_eq!(split_class(0x0c_03_30), (0x0c, 0x03, 0x30));
        // PCI bridge; reading this as 16 bits would yield base class 0x00.
        assert_eq!(split_class(0x06_04_00), (0x06, 0x04, 0x00));
        assert_eq!(split_class(0), (0, 0, 0));
    }

    #[test]
    fn lspci_blocks_are_parsed_with_and_without_modules() {
        let fixture = "\
00:14.2 RAM memory [0500]: Intel Corporation 700 Series Chipset Family Shared SRAM [8086:7a27] (rev 11)
\tSubsystem: ASUSTeK Computer Inc. Device [1043:8882]
01:00.0 VGA compatible controller [0300]: NVIDIA Corporation AD104 [GeForce RTX 4070 SUPER] [10de:2783] (rev a1)
\tSubsystem: ASUSTeK Computer Inc. Device [1043:8974]
\tKernel driver in use: nvidia
\tKernel modules: nouveau, nvidia_drm, nvidia
";
        let parsed = parse_lspci(fixture);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].0, "00:14.2");
        // A device with neither a driver nor candidate modules is normal.
        assert!(parsed[0].1.is_empty());
        assert_eq!(parsed[1].0, "01:00.0");
        assert_eq!(parsed[1].1, ["nouveau", "nvidia_drm", "nvidia"]);
    }

    #[test]
    fn lspci_parser_accepts_full_slots_and_ignores_noise() {
        let fixture = "\
pcilib: sysfs: unable to read something
0000:0a:00.0 Non-Volatile memory controller [0108]: Sandisk Corp WD Blue SN580 [15b7:5041]
\tKernel modules: nvme
";
        let parsed = parse_lspci(fixture);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].0, "0000:0a:00.0");
        assert_eq!(parsed[0].1, ["nvme"]);
    }

    #[test]
    fn slots_are_matched_across_the_domain_prefix() {
        assert_eq!(short_slot("0000:01:00.0"), "01:00.0");
        assert_eq!(short_slot("01:00.0"), "01:00.0");
        assert!(is_slot("00:1f.5"));
        assert!(is_slot("10000:00:00.0"));
        assert!(!is_slot("pcilib:"));
        assert!(!is_slot("Subsystem:"));
        assert!(!is_slot("00:1f.g"));
    }

    #[test]
    fn parent_comes_from_the_sysfs_device_path() {
        // Behind a bridge: the component before the device itself is the parent slot.
        assert_eq!(
            parent_from_link(Path::new(
                "../../../devices/pci0000:00/0000:00:1c.2/0000:07:00.0"
            )),
            Some("0000:00:1c.2".to_owned())
        );
        // Straight on the root complex: the preceding component is `pci0000:00`, which is
        // not a slot, so the device is a root of the tree.
        assert_eq!(
            parent_from_link(Path::new("../../../devices/pci0000:00/0000:00:00.0")),
            None
        );
        // Two bridges deep.
        assert_eq!(
            parent_from_link(Path::new(
                "../../../devices/pci0000:00/0000:00:06.0/0000:02:00.0/0000:03:04.0"
            )),
            Some("0000:02:00.0".to_owned())
        );
        // Degenerate targets must not panic.
        assert_eq!(parent_from_link(Path::new("0000:00:00.0")), None);
        assert_eq!(parent_from_link(Path::new("")), None);
    }
}
