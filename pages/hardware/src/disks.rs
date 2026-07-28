// SPDX-License-Identifier: GPL-3.0-only

//! Block devices: geometry from `/sys/block`, mount points and free space from
//! `/proc/self/mountinfo` plus `statvfs(3)`.
//!
//! Only real hardware is reported. Everything here works as an unprivileged user; the
//! one attribute that commonly is not readable is `device/serial` on some SCSI stacks,
//! which simply yields `None`.

use std::collections::HashMap;
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use super::util::{read_bool, read_dir_sorted, read_trim, read_u32, read_u64, unescape_octal};
use super::{Disk, DiskKind, Partition, Usage};

const SYS_BLOCK: &str = "/sys/block";
const MOUNTINFO: &str = "/proc/self/mountinfo";

/// `size` in sysfs is *always* counted in 512-byte sectors, for both whole disks and
/// partitions, whatever `queue/logical_block_size` says. Scaling by the logical block
/// size instead is wrong by a factor of 8 on 4Kn drives: a 2 TB disk reports 3907029168,
/// and 3907029168 * 512 = 2000398934016 bytes.
const SECTOR_BYTES: u64 = 512;

/// Guards against a cyclic or absurdly deep `holders` chain; three levels already covers
/// LVM on LUKS on md.
const HOLDER_DEPTH: u32 = 4;

pub fn probe() -> Vec<Disk> {
    let mounts = read_mounts();
    read_dir_sorted(SYS_BLOCK)
        .into_iter()
        .filter_map(|path| disk(&path, &mounts))
        .collect()
}

fn disk(path: &Path, mounts: &MountsByDev) -> Option<Disk> {
    // A `device` link means the block device is backed by real hardware. It is absent for
    // loop, ram, zram, dm-* and md*, which is exactly the set we want to drop.
    if !path.join("device").exists() {
        return None;
    }
    let name = path.file_name()?.to_str()?.to_owned();
    let removable = read_bool(path.join("removable")).unwrap_or(false);
    let rotational = read_bool(path.join("queue/rotational"));

    Some(Disk {
        kind: classify(&name, removable, rotational),
        // These three are right-padded with spaces to a fixed width in both the NVMe and
        // the SCSI drivers, so read_trim is mandatory, not cosmetic.
        model: read_trim(path.join("device/model")),
        serial: read_trim(path.join("device/serial")),
        // NVMe spells it `firmware_rev`, the SCSI/ATA stack spells it `rev`.
        firmware: read_trim(path.join("device/firmware_rev"))
            .or_else(|| read_trim(path.join("device/rev"))),
        size_bytes: sectors_to_bytes(read_u64(path.join("size")).unwrap_or(0)),
        read_only: read_bool(path.join("ro")).unwrap_or(false),
        logical_block_size: read_u32(path.join("queue/logical_block_size")),
        partitions: partitions(path, mounts),
        name,
    })
}

const fn sectors_to_bytes(sectors: u64) -> u64 {
    sectors.saturating_mul(SECTOR_BYTES)
}

fn classify(name: &str, removable: bool, rotational: Option<bool>) -> DiskKind {
    if name.starts_with("nvme") {
        // Checked first: NVMe namespaces report rotational 0, so they would otherwise all
        // be flattened into plain SSDs.
        DiskKind::Nvme
    } else if removable {
        DiskKind::Removable
    } else {
        match rotational {
            Some(true) => DiskKind::Hdd,
            Some(false) => DiskKind::Ssd,
            None => DiskKind::Unknown,
        }
    }
}

// ---------------------------------------------------------------- partitions

fn partitions(disk_path: &Path, mounts: &MountsByDev) -> Vec<Partition> {
    let mut found: Vec<(u32, Partition)> = Vec::new();

    for path in read_dir_sorted(disk_path) {
        // The `partition` attribute is what distinguishes a partition directory from the
        // disk's other children (queue, holders, mq, power, ...), and its value is the
        // partition index.
        let Some(index) = read_u32(path.join("partition")) else {
            continue;
        };
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };

        let mut mountpoints: Vec<PathBuf> = Vec::new();
        let mut fstype: Option<String> = None;
        for dev in device_numbers(&path) {
            for mount in mounts.get(&dev).into_iter().flatten() {
                // Only block-device mounts are kept: statvfs() on a hung NFS/SMB mount
                // blocks indefinitely and would freeze the whole probe.
                if !mount.source.starts_with("/dev/") {
                    continue;
                }
                if fstype.is_none() {
                    fstype = Some(mount.fstype.clone());
                }
                if !mountpoints.contains(&mount.mountpoint) {
                    mountpoints.push(mount.mountpoint.clone());
                }
            }
        }

        // Bind mounts and btrfs subvolumes of one filesystem all report the same figures,
        // so the first mount point that answers is enough.
        let usage = mountpoints.iter().find_map(|mount| statvfs_usage(mount));

        found.push((
            index,
            Partition {
                name: name.to_owned(),
                size_bytes: sectors_to_bytes(read_u64(path.join("size")).unwrap_or(0)),
                fstype,
                mountpoints,
                usage,
            },
        ));
    }

    // Sorted by partition number, not by name: a lexicographic sort puts p10 before p2.
    found.sort_by_key(|(index, _)| *index);
    found.into_iter().map(|(_, part)| part).collect()
}

