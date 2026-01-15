use super::*;

use forgeffi_base::{
    AdminState, IfaceFlags, IfaceKind, IpAddrEntry, IpAddrFlags, IpOrigin, IpScope,
    NetIfCapabilities, OperState,
};
use serde::Deserialize;
use std::process::Command;
use std::sync::OnceLock;
use std::{fs, io, path::Path};

#[derive(Debug, Deserialize)]
struct IpAddrInfo {
    family: String,
    local: String,
    prefixlen: u8,
    scope: Option<String>,
    #[serde(default)]
    deprecated: bool,
    #[serde(default)]
    tentative: bool,
    #[serde(default)]
    temporary: bool,
    #[serde(default)]
    dynamic: bool,
}

#[derive(Debug, Deserialize)]
struct IpIface {
    ifindex: u32,
    ifname: String,
    #[serde(default)]
    flags: Vec<String>,
    mtu: Option<u32>,
    operstate: Option<String>,
    address: Option<String>,
    #[serde(default)]
    addr_info: Vec<IpAddrInfo>,
}

pub(super) fn list_interfaces() -> Result<Vec<NetInterface>, ForgeFfiError> {
    let out = Command::new("ip")
        .arg("-j")
        .arg("address")
        .output()
        .map_err(|e| ForgeFfiError::unsupported(format!("无法执行 ip 命令（需要 iproute2）: {e}")))?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(ForgeFfiError::system_error(format!(
            "ip -j address 失败: {stderr}"
        )));
    }

    let ifaces: Vec<IpIface> = serde_json::from_slice(&out.stdout)
        .map_err(|e| ForgeFfiError::system_error(format!("解析 ip JSON 失败: {e}")))?;

    Ok(ifaces.into_iter().map(map_iface).collect())
}

