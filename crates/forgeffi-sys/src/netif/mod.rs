use forgeffi_base::{
    ForgeFfiError, IfaceSelector, NetIfApplyRequest, NetIfApplyResponse, NetIfListResponse, NetIfOp,
    NetIfOpResult, NetInterface, ABI_VERSION,
};

#[cfg(target_os = "linux")]
mod platform_linux;
#[cfg(target_os = "macos")]
mod platform_macos;
#[cfg(target_os = "windows")]
mod platform_windows;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod platform_unsupported;

#[cfg(target_os = "linux")]
use platform_linux as platform;
#[cfg(target_os = "macos")]
use platform_macos as platform;
#[cfg(target_os = "windows")]
use platform_windows as platform;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
use platform_unsupported as platform;

pub const NETIF_ABI_VERSION: u32 = ABI_VERSION;

pub fn list_interfaces() -> Result<Vec<NetInterface>, ForgeFfiError> {
    platform::list_interfaces()
}

pub fn list_response() -> Result<NetIfListResponse, ForgeFfiError> {
    Ok(NetIfListResponse {
        abi: NETIF_ABI_VERSION,
        items: list_interfaces()?,
    })
}

pub fn list_json_bytes() -> Result<Vec<u8>, ForgeFfiError> {
    let resp = list_response()?;
    serde_json::to_vec(&resp)
        .map_err(|e| ForgeFfiError::system_error(format!("序列化 list 响应失败: {e}")))
}

pub fn apply_request(req: NetIfApplyRequest) -> Result<NetIfApplyResponse, ForgeFfiError> {
    if req.abi != NETIF_ABI_VERSION {
        return Err(ForgeFfiError::invalid_argument(format!(
            "abi 版本不匹配: expected={} got={}"
            , NETIF_ABI_VERSION, req.abi
        )));
    }

    let ifaces = list_interfaces()?;
    let target = resolve_target(&req.target, &ifaces)?;

    let mut results = Vec::with_capacity(req.ops.len());
    let mut all_ok = true;

    for (i, op) in req.ops.iter().cloned().enumerate() {
        let r = validate_op(&op).and_then(|_| platform::apply_one(&target, &op));
        match r {
            Ok(()) => results.push(NetIfOpResult {
                i,
                ok: true,
                error: None,
            }),
            Err(e) => {
                all_ok = false;
                results.push(NetIfOpResult {
                    i,
                    ok: false,
                    error: Some(e),
                });
            }
        }
    }

    Ok(NetIfApplyResponse {
        abi: NETIF_ABI_VERSION,
        ok: all_ok,
        results,
    })
}

pub fn apply_json_bytes(req_json: &str) -> Result<Vec<u8>, ForgeFfiError> {
    let req: NetIfApplyRequest = serde_json::from_str(req_json)
        .map_err(|e| ForgeFfiError::invalid_argument(format!("解析请求 JSON 失败: {e}")))?;
    let resp = apply_request(req)?;
    serde_json::to_vec(&resp)
        .map_err(|e| ForgeFfiError::system_error(format!("序列化 apply 响应失败: {e}")))
}

#[cfg(target_os = "windows")]
#[derive(Clone, Debug)]
struct ResolvedTarget {
    if_index: u32,
    name: String,
}

#[cfg(not(target_os = "windows"))]
#[derive(Clone, Debug)]
struct ResolvedTarget {
    name: String,
}

fn resolve_target(sel: &IfaceSelector, ifaces: &[NetInterface]) -> Result<ResolvedTarget, ForgeFfiError> {
    if let Some(idx) = sel.if_index
        && idx != 0
    {
        if let Some(i) = ifaces.iter().find(|it| it.if_index == idx) {
            return Ok(ResolvedTarget {
                #[cfg(target_os = "windows")]
                if_index: i.if_index,
                name: i.name.clone(),
            });
        }
        return Err(ForgeFfiError::not_found(format!("未找到网卡 if_index={idx}")));
    }

    if let Some(ref name) = sel.name {
        if let Some(i) = ifaces.iter().find(|it| it.name == *name) {
            return Ok(ResolvedTarget {
                #[cfg(target_os = "windows")]
                if_index: i.if_index,
                name: i.name.clone(),
            });
        }
        return Err(ForgeFfiError::not_found(format!("未找到网卡 name={name}")));
    }

    Err(ForgeFfiError::invalid_argument(
        "target 必须至少包含 if_index 或 name".to_string(),
    ))
}

