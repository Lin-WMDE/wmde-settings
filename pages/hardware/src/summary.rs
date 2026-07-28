// SPDX-License-Identifier: GPL-3.0-only

//! The headline "what is this machine" block: identity, OS, CPU, memory, session.
//!
//! Everything here comes from `/sys/class/dmi/id`, `/proc` and the environment, so it
//! works unprivileged. GPU and disk facts are not re-probed - they are passed in from
//! the modules that already did the work.

use std::collections::BTreeSet;
use std::path::Path;

use super::util::{env_var, parse_env_file, read_dir_sorted, read_i64, read_trim, read_u64};
use super::{Cpu, Disk, DisplayServer, Graphics, Summary, Value};

const DMI: &str = "/sys/class/dmi/id";
const CPU_ROOT: &str = "/sys/devices/system/cpu";

pub fn probe(graphics: &Graphics, disks: &[Disk]) -> Summary {
    let dmi = Path::new(DMI);

    let board_vendor = Value::from_dmi(dmi.join("board_vendor"));
    let board_name = Value::from_dmi(dmi.join("board_name"));
    let board = match (board_vendor.as_str(), board_name.as_str()) {
        (Some(vendor), Some(name)) => Value::known(format!("{vendor} {name}")),
        (Some(vendor), None) => Value::known(vendor),
        (None, Some(name)) => Value::known(name),
        // Keeps a RequiresRoot from either half instead of flattening it to Unavailable.
        (None, None) => Value::first_known([board_vendor.clone(), board_name.clone()]),
    };

    let os = os_release();
    let (mem_total_bytes, swap_total_bytes) =
        parse_meminfo(&read_trim("/proc/meminfo").unwrap_or_default());

    Summary {
        // product_name / product_version are OEM junk on most retail boards
        // ("System Product Name", "To be filled by O.E.M."), hence from_dmi: the UI
        // falls back to `board` when `model` comes back Unavailable.
        vendor: Value::from_dmi(dmi.join("sys_vendor")),
        model: Value::from_dmi(dmi.join("product_name")),
        board,
        os_name: Value::first_known([
            os_field(&os, "PRETTY_NAME"),
            os_field(&os, "NAME"),
            os_field(&os, "ID"),
        ]),
        os_build: os_field(&os, "BUILD_ID"),
        kernel: Value::from_path("/proc/sys/kernel/osrelease"),
        architecture: Value::first_known([
            Value::from_option(uname_machine()),
            Value::known(std::env::consts::ARCH),
        ]),
        hostname: Value::from_path("/proc/sys/kernel/hostname"),
        uptime_secs: parse_uptime(&read_trim("/proc/uptime").unwrap_or_default()),
        cpu: cpu(),
        mem_total_bytes,
        swap_total_bytes,
        gpu_names: graphics.gpus.iter().map(super::Gpu::title).collect(),
        disk_total_bytes: disks
            .iter()
            .fold(0u64, |acc, disk| acc.saturating_add(disk.size_bytes)),
        disk_count: disks.len(),
        desktop: Value::first_known([
            Value::from_option(env_var("XDG_CURRENT_DESKTOP")),
            Value::from_option(env_var("DESKTOP_SESSION")),
        ]),
        // The WMDE stack exposes no runtime version anywhere, so this app's own version
        // stands in for the desktop's. Deliberately not asking pacman.
        desktop_version: env!("CARGO_PKG_VERSION"),
        display_server: display_server(),
    }
}

// ---------------------------------------------------------------- os-release

fn os_release() -> Vec<(String, String)> {
    // /etc/os-release is normally a symlink to the /usr/lib copy, but only the latter
    // exists on a stateless or minimal-/etc system.
    let contents = read_trim("/etc/os-release")
        .or_else(|| read_trim("/usr/lib/os-release"))
        .unwrap_or_default();
    parse_env_file(&contents)
}

fn os_field(pairs: &[(String, String)], key: &str) -> Value {
    Value::from_option(
        pairs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str()),
    )
}