pub(super) fn apply_one(target: &ResolvedTarget, op: &NetIfOp) -> Result<(), ForgeFfiError> {
    match op {
        NetIfOp::SetAdminState { up } => {
            let state = if *up { "up" } else { "down" };
            run_checked("ip", &["link", "set", "dev", target.name.as_str(), state])
        }
        NetIfOp::SetMtu { mtu } => run_checked(
            "ip",
            &[
                "link",
                "set",
                "dev",
                target.name.as_str(),
                "mtu",
                &mtu.to_string(),
            ],
        ),
        NetIfOp::AddIp { ip, prefix_len } => {
            if let Some(conn) = nmcli_connection_for_dev(&target.name)? {
                let cidr = format!("{ip}/{prefix_len}");
                nmcli_checked(&[
                    "con",
                    "mod",
                    "id",
                    conn.as_str(),
                    "ipv4.method",
                    "manual",
                    "+ipv4.addresses",
                    cidr.as_str(),
                ])?;
                nmcli_checked(&["con", "up", "id", conn.as_str()])
            } else {
                run_checked(
                    "ip",
                    &["addr", "add", &format!("{ip}/{prefix_len}"), "dev", target.name.as_str()],
                )
            }
        }
        NetIfOp::DelIp { ip, prefix_len } => {
            if let Some(conn) = nmcli_connection_for_dev(&target.name)? {
                let cidr = format!("{ip}/{prefix_len}");
                match nmcli_try(&[
                    "con",
                    "mod",
                    "id",
                    conn.as_str(),
                    "-ipv4.addresses",
                    cidr.as_str(),
                ]) {
                    Ok(()) => nmcli_checked(&["con", "up", "id", conn.as_str()]),
                    Err(e) => {
                        if e.contains("ipv4.addresses") && e.contains("不允许") {
                            nmcli_checked(&[
                                "con",
                                "mod",
                                "id",
                                conn.as_str(),
                                "ipv4.method",
                                "auto",
                                "ipv4.addresses",
                                "",
                                "ipv4.gateway",
                                "",
                            ])?;
                            nmcli_checked(&["con", "up", "id", conn.as_str()])
                        } else {
                            Err(ForgeFfiError::system_error(format!(
                                "nmcli 命令失败: nmcli {:?}: {}",
                                [
                                    "con",
                                    "mod",
                                    "id",
                                    conn.as_str(),
                                    "-ipv4.addresses",
                                    cidr.as_str(),
                                ],
                                e
                            )))
                        }
                    }
                }
            } else {
                run_checked(
                    "ip",
                    &["addr", "del", &format!("{ip}/{prefix_len}"), "dev", target.name.as_str()],
                )
            }
        }
        NetIfOp::SetIpv4Dhcp { enable } => {
            let Some(conn) = nmcli_connection_for_dev(&target.name)? else {
                return Err(ForgeFfiError::unsupported(
                    "未检测到 NetworkManager（nmcli），无法通过本接口切换 DHCP；请使用系统网络管理工具".to_string(),
                ));
            };

            if *enable {
                nmcli_checked(&[
                    "con",
                    "mod",
                    "id",
                    conn.as_str(),
                    "ipv4.method",
                    "auto",
                ])?;
                nmcli_checked(&[
                    "con",
                    "mod",
                    "id",
                    conn.as_str(),
                    "ipv4.addresses",
                    "",
                ])?;
                nmcli_checked(&["con", "up", "id", conn.as_str()])
            } else {
                let addr = current_ipv4_cidr_for_dev(&target.name)?.ok_or_else(|| {
                    ForgeFfiError::invalid_argument(
                        "切换为手动前需要先有一个 IPv4 地址（当前未检测到）".to_string(),
                    )
                })?;

                nmcli_checked(&[
                    "con",
                    "mod",
                    "id",
                    conn.as_str(),
                    "ipv4.method",
                    "manual",
                ])?;
                nmcli_checked(&[
                    "con",
                    "mod",
                    "id",
                    conn.as_str(),
                    "ipv4.addresses",
                    addr.as_str(),
                ])?;
                if let Some(gw) = current_ipv4_gateway_for_dev(&target.name)? {
                    nmcli_checked(&[
                        "con",
                        "mod",
                        "id",
                        conn.as_str(),
                        "ipv4.gateway",
                        gw.as_str(),
                    ])?;
                }
                nmcli_checked(&["con", "up", "id", conn.as_str()])
            }
        }
        NetIfOp::SetIpv4Static {
            ip,
            prefix_len,
            gateway,
        } => {
            let cidr = format!("{ip}/{prefix_len}");
            let gw = gateway.as_deref();

            if let Some(conn) = nmcli_connection_for_dev(&target.name)? {
                nmcli_checked(&[
                    "con",
                    "mod",
                    "id",
                    conn.as_str(),
                    "ipv4.method",
                    "manual",
                    "ipv4.addresses",
                    cidr.as_str(),
                    "ipv4.gateway",
                    gw.unwrap_or(""),
                ])?;
                nmcli_checked(&["con", "up", "id", conn.as_str()])
            } else {
                apply_runtime_static_ipv4(&target.name, cidr.as_str(), gw)?;
                persist_systemd_networkd_static_ipv4(&target.name, cidr.as_str(), gw)?;
                Ok(())
            }
        }
    }
}

fn apply_runtime_static_ipv4(dev: &str, cidr: &str, gateway: Option<&str>) -> Result<(), ForgeFfiError> {
    run_checked("ip", &["addr", "flush", "dev", dev, "scope", "global"])?;
    run_checked("ip", &["addr", "add", cidr, "dev", dev])?;
    if let Some(gw) = gateway
        && !gw.is_empty()
    {
        run_checked("ip", &["route", "replace", "default", "via", gw, "dev", dev])?;
    }
    Ok(())
}

fn persist_systemd_networkd_static_ipv4(
    dev: &str,
    cidr: &str,
    gateway: Option<&str>,
) -> Result<(), ForgeFfiError> {
    let dir = Path::new("/etc/systemd/network");
    if !dir.is_dir() {
        return Err(ForgeFfiError::unsupported(
            "未检测到 NetworkManager（nmcli）且系统未使用 systemd-networkd（缺少 /etc/systemd/network），无法持久化；已通过 ip 命令临时生效".to_string(),
        ));
    }

    let file_name = format!("99-forgeffi-{dev}.network");
    let path = dir.join(file_name);
    let gw_line = gateway
        .filter(|s| !s.is_empty())
        .map(|s| format!("Gateway={s}\n"))
        .unwrap_or_default();

    let content = format!(
        "[Match]\nName={dev}\n\n[Network]\nDHCP=no\nAddress={cidr}\n{gw_line}",
    );

    write_atomic(&path, content.as_bytes()).map_err(map_io_error)
}

