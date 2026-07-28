// SPDX-License-Identifier: GPL-3.0-only

//! Network interfaces from `/sys/class/net`, plus per-interface IP addresses from
//! `getifaddrs(3)`.
//!
//! sysfs alone cannot answer "which IPv4 does this interface hold": `/proc/net/fib_trie`
//! is a routing-table dump that only accidentally contains local addresses, and
//! `/proc/net/if_inet6` is IPv6 only. `getifaddrs` is the one call that gives both
//! families keyed by interface name, so it is used once per scan and the result is
//! joined onto the sysfs walk.

use std::collections::HashMap;
use std::ffi::CStr;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::Path;

use super::util::{link_basename, link_target, read_dir_sorted, read_i64, read_trim, read_u32};
use super::{IfKind, IfState, Interface};

const NET_DIR: &str = "/sys/class/net";

/// `ARPHRD_LOOPBACK`. The only reliable loopback marker; the name `lo` is convention.
const ARPHRD_LOOPBACK: u32 = 772;
/// `ARPHRD_TUNNEL` (ipip) and `ARPHRD_TUNNEL6`.
const ARPHRD_TUNNEL: u32 = 768;
const ARPHRD_TUNNEL6: u32 = 769;
/// `ARPHRD_NONE`, used by tun/wireguard devices.
const ARPHRD_NONE: u32 = 65534;

pub fn probe() -> Vec<Interface> {
    // One getifaddrs for the whole scan; a 40-veth machine would otherwise pay for it 40
    // times over.
    let mut addresses = interface_addresses();

    let mut out = Vec::new();
    for path in read_dir_sorted(NET_DIR) {
        // `/sys/class/net` is not purely interface symlinks: loading the bonding module
        // adds a plain `bonding_masters` file next to them.
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()).map(str::to_owned) else {
            continue;
        };

        let (ipv4, ipv6) = addresses.remove(&name).unwrap_or_default();
        out.push(read_interface(&path, name, ipv4, ipv6));
    }

    out.sort_by(|a, b| {
        sort_rank(a.kind)
            .cmp(&sort_rank(b.kind))
            .then_with(|| a.name.cmp(&b.name))
    });
    out
}

fn read_interface(
    path: &Path,
    name: String,
    mut ipv4: Vec<Ipv4Addr>,
    mut ipv6: Vec<Ipv6Addr>,
) -> Interface {
    // Stable output for the copied report; getifaddrs order is kernel-internal.
    ipv4.sort();
    ipv6.sort();

    let kind = classify(
        &name,
        read_u32(path.join("type")),
        path.join("wireless").is_dir() || path.join("phy80211").exists(),
        path.join("bridge").is_dir(),
        path.join("device").exists(),
        link_target(path).is_some_and(|t| t.to_string_lossy().contains("devices/virtual/net")),
    );

    Interface {
        name,
        kind,
        mac: read_trim(path.join("address")),
        state: parse_state(read_trim(path.join("operstate")).as_deref()),
        // `speed` and `duplex` both return EINVAL - a read *error*, not an empty value -
        // while the link is down, so `None` here is the normal case for anything unplugged
        // and not an indication that the attribute is missing.
        speed_mbps: parse_speed(read_i64(path.join("speed"))),
        // Bridges and vlans answer the literal string "unknown" (DUPLEX_UNKNOWN) rather
        // than erroring, which would render as a "Duplex: unknown" row; treat it as absent.
        duplex: read_trim(path.join("duplex")).filter(|d| d != "unknown"),
        mtu: read_u32(path.join("mtu")),
        driver: link_basename(path.join("device/driver")),
        ipv4,
        ipv6,
    }
}

/// Normalises the kernel's `speed` attribute.
///
/// `virbr0` reports `-1` for "no meaningful link speed". Parsing the attribute as `u64`
/// would drop it as a parse failure, which reads like a missing file and hides the
/// distinction; parse signed and discard negatives explicitly instead.
fn parse_speed(raw: Option<i64>) -> Option<u32> {
    u32::try_from(raw?).ok()
}

fn parse_state(raw: Option<&str>) -> IfState {
    match raw {
        Some("up") => IfState::Up,
        Some("dormant") => IfState::Dormant,
        Some("down" | "lowerlayerdown" | "notpresent") => IfState::Down,
        // "unknown" and "testing", plus the missing-attribute case. Note that plain
        // `lo` genuinely reports "unknown", so this is not an error path.
        _ => IfState::Unknown,
    }
}