/// Every `major:minor` under which this partition can show up in the mount table.
///
/// On a LUKS/LVM/md system the mounted filesystem lives on the mapped device, so
/// mountinfo carries the mapper's dev_t, not the partition's. `holders/` is the link back
/// down to the partition, and it can be stacked (LVM on top of LUKS), hence the recursion.
fn device_numbers(path: &Path) -> Vec<String> {
    let mut out = Vec::new();
    collect_device_numbers(path, &mut out, HOLDER_DEPTH);
    out
}

fn collect_device_numbers(path: &Path, out: &mut Vec<String>, depth: u32) {
    if let Some(dev) = read_trim(path.join("dev"))
        && !out.contains(&dev)
    {
        out.push(dev);
    }
    if depth == 0 {
        return;
    }
    for holder in read_dir_sorted(path.join("holders")) {
        collect_device_numbers(&holder, out, depth - 1);
    }
}

// ---------------------------------------------------------------- mount table

type MountsByDev = HashMap<String, Vec<Mount>>;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Mount {
    /// `major:minor`, spelled exactly as sysfs spells `<block device>/dev`.
    dev: String,
    mountpoint: PathBuf,
    fstype: String,
    source: String,
}

fn read_mounts() -> MountsByDev {
    let text = match std::fs::read_to_string(MOUNTINFO) {
        Ok(text) => text,
        Err(err) => {
            log::debug!("cannot read {MOUNTINFO}: {err}");
            return MountsByDev::new();
        }
    };
    let mut map = MountsByDev::new();
    for mount in parse_mountinfo(&text) {
        map.entry(mount.dev.clone()).or_default().push(mount);
    }
    map
}

/// Parses `/proc/self/mountinfo`.
///
/// Deliberately not `/proc/mounts`: that table identifies a filesystem only by its source
/// *string*, so on a LUKS/LVM/md machine the root filesystem appears as `/dev/mapper/xxx`
/// and matches no `/sys/block/<disk>/<part>` name at all - the single most useful row in
/// the whole view (root, used, free) would silently render blank. mountinfo carries the
/// device's `major:minor`, which maps directly onto `<part>/dev` (and, via `holders/`,
/// onto the partition underneath a mapped device). It also lists bind mounts and btrfs
/// subvolumes as separate rows, which is why `Partition::mountpoints` is a `Vec`.
///
/// Line format:
///
/// ```text
/// ID PARENT MAJOR:MINOR ROOT MOUNTPOINT OPTIONS [OPTIONAL...] - FSTYPE SOURCE SUPEROPTS
/// ```
fn parse_mountinfo(text: &str) -> Vec<Mount> {
    let mut out = Vec::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split_ascii_whitespace().collect();
        // The optional-fields block (shared:, master:, propagate_from:, unbindable) is
        // variable length and terminated by a lone "-", so everything after OPTIONS can
        // only be addressed relative to that separator. It cannot be confused with a real
        // value: ROOT and MOUNTPOINT always start with "/", and OPTIONS is never "-".
        let Some(sep) = fields.iter().skip(6).position(|f| *f == "-").map(|i| i + 6) else {
            continue;
        };
        let (head, tail) = (&fields[..sep], &fields[sep + 1..]);
        if tail.len() < 2 {
            continue;
        }
        out.push(Mount {
            dev: head[2].to_owned(),
            // MOUNTPOINT (and ROOT, which we do not use) is octal-escaped, so a mount
            // point containing a space arrives as one field and must be decoded. The
            // escaping is also why taking fields by index is safe.
            mountpoint: PathBuf::from(unescape_octal(head[4])),
            fstype: tail[0].to_owned(),
            source: unescape_octal(tail[1]),
        });
    }
    out
}

// ---------------------------------------------------------------- usage

