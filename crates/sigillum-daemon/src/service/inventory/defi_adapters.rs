pub(super) const DEFI_EXIT_ADAPTER_AAVE_V3_WITHDRAW: &str = "aave-v3-withdraw";
pub(super) const DEFI_EXIT_ADAPTER_ERC4626_REDEEM: &str = "erc4626-redeem";
pub(super) const DEFI_EXIT_ADAPTER_LIDO_WSTETH_UNWRAP: &str = "lido-wsteth-unwrap";
pub(super) const DEFI_EXIT_ADAPTER_UNISWAP_V2_REMOVE_LIQUIDITY: &str =
    "uniswap-v2-remove-liquidity";

pub(super) trait DefiExitAdapter {
    fn protocol(&self) -> &'static str;
    fn adapter_id(&self) -> &'static str;
}

pub(super) struct AaveV3WithdrawAdapter;
pub(super) struct Erc4626RedeemAdapter;
pub(super) struct LidoWstethUnwrapAdapter;
pub(super) struct UniswapV2RemoveLiquidityAdapter;

impl DefiExitAdapter for AaveV3WithdrawAdapter {
    fn protocol(&self) -> &'static str {
        "aave-v3"
    }

    fn adapter_id(&self) -> &'static str {
        DEFI_EXIT_ADAPTER_AAVE_V3_WITHDRAW
    }
}

impl DefiExitAdapter for Erc4626RedeemAdapter {
    fn protocol(&self) -> &'static str {
        "erc4626"
    }

    fn adapter_id(&self) -> &'static str {
        DEFI_EXIT_ADAPTER_ERC4626_REDEEM
    }
}

impl DefiExitAdapter for LidoWstethUnwrapAdapter {
    fn protocol(&self) -> &'static str {
        "lido-wsteth"
    }

    fn adapter_id(&self) -> &'static str {
        DEFI_EXIT_ADAPTER_LIDO_WSTETH_UNWRAP
    }
}

impl DefiExitAdapter for UniswapV2RemoveLiquidityAdapter {
    fn protocol(&self) -> &'static str {
        "uniswap-v2"
    }

    fn adapter_id(&self) -> &'static str {
        DEFI_EXIT_ADAPTER_UNISWAP_V2_REMOVE_LIQUIDITY
    }
}

pub(super) fn adapter_for_protocol(protocol: &str) -> Option<&'static str> {
    let adapters: [&dyn DefiExitAdapter; 4] = [
        &AaveV3WithdrawAdapter,
        &Erc4626RedeemAdapter,
        &LidoWstethUnwrapAdapter,
        &UniswapV2RemoveLiquidityAdapter,
    ];
    adapters
        .iter()
        .find(|adapter| protocol.eq_ignore_ascii_case(adapter.protocol()))
        .map(|adapter| adapter.adapter_id())
}

pub(super) fn supported_defi_exit_adapter(adapter: &str) -> bool {
    matches!(
        adapter,
        DEFI_EXIT_ADAPTER_AAVE_V3_WITHDRAW
            | DEFI_EXIT_ADAPTER_ERC4626_REDEEM
            | DEFI_EXIT_ADAPTER_LIDO_WSTETH_UNWRAP
            | DEFI_EXIT_ADAPTER_UNISWAP_V2_REMOVE_LIQUIDITY
    )
}