// ---------------------------------------------------------------- cpu

fn cpu() -> Cpu {
    let text = parse_cpuinfo(&read_trim("/proc/cpuinfo").unwrap_or_default());
    let cpus = present_cpus();
    let (physical_cores, sockets) = topology(&cpus);

    Cpu {
        model: Value::from_option(text.model),
        vendor: Value::from_option(text.vendor),
        physical_cores,
        logical_threads: (text.threads > 0).then_some(text.threads),
        sockets,
        max_freq_khz: max_freq_khz(&cpus),
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct CpuText {
    model: Option<String>,
    vendor: Option<String>,
    threads: u32,
}

/// Pulls the first occurrence of each interesting key out of `/proc/cpuinfo`.
///
/// The per-CPU blocks repeat the same model and vendor, so only the first one is kept;
/// `processor` lines are what we count instead. The microcode revision is not read here -
/// it belongs to the firmware page and `probe::firmware` owns it.
fn parse_cpuinfo(contents: &str) -> CpuText {
    let mut out = CpuText::default();
    for line in contents.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        let slot = match key.trim() {
            "processor" => {
                out.threads += 1;
                continue;
            }
            "model name" => &mut out.model,
            "vendor_id" => &mut out.vendor,
            _ => continue,
        };
        if slot.is_none() && !value.is_empty() {
            *slot = Some(value.to_owned());
        }
    }
    out
}

/// Expands the kernel's CPU range-list syntax (`0-31`, `0-3,8-11`, `5`).
fn parse_cpu_list(list: &str) -> Vec<u32> {
    let mut out = Vec::new();
    for part in list.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        match part.split_once('-') {
            Some((lo, hi)) => {
                let (Ok(lo), Ok(hi)) = (lo.trim().parse::<u32>(), hi.trim().parse::<u32>()) else {
                    continue;
                };
                // A corrupt or hostile range must not turn into a multi-gigabyte Vec;
                // the kernel's own CONFIG_NR_CPUS ceiling is far below this.
                if lo > hi || hi - lo > 65_535 {
                    continue;
                }
                out.extend(lo..=hi);
            }
            None => {
                if let Ok(n) = part.parse::<u32>() {
                    out.push(n);
                }
            }
        }
    }
    out
}

fn present_cpus() -> Vec<u32> {
    if let Some(list) = read_trim(Path::new(CPU_ROOT).join("present")) {
        let cpus = parse_cpu_list(&list);
        if !cpus.is_empty() {
            return cpus;
        }
    }
    // Fallback for the odd kernel/container where `present` is missing: the cpuN
    // directories themselves. "cpufreq" and "cpuidle" survive the prefix strip but fail
    // to parse, so they drop out here.
    read_dir_sorted(CPU_ROOT)
        .iter()
        .filter_map(|path| {
            path.file_name()?
                .to_str()?
                .strip_prefix("cpu")?
                .parse::<u32>()
                .ok()
        })
        .collect()
}

/// Counts physical cores and sockets from sysfs topology.
///
/// `cpu cores` in /proc/cpuinfo is deliberately not used: it is a per-package number
/// that cannot describe a hybrid part (a 14900KF is 8 P-cores + 16 E-cores = 24 cores /
/// 32 threads, and every block still says `cpu cores: 24` only by coincidence of the
/// package layout). Distinct (physical_package_id, core_id) pairs are the real answer.
fn topology(cpus: &[u32]) -> (Option<u32>, Option<u32>) {
    let root = Path::new(CPU_ROOT);
    let mut cores = BTreeSet::new();
    let mut packages = BTreeSet::new();
    for n in cpus {
        let dir = root.join(format!("cpu{n}")).join("topology");
        // Offline CPUs are listed in `present` but expose no topology directory; they
        // must not be counted as cores of their own.
        let (Some(package), Some(core)) = (
            read_i64(dir.join("physical_package_id")),
            read_i64(dir.join("core_id")),
        ) else {
            continue;
        };
        packages.insert(package);
        cores.insert((package, core));
    }
    (count(cores.len()), count(packages.len()))
}

