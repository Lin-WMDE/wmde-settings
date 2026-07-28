// SPDX-License-Identifier: GPL-3.0-only

//! Firmware facts: DMI/SMBIOS strings, UEFI vs legacy BIOS, Secure Boot, TPM, microcode.
//!
//! All of it comes from sysfs, so it works without root. The four DMI attributes the
//! kernel keeps at mode 0400 (`board_serial`, `chassis_serial`, `product_serial`,
//! `product_uuid`) are still attempted: `Value::from_path` turns the `EACCES` into
//! `Value::RequiresRoot`, which is the honest thing to show.

use std::path::Path;

use super::util::{read_bytes, read_dir_sorted, read_trim, read_u32};
use super::{Firmware, FirmwareType, SecureBoot, Tpm, Value};

const DMI: &str = "/sys/class/dmi/id";
const EFI: &str = "/sys/firmware/efi";

/// EFI global variable namespace; both variables below live under it.
const SECURE_BOOT: &str =
    "/sys/firmware/efi/efivars/SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c";
const SETUP_MODE: &str = "/sys/firmware/efi/efivars/SetupMode-8be4df61-93ca-11d2-aa0d-00e098032b8c";

pub fn probe() -> Firmware {
    if !Path::new(DMI).exists() {
        // Normal inside containers and on boards without SMBIOS; not an error.
        log::debug!("{DMI} is absent, DMI fields will be unavailable");
    }

    let chassis_type = read_u32(Path::new(DMI).join("chassis_type"))
        .and_then(|value| u8::try_from(value).ok())
        // Bit 7 of the SMBIOS chassis-type byte is the "chassis lock present" flag rather
        // than part of the enumeration. dmi_save_type() already strips it, so this only
        // guards against firmware exported through some other path.
        .map(|value| value & 0x7f);

    let uefi = Path::new(EFI).is_dir();

    Firmware {
        bios_vendor: dmi("bios_vendor"),
        bios_version: dmi("bios_version"),
        bios_date: dmi("bios_date"),
        bios_release: dmi("bios_release"),
        board_vendor: dmi("board_vendor"),
        board_name: dmi("board_name"),
        board_version: dmi("board_version"),
        board_serial: dmi_privileged("board_serial"),
        chassis_vendor: dmi("chassis_vendor"),
        chassis_version: dmi("chassis_version"),
        chassis_serial: dmi_privileged("chassis_serial"),
        chassis_type,
        chassis_type_name: chassis_type.and_then(chassis_type_name),
        product_serial: dmi_privileged("product_serial"),
        product_uuid: dmi_privileged("product_uuid"),
        firmware_type: if uefi {
            FirmwareType::Uefi
        } else {
            FirmwareType::LegacyBios
        },
        efi_bits: read_u32(Path::new(EFI).join("fw_platform_size"))
            .and_then(|bits| u8::try_from(bits).ok()),
        secure_boot: probe_secure_boot(uefi),
        tpm: probe_tpm(),
        cpu_microcode: Value::from_option(
            std::fs::read_to_string("/proc/cpuinfo")
                .ok()
                .as_deref()
                .and_then(parse_microcode),
        ),
    }
}

/// A descriptive DMI string, with OEM placeholders (`To be filled by O.E.M.`) dropped.
fn dmi(attr: &str) -> Value {
    Value::from_dmi(Path::new(DMI).join(attr))
}

/// A mode 0400 DMI string. Read unfiltered so that "denied" survives as `RequiresRoot`.
fn dmi_privileged(attr: &str) -> Value {
    Value::from_path(Path::new(DMI).join(attr))
}

fn probe_secure_boot(uefi: bool) -> SecureBoot {
    if !uefi {
        return SecureBoot::NotUefi;
    }
    decode_secure_boot(
        read_bytes(SECURE_BOOT).as_deref(),
        read_bytes(SETUP_MODE).as_deref(),
    )
}

/// Reads the payload byte of an efivarfs variable.
///
/// efivarfs prepends a 4-byte little-endian EFI attribute mask to every variable, so a
/// one-byte boolean is a 5-byte file and the value lives at index 4, not index 0. Reading
/// index 0 yields the attribute mask (typically 6 = NON_VOLATILE | BOOTSERVICE_ACCESS |
/// RUNTIME_ACCESS) and would report Secure Boot as enabled on every machine.
fn efivar_byte(data: &[u8]) -> Option<u8> {
    data.get(4).copied()
}

fn decode_secure_boot(secure_boot: Option<&[u8]>, setup_mode: Option<&[u8]>) -> SecureBoot {
    let Some(enabled) = secure_boot.and_then(efivar_byte) else {
        // The variable is missing or truncated; firmware is UEFI but says nothing useful.
        return SecureBoot::Unknown;
    };
    if enabled == 1 {
        return SecureBoot::Enabled;
    }
    // Secure Boot off while the platform is in setup mode means "no keys enrolled yet",
    // which is a different thing to tell the user than "turned off in the firmware".
    if setup_mode.and_then(efivar_byte) == Some(1) {
        return SecureBoot::SetupMode;
    }
    SecureBoot::Disabled
}