fn validate_op(op: &NetIfOp) -> Result<(), ForgeFfiError> {
    match op {
        NetIfOp::SetAdminState { .. } => Ok(()),
        NetIfOp::SetMtu { mtu } => {
            if *mtu == 0 {
                return Err(ForgeFfiError::invalid_argument("mtu 不能为 0"));
            }
            Ok(())
        }
        NetIfOp::AddIp { ip, prefix_len } => {
            if *prefix_len == 0 {
                return Err(ForgeFfiError::invalid_argument(
                    "添加 IP 不允许 prefix_len=0".to_string(),
                ));
            }
            let addr: std::net::IpAddr = ip
                .parse()
                .map_err(|_| ForgeFfiError::invalid_argument(format!("非法 IP: {ip}")))?;
            match addr {
                std::net::IpAddr::V4(_) => {
                    if *prefix_len > 32 {
                        return Err(ForgeFfiError::invalid_argument(
                            "IPv4 prefix_len 必须在 0..=32".to_string(),
                        ));
                    }
                }
                std::net::IpAddr::V6(_) => {
                    if *prefix_len > 128 {
                        return Err(ForgeFfiError::invalid_argument(
                            "IPv6 prefix_len 必须在 0..=128".to_string(),
                        ));
                    }
                }
            }
            Ok(())
        }
        NetIfOp::DelIp { ip, prefix_len } => {
            let addr: std::net::IpAddr = ip
                .parse()
                .map_err(|_| ForgeFfiError::invalid_argument(format!("非法 IP: {ip}")))?;
            match addr {
                std::net::IpAddr::V4(_) => {
                    if *prefix_len > 32 {
                        return Err(ForgeFfiError::invalid_argument(
                            "IPv4 prefix_len 必须在 0..=32".to_string(),
                        ));
                    }
                }
                std::net::IpAddr::V6(_) => {
                    if *prefix_len > 128 {
                        return Err(ForgeFfiError::invalid_argument(
                            "IPv6 prefix_len 必须在 0..=128".to_string(),
                        ));
                    }
                }
            }
            Ok(())
        }
        NetIfOp::SetIpv4Dhcp { .. } => Ok(()),
        NetIfOp::SetIpv4Static {
            ip,
            prefix_len,
            gateway,
        } => {
            if *prefix_len == 0 {
                return Err(ForgeFfiError::invalid_argument(
                    "IPv4 prefix_len 必须在 1..=32".to_string(),
                ));
            }
            if *prefix_len > 32 {
                return Err(ForgeFfiError::invalid_argument(
                    "IPv4 prefix_len 必须在 1..=32".to_string(),
                ));
            }
            let addr: std::net::IpAddr = ip
                .parse()
                .map_err(|_| ForgeFfiError::invalid_argument(format!("非法 IP: {ip}")))?;
            if !matches!(addr, std::net::IpAddr::V4(_)) {
                return Err(ForgeFfiError::invalid_argument(
                    "SetIpv4Static 仅支持 IPv4".to_string(),
                ));
            }
            if let Some(gw) = gateway {
                let gw_addr: std::net::IpAddr = gw
                    .parse()
                    .map_err(|_| ForgeFfiError::invalid_argument(format!("非法网关: {gw}")))?;
                if !matches!(gw_addr, std::net::IpAddr::V4(_)) {
                    return Err(ForgeFfiError::invalid_argument(
                        "网关必须是 IPv4".to_string(),
                    ));
                }
            }
            Ok(())
        }
    }
}
