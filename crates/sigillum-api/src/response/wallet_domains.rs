use serde::{Deserialize, Serialize};

macro_rules! wire_string_enum {
    (
        $(#[$meta:meta])*
        pub enum $name:ident {
            $($variant:ident => $literal:literal,)+
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum $name {
            $($variant,)+
            Other(String),
        }

        impl $name {
            pub fn as_str(&self) -> &str {
                match self {
                    $(Self::$variant => $literal,)+
                    Self::Other(value) => value.as_str(),
                }
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                match value {
                    $($literal => Self::$variant,)+
                    other => Self::Other(other.to_string()),
                }
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self::from(value.as_str())
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl PartialEq<&str> for $name {
            fn eq(&self, other: &&str) -> bool {
                self.as_str() == *other
            }
        }

        impl PartialEq<$name> for &str {
            fn eq(&self, other: &$name) -> bool {
                *self == other.as_str()
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                String::deserialize(deserializer).map(Self::from)
            }
        }
    };
}

wire_string_enum! {
    pub enum WalletAddressActivityState {
        Funded => "funded",
        Active => "active",
        Empty => "empty",
    }
}

wire_string_enum! {
    pub enum WalletAddressClassification {
        SignerAvailable => "signer_available",
        WatchOnly => "watch_only",
        SignerUnknown => "signer_unknown",
        GasAvailable => "gas_available",
        TransactionHistory => "transaction_history",
        TokenHolding => "token_holding",
        NftHolding => "nft_holding",
        ProtocolHolding => "protocol_holding",
        ValueDetected => "value_detected",
        AssetValueDetected => "asset_value_detected",
        StrandedValue => "stranded_value",
        ApprovalExposure => "approval_exposure",
        DormantCandidate => "dormant_candidate",
        EmptyCandidate => "empty_candidate",
    }
}

wire_string_enum! {
    pub enum WalletAssetKind {
        Native => "native",
        Erc20 => "erc20",
        Erc721 => "erc721",
        Erc1155 => "erc1155",
        Nft => "nft",
        Approval => "approval",
        Defi => "defi",
        Airdrop => "airdrop",
        Reward => "reward",
    }
}

wire_string_enum! {
    pub enum WalletPlanStepAction {
        SweepNative => "sweep_native",
        SweepErc20 => "sweep_erc20",
        SweepNft => "sweep_nft",
        RevokeErc20Approval => "revoke_erc20_approval",
        RevokePermit2Allowance => "revoke_permit2_allowance",
        RevokeNftOperatorApproval => "revoke_nft_operator_approval",
        RevokeApproval => "revoke_approval",
        ExitDefiPosition => "exit_defi_position",
        ClaimReward => "claim_reward",
        ReviewAsset => "review_asset",
    }
}

wire_string_enum! {
    pub enum WalletPlanStepStatus {
        ReviewRequired => "review_required",
        Blocked => "blocked",
        Approved => "approved",
    }
}

wire_string_enum! {
    pub enum WalletSignerStatus {
        WatchOnly => "watch_only",
        Available => "available",
        Unknown => "unknown",
    }
}

wire_string_enum! {
    pub enum WalletSimulationStatus {
        Required => "required",
        NotRun => "not_run",
        Passed => "passed",
        Failed => "failed",
        Unsupported => "unsupported",
        Blocked => "blocked",
    }
}

wire_string_enum! {
    pub enum WalletPlanStatus {
        Empty => "empty",
        Blocked => "blocked",
        ReviewRequired => "review_required",
        Approved => "approved",
    }
}