fn write_atomic(path: &Path, content: &[u8]) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("/"));
    let tmp = parent.join(format!(
        ".{}.tmp.{}",
        path.file_name().and_then(|s| s.to_str()).unwrap_or("forgeffi"),
        std::process::id()
    ));
    fs::write(&tmp, content)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

fn map_io_error(e: io::Error) -> ForgeFfiError {
    if e.kind() == io::ErrorKind::PermissionDenied {
        ForgeFfiError::permission_denied(e.to_string())
    } else {
        ForgeFfiError::system_error(e.to_string())
    }
}

fn nmcli_available() -> bool {
    static CACHED: OnceLock<bool> = OnceLock::new();
    *CACHED.get_or_init(|| {
        Command::new("nmcli")
            .arg("-v")
            .output()
            .is_ok_and(|o| o.status.success())
    })
}

fn nmcli_checked(args: &[&str]) -> Result<(), ForgeFfiError> {
    let out = Command::new("nmcli")
        .args(args)
        .output()
        .map_err(|e| ForgeFfiError::system_error(format!("执行 nmcli 失败: {e}")))?;
    if out.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr);
        Err(ForgeFfiError::system_error(format!(
            "nmcli 命令失败: nmcli {:?}: {}",
            args,
            stderr.trim()
        )))
    }
}

fn nmcli_try(args: &[&str]) -> Result<(), String> {
    let out = Command::new("nmcli").args(args).output();
    let Ok(out) = out else {
        return Err("执行 nmcli 失败".to_string());
    };
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

fn nmcli_connection_for_dev(dev: &str) -> Result<Option<String>, ForgeFfiError> {
    if !nmcli_available() {
        return Ok(None);
    }

    let out = Command::new("nmcli")
        .args(["-t", "-f", "GENERAL.CONNECTION", "dev", "show", dev])
        .output()
        .map_err(|e| ForgeFfiError::system_error(format!("执行 nmcli 失败: {e}")))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(ForgeFfiError::system_error(format!(
            "nmcli 查询连接失败: {}",
            stderr.trim()
        )));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().next().unwrap_or("").trim();
    let v = line
        .splitn(2, ':')
        .nth(1)
        .unwrap_or("")
        .trim();
    if v.is_empty() || v == "--" {
        Ok(None)
    } else {
        Ok(Some(v.to_string()))
    }
}

fn current_ipv4_cidr_for_dev(dev: &str) -> Result<Option<String>, ForgeFfiError> {
    let out = Command::new("ip")
        .args(["-j", "address", "show", "dev", dev])
        .output()
        .map_err(|e| ForgeFfiError::system_error(format!("执行 ip 命令失败: {e}")))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(ForgeFfiError::system_error(format!(
            "ip -j address show dev {dev} 失败: {}",
            stderr.trim()
        )));
    }

    let v: serde_json::Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| ForgeFfiError::system_error(format!("解析 ip JSON 失败: {e}")))?;
    let Some(arr) = v.as_array() else {
        return Ok(None);
    };
    let Some(first) = arr.first() else {
        return Ok(None);
    };
    let Some(addr_info) = first.get("addr_info").and_then(|x| x.as_array()) else {
        return Ok(None);
    };

    for a in addr_info {
        let family = a.get("family").and_then(|x| x.as_str()).unwrap_or("");
        if family != "inet" {
            continue;
        }
        let ip = a.get("local").and_then(|x| x.as_str()).unwrap_or("");
        if ip.is_empty() {
            continue;
        }
        if ip.starts_with("169.254.") {
            continue;
        }
        let prefix = a.get("prefixlen").and_then(|x| x.as_u64()).unwrap_or(0);
        if prefix == 0 || prefix > 32 {
            continue;
        }
        return Ok(Some(format!("{ip}/{prefix}")));
    }
    Ok(None)
}