/// Free-space figures for one mount point. The only `unsafe` in this module.
fn statvfs_usage(mount: &Path) -> Option<Usage> {
    let c = CString::new(mount.as_os_str().as_bytes()).ok()?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
        return None;
    }
    // f_blocks/f_bfree/f_bavail count fragments, so f_frsize is the correct multiplier;
    // f_bsize is only the preferred I/O size and differs on ext4 with bigalloc.
    let unit = if s.f_frsize > 0 {
        s.f_frsize
    } else {
        s.f_bsize
    } as u64;
    let total = (s.f_blocks as u64).saturating_mul(unit);
    let free = (s.f_bfree as u64).saturating_mul(unit);
    // f_bavail excludes the root-reserved blocks, which is what df prints as "Avail".
    let available = (s.f_bavail as u64).saturating_mul(unit);
    // FUSE and network filesystems happily report f_bfree > f_blocks; a plain subtraction
    // would panic on overflow in a debug build.
    let used = total.saturating_sub(free);
    Some(Usage {
        total,
        used,
        available,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real shapes: one line with a single optional field, one with two, one with none,
    // an octal-escaped mount point containing a space, one device mounted twice (btrfs
    // subvolumes), a non-block source, and a malformed line with no separator.
    const MOUNTINFO_FIXTURE: &str = r"25 30 0:22 / /proc rw,nosuid,nodev,noexec shared:5 - proc proc rw
31 25 259:9 / / rw,relatime shared:1 - ext4 /dev/nvme0n1p2 rw
56 31 8:1 / /mnt/My\040Disk rw,relatime shared:102 master:7 - ext4 /dev/sda1 rw
70 31 259:3 /@ /data rw,relatime shared:3 - btrfs /dev/nvme1n1p1 rw,subvol=/@
71 31 259:3 /@home /data/home rw,relatime shared:4 - btrfs /dev/nvme1n1p1 rw,subvol=/@home
72 31 0:55 / /net rw,relatime - nfs4 server:/export rw
this line is not a mount entry
";

    #[test]
    fn mountinfo_optional_fields_do_not_shift_the_tail() {
        let mounts = parse_mountinfo(MOUNTINFO_FIXTURE);
        assert_eq!(mounts.len(), 6, "the malformed line must be dropped");

        // One optional field.
        assert_eq!(mounts[0].fstype, "proc");
        assert_eq!(mounts[0].source, "proc");
        // Two optional fields.
        assert_eq!(mounts[2].fstype, "ext4");
        assert_eq!(mounts[2].source, "/dev/sda1");
        // None at all.
        assert_eq!(mounts[5].fstype, "nfs4");
        assert_eq!(mounts[5].source, "server:/export");
    }

    #[test]
    fn mountinfo_root_row_keeps_its_device_number() {
        let mounts = parse_mountinfo(MOUNTINFO_FIXTURE);
        assert_eq!(mounts[1].dev, "259:9");
        assert_eq!(mounts[1].mountpoint, PathBuf::from("/"));
        assert_eq!(mounts[1].source, "/dev/nvme0n1p2");
    }

    #[test]
    fn mountinfo_mount_point_with_a_space_survives() {
        let mounts = parse_mountinfo(MOUNTINFO_FIXTURE);
        assert_eq!(mounts[2].mountpoint, PathBuf::from("/mnt/My Disk"));
        assert_eq!(mounts[2].dev, "8:1");
    }

    #[test]
    fn mountinfo_one_device_can_carry_several_mount_points() {
        let mounts = parse_mountinfo(MOUNTINFO_FIXTURE);
        let btrfs: Vec<&PathBuf> = mounts
            .iter()
            .filter(|m| m.dev == "259:3")
            .map(|m| &m.mountpoint)
            .collect();
        assert_eq!(
            btrfs,
            vec![&PathBuf::from("/data"), &PathBuf::from("/data/home")]
        );
    }

    #[test]
    fn only_block_sources_are_eligible_for_statvfs() {
        let mounts = parse_mountinfo(MOUNTINFO_FIXTURE);
        assert!(mounts[1].source.starts_with("/dev/"));
        // An NFS mount would hang statvfs, and its source rules it out.
        assert!(!mounts[5].source.starts_with("/dev/"));
        assert!(!mounts[0].source.starts_with("/dev/"));
    }

    #[test]
    fn sizes_are_always_512_byte_sectors() {
        assert_eq!(sectors_to_bytes(3_907_029_168), 2_000_398_934_016);
        assert_eq!(sectors_to_bytes(487_346_176), 249_521_242_112);
        assert_eq!(sectors_to_bytes(0), 0);
    }

    #[test]
    fn disk_kind_classification() {
        assert_eq!(classify("nvme0n1", false, Some(false)), DiskKind::Nvme);
        // An NVMe in a USB enclosure is still reported as NVMe: the name wins.
        assert_eq!(classify("nvme0n1", true, None), DiskKind::Nvme);
        assert_eq!(classify("sdb", true, Some(true)), DiskKind::Removable);
        assert_eq!(classify("sda", false, Some(true)), DiskKind::Hdd);
        assert_eq!(classify("sda", false, Some(false)), DiskKind::Ssd);
        assert_eq!(classify("sda", false, None), DiskKind::Unknown);
    }
}
