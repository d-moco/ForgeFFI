use super::*;

use forgeffi_base::{
    AdminState, IfaceFlags, IfaceKind, IpAddrEntry, NetIfCapabilities, OperState,
};
use std::process::Command;

pub(super) fn list_interfaces() -> Result<Vec<NetInterface>, ForgeFfiError> {
    let out = Command::new("ifconfig")
        .arg("-a")
        .output()
        .map_err(|e| ForgeFfiError::unsupported(format!("无法执行 ifconfig: {e}")))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(ForgeFfiError::system_error(format!(
            "ifconfig -a 失败: {stderr}"
        )));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(parse_ifconfig(&text))
}

pub(super) fn apply_one(target: &ResolvedTarget, op: &NetIfOp) -> Result<(), ForgeFfiError> {
    match op {
        NetIfOp::SetAdminState { up } => {
            let state = if *up { "up" } else { "down" };
            run_checked("ifconfig", &[target.name.as_str(), state])
        }
        NetIfOp::SetMtu { mtu } => {
            run_checked("ifconfig", &[target.name.as_str(), "mtu", &mtu.to_string()])
        }
        NetIfOp::AddIp { ip, prefix_len } => apply_ip(target, ip, *prefix_len, true),
        NetIfOp::DelIp { ip, prefix_len } => apply_ip(target, ip, *prefix_len, false),
        NetIfOp::SetIpv4Dhcp { .. } => Err(ForgeFfiError::unsupported(
            "macOS 下 DHCP 配置不在 V1 范围（可在 V2 通过 networksetup 支持）".to_string(),
        )),
        NetIfOp::SetIpv4Static { .. } => Err(ForgeFfiError::unsupported(
            "macOS 下暂未提供 SetIpv4Static（网关/持久化）封装".to_string(),
        )),
    }
}

fn apply_ip(target: &ResolvedTarget, ip: &str, prefix_len: u8, is_add: bool) -> Result<(), ForgeFfiError> {
    let addr: std::net::IpAddr = ip
        .parse()
        .map_err(|_| ForgeFfiError::invalid_argument(format!("非法 IP: {ip}")))?;
    match addr {
        std::net::IpAddr::V4(_) => {
            let verb = if is_add { "add" } else { "delete" };
            run_checked(
                "ifconfig",
                &[target.name.as_str(), "inet", &format!("{ip}/{prefix_len}"), verb],
            )
        }
        std::net::IpAddr::V6(_) => {
            let verb = if is_add { "add" } else { "delete" };
            run_checked(
                "ifconfig",
                &[
                    target.name.as_str(),
                    "inet6",
                    ip,
                    "prefixlen",
                    &prefix_len.to_string(),
                    verb,
                ],
            )
        }
    }
}

fn run_checked(program: &str, args: &[&str]) -> Result<(), ForgeFfiError> {
    let out = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| ForgeFfiError::system_error(format!("执行命令失败: {program}: {e}")))?;
    if out.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr);
        Err(ForgeFfiError::system_error(format!(
            "命令失败: {program} {:?}: {stderr}",
            args
        )))
    }
}

fn parse_ifconfig(s: &str) -> Vec<NetInterface> {
    let mut out = Vec::new();
    for block in s.split("\n\n") {
        let block = block.trim();
        if block.is_empty() {
            continue;
        }
        if let Some(i) = parse_ifconfig_block(block) {
            out.push(i);
        }
    }
    out
}

