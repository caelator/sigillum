use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConsolidationPlanExportCall {
    pub step_id: String,
    pub action: String,
    pub from_address: String,
    pub to_address: String,
    pub value_wei_hex: String,
    pub data_hex: String,
    pub operation: u8,
    pub chain_id: u64,
    pub provider_profile: String,
    pub asset_kind: String,
    pub amount_hex: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SafeTransactionBuilderTransaction {
    pub to: String,
    pub value: String,
    pub data: String,
    pub operation: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SafeTransactionBuilderMeta {
    pub name: String,
    pub description: String,
    #[serde(rename = "txBuilderVersion")]
    pub tx_builder_version: String,
    #[serde(
        rename = "createdFromSafeAddress",
        skip_serializing_if = "Option::is_none"
    )]
    pub created_from_safe_address: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SafeTransactionBuilderBatch {
    pub version: String,
    #[serde(rename = "chainId")]
    pub chain_id: String,
    pub meta: SafeTransactionBuilderMeta,
    pub transactions: Vec<SafeTransactionBuilderTransaction>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConsolidationPlanExportBundle {
    pub chain_id: u64,
    pub provider_profile: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub safe_address: Option<String>,
    pub calls: Vec<ConsolidationPlanExportCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub safe_transaction_builder: Option<SafeTransactionBuilderBatch>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConsolidationPlanExportSkippedStep {
    pub step_id: String,
    pub action: String,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blockers: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConsolidationPlanExportResponse {
    pub status: String,
    pub plan_id: String,
    pub format: String,
    pub exported_steps: usize,
    pub skipped_steps: Vec<ConsolidationPlanExportSkippedStep>,
    pub bundles: Vec<ConsolidationPlanExportBundle>,
}
