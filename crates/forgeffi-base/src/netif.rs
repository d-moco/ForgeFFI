use serde::{Deserialize, Serialize};

use crate::{ErrorCode, ForgeFfiError, ABI_VERSION};

#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IfaceKind {
    Unknown,
    Physical,
    Virtual,
    Loopback,
    Tunnel,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdminState {
    Unknown,
    Up,
    Down,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperState {
    Unknown,
    Up,
    Down,
    Dormant,
    LowerLayerDown,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct IfaceFlags(pub u32);

impl IfaceFlags {
    pub const UP: u32 = 1 << 0;
    pub const RUNNING: u32 = 1 << 1;
    pub const LOOPBACK: u32 = 1 << 2;
    pub const BROADCAST: u32 = 1 << 3;
    pub const MULTICAST: u32 = 1 << 4;
    pub const POINT_TO_POINT: u32 = 1 << 5;
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IpScope {
    Unknown,
    Host,
    Link,
    Site,
    Global,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IpOrigin {
    Unknown,
    Static,
    Dhcp,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct IpAddrFlags(pub u32);

impl IpAddrFlags {
    pub const TEMPORARY: u32 = 1 << 0;
    pub const DEPRECATED: u32 = 1 << 1;
    pub const TENTATIVE: u32 = 1 << 2;
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct IpAddrEntry {
    pub ip: String,
    pub prefix_len: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<IpScope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<IpOrigin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flags: Option<IpAddrFlags>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NetIfCapabilities {
    pub can_set_admin_state: bool,
    pub can_set_mtu: bool,
    pub can_add_del_ip: bool,
    pub can_set_dhcp: bool,
    pub can_set_dns: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NetInterface {
    pub if_index: u32,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub kind: IfaceKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_physical: Option<bool>,
    pub admin_state: AdminState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oper_state: Option<OperState>,
    pub flags: IfaceFlags,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mac: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mtu: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_bps: Option<u64>,
    #[serde(default)]
    pub ipv4: Vec<IpAddrEntry>,
    #[serde(default)]
    pub ipv6: Vec<IpAddrEntry>,
    pub capabilities: NetIfCapabilities,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct IfaceSelector {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub if_index: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum NetIfOp {
    SetAdminState { up: bool },
    SetMtu { mtu: u32 },
    AddIp { ip: String, prefix_len: u8 },
    DelIp { ip: String, prefix_len: u8 },
    SetIpv4Dhcp { enable: bool },
    SetIpv4Static {
        ip: String,
        prefix_len: u8,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        gateway: Option<String>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NetIfOpResult {
    pub i: usize,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ForgeFfiError>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NetIfListResponse {
    pub abi: u32,
    pub items: Vec<NetInterface>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NetIfApplyRequest {
    pub abi: u32,
    pub target: IfaceSelector,
    pub ops: Vec<NetIfOp>,
}

impl NetIfApplyRequest {
    #[must_use]
    pub fn v1(target: IfaceSelector, ops: Vec<NetIfOp>) -> Self {
        Self {
            abi: ABI_VERSION,
            target,
            ops,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NetIfApplyResponse {
    pub abi: u32,
    pub ok: bool,
    pub results: Vec<NetIfOpResult>,
}

impl NetIfApplyResponse {
    #[must_use]
    pub fn error(abi: u32, e: ForgeFfiError) -> Self {
        Self {
            abi,
            ok: false,
            results: vec![NetIfOpResult {
                i: 0,
                ok: false,
                error: Some(e),
            }],
        }
    }

    #[must_use]
    pub fn invalid_abi(expected: u32, got: u32) -> Self {
        Self::error(
            expected,
            ForgeFfiError {
                code: ErrorCode::InvalidArgument,
                message: format!("abi 版本不匹配: expected={expected} got={got}"),
            },
        )
    }
}
