use super::*;

pub(super) fn list_interfaces() -> Result<Vec<NetInterface>, ForgeFfiError> {
    Err(ForgeFfiError::unsupported("当前平台暂不支持 netif".to_string()))
}

pub(super) fn apply_one(_target: &ResolvedTarget, _op: &NetIfOp) -> Result<(), ForgeFfiError> {
    Err(ForgeFfiError::unsupported("当前平台暂不支持 netif".to_string()))
}

