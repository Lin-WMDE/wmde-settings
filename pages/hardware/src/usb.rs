// SPDX-License-Identifier: GPL-3.0-only

//! USB topology from `/sys/bus/usb/devices`.
//!
//! The kernel exposes devices and their interfaces side by side in one flat directory,
//! and encodes the whole tree in the directory *name*. Everything here is name parsing
//! plus reading a handful of optional attributes; nothing needs root and nothing needs
//! libusb.

use std::path::Path;

use super::hwids::{HwIds, Wanted};
use super::util::{link_basename, read_dir_sorted, read_hex8, read_hex16, read_trim, read_u32};
use super::{UsbDevice, UsbInterface, UsbRaw};

const USB_ROOT: &str = "/sys/bus/usb/devices";

/// What a directory name in `/sys/bus/usb/devices` denotes.
#[derive(Debug, PartialEq, Eq)]
enum NameKind {
    /// `usb1` - the bus's root hub.
    RootHub {
        bus: u32,
    },
    /// `1-2`, `1-6.1` - a device, with its port path relative to the root hub.
    Device {
        bus: u32,
        ports: Vec<u32>,
    },
    /// `1-2:1.0` - an interface of the device named before the colon.
    Interface {
        device: String,
    },
    Unknown,
}

/// Classifies a `/sys/bus/usb/devices` entry name.
///
/// The colon test has to come first: an interface name starts with a device name, so
/// `1-2:1.0` would otherwise parse as the device `1-2`.
fn classify(name: &str) -> NameKind {
    if let Some((device, _)) = name.split_once(':') {
        return NameKind::Interface {
            device: canonical_device_name(device),
        };
    }
    if let Some(bus) = name.strip_prefix("usb") {
        return match bus.parse() {
            Ok(bus) => NameKind::RootHub { bus },
            Err(_) => NameKind::Unknown,
        };
    }
    let Some((bus, ports)) = name.split_once('-') else {
        return NameKind::Unknown;
    };
    let Ok(bus) = bus.parse::<u32>() else {
        return NameKind::Unknown;
    };
    let mut parsed = Vec::new();
    for segment in ports.split('.') {
        match segment.parse::<u32>() {
            Ok(port) => parsed.push(port),
            Err(_) => return NameKind::Unknown,
        }
    }
    NameKind::Device { bus, ports: parsed }
}

/// Maps the name in front of an interface's colon to the directory that really exists.
///
/// A root hub's own interface is called `1-0:1.0`, but there is no `1-0` directory: the
/// root hub is `usb1`. Every other prefix is already a real device directory.
fn canonical_device_name(name: &str) -> String {
    match name.split_once('-') {
        Some((bus, "0")) => format!("usb{bus}"),
        _ => name.to_owned(),
    }
}

/// Enumerates every USB device, sorted so the list reads like the bus tree.
pub fn scan() -> Vec<UsbRaw> {
    let mut devices: Vec<(u32, Vec<u32>, UsbRaw)> = Vec::new();
    let mut interfaces: Vec<(String, UsbInterface)> = Vec::new();

    for path in read_dir_sorted(USB_ROOT) {
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        match classify(name) {
            NameKind::Interface { device } => {
                if let Some(interface) = read_interface(&path) {
                    interfaces.push((device, interface));
                }
            }
            NameKind::RootHub { bus } => {
                // A root hub hangs off no port at all, so its port path is empty and it
                // sorts ahead of every device on the same bus.
                devices.push((bus, Vec::new(), read_device(&path, name, bus, 0, None)));
            }
            NameKind::Device { bus, ports } => {
                let depth = ports.len().saturating_sub(1);
                let raw = read_device(&path, name, bus, depth, parent_name(bus, &ports));
                devices.push((bus, ports, raw));
            }
            NameKind::Unknown => {}
        }
    }

    for (device_name, interface) in interfaces {
        if let Some((_, _, raw)) = devices
            .iter_mut()
            .find(|(_, _, r)| r.sysfs_name == device_name)
        {
            raw.interfaces.push(interface);
        }
    }
    for (_, _, raw) in &mut devices {
        raw.interfaces.sort_by_key(|i| i.number);
    }

    // `Vec<u32>` compares element by element, which is exactly the numeric,
    // segment-by-segment order we want: `1-2` < `1-6` < `1-6.1` < `1-13`. Sorting the
    // names as strings would put `1-13` before `1-2`.
    devices.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
    devices.into_iter().map(|(_, _, raw)| raw).collect()
}

