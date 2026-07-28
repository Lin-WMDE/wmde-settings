// SPDX-License-Identifier: GPL-3.0-only

//! Small sysfs/procfs reading helpers shared by every probe module.
//!
//! Everything here is deliberately infallible-ish: a missing or unreadable attribute
//! yields `None`, never a panic. sysfs is full of attributes that exist on one machine
//! and not on the next, and of attributes that return `EINVAL` depending on link state.

use std::path::{Path, PathBuf};

/// Reads a file, trims surrounding whitespace, and discards empty results.
///
/// Several sysfs attributes are space padded (`bNumInterfaces` is `" 2"`, SCSI/NVMe
/// `model` is right padded to a fixed width), so trimming is mandatory rather than
/// cosmetic.
pub fn read_trim(path: impl AsRef<Path>) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

/// Parses a sysfs hex attribute, with or without the `0x` prefix.
///
/// PCI attributes use `0x8086`; USB ones use a bare `0b05`.
pub fn read_hex(path: impl AsRef<Path>) -> Option<u32> {
    let s = read_trim(path)?;
    u32::from_str_radix(s.trim_start_matches("0x"), 16).ok()
}

/// Parses a hex attribute into `u16` (vendor/product ids).
pub fn read_hex16(path: impl AsRef<Path>) -> Option<u16> {
    read_hex(path).and_then(|v| u16::try_from(v).ok())
}

/// Parses a hex attribute into `u8` (device class/subclass/protocol).
pub fn read_hex8(path: impl AsRef<Path>) -> Option<u8> {
    read_hex(path).and_then(|v| u8::try_from(v).ok())
}

pub fn read_u64(path: impl AsRef<Path>) -> Option<u64> {
    read_trim(path)?.parse().ok()
}

pub fn read_u32(path: impl AsRef<Path>) -> Option<u32> {
    read_trim(path)?.parse().ok()
}

/// Reads a signed sysfs counter.
///
/// Needed because `/sys/class/net/*/speed` reports `-1` on interfaces that have no
/// meaningful link speed (`virbr0` does), and returns `EINVAL` while the link is down.
pub fn read_i64(path: impl AsRef<Path>) -> Option<i64> {
    read_trim(path)?.parse().ok()
}

/// Reads a `0`/`1` sysfs flag.
pub fn read_bool(path: impl AsRef<Path>) -> Option<bool> {
    match read_trim(path)?.as_str() {
        "0" => Some(false),
        "1" => Some(true),
        _ => None,
    }
}

/// Resolves a symlink and returns the last component of its target.
///
/// This is how `driver` links are turned into a driver name. Note the link is usually
/// *absent* rather than empty when no driver is bound, so `None` is the normal case.
pub fn link_basename(path: impl AsRef<Path>) -> Option<String> {
    std::fs::read_link(path)
        .ok()?
        .file_name()?
        .to_str()
        .map(str::to_owned)
}

/// Resolves a symlink and returns its raw (possibly relative) target.
pub fn link_target(path: impl AsRef<Path>) -> Option<PathBuf> {
    std::fs::read_link(path).ok()
}

/// Lists a directory's entries sorted by file name.
///
/// Sorting matters only so that repeated runs and the copied report are stable.
pub fn read_dir_sorted(path: impl AsRef<Path>) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(path) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    out.sort();
    out
}

/// Reads a sysfs binary attribute in full.
///
/// `stat()` reports size 0 for sysfs binary attributes - `/sys/class/drm/*/edid` is a
/// real 128-byte blob that `metadata().len()` claims is empty - so any pre-sized buffer
/// reads nothing. Always read to end.
pub fn read_bytes(path: impl AsRef<Path>) -> Option<Vec<u8>> {
    std::fs::read(path).ok().filter(|v| !v.is_empty())
}