fn probe_tpm() -> Option<Tpm> {
    // Entries here are symlinks into /sys/devices; joining onto the link path is fine,
    // the kernel resolves it. tpmrm0 (the resource-manager node) lives in a different
    // class, so the first entry is the TPM chip itself.
    read_dir_sorted("/sys/class/tpm")
        .into_iter()
        .find_map(|path| {
            let device = path.file_name()?.to_str()?.to_owned();
            let version_major = read_trim(path.join("tpm_version_major"));
            Some(Tpm {
                device,
                version_major,
            })
        })
}

/// Extracts the first `microcode` line of `/proc/cpuinfo`.
///
/// Absent on non-x86 and inside some VMs, and identical for every core on a sane machine,
/// so only the first occurrence is taken.
fn parse_microcode(cpuinfo: &str) -> Option<String> {
    cpuinfo
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(key, _)| key.trim() == "microcode")
        .map(|(_, value)| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// SMBIOS type 3 (System Enclosure) chassis types, DSP0134 section 7.4.1.
const fn chassis_type_name(code: u8) -> Option<&'static str> {
    Some(match code {
        1 => "Other",
        2 => "Unknown",
        3 => "Desktop",
        4 => "Low Profile Desktop",
        5 => "Pizza Box",
        6 => "Mini Tower",
        7 => "Tower",
        8 => "Portable",
        9 => "Laptop",
        10 => "Notebook",
        11 => "Hand Held",
        12 => "Docking Station",
        13 => "All In One",
        14 => "Sub Notebook",
        15 => "Space-saving",
        16 => "Lunch Box",
        17 => "Main Server Chassis",
        18 => "Expansion Chassis",
        19 => "SubChassis",
        20 => "Bus Expansion Chassis",
        21 => "Peripheral Chassis",
        22 => "RAID Chassis",
        23 => "Rack Mount Chassis",
        24 => "Sealed-case PC",
        25 => "Multi-system Chassis",
        26 => "Compact PCI",
        27 => "Advanced TCA",
        28 => "Blade",
        29 => "Blade Enclosure",
        30 => "Tablet",
        31 => "Convertible",
        32 => "Detachable",
        33 => "IoT Gateway",
        34 => "Embedded PC",
        35 => "Mini PC",
        36 => "Stick PC",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chassis_table_covers_the_smbios_range() {
        assert_eq!(chassis_type_name(3), Some("Desktop"));
        assert_eq!(chassis_type_name(10), Some("Notebook"));
        assert_eq!(chassis_type_name(31), Some("Convertible"));
        assert_eq!(chassis_type_name(36), Some("Stick PC"));
        // Codes outside the spec (and 0, which no chassis ever reports) stay unnamed.
        assert_eq!(chassis_type_name(0), None);
        assert_eq!(chassis_type_name(37), None);
        assert_eq!(chassis_type_name(255), None);
    }

    #[test]
    fn secure_boot_reads_the_byte_after_the_attribute_mask() {
        // Live host layout: attributes 0x00000006, value 0.
        let disabled: &[u8] = &[6, 0, 0, 0, 0];
        let enabled: &[u8] = &[6, 0, 0, 0, 1];

        assert_eq!(
            decode_secure_boot(Some(enabled), Some(disabled)),
            SecureBoot::Enabled
        );
        assert_eq!(
            decode_secure_boot(Some(disabled), Some(disabled)),
            SecureBoot::Disabled
        );
        assert_eq!(
            decode_secure_boot(Some(disabled), Some(enabled)),
            SecureBoot::SetupMode
        );
        // Setup mode is only reported when Secure Boot is actually off.
        assert_eq!(
            decode_secure_boot(Some(enabled), Some(enabled)),
            SecureBoot::Enabled
        );
    }

    #[test]
    fn secure_boot_survives_short_and_missing_buffers() {
        // A buffer with the attribute mask but no payload must not index past the end.
        assert_eq!(
            decode_secure_boot(Some(&[6, 0, 0, 0]), None),
            SecureBoot::Unknown
        );
        assert_eq!(decode_secure_boot(Some(&[]), None), SecureBoot::Unknown);
        assert_eq!(decode_secure_boot(None, None), SecureBoot::Unknown);
        // An unreadable SetupMode leaves a disabled Secure Boot simply disabled.
        assert_eq!(
            decode_secure_boot(Some(&[6, 0, 0, 0, 0]), None),
            SecureBoot::Disabled
        );
    }

    #[test]
    fn microcode_takes_the_first_revision_only() {
        let cpuinfo = "processor\t: 0\nmodel name\t: Test CPU\nmicrocode\t: 0x133\n\
                       processor\t: 1\nmicrocode\t: 0x133\n";
        assert_eq!(parse_microcode(cpuinfo), Some("0x133".to_owned()));
        // ARM /proc/cpuinfo has no microcode line at all.
        assert_eq!(parse_microcode("processor\t: 0\nBogoMIPS\t: 48.00\n"), None);
        assert_eq!(parse_microcode("microcode\t: \n"), None);
    }
}