/// sysfs name of the hub a device is plugged into.
///
/// The topology is in the name, so nothing has to be read: `1-6.1` hangs off `1-6`, and a
/// device straight on a bus (`1-2`) hangs off that bus's root hub, whose directory is
/// `usb1` and not `1-0`.
fn parent_name(bus: u32, ports: &[u32]) -> Option<String> {
    match ports.len() {
        0 => None,
        1 => Some(format!("usb{bus}")),
        _ => Some(format!(
            "{bus}-{}",
            ports[..ports.len() - 1]
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(".")
        )),
    }
}

fn read_device(
    dir: &Path,
    name: &str,
    bus_from_name: u32,
    depth: usize,
    parent: Option<String>,
) -> UsbRaw {
    // `version` is `" 2.00"` and `bNumInterfaces` is `" 2"` - the kernel space pads both.
    // `read_trim` already deals with that; do not "optimise" it into a raw read.
    let usb_version = read_trim(dir.join("version"));

    UsbRaw {
        sysfs_name: name.to_owned(),
        parent,
        bus: read_u32(dir.join("busnum")).unwrap_or(bus_from_name),
        device: read_u32(dir.join("devnum")).unwrap_or_default(),
        depth,
        // USB ids carry no `0x` prefix, unlike their PCI counterparts; `read_hex16`
        // accepts both forms.
        vendor_id: read_hex16(dir.join("idVendor")).unwrap_or_default(),
        product_id: read_hex16(dir.join("idProduct")).unwrap_or_default(),
        manufacturer: read_trim(dir.join("manufacturer")),
        product: read_trim(dir.join("product")),
        serial: read_trim(dir.join("serial")),
        // A decimal Mbps string: 1.5, 12, 480, 5000, 20000 - hence f32 and not an integer.
        speed_mbps: read_trim(dir.join("speed")).and_then(|s| s.parse().ok()),
        usb_version,
        class: read_hex8(dir.join("bDeviceClass")).unwrap_or_default(),
        subclass: read_hex8(dir.join("bDeviceSubClass")).unwrap_or_default(),
        protocol: read_hex8(dir.join("bDeviceProtocol")).unwrap_or_default(),
        interfaces: Vec::new(),
    }
}

fn read_interface(dir: &Path) -> Option<UsbInterface> {
    Some(UsbInterface {
        number: read_hex8(dir.join("bInterfaceNumber"))?,
        class: read_hex8(dir.join("bInterfaceClass")).unwrap_or_default(),
        subclass: read_hex8(dir.join("bInterfaceSubClass")).unwrap_or_default(),
        protocol: read_hex8(dir.join("bInterfaceProtocol")).unwrap_or_default(),
        // With no driver bound the symlink is absent rather than empty, so `None` here is
        // the ordinary case for e.g. a vendor-specific interface nothing claims.
        driver: link_basename(dir.join("driver")),
        class_name: None,
    })
}

/// Registers the hwdata lookups this device list will need.
pub fn collect_wanted(raw: &[UsbRaw], wanted: &mut Wanted) {
    for device in raw {
        wanted.usb_vendors.insert(device.vendor_id);
        wanted
            .usb_devices
            .insert((device.vendor_id, device.product_id));
    }
}

