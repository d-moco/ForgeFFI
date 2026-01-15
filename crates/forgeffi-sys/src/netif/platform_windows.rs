use super::*;

use forgeffi_base::{
    AdminState, IfaceFlags, IfaceKind, IpAddrEntry, NetIfCapabilities, OperState,
};
use serde_json::Value;
use std::collections::BTreeMap;
use std::process::Command;

pub(super) fn list_interfaces() -> Result<Vec<NetInterface>, ForgeFfiError> {
    let script = r#"
$adapters = Get-NetAdapter | Select-Object ifIndex, Name, InterfaceDescription, Status, MacAddress, LinkSpeed
$ipif = Get-NetIPInterface | Select-Object ifIndex, AddressFamily, Dhcp, NlMtu, ConnectionState
$ips = Get-NetIPAddress | Select-Object ifIndex, AddressFamily, IPAddress, PrefixLength
[pscustomobject]@{ adapters=$adapters; ipif=$ipif; ips=$ips } | ConvertTo-Json -Depth 5
"#;

    let text = run_powershell_capture(script)?;
    let v: Value = serde_json::from_str(&text)
        .map_err(|e| ForgeFfiError::system_error(format!("解析 PowerShell JSON 失败: {e}")))?;

    let adapters = normalize_array(v.get("adapters"));
    let ipif = normalize_array(v.get("ipif"));
    let ips = normalize_array(v.get("ips"));

    let mut mtu_by_idx: BTreeMap<u32, u32> = BTreeMap::new();
    let mut conn_by_idx: BTreeMap<u32, OperState> = BTreeMap::new();

    for it in ipif {
        let idx = it.get("ifIndex").and_then(Value::as_u64).unwrap_or(0) as u32;
        if idx == 0 {
            continue;
        }
        if let Some(mtu) = it.get("NlMtu").and_then(Value::as_u64) {
            mtu_by_idx.insert(idx, mtu as u32);
        }
        if let Some(cs) = it.get("ConnectionState").and_then(Value::as_str) {
            let st = if cs.eq_ignore_ascii_case("Connected") {
                OperState::Up
            } else {
                OperState::Down
            };
            conn_by_idx.insert(idx, st);
        }
    }

    let mut ips_by_idx: BTreeMap<u32, (Vec<IpAddrEntry>, Vec<IpAddrEntry>)> = BTreeMap::new();
    for it in ips {
        let idx = it.get("ifIndex").and_then(Value::as_u64).unwrap_or(0) as u32;
        if idx == 0 {
            continue;
        }
        let af = parse_windows_address_family(it.get("AddressFamily"));
        let ip = it.get("IPAddress").and_then(Value::as_str).unwrap_or("");
        let prefix = it.get("PrefixLength").and_then(Value::as_u64).unwrap_or(0) as u8;
        if ip.is_empty() {
            continue;
        }
        let ent = IpAddrEntry {
            ip: ip.to_string(),
            prefix_len: prefix,
            scope: None,
            origin: None,
            flags: None,
        };
        let e = ips_by_idx.entry(idx).or_insert_with(|| (Vec::new(), Vec::new()));
        if af == WindowsAddressFamily::Ipv4 {
            e.0.push(ent);
        } else if af == WindowsAddressFamily::Ipv6 {
            e.1.push(ent);
        }
    }

    let mut out = Vec::new();
    for it in adapters {
        let idx = it.get("ifIndex").and_then(Value::as_u64).unwrap_or(0) as u32;
        if idx == 0 {
            continue;
        }
        let name = it.get("Name").and_then(Value::as_str).unwrap_or("").to_string();
        if name.is_empty() {
            continue;
        }
        let display_name = it
            .get("InterfaceDescription")
            .and_then(Value::as_str)
            .map(|s| s.to_string());
        let status = it.get("Status").and_then(Value::as_str).unwrap_or("");
        let admin_state = if status.eq_ignore_ascii_case("Up") {
            AdminState::Up
        } else if status.eq_ignore_ascii_case("Disabled") {
            AdminState::Down
        } else {
            AdminState::Unknown
        };
        let mac = it
            .get("MacAddress")
            .and_then(Value::as_str)
            .map(|s| s.replace('-', ":"));

        let speed_bps = it
            .get("LinkSpeed")
            .and_then(Value::as_str)
            .and_then(parse_link_speed_bps);

        let mut flags = 0u32;
        if admin_state == AdminState::Up {
            flags |= IfaceFlags::UP;
        }

        let (ipv4, ipv6) = ips_by_idx.remove(&idx).unwrap_or_default();

        out.push(NetInterface {
            if_index: idx,
            name,
            display_name,
            kind: IfaceKind::Unknown,
            is_physical: None,
            admin_state,
            oper_state: conn_by_idx.get(&idx).copied(),
            flags: IfaceFlags(flags),
            mac,
            mtu: mtu_by_idx.get(&idx).copied(),
            speed_bps,
            ipv4,
            ipv6,
            capabilities: NetIfCapabilities {
                can_set_admin_state: true,
                can_set_mtu: true,
                can_add_del_ip: true,
                can_set_dhcp: true,
                can_set_dns: false,
                notes: None,
            },
        });
    }

    Ok(out)
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum WindowsAddressFamily {
    Unknown,
    Ipv4,
    Ipv6,
}

fn parse_windows_address_family(v: Option<&Value>) -> WindowsAddressFamily {
    match v {
        None => WindowsAddressFamily::Unknown,
        Some(Value::String(s)) => {
            if s.eq_ignore_ascii_case("IPv4") {
                WindowsAddressFamily::Ipv4
            } else if s.eq_ignore_ascii_case("IPv6") {
                WindowsAddressFamily::Ipv6
            } else {
                WindowsAddressFamily::Unknown
            }
        }
        Some(Value::Number(n)) => match n.as_u64() {
            Some(2) => WindowsAddressFamily::Ipv4,
            Some(23) => WindowsAddressFamily::Ipv6,
            _ => WindowsAddressFamily::Unknown,
        },
        _ => WindowsAddressFamily::Unknown,
    }
}

pub(super) fn apply_one(target: &ResolvedTarget, op: &NetIfOp) -> Result<(), ForgeFfiError> {
    let idx = target.if_index;
    if idx == 0 {
        return Err(ForgeFfiError::invalid_argument(format!(
            "Windows 下必须提供有效 if_index（name={}）",
            target.name
        )));
    }

    match op {
        NetIfOp::SetAdminState { up } => {
            if *up {
                run_powershell_checked(&format!(
                    "Enable-NetAdapter -InterfaceIndex {idx} -Confirm:$false | Out-Null"
                ))
            } else {
                run_powershell_checked(&format!(
                    "Disable-NetAdapter -InterfaceIndex {idx} -Confirm:$false | Out-Null"
                ))
            }
        }
        NetIfOp::SetMtu { mtu } => run_powershell_checked(&format!(
            "Set-NetIPInterface -InterfaceIndex {idx} -NlMtuBytes {mtu} -Confirm:$false | Out-Null"
        )),
        NetIfOp::AddIp { ip, prefix_len } => {
            let family = ip_family(ip)?;
            run_powershell_checked(&format!(
                "New-NetIPAddress -InterfaceIndex {idx} -IPAddress '{ip}' -PrefixLength {prefix_len} -AddressFamily {family} | Out-Null"
            ))
        }
        NetIfOp::DelIp { ip, .. } => {
            let family = ip_family(ip)?;
            run_powershell_checked(&format!(
                "Remove-NetIPAddress -InterfaceIndex {idx} -IPAddress '{ip}' -AddressFamily {family} -Confirm:$false | Out-Null"
            ))
        }
        NetIfOp::SetIpv4Dhcp { enable } => {
            let mode = if *enable { "Enabled" } else { "Disabled" };
            run_powershell_checked(&format!(
                "Set-NetIPInterface -InterfaceIndex {idx} -AddressFamily IPv4 -Dhcp {mode} -Confirm:$false | Out-Null"
            ))
        }
        NetIfOp::SetIpv4Static { .. } => Err(ForgeFfiError::unsupported(
            "Windows 下暂未提供 SetIpv4Static（网关/持久化）封装，请使用 add_ip/del_ip + 系统网络配置工具".to_string(),
        )),
    }
}

fn ip_family(ip: &str) -> Result<&'static str, ForgeFfiError> {
    let addr: std::net::IpAddr = ip
        .parse()
        .map_err(|_| ForgeFfiError::invalid_argument(format!("非法 IP: {ip}")))?;
    Ok(match addr {
        std::net::IpAddr::V4(_) => "IPv4",
        std::net::IpAddr::V6(_) => "IPv6",
    })
}