/// True for DMI strings that are OEM placeholders rather than real values.
///
/// Board vendors ship `System Product Name`, `To be filled by O.E.M.` and
/// `Default string` in DMI on most retail motherboards, so the headline "computer
/// model" row is junk on any self-built machine unless these are filtered out.
pub fn is_placeholder(value: &str) -> bool {
    const JUNK: &[&str] = &[
        "to be filled by o.e.m.",
        "to be filled by oem",
        "system product name",
        "system version",
        "system manufacturer",
        "system name",
        "default string",
        "not applicable",
        "not specified",
        "not available",
        "no enclosure",
        "unknown",
        "none",
        "n/a",
        "na",
        "o.e.m.",
        "oem",
        "empty",
        "chassis manufacture",
        "chassis version",
        "0123456789",
        "x.x",
        "*",
        "-",
    ];
    let lower = value.trim().to_ascii_lowercase();
    lower.is_empty() || JUNK.contains(&lower.as_str())
}

/// Un-escapes the octal sequences the kernel writes into mount tables.
///
/// `/proc/mounts` and `/proc/self/mountinfo` encode space, tab, newline and backslash as
/// `\040`, `\011`, `\012` and `\134`, so a mount point like `/mnt/My Disk` otherwise
/// breaks the whitespace field split.
pub fn unescape_octal(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 3 < bytes.len() {
            let digits = &s[i + 1..i + 4];
            if let Ok(code) = u8::from_str_radix(digits, 8) {
                out.push(code as char);
                i += 4;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// Formats a byte count with decimal (SI) units: what drive vendors print on the box.
pub fn fmt_bytes_dec(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "kB", "MB", "GB", "TB", "PB"];
    fmt_scaled(bytes, 1000.0, UNITS)
}

/// Formats a byte count with binary (IEC) units: what the kernel actually reports.
pub fn fmt_bytes_bin(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    fmt_scaled(bytes, 1024.0, UNITS)
}

fn fmt_scaled(bytes: u64, step: f64, units: &[&str]) -> String {
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= step && unit + 1 < units.len() {
        value /= step;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", units[0])
    } else if value >= 100.0 {
        format!("{value:.0} {}", units[unit])
    } else {
        format!("{value:.1} {}", units[unit])
    }
}

/// Formats a kHz clock as GHz/MHz.
pub fn fmt_khz(khz: u64) -> String {
    if khz >= 1_000_000 {
        format!("{:.2} GHz", khz as f64 / 1_000_000.0)
    } else {
        format!("{} MHz", khz / 1000)
    }
}

/// Splits an uptime in seconds into days/hours/minutes for the caller to localise.
pub fn split_duration(secs: u64) -> (u64, u64, u64) {
    (secs / 86400, (secs % 86400) / 3600, (secs % 3600) / 60)
}

/// Parses a `key=value` file such as `/etc/os-release`, stripping surrounding quotes.
pub fn parse_env_file(contents: &str) -> Vec<(String, String)> {
    contents
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let (key, value) = line.split_once('=')?;
            let value = value.trim().trim_matches('"').trim_matches('\'');
            Some((key.trim().to_owned(), value.to_owned()))
        })
        .collect()
}

/// Reads an environment variable, discarding empty values.
pub fn env_var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn octal_unescape_handles_spaces_and_backslashes() {
        assert_eq!(unescape_octal(r"/mnt/My\040Disk"), "/mnt/My Disk");
        assert_eq!(unescape_octal(r"/a\134b"), r"/a\b");
        assert_eq!(unescape_octal("/plain/path"), "/plain/path");
        // A trailing lone backslash must not panic or eat past the end.
        assert_eq!(unescape_octal(r"/tail\"), r"/tail\");
    }

    #[test]
    fn placeholder_detection_catches_the_common_oem_junk() {
        assert!(is_placeholder("To be filled by O.E.M."));
        assert!(is_placeholder("System Product Name"));
        assert!(is_placeholder("Default string"));
        assert!(is_placeholder("  "));
        assert!(!is_placeholder("TUF GAMING Z790-PRO WIFI"));
    }

    #[test]
    fn byte_formatting_uses_the_right_base() {
        assert_eq!(fmt_bytes_dec(2_000_398_934_016), "2.0 TB");
        assert_eq!(fmt_bytes_bin(134_803_255_296), "126 GiB");
        assert_eq!(fmt_bytes_dec(512), "512 B");
    }

    #[test]
    fn env_file_parsing_strips_quotes() {
        let parsed = parse_env_file("# comment\nNAME=\"Arch Linux\"\nID=arch\n\n");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0], ("NAME".into(), "Arch Linux".into()));
        assert_eq!(parsed[1], ("ID".into(), "arch".into()));
    }
}