/// Attaches hwdata names to the scanned devices.
pub fn resolve(raw: Vec<UsbRaw>, ids: &HwIds) -> Vec<UsbDevice> {
    raw.into_iter()
        .map(|mut raw| {
            let vendor_name = ids.usb_vendor(raw.vendor_id).map(Into::into);
            let product_name = ids
                .usb_device(raw.vendor_id, raw.product_id)
                .map(Into::into);
            let class_name = ids.usb_class(raw.class, raw.subclass, raw.protocol);
            for interface in &mut raw.interfaces {
                interface.class_name =
                    ids.usb_class(interface.class, interface.subclass, interface.protocol);
            }
            UsbDevice {
                raw,
                vendor_name,
                product_name,
                class_name,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interfaces_are_told_apart_from_devices() {
        assert_eq!(
            classify("1-2:1.0"),
            NameKind::Interface {
                device: "1-2".to_owned()
            }
        );
        assert_eq!(
            classify("1-6.1:1.2"),
            NameKind::Interface {
                device: "1-6.1".to_owned()
            }
        );
        // The root hub's interface points at a directory named `usb1`, not `1-0`.
        assert_eq!(
            classify("1-0:1.0"),
            NameKind::Interface {
                device: "usb1".to_owned()
            }
        );
        assert_eq!(classify("usb2"), NameKind::RootHub { bus: 2 });
        assert_eq!(
            classify("1-2"),
            NameKind::Device {
                bus: 1,
                ports: vec![2]
            }
        );
        assert_eq!(classify("usbmisc"), NameKind::Unknown);
        assert_eq!(classify("1-a"), NameKind::Unknown);
        assert_eq!(classify("garbage"), NameKind::Unknown);
    }

    #[test]
    fn port_path_gives_depth_and_parent() {
        let depth = |name: &str| match classify(name) {
            NameKind::Device { ports, .. } => ports.len() - 1,
            _ => panic!("not a device"),
        };
        assert_eq!(depth("1-2"), 0);
        assert_eq!(depth("1-6.1"), 1);
        assert_eq!(depth("1-6.1.3"), 2);

        // `1-6.1` is a child of `1-6`: the parent is the port path minus its last segment.
        let NameKind::Device { bus, ports } = classify("1-6.1") else {
            panic!("not a device");
        };
        assert_eq!((bus, ports.as_slice()), (1, [6, 1].as_slice()));
        assert_eq!(&ports[..ports.len() - 1], [6]);
    }

    #[test]
    fn tree_order_is_numeric_not_lexicographic() {
        let mut keys: Vec<(u32, Vec<u32>)> =
            ["1-13", "1-2", "1-6.2", "usb2", "1-6", "1-6.1", "usb1"]
                .iter()
                .map(|name| match classify(name) {
                    NameKind::Device { bus, ports } => (bus, ports),
                    NameKind::RootHub { bus } => (bus, Vec::new()),
                    other => panic!("unexpected {other:?}"),
                })
                .collect();
        keys.sort();
        let ports: Vec<&[u32]> = keys.iter().map(|(_, p)| p.as_slice()).collect();
        assert_eq!(
            keys.iter().map(|(bus, _)| *bus).collect::<Vec<_>>(),
            [1, 1, 1, 1, 1, 1, 2]
        );
        // Root hub first, then port 2, then the 1-6 subtree, and `1-13` last - a string
        // sort would have put it second.
        assert_eq!(
            ports,
            [
                [].as_slice(),
                [2].as_slice(),
                [6].as_slice(),
                [6, 1].as_slice(),
                [6, 2].as_slice(),
                [13].as_slice(),
                [].as_slice()
            ]
        );
    }

    #[test]
    fn parent_name_walks_up_the_port_path() {
        // A device straight on the bus hangs off the root hub, whose directory is `usbN`.
        assert_eq!(parent_name(1, &[2]), Some("usb1".to_owned()));
        // Behind a hub: drop the last port segment.
        assert_eq!(parent_name(1, &[6, 1]), Some("1-6".to_owned()));
        assert_eq!(parent_name(2, &[8, 3, 4]), Some("2-8.3".to_owned()));
        // The root hub itself has no ports and no parent.
        assert_eq!(parent_name(1, &[]), None);
    }
}