fn count(len: usize) -> Option<u32> {
    u32::try_from(len).ok().filter(|&n| n > 0)
}

/// Nominal maximum clock, in kHz.
///
/// `cpufreq/` is absent entirely under VMs and with some drivers (intel_idle without a
/// scaling driver, most virtio guests), so this is an Option rather than a failure. cpu0
/// is checked first and the rest only as a fallback, because cpu0 can be offline.
fn max_freq_khz(cpus: &[u32]) -> Option<u64> {
    let root = Path::new(CPU_ROOT);
    let path = |n: u32| root.join(format!("cpu{n}/cpufreq/cpuinfo_max_freq"));
    read_u64(path(0)).or_else(|| cpus.iter().find_map(|&n| read_u64(path(n))))
}

// ---------------------------------------------------------------- misc

/// `uname().machine`, e.g. `x86_64`.
fn uname_machine() -> Option<String> {
    // SAFETY: uname() writes only into the caller-provided utsname, which is zeroed
    // first so that every field is NUL terminated even on a partial failure.
    let mut buf: libc::utsname = unsafe { std::mem::zeroed() };
    if unsafe { libc::uname(&mut buf) } != 0 {
        return None;
    }
    // The fields are fixed [c_char; 65] buffers, not Rust strings: stop at the NUL.
    let bytes: Vec<u8> = buf
        .machine
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c as u8)
        .collect();
    let machine = String::from_utf8_lossy(&bytes).into_owned();
    (!machine.is_empty()).then_some(machine)
}

fn display_server() -> DisplayServer {
    if let Some(kind) = env_var("XDG_SESSION_TYPE") {
        match kind.to_ascii_lowercase().as_str() {
            "wayland" => return DisplayServer::Wayland,
            "x11" => return DisplayServer::X11,
            "tty" => return DisplayServer::Tty,
            // Display managers also emit "unspecified"; treat anything unrecognised as
            // unset and let the socket variables decide.
            _ => {}
        }
    }
    if env_var("WAYLAND_DISPLAY").is_some() {
        DisplayServer::Wayland
    } else if env_var("DISPLAY").is_some() {
        DisplayServer::X11
    } else {
        DisplayServer::Unknown
    }
}

/// Seconds since boot, from the first field of `/proc/uptime`.
fn parse_uptime(contents: &str) -> Option<u64> {
    let secs: f64 = contents.split_whitespace().next()?.parse().ok()?;
    (secs.is_finite() && secs >= 0.0).then_some(secs as u64)
}

