use crate::{
    error::{AppError, Result},
    extractor::{RawBlock, RawLog, RawTransaction},
    hex::encode_prefixed,
    proto::{Block, BlockRef},
};

/// Purpose: proto 블록을 extractor 입력 모델로 변환
/// Param:
/// - `block`: 변환할 proto block
pub fn raw_block(block: Block) -> Result<RawBlock> {
    Ok(RawBlock {
        chain_id: i32::try_from(block.chain_id)
            .map_err(|error| AppError::with_source("chain_id does not fit in i32", error))?,
        block_number: i64::try_from(block.number)
            .map_err(|error| AppError::with_source("block number does not fit in i64", error))?,
        block_hash: format_hash(&block.hash)?,
        parent_hash: format_hash(&block.parent_hash)?,
        block_timestamp: block.timestamp.to_string(),
        transactions: block
            .transactions
            .into_iter()
            .map(|transaction| {
                Ok(RawTransaction {
                    tx_hash: format_hash(&transaction.hash)?,
                    tx_index: i32::try_from(transaction.index).map_err(|error| {
                        AppError::with_source("transaction index does not fit in i32", error)
                    })?,
                    from_address: format_address(&transaction.from)?,
                    to_address: transaction.to.as_deref().map(format_address).transpose()?,
                    value_raw: transaction.value_raw,
                    logs: transaction
                        .logs
                        .into_iter()
                        .map(|log| {
                            Ok(RawLog {
                                log_index: i32::try_from(log.index).map_err(|error| {
                                    AppError::with_source("log index does not fit in i32", error)
                                })?,
                                contract_address: format_address(&log.contract_address)?,
                                topics: log
                                    .topics
                                    .iter()
                                    .map(|topic| format_hash(topic))
                                    .collect::<Result<Vec<_>>>()?,
                                data: encode_prefixed(&log.data),
                            })
                        })
                        .collect::<Result<Vec<_>>>()?,
                })
            })
            .collect::<Result<Vec<_>>>()?,
    })
}

/// Purpose: block ref hash를 0x hex 문자열로 변환
/// Param:
/// - `block_ref`: 변환할 block ref
pub fn block_ref_hash(block_ref: &BlockRef) -> Result<String> {
    format_hash(&block_ref.hash)
}

/// Purpose: 32바이트 해시를 0x hex 문자열로 변환
/// Param:
/// - `bytes`: 32-byte hash bytes
pub fn format_hash(bytes: &[u8]) -> Result<String> {
    if bytes.len() != 32 {
        return Err(AppError::msg(format!(
            "invalid block hash length: expected 32 bytes, got {}",
            bytes.len()
        )));
    }

    Ok(encode_prefixed(bytes))
}

/// Purpose: 20바이트 주소를 0x hex 문자열로 변환
/// Param:
/// - `bytes`: 20-byte address bytes
fn format_address(bytes: &[u8]) -> Result<String> {
    if bytes.len() != 20 {
        return Err(AppError::msg(format!(
            "invalid address length: expected 20 bytes, got {}",
            bytes.len()
        )));
    }

    Ok(encode_prefixed(bytes))
}
