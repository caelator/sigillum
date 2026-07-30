use std::collections::BTreeMap;

use sigillum_api::{WalletDiscoveryBlockCursor, WalletDiscoveryCheckpoint, WalletDiscoveryJob};

use super::wallet_selection::{DERIVATION_PATTERN_PROJECT, DiscoveryWallet};

pub(super) const TOPIC_FAMILY_ERC20_TRANSFER: &str = "erc20-transfer";
pub(super) const TOPIC_FAMILY_ERC721_TRANSFER: &str = "erc721-transfer";
pub(super) const TOPIC_FAMILY_ERC1155_TRANSFER: &str = "erc1155-transfer";

pub(super) struct ScanCheckpointProgress {
    pub(super) next_index: u32,
    pub(super) last_scanned_index: Option<u32>,
    pub(super) consecutive_empty: u32,
    pub(super) completed: bool,
    pub(super) updated_at_unix: u64,
}

pub(super) struct BlockCursorProgress<'a> {
    pub(super) address: &'a str,
    pub(super) chain_id: u64,
    pub(super) topic_family: &'a str,
    pub(super) last_scanned_block: u64,
    pub(super) updated_at_unix: u64,
}

pub(super) fn latest_block_scan_cursors(
    jobs: &[WalletDiscoveryJob],
    chain_ids: impl IntoIterator<Item = u64>,
) -> Vec<WalletDiscoveryBlockCursor> {
    let selected_chains = chain_ids.into_iter().collect::<Vec<_>>();
    let mut cursors: BTreeMap<(String, u64, String), WalletDiscoveryBlockCursor> = BTreeMap::new();
    for cursor in jobs
        .iter()
        .flat_map(|job| job.block_cursors.iter())
        .filter(|cursor| selected_chains.contains(&cursor.chain_id))
    {
        cursors.insert(
            (
                cursor.address.to_ascii_lowercase(),
                cursor.chain_id,
                cursor.topic_family.clone(),
            ),
            cursor.clone(),
        );
    }
    cursors.into_values().collect()
}

pub(super) fn update_scan_checkpoint(
    checkpoints: &mut Vec<WalletDiscoveryCheckpoint>,
    wallet: &DiscoveryWallet,
    provider: &sigillum_api::EvmProviderProfile,
    progress: ScanCheckpointProgress,
) {
    let next = WalletDiscoveryCheckpoint {
        wallet_family: wallet.family.clone(),
        wallet_profile: wallet.profile.clone(),
        provider_profile: provider.name.clone(),
        derivation_pattern: Some(wallet.derivation_pattern.clone()),
        account_index: Some(wallet.account_index),
        next_index: progress.next_index,
        last_scanned_index: progress.last_scanned_index,
        consecutive_empty: progress.consecutive_empty,
        completed: progress.completed,
        updated_at_unix: progress.updated_at_unix,
    };
    if let Some(existing) = checkpoints.iter_mut().find(|existing| {
        existing.wallet_family == next.wallet_family
            && existing.wallet_profile == next.wallet_profile
            && existing.provider_profile == next.provider_profile
            && existing
                .derivation_pattern
                .as_deref()
                .unwrap_or(DERIVATION_PATTERN_PROJECT)
                == next
                    .derivation_pattern
                    .as_deref()
                    .unwrap_or(DERIVATION_PATTERN_PROJECT)
            && existing
                .account_index
                .unwrap_or(next.account_index.unwrap_or(0))
                == next.account_index.unwrap_or(0)
    }) {
        *existing = next;
    } else {
        checkpoints.push(next);
    }
}

pub(super) fn latest_cursor_block(
    cursors: &[WalletDiscoveryBlockCursor],
    address: &str,
    chain_id: u64,
    topic_family: &str,
) -> Option<u64> {
    cursors
        .iter()
        .filter(|cursor| {
            cursor.chain_id == chain_id
                && cursor.topic_family == topic_family
                && cursor.address.eq_ignore_ascii_case(address)
        })
        .map(|cursor| cursor.last_scanned_block)
        .max()
}

pub(super) fn effective_from_block(config_from_block: &str, cursor_block: Option<u64>) -> String {
    let Some(cursor_block) = cursor_block else {
        return config_from_block.to_string();
    };
    let next_block = cursor_block.saturating_add(1);
    match parse_block_quantity(config_from_block) {
        Some(config_from) if config_from > next_block => encode_block_quantity(config_from),
        _ => encode_block_quantity(next_block),
    }
}

pub(super) fn update_block_cursor(
    cursors: &mut Vec<WalletDiscoveryBlockCursor>,
    progress: BlockCursorProgress<'_>,
) {
    let next = WalletDiscoveryBlockCursor {
        address: progress.address.to_ascii_lowercase(),
        chain_id: progress.chain_id,
        topic_family: progress.topic_family.to_string(),
        last_scanned_block: progress.last_scanned_block,
        updated_at_unix: progress.updated_at_unix,
    };
    if let Some(existing) = cursors.iter_mut().find(|existing| {
        existing.chain_id == next.chain_id
            && existing.topic_family == next.topic_family
            && existing.address.eq_ignore_ascii_case(&next.address)
    }) {
        existing.address = next.address;
        existing.updated_at_unix = next.updated_at_unix;
        existing.last_scanned_block = existing.last_scanned_block.max(next.last_scanned_block);
    } else {
        cursors.push(next);
    }
}

pub(super) fn parse_block_quantity(value: &str) -> Option<u64> {
    let raw = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))?;
    if raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    u64::from_str_radix(raw, 16).ok()
}

pub(super) fn encode_block_quantity(value: u64) -> String {
    format!("0x{value:x}")
}

pub(super) fn sync_inventory_job(
    inventory: &mut crate::inventory::WalletInventoryState,
    job: &WalletDiscoveryJob,
) {
    if let Some(existing) = inventory
        .jobs
        .iter_mut()
        .find(|existing| existing.id == job.id)
    {
        *existing = job.clone();
    } else {
        inventory.jobs.push(job.clone());
    }
}

#[cfg(test)]
mod tests {
    use sigillum_api::WalletDiscoveryBlockCursor;

    use super::{
        BlockCursorProgress, TOPIC_FAMILY_ERC20_TRANSFER, latest_cursor_block, update_block_cursor,
    };

    #[test]
    fn block_cursor_updates_are_monotonic() {
        let mut cursors = vec![WalletDiscoveryBlockCursor {
            address: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            chain_id: 1,
            topic_family: TOPIC_FAMILY_ERC20_TRANSFER.to_string(),
            last_scanned_block: 100,
            updated_at_unix: 10,
        }];

        update_block_cursor(
            &mut cursors,
            BlockCursorProgress {
                address: "0xAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
                chain_id: 1,
                topic_family: TOPIC_FAMILY_ERC20_TRANSFER,
                last_scanned_block: 90,
                updated_at_unix: 20,
            },
        );

        assert_eq!(
            latest_cursor_block(
                &cursors,
                "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                1,
                TOPIC_FAMILY_ERC20_TRANSFER
            ),
            Some(100)
        );
        assert_eq!(cursors[0].updated_at_unix, 20);
    }
}
