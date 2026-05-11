use crate::{
    error::Result,
    models::{AssetMovement, AssetType, SourceType},
};

use super::{
    RawBlock, RawTransaction,
    abi::normalize_uint,
    hex::{normalize_address, normalize_hash},
};

/// Purpose: 트랜잭션 value 기반 native 자산 이동 생성
/// Param:
/// - `block`: transaction 포함 RawBlock
/// - `transaction`: native value 포함 RawTransaction
pub(super) fn native_movement(
    block: &RawBlock,
    transaction: &RawTransaction,
) -> Result<AssetMovement> {
    Ok(AssetMovement {
        chain_id: block.chain_id,
        block_number: block.block_number,
        block_hash: normalize_hash(&block.block_hash)?,
        block_timestamp: block.block_timestamp.clone(),
        tx_hash: normalize_hash(&transaction.tx_hash)?,
        tx_index: transaction.tx_index,
        source_type: SourceType::TxValue,
        asset_type: AssetType::Native,
        token_address: None,
        from_address: normalize_address(&transaction.from_address)?,
        to_address: transaction
            .to_address
            .as_deref()
            .map(normalize_address)
            .transpose()?,
        token_id: None,
        amount_raw: normalize_uint(&transaction.value_raw)?,
        log_index: None,
        log_sub_index: 0,
    })
}
