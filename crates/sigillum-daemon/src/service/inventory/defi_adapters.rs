pub(super) const DEFI_EXIT_ADAPTER_AAVE_V3_WITHDRAW: &str = "aave-v3-withdraw";

pub(super) trait DefiExitAdapter {
    fn protocol(&self) -> &'static str;
    fn adapter_id(&self) -> &'static str;
}

pub(super) struct AaveV3WithdrawAdapter;

impl DefiExitAdapter for AaveV3WithdrawAdapter {
    fn protocol(&self) -> &'static str {
        "aave-v3"
    }

    fn adapter_id(&self) -> &'static str {
        DEFI_EXIT_ADAPTER_AAVE_V3_WITHDRAW
    }
}

pub(super) fn adapter_for_protocol(protocol: &str) -> Option<&'static str> {
    let adapter = AaveV3WithdrawAdapter;
    if protocol.eq_ignore_ascii_case(adapter.protocol()) {
        Some(adapter.adapter_id())
    } else {
        None
    }
}

pub(super) fn supported_defi_exit_adapter(adapter: &str) -> bool {
    adapter == DEFI_EXIT_ADAPTER_AAVE_V3_WITHDRAW
}