/// Total RAM and swap, in bytes.
fn parse_meminfo(contents: &str) -> (Option<u64>, Option<u64>) {
    let mut mem = None;
    let mut swap = None;
    for line in contents.lines() {
        let Some((key, rest)) = line.split_once(':') else {
            continue;
        };
        let slot = match key.trim() {
            "MemTotal" => &mut mem,
            "SwapTotal" => &mut swap,
            _ => continue,
        };
        let mut fields = rest.split_whitespace();
        let Some(value) = fields.next().and_then(|v| v.parse::<u64>().ok()) else {
            continue;
        };
        // /proc/meminfo prints "kB" but means KiB, so the factor is 1024, not 1000. A
        // few counters carry no unit column at all; those are already byte counts.
        *slot = Some(match fields.next() {
            Some(unit) if unit.eq_ignore_ascii_case("kB") => value.saturating_mul(1024),
            _ => value,
        });
    }
    (mem, swap)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_range_lists_expand() {
        assert_eq!(parse_cpu_list("0-31").len(), 32);
        assert_eq!(parse_cpu_list("0-31")[31], 31);
        assert_eq!(parse_cpu_list("0-3,8-11"), vec![0, 1, 2, 3, 8, 9, 10, 11]);
        assert_eq!(parse_cpu_list("5"), vec![5]);
        assert_eq!(parse_cpu_list("0,2-2,4"), vec![0, 2, 4]);
    }

    #[test]
    fn cpu_range_lists_survive_junk() {
        assert!(parse_cpu_list("").is_empty());
        assert!(parse_cpu_list("garbage").is_empty());
        // Reversed and absurdly wide ranges are dropped, not expanded.
        assert!(parse_cpu_list("9-2").is_empty());
        assert!(parse_cpu_list("0-999999").is_empty());
        assert_eq!(parse_cpu_list("0-1,,x,7"), vec![0, 1, 7]);
    }

    #[test]
    fn meminfo_values_are_kib() {
        let fixture = "MemTotal:       131643804 kB\n\
                       MemFree:         2000000 kB\n\
                       SwapTotal:      33554428 kB\n\
                       HugePages_Total:       0\n";
        let (mem, swap) = parse_meminfo(fixture);
        assert_eq!(mem, Some(131_643_804 * 1024));
        assert_eq!(swap, Some(33_554_428 * 1024));
    }

    #[test]
    fn meminfo_tolerates_missing_swap() {
        let (mem, swap) = parse_meminfo("MemTotal:  1024 kB\n");
        assert_eq!(mem, Some(1_048_576));
        assert_eq!(swap, None);
        assert_eq!(parse_meminfo("nonsense\n"), (None, None));
    }

    #[test]
    fn uptime_takes_the_first_field() {
        assert_eq!(parse_uptime("312052.16 8628804.64\n"), Some(312_052));
        assert_eq!(parse_uptime("0.42 0.40"), Some(0));
        assert_eq!(parse_uptime(""), None);
        assert_eq!(parse_uptime("nan 1.0"), None);
    }

    #[test]
    fn cpuinfo_keeps_the_first_block_and_counts_threads() {
        let fixture = "processor\t: 0\n\
                       vendor_id\t: GenuineIntel\n\
                       model\t\t: 183\n\
                       model name\t: Intel(R) Core(TM) i9-14900KF\n\
                       microcode\t: 0x133\n\
                       cpu cores\t: 24\n\
                       \n\
                       processor\t: 1\n\
                       vendor_id\t: GenuineIntel\n\
                       model name\t: Intel(R) Core(TM) i9-14900KF\n";
        let parsed = parse_cpuinfo(fixture);
        assert_eq!(parsed.threads, 2);
        assert_eq!(
            parsed.model.as_deref(),
            Some("Intel(R) Core(TM) i9-14900KF")
        );
        assert_eq!(parsed.vendor.as_deref(), Some("GenuineIntel"));
    }

    #[test]
    fn cpuinfo_on_a_machine_without_the_x86_keys() {
        // ARM /proc/cpuinfo has no "model name" or "vendor_id"; thread counting must
        // still work.
        let fixture = "processor\t: 0\nBogoMIPS\t: 50.00\n\nprocessor\t: 1\n";
        let parsed = parse_cpuinfo(fixture);
        assert_eq!(parsed.threads, 2);
        assert_eq!(parsed.model, None);
        assert_eq!(parsed.vendor, None);
    }

    #[test]
    fn os_release_lookup_prefers_pretty_name() {
        let pairs = parse_env_file("NAME=\"Arch Linux\"\nPRETTY_NAME=\"Arch Linux\"\nID=arch\n");
        assert_eq!(
            Value::first_known([
                os_field(&pairs, "PRETTY_NAME"),
                os_field(&pairs, "NAME"),
                os_field(&pairs, "ID"),
            ]),
            Value::Known("Arch Linux".into())
        );
        // Falls all the way through when only ID is present.
        let sparse = parse_env_file("ID=wmde\n");
        assert_eq!(
            Value::first_known([
                os_field(&sparse, "PRETTY_NAME"),
                os_field(&sparse, "NAME"),
                os_field(&sparse, "ID"),
            ]),
            Value::Known("wmde".into())
        );
        assert_eq!(os_field(&sparse, "BUILD_ID"), Value::Unavailable);
    }
}