fn parse_ifconfig_block(block: &str) -> Option<NetInterface> {
    let mut lines = block.lines();
    let first = lines.next()?.trim();
    let name = first.split(':').next()?.trim().to_string();

    let mut flags_val = 0u32;
    if let Some(start) = first.find('<') {
        if let Some(end) = first[start + 1..].find('>') {
            let inside = &first[start + 1..start + 1 + end];
            for f in inside.split(',') {
                match f.trim() {
                    "UP" => flags_val |= IfaceFlags::UP,
                    "RUNNING" => flags_val |= IfaceFlags::RUNNING,
                    "LOOPBACK" => flags_val |= IfaceFlags::LOOPBACK,
                    "BROADCAST" => flags_val |= IfaceFlags::BROADCAST,
                    "MULTICAST" => flags_val |= IfaceFlags::MULTICAST,
                    "POINTOPOINT" => flags_val |= IfaceFlags::POINT_TO_POINT,
                    _ => {}
                }
            }
        }
    }

    let mtu = parse_mtu(first);
    let admin_state = if (flags_val & IfaceFlags::UP) != 0 {
        AdminState::Up
    } else {
        AdminState::Down
    };

    let mut oper_state = None;
    let mut mac = None;
    let mut ipv4 = Vec::new();
    let mut ipv6 = Vec::new();

    for l in std::iter::once("").chain(lines) {
        let line = l.trim();
        if line.starts_with("status:") {
            let v = line.split_whitespace().nth(1).unwrap_or("");
            oper_state = Some(if v.eq_ignore_ascii_case("active") {
                OperState::Up
            } else {
                OperState::Down
            });
        } else if line.starts_with("ether ") {
            mac = line.split_whitespace().nth(1).map(|s| s.to_string());
        } else if line.starts_with("inet ") {
            if let Some(ent) = parse_inet(line) {
                ipv4.push(ent);
            }
        } else if line.starts_with("inet6 ") {
            if let Some(ent) = parse_inet6(line) {
                ipv6.push(ent);
            }
        }
    }

    let kind = if name == "lo0" {
        IfaceKind::Loopback
    } else {
        IfaceKind::Unknown
    };

    Some(NetInterface {
        if_index: 0,
        name,
        display_name: None,
        kind,
        is_physical: None,
        admin_state,
        oper_state,
        flags: IfaceFlags(flags_val),
        mac,
        mtu,
        speed_bps: None,
        ipv4,
        ipv6,
        capabilities: NetIfCapabilities {
            can_set_admin_state: true,
            can_set_mtu: true,
            can_add_del_ip: true,
            can_set_dhcp: false,
            can_set_dns: false,
            notes: Some("macOS 下 if_index 可能不可用，建议使用 name 定位".to_string()),
        },
    })
}

fn parse_mtu(first: &str) -> Option<u32> {
    let idx = first.find("mtu ")?;
    let rest = &first[idx + 4..];
    rest.split_whitespace().next()?.parse().ok()
}

fn parse_inet(line: &str) -> Option<IpAddrEntry> {
    let mut it = line.split_whitespace();
    let _ = it.next()?;
    let ip = it.next()?.to_string();
    let mut prefix_len = None;
    while let Some(k) = it.next() {
        if k == "netmask" {
            if let Some(mask) = it.next() {
                prefix_len = parse_netmask_to_prefix(mask);
            }
        }
    }
    Some(IpAddrEntry {
        ip,
        prefix_len: prefix_len.unwrap_or(32),
        scope: None,
        origin: None,
        flags: None,
    })
}

fn parse_inet6(line: &str) -> Option<IpAddrEntry> {
    let mut it = line.split_whitespace();
    let _ = it.next()?;
    let raw_ip = it.next()?;
    let ip = raw_ip.split('%').next().unwrap_or(raw_ip).to_string();
    let mut prefix_len = None;
    while let Some(k) = it.next() {
        if k == "prefixlen" {
            prefix_len = it.next().and_then(|v| v.parse::<u8>().ok());
        }
    }
    Some(IpAddrEntry {
        ip,
        prefix_len: prefix_len.unwrap_or(128),
        scope: None,
        origin: None,
        flags: None,
    })
}

fn parse_netmask_to_prefix(mask: &str) -> Option<u8> {
    if let Some(hex) = mask.strip_prefix("0x") {
        let v = u32::from_str_radix(hex, 16).ok()?;
        return Some(v.count_ones() as u8);
    }
    let parts: Vec<u8> = mask
        .split('.')
        .map(|p| p.parse::<u8>().ok())
        .collect::<Option<Vec<u8>>>()?;
    if parts.len() != 4 {
        return None;
    }
    let v = u32::from_be_bytes([parts[0], parts[1], parts[2], parts[3]]);
    Some(v.count_ones() as u8)
}