fn normalize_array(v: Option<&Value>) -> Vec<Value> {
    match v {
        None => Vec::new(),
        Some(Value::Array(a)) => a.clone(),
        Some(Value::Object(_)) => vec![v.unwrap().clone()],
        _ => Vec::new(),
    }
}

fn run_powershell_capture(script: &str) -> Result<String, ForgeFfiError> {
    let script = format!(
        "$OutputEncoding = [System.Text.UTF8Encoding]::new(); [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new(); {script}"
    );
    let out = Command::new("powershell")
        .arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-Command")
        .arg(&script)
        .output()
        .map_err(|e| ForgeFfiError::unsupported(format!("无法执行 PowerShell: {e}")))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr);
        Err(ForgeFfiError::system_error(format!(
            "PowerShell 失败: {stderr}"
        )))
    }
}

fn run_powershell_checked(script: &str) -> Result<(), ForgeFfiError> {
    let out = Command::new("powershell")
        .arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-Command")
        .arg(script)
        .output()
        .map_err(|e| ForgeFfiError::unsupported(format!("无法执行 PowerShell: {e}")))?;
    if out.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr);
        Err(map_windows_error(&stderr))
    }
}

fn map_windows_error(stderr: &str) -> ForgeFfiError {
    let s = stderr.to_lowercase();
    if s.contains("access is denied") || s.contains("权限") {
        ForgeFfiError::permission_denied(stderr.trim().to_string())
    } else if s.contains("no msft_netadapter objects") || s.contains("cannot find") {
        ForgeFfiError::not_found(stderr.trim().to_string())
    } else {
        ForgeFfiError::system_error(stderr.trim().to_string())
    }
}

fn parse_link_speed_bps(s: &str) -> Option<u64> {
    let s = s.trim();
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() < 2 {
        return None;
    }
    let num: f64 = parts[0].parse().ok()?;
    let unit = parts[1].to_ascii_lowercase();
    let mul = if unit.contains("gbps") {
        1_000_000_000f64
    } else if unit.contains("mbps") {
        1_000_000f64
    } else if unit.contains("kbps") {
        1_000f64
    } else if unit.contains("bps") {
        1f64
    } else {
        return None;
    };
    Some((num * mul) as u64)
}