/// Pure classification so the decision table is testable without a filesystem.
fn classify(
    name: &str,
    if_type: Option<u32>,
    wireless: bool,
    bridge: bool,
    device: bool,
    virtual_path: bool,
) -> IfKind {
    if if_type == Some(ARPHRD_LOOPBACK) {
        return IfKind::Loopback;
    }
    // Wi-Fi is detected by the `wireless/` directory or the `phy80211` link only. The
    // `type` attribute is useless here: a wlan0 in managed mode reports 1
    // (ARPHRD_ETHER), exactly like an Ethernet port.
    if wireless {
        return IfKind::Wireless;
    }
    if bridge {
        return IfKind::Bridge;
    }
    if device {
        return IfKind::Wired;
    }
    // Tunnels are checked before the virtual-path fallback: tun/tap/wg/ppp/sit/gre all
    // resolve under devices/virtual/net too, so testing the path first would make this
    // branch unreachable and label every VPN link a generic virtual device.
    if is_tunnel_name(name) || matches!(if_type, Some(ARPHRD_TUNNEL | ARPHRD_TUNNEL6 | ARPHRD_NONE))
    {
        return IfKind::Tunnel;
    }
    if virtual_path {
        return IfKind::Virtual;
    }
    IfKind::Other
}

fn is_tunnel_name(name: &str) -> bool {
    const PREFIXES: &[&str] = &["tun", "tap", "wg", "ppp", "sit", "gre"];
    PREFIXES.iter().any(|p| name.starts_with(p))
}

/// Physical first, then loopback, then everything synthetic.
const fn sort_rank(kind: IfKind) -> u8 {
    match kind {
        IfKind::Wired | IfKind::Wireless => 0,
        IfKind::Loopback => 1,
        IfKind::Bridge | IfKind::Virtual | IfKind::Tunnel | IfKind::Other => 2,
    }
}

/// Per-interface IPv4/IPv6 addresses, keyed by interface name.
fn interface_addresses() -> HashMap<String, (Vec<Ipv4Addr>, Vec<Ipv6Addr>)> {
    let mut map: HashMap<String, (Vec<Ipv4Addr>, Vec<Ipv6Addr>)> = HashMap::new();

    // SAFETY: getifaddrs allocates a single linked list that the caller owns as a whole
    // and must release with exactly one freeifaddrs of the head pointer - individual
    // nodes are not separately allocated and must never be freed. The list stays valid,
    // and the `ifa_name`/`ifa_addr` pointers inside it stay valid, until that call, and
    // nothing here escapes the block: every name and address is copied into owned values
    // first. `head` is freed on every path that reaches the end of the block, and the
    // only early return is the failure path, where no list was allocated.
    unsafe {
        let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut head) != 0 {
            log::debug!("getifaddrs failed: {}", std::io::Error::last_os_error());
            return map;
        }

        let mut cursor = head;
        while !cursor.is_null() {
            let entry = &*cursor;
            cursor = entry.ifa_next;

            // ifa_addr is NULL for an interface with no address of any family. On a
            // machine with dozens of down veth pairs that is the common case, not a
            // corner case, so this check is what stands between us and a segfault.
            if entry.ifa_addr.is_null() || entry.ifa_name.is_null() {
                continue;
            }

            let family = (*entry.ifa_addr).sa_family;
            let Ok(name) = CStr::from_ptr(entry.ifa_name).to_str() else {
                continue;
            };

            if family == libc::AF_INET as libc::sa_family_t {
                let sin = entry.ifa_addr.cast::<libc::sockaddr_in>();
                // s_addr is in network byte order.
                let addr = Ipv4Addr::from(u32::from_be((*sin).sin_addr.s_addr));
                map.entry(name.to_owned()).or_default().0.push(addr);
            } else if family == libc::AF_INET6 as libc::sa_family_t {
                let sin6 = entry.ifa_addr.cast::<libc::sockaddr_in6>();
                // s6_addr is already a big-endian [u8; 16], which is what Ipv6Addr wants.
                let addr = Ipv6Addr::from((*sin6).sin6_addr.s6_addr);
                map.entry(name.to_owned()).or_default().1.push(addr);
            }
            // Everything else (AF_PACKET link-layer entries above all) is skipped without
            // creating a map entry, so interfaces with no IP stay absent rather than
            // showing up with two empty vectors.
        }

        libc::freeifaddrs(head);
    }

    map
}

