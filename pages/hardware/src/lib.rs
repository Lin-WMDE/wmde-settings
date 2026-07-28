// SPDX-License-Identifier: GPL-3.0-only

//! Hardware and system facts for WMDE Settings.
//!
//! This crate is pure data: no libcosmic, no `fl!`, no localisation. That is what lets it
//! be unit tested without a language loader or a display, and it is why the settings
//! pages can be thin - they only decide how to render what is collected here.
//!
//! It replaced `cosmic-settings/src/pages/system/info.rs`, which shelled out to `lspci`
//! twice and pulled the `sysinfo` crate to answer a subset of the same questions.
//!
//! Every source is readable by an unprivileged user. `dmidecode` is deliberately not
//! used (it needs root); the DMI fields it would give us come from `/sys/class/dmi/id`
//! instead, minus the four serial/UUID attributes the kernel keeps at mode 0400.

pub mod disks;
pub mod firmware;
pub mod graphics;
pub mod hwids;
pub mod network;
pub mod pci;
pub mod summary;
pub mod usb;
pub mod util;

use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};

/// An attribute that may be missing, or present but readable only by root.
///
/// Rendering "requires root" instead of silently dropping the row is the difference
/// between "this machine has no TPM" and "we were not allowed to look".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Value {
    Known(String),
    #[default]
    Unavailable,
    RequiresRoot,
}

impl Value {
    /// Reads a sysfs attribute, distinguishing "absent" from "permission denied".
    pub fn from_path(path: impl AsRef<Path>) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    Self::Unavailable
                } else {
                    Self::Known(trimmed.to_owned())
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => Self::RequiresRoot,
            Err(_) => Self::Unavailable,
        }
    }

    /// Like [`Value::from_path`], but also rejects OEM placeholder strings such as
    /// `To be filled by O.E.M.`.
    pub fn from_dmi(path: impl AsRef<Path>) -> Self {
        match Self::from_path(path) {
            Self::Known(value) if util::is_placeholder(&value) => Self::Unavailable,
            other => other,
        }
    }

    pub fn known(value: impl Into<String>) -> Self {
        let value = value.into();
        if value.trim().is_empty() {
            Self::Unavailable
        } else {
            Self::Known(value)
        }
    }

    pub fn from_option(value: Option<impl Into<String>>) -> Self {
        value.map_or(Self::Unavailable, Self::known)
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Known(value) => Some(value),
            _ => None,
        }
    }

    /// First known value out of several candidates.
    pub fn first_known(candidates: impl IntoIterator<Item = Self>) -> Self {
        let mut fallback = Self::Unavailable;
        for candidate in candidates {
            match candidate {
                Self::Known(_) => return candidate,
                // Remember that something was root-gated, in case nothing better shows up.
                Self::RequiresRoot => fallback = Self::RequiresRoot,
                Self::Unavailable => {}
            }
        }
        fallback
    }
}

/// Everything the app knows about the machine.
#[derive(Debug, Clone, Default)]
pub struct SysInfo {
    pub summary: Summary,
    pub usb: Vec<UsbDevice>,
    pub graphics: Graphics,
    pub pci: Vec<PciDevice>,
    pub firmware: Firmware,
    pub disks: Vec<Disk>,
    pub network: Vec<Interface>,
    /// True when `/usr/share/hwdata` was missing, so device names are raw hex ids.
    pub hwids_missing: bool,
}

impl SysInfo {
    /// Collects everything. Blocking file I/O throughout - never call this on the UI
    /// thread; the settings pages hand it to `tokio::task::spawn_blocking`.
    pub fn collect() -> Self {
        let pci_raw = pci::scan();
        let usb_raw = usb::scan();

        // Two-phase hwdata resolution: gather the ~40 ids we actually care about first,
        // then stream pci.ids/usb.ids once and allocate only for those. A full parse
        // would build ~68k Strings to answer 40 questions.
        let mut wanted = hwids::Wanted::default();
        pci::collect_wanted(&pci_raw, &mut wanted);
        usb::collect_wanted(&usb_raw, &mut wanted);
        let ids = hwids::HwIds::load(&wanted);

        let pci = pci::resolve(pci_raw, &ids);
        let usb = usb::resolve(usb_raw, &ids);
        let graphics = graphics::probe(&pci, &ids);
        let firmware = firmware::probe();
        let disks = disks::probe();
        let network = network::probe();
        let summary = summary::probe(&graphics, &disks);

        Self {
            summary,
            usb,
            graphics,
            pci,
            firmware,
            disks,
            network,
            hwids_missing: !ids.available(),
        }
    }
}