fn current_ipv4_gateway_for_dev(dev: &str) -> Result<Option<String>, ForgeFfiError> {
    let out = Command::new("ip")
        .args(["-j", "route", "show", "default", "dev", dev])
        .output()
        .map_err(|e| ForgeFfiError::system_error(format!("执行 ip 命令失败: {e}")))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(ForgeFfiError::system_error(format!(
            "ip -j route show default dev {dev} 失败: {}",
            stderr.trim()
        )));
    }

    let v: serde_json::Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| ForgeFfiError::system_error(format!("解析 ip JSON 失败: {e}")))?;
    let Some(arr) = v.as_array() else {
        return Ok(None);
    };
    for r in arr {
        let gw = r.get("gateway").and_then(|x| x.as_str()).unwrap_or("");
        if !gw.is_empty() {
            return Ok(Some(gw.to_string()));
        }
    }
    Ok(None)
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

fn map_iface(i: IpIface) -> NetInterface {
    let mut flags = 0u32;
    for f in &i.flags {
        match f.as_str() {
            "UP" => flags |= IfaceFlags::UP,
            "LOWER_UP" => flags |= IfaceFlags::RUNNING,
            "RUNNING" => flags |= IfaceFlags::RUNNING,
            "LOOPBACK" => flags |= IfaceFlags::LOOPBACK,
            "BROADCAST" => flags |= IfaceFlags::BROADCAST,
            "MULTICAST" => flags |= IfaceFlags::MULTICAST,
            "POINTOPOINT" => flags |= IfaceFlags::POINT_TO_POINT,
            _ => {}
        }
    }

    let admin_state = if (flags & IfaceFlags::UP) != 0 {
        AdminState::Up
    } else {
        AdminState::Down
    };

    let oper_state = i.operstate.as_deref().map(map_oper_state);

    let (mut ipv4, mut ipv6) = (Vec::new(), Vec::new());
    for a in i.addr_info {
        let scope = a.scope.as_deref().map(map_scope);
        let mut addr_flags = 0u32;
        if a.temporary {
            addr_flags |= IpAddrFlags::TEMPORARY;
        }
        if a.deprecated {
            addr_flags |= IpAddrFlags::DEPRECATED;
        }
        if a.tentative {
            addr_flags |= IpAddrFlags::TENTATIVE;
        }

        let origin = if a.dynamic { Some(IpOrigin::Dhcp) } else { None };

        let ent = IpAddrEntry {
            ip: a.local,
            prefix_len: a.prefixlen,
            scope,
            origin,
            flags: if addr_flags == 0 { None } else { Some(IpAddrFlags(addr_flags)) },
        };
        if a.family == "inet" {
            ipv4.push(ent);
        } else if a.family == "inet6" {
            ipv6.push(ent);
        }
    }

    let kind = if i.ifname == "lo" || i.ifname.starts_with("lo") {
        IfaceKind::Loopback
    } else if i.ifname.starts_with("tun") {
        IfaceKind::Tunnel
    } else if i.ifname.starts_with("tap") {
        IfaceKind::Virtual
    } else {
        IfaceKind::Unknown
    };

    NetInterface {
        if_index: i.ifindex,
        name: i.ifname,
        display_name: None,
        kind,
        is_physical: None,
        admin_state,
        oper_state,
        flags: IfaceFlags(flags),
        mac: i.address,
        mtu: i.mtu,
        speed_bps: None,
        ipv4,
        ipv6,
        capabilities: NetIfCapabilities {
            can_set_admin_state: true,
            can_set_mtu: true,
            can_add_del_ip: true,
            can_set_dhcp: nmcli_available(),
            can_set_dns: false,
            notes: None,
        },
    }
}

fn map_oper_state(s: &str) -> OperState {
    match s {
        "UP" => OperState::Up,
        "DOWN" => OperState::Down,
        "DORMANT" => OperState::Dormant,
        "LOWERLAYERDOWN" => OperState::LowerLayerDown,
        _ => OperState::Unknown,
    }
}

fn map_scope(s: &str) -> IpScope {
    match s {
        "host" => IpScope::Host,
        "link" => IpScope::Link,
        "global" => IpScope::Global,
        "site" => IpScope::Site,
        _ => IpScope::Unknown,
    }
}