#[cfg(test)]
mod tests {
    use super::*;

    /// name, `type`, `wireless/`, `bridge/`, `device/`, virtual path, expected kind.
    type Case = (&'static str, Option<u32>, bool, bool, bool, bool, IfKind);

    // Kept one row per case; rustfmt would otherwise explode each tuple over eight lines.
    #[rustfmt::skip]
    #[test]
    fn classification_decision_table() {
        let cases: &[Case] = &[
            // lo: type 772 wins even though it also lives under devices/virtual/net.
            ("lo", Some(772), false, false, false, true, IfKind::Loopback),
            // wlan0 reports type 1 exactly like Ethernet; only wireless/ separates them.
            ("wlan0", Some(1), true, false, true, false, IfKind::Wireless),
            ("eno1", Some(1), false, false, true, false, IfKind::Wired),
            // A bridge has no device/ but does have bridge/.
            ("br0", Some(1), false, true, false, true, IfKind::Bridge),
            ("docker0", Some(1), false, true, false, true, IfKind::Bridge),
            // veth: no device, no bridge, resolves under devices/virtual/net.
            ("veth0474ffe", Some(1), false, false, false, true, IfKind::Virtual),
            ("vboxnet0", Some(1), false, false, false, true, IfKind::Virtual),
            // Tunnels are virtual-path too, so name/type must be checked first.
            ("wg0", Some(65534), false, false, false, true, IfKind::Tunnel),
            ("tun0", Some(65534), false, false, false, true, IfKind::Tunnel),
            ("ppp0", Some(512), false, false, false, true, IfKind::Tunnel),
            ("sit0", Some(776), false, false, false, true, IfKind::Tunnel),
            // Type alone is enough when the name is unfamiliar.
            ("ip6tnl0", Some(769), false, false, false, true, IfKind::Tunnel),
            // Nothing known at all.
            ("weird0", None, false, false, false, false, IfKind::Other),
        ];

        for &(name, ty, wireless, bridge, device, virt, expected) in cases {
            let got = classify(name, ty, wireless, bridge, device, virt);
            assert_eq!(got, expected, "{name}");
        }
    }

    #[test]
    fn bridge_beats_wired_when_both_markers_exist() {
        // A bridge over a physical port keeps no device/ link of its own, but be explicit
        // about the precedence anyway.
        assert_eq!(
            classify("br0", Some(1), false, true, true, true),
            IfKind::Bridge
        );
    }

    #[test]
    fn negative_speed_is_dropped_not_misparsed() {
        // virbr0 reports -1: "no meaningful link speed", not "1 Mbps" and not a missing
        // attribute.
        assert_eq!(parse_speed(Some(-1)), None);
        assert_eq!(parse_speed(Some(1000)), Some(1000));
        assert_eq!(parse_speed(Some(0)), Some(0));
        // EINVAL while the link is down surfaces as a read failure.
        assert_eq!(parse_speed(None), None);
        // 100 GbE, still well inside u32.
        assert_eq!(parse_speed(Some(100_000)), Some(100_000));
        assert_eq!(parse_speed(Some(i64::from(u32::MAX) + 1)), None);
    }

    #[test]
    fn operstate_mapping() {
        assert_eq!(parse_state(Some("up")), IfState::Up);
        assert_eq!(parse_state(Some("down")), IfState::Down);
        assert_eq!(parse_state(Some("lowerlayerdown")), IfState::Down);
        assert_eq!(parse_state(Some("dormant")), IfState::Dormant);
        // lo really does report this.
        assert_eq!(parse_state(Some("unknown")), IfState::Unknown);
        assert_eq!(parse_state(None), IfState::Unknown);
    }

    #[test]
    fn sort_groups_physical_then_loopback_then_virtual() {
        assert!(sort_rank(IfKind::Wired) < sort_rank(IfKind::Loopback));
        assert!(sort_rank(IfKind::Wireless) < sort_rank(IfKind::Loopback));
        assert!(sort_rank(IfKind::Loopback) < sort_rank(IfKind::Bridge));
        assert_eq!(sort_rank(IfKind::Tunnel), sort_rank(IfKind::Virtual));
    }
}