// ---------------------------------------------------------------- summary

#[derive(Debug, Clone, Default)]
pub struct Summary {
    pub vendor: Value,
    pub model: Value,
    pub board: Value,
    pub os_name: Value,
    pub os_build: Value,
    pub kernel: Value,
    pub architecture: Value,
    pub hostname: Value,
    pub uptime_secs: Option<u64>,
    pub cpu: Cpu,
    pub mem_total_bytes: Option<u64>,
    pub swap_total_bytes: Option<u64>,
    pub gpu_names: Vec<String>,
    pub disk_total_bytes: u64,
    pub disk_count: usize,
    pub desktop: Value,
    pub desktop_version: &'static str,
    pub display_server: DisplayServer,
}

impl Summary {
    /// Vendor plus model, as a human would name the machine.
    ///
    /// `product_name` is OEM junk on most retail boards and gets filtered to
    /// `Unavailable`, so the board name is the fallback. The vendor is dropped when the
    /// model already starts with it ("ASUSTeK COMPUTER INC." / "TUF GAMING Z790-PRO").
    pub fn machine(&self) -> Option<String> {
        let model = Value::first_known([self.model.clone(), self.board.clone()]);
        match (self.vendor.as_str(), model.as_str()) {
            (Some(vendor), Some(model)) => {
                let head = vendor
                    .split_whitespace()
                    .next()
                    .unwrap_or(vendor)
                    .trim_matches(|c: char| !c.is_alphanumeric())
                    .to_lowercase();
                if model.to_lowercase().contains(&head) {
                    Some(model.to_owned())
                } else {
                    Some(format!("{vendor} {model}"))
                }
            }
            (None, Some(model)) => Some(model.to_owned()),
            (Some(vendor), None) => Some(vendor.to_owned()),
            (None, None) => None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Cpu {
    pub model: Value,
    pub vendor: Value,
    pub physical_cores: Option<u32>,
    pub logical_threads: Option<u32>,
    pub sockets: Option<u32>,
    pub max_freq_khz: Option<u64>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DisplayServer {
    Wayland,
    X11,
    Tty,
    #[default]
    Unknown,
}

impl DisplayServer {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Wayland => "Wayland",
            Self::X11 => "X11",
            Self::Tty => "TTY",
            Self::Unknown => "unknown",
        }
    }
}

// ---------------------------------------------------------------- usb

/// A USB device as read from sysfs, before hwdata name resolution.
#[derive(Debug, Clone, Default)]
pub struct UsbRaw {
    pub sysfs_name: String,
    /// sysfs name of the hub this device hangs off; `None` for a root hub.
    pub parent: Option<String>,
    pub bus: u32,
    pub device: u32,
    pub depth: usize,
    pub vendor_id: u16,
    pub product_id: u16,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    pub serial: Option<String>,
    pub speed_mbps: Option<f32>,
    pub usb_version: Option<String>,
    pub class: u8,
    pub subclass: u8,
    pub protocol: u8,
    pub interfaces: Vec<UsbInterface>,
}

#[derive(Debug, Clone)]
pub struct UsbDevice {
    pub raw: UsbRaw,
    /// hwdata vendor name, used when sysfs has no `manufacturer` string.
    pub vendor_name: Option<String>,
    /// hwdata product name, used when sysfs has no `product` string.
    pub product_name: Option<String>,
    pub class_name: Option<String>,
}

impl UsbDevice {
    /// Best available human name for the device.
    pub fn title(&self) -> String {
        let vendor = self
            .raw
            .manufacturer
            .clone()
            .or_else(|| self.vendor_name.clone());
        let product = self
            .raw
            .product
            .clone()
            .or_else(|| self.product_name.clone());
        match (vendor, product) {
            (Some(v), Some(p)) => format!("{v} {p}"),
            (Some(v), None) => v,
            (None, Some(p)) => p,
            (None, None) => format!("{:04x}:{:04x}", self.raw.vendor_id, self.raw.product_id),
        }
    }

    pub fn id_string(&self) -> String {
        format!("{:04x}:{:04x}", self.raw.vendor_id, self.raw.product_id)
    }

    /// Human label for the negotiated link speed.
    pub fn speed_label(&self) -> Option<String> {
        let mbps = self.raw.speed_mbps?;
        let label = match mbps {
            m if (m - 1.5).abs() < 0.01 => "1.5 Mbps (USB 1.0 low speed)".to_owned(),
            m if (m - 12.0).abs() < 0.01 => "12 Mbps (USB 1.1 full speed)".to_owned(),
            m if (m - 480.0).abs() < 0.01 => "480 Mbps (USB 2.0 high speed)".to_owned(),
            m if (m - 5000.0).abs() < 0.01 => "5 Gbps (USB 3.0)".to_owned(),
            m if (m - 10000.0).abs() < 0.01 => "10 Gbps (USB 3.1 Gen 2)".to_owned(),
            m if (m - 20000.0).abs() < 0.01 => "20 Gbps (USB 3.2 Gen 2x2)".to_owned(),
            m if (m - 40000.0).abs() < 0.01 => "40 Gbps (USB4)".to_owned(),
            m => format!("{m} Mbps"),
        };
        Some(label)
    }
}

#[derive(Debug, Clone, Default)]
pub struct UsbInterface {
    pub number: u8,
    pub class: u8,
    pub subclass: u8,
    pub protocol: u8,
    pub driver: Option<String>,
    pub class_name: Option<String>,
}

// ---------------------------------------------------------------- pci

/// A PCI device as read from sysfs, before hwdata name resolution.
#[derive(Debug, Clone, Default)]
pub struct PciRaw {
    pub slot: String,
    /// Slot of the bridge this device sits behind; `None` for a root-complex device.
    pub parent: Option<String>,
    pub class_base: u8,
    pub class_sub: u8,
    pub class_progif: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    pub subsystem_vendor_id: Option<u16>,
    pub subsystem_device_id: Option<u16>,
    pub revision: u8,
    pub driver: Option<String>,
    pub modules: Vec<String>,
    pub irq: Option<u32>,
    pub numa_node: Option<i32>,
    /// DRM nodes bound to this device, e.g. `["card1", "renderD128"]`.
    pub drm_nodes: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct PciDevice {
    pub raw: PciRaw,
    pub vendor_name: Option<String>,
    pub device_name: Option<String>,
    pub subsystem_name: Option<String>,
    pub class_name: Option<String>,
}

impl PciDevice {
    pub fn title(&self) -> String {
        match (&self.vendor_name, &self.device_name) {
            (Some(v), Some(d)) => format!("{v} {d}"),
            (Some(v), None) => format!("{v} {}", self.id_string()),
            (None, Some(d)) => d.clone(),
            (None, None) => self.id_string(),
        }
    }

    pub fn id_string(&self) -> String {
        format!("{:04x}:{:04x}", self.raw.vendor_id, self.raw.device_id)
    }
}

// ---------------------------------------------------------------- graphics

#[derive(Debug, Clone, Default)]
pub struct Graphics {
    pub gpus: Vec<Gpu>,
    pub displays: Vec<Display>,
}

#[derive(Debug, Clone)]
pub struct Gpu {
    pub slot: String,
    pub vendor_name: Option<String>,
    pub device_name: Option<String>,
    pub subsystem_name: Option<String>,
    pub id_string: String,
    pub revision: u8,
    pub driver: Option<String>,
    /// From `/sys/module/<driver>/version`; absent for in-tree DRM drivers (i915, amdgpu).
    pub driver_version: Option<String>,
    pub drm_nodes: Vec<String>,
}

impl Gpu {
    pub fn title(&self) -> String {
        match (&self.vendor_name, &self.device_name) {
            (Some(v), Some(d)) => format!("{v} {d}"),
            (Some(v), None) => v.clone(),
            (None, Some(d)) => d.clone(),
            (None, None) => self.id_string.clone(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Display {
    /// Connector name with the `card<N>-` prefix stripped, e.g. `DP-1`.
    pub connector: String,
    pub connected: bool,
    pub make: Option<String>,
    pub model: Option<String>,
    pub serial: Option<String>,
    pub year: Option<u32>,
    pub physical_mm: Option<(u32, u32)>,
    /// First entry of the connector's `modes` attribute: the preferred mode.
    pub preferred_mode: Option<String>,
    pub mode_count: usize,
}

// ---------------------------------------------------------------- firmware

#[derive(Debug, Clone, Default)]
pub struct Firmware {
    pub bios_vendor: Value,
    pub bios_version: Value,
    pub bios_date: Value,
    pub bios_release: Value,
    pub board_vendor: Value,
    pub board_name: Value,
    pub board_version: Value,
    pub board_serial: Value,
    pub chassis_vendor: Value,
    pub chassis_version: Value,
    pub chassis_serial: Value,
    pub chassis_type: Option<u8>,
    pub chassis_type_name: Option<&'static str>,
    pub product_serial: Value,
    pub product_uuid: Value,
    pub firmware_type: FirmwareType,
    pub efi_bits: Option<u8>,
    pub secure_boot: SecureBoot,
    pub tpm: Option<Tpm>,
    pub cpu_microcode: Value,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FirmwareType {
    Uefi,
    LegacyBios,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SecureBoot {
    Enabled,
    Disabled,
    SetupMode,
    NotUefi,
    #[default]
    Unknown,
}

#[derive(Debug, Clone)]
pub struct Tpm {
    pub device: String,
    pub version_major: Option<String>,
}

// ---------------------------------------------------------------- disks

#[derive(Debug, Clone, Default)]
pub struct Disk {
    pub name: String,
    pub model: Option<String>,
    pub serial: Option<String>,
    pub firmware: Option<String>,
    pub size_bytes: u64,
    pub kind: DiskKind,
    pub read_only: bool,
    pub logical_block_size: Option<u32>,
    pub partitions: Vec<Partition>,
}

impl Disk {
    pub fn title(&self) -> String {
        match &self.model {
            Some(model) => format!("{} ({})", model, self.name),
            None => self.name.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DiskKind {
    Nvme,
    Ssd,
    Hdd,
    Removable,
    #[default]
    Unknown,
}

impl DiskKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Nvme => "NVMe",
            Self::Ssd => "SSD",
            Self::Hdd => "HDD",
            Self::Removable => "removable",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Partition {
    pub name: String,
    pub size_bytes: u64,
    pub fstype: Option<String>,
    /// A partition can be mounted in several places (bind mounts, btrfs subvolumes).
    pub mountpoints: Vec<PathBuf>,
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, Copy)]
pub struct Usage {
    pub total: u64,
    pub used: u64,
    pub available: u64,
}

impl Usage {
    pub fn percent(self) -> u32 {
        if self.total == 0 {
            return 0;
        }
        ((self.used as f64 / self.total as f64) * 100.0).round() as u32
    }
}

// ---------------------------------------------------------------- network

#[derive(Debug, Clone, Default)]
pub struct Interface {
    pub name: String,
    pub kind: IfKind,
    pub mac: Option<String>,
    pub state: IfState,
    pub speed_mbps: Option<u32>,
    pub duplex: Option<String>,
    pub mtu: Option<u32>,
    pub driver: Option<String>,
    pub ipv4: Vec<Ipv4Addr>,
    pub ipv6: Vec<Ipv6Addr>,
}

impl Interface {
    /// Physical interfaces (and loopback) are shown by default; the rest - veth, docker
    /// bridges, vboxnet, virbr - hide behind a toggle, because a developer machine
    /// routinely has 40+ of them and they bury the useful rows.
    pub fn is_virtual(&self) -> bool {
        matches!(self.kind, IfKind::Bridge | IfKind::Virtual | IfKind::Tunnel)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum IfKind {
    Loopback,
    Wired,
    Wireless,
    Bridge,
    Virtual,
    Tunnel,
    #[default]
    Other,
}

impl IfKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Loopback => "loopback",
            Self::Wired => "wired",
            Self::Wireless => "wireless",
            Self::Bridge => "bridge",
            Self::Virtual => "virtual",
            Self::Tunnel => "tunnel",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum IfState {
    Up,
    Down,
    Dormant,
    #[default]
    Unknown,
}

impl IfState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Down => "down",
            Self::Dormant => "dormant",
            Self::Unknown => "unknown",
        }
    }
}
