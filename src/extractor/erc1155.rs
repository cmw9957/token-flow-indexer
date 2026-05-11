use crate::{
    error::Result,
    models::{AssetMovement, AssetType},
};

use super::{
    RawBlock, RawLog, RawTransaction,
    abi::{data_words, decode_two_uint_arrays, hex_word_to_decimal},
    base_log_movement,
    normalize::topic_address,
};

/// Purpose: ERC1155 TransferSingle 로그를 자산 이동으로 변환
/// Param:
/// - `block`: log 포함 RawBlock
/// - `transaction`: log 포함 RawTransaction
/// - `log`: TransferSingle RawLog
pub(super) fn erc1155_single_movement(
    block: &RawBlock,
    transaction: &RawTransaction,
    log: &RawLog,
) -> Result<Option<AssetMovement>> {
    if log.topics.len() != 4 {
        return Ok(None);
    }

    let words = data_words(&log.data)?;
    if words.len() != 2 {
        return Ok(None);
    }

    Ok(Some(base_log_movement(
        block,
        transaction,
        log,
        AssetType::Erc1155,
        topic_address(&log.topics[2])?,
        topic_address(&log.topics[3])?,
        Some(hex_word_to_decimal(&words[0])?),
        hex_word_to_decimal(&words[1])?,
        0,
    )?))
}

/// Purpose: ERC1155 TransferBatch 로그를 자산 이동 목록으로 변환
/// Param:
/// - `block`: log 포함 RawBlock
/// - `transaction`: log 포함 RawTransaction
/// - `log`: TransferBatch RawLog
pub(super) fn erc1155_batch_movements(
    block: &RawBlock,
    transaction: &RawTransaction,
    log: &RawLog,
) -> Result<Vec<AssetMovement>> {
    if log.topics.len() != 4 {
        return Ok(Vec::new());
    }

    let words = data_words(&log.data)?;
    let (ids, values) = decode_two_uint_arrays(&words)?;
    if ids.len() != values.len() {
        return Ok(Vec::new());
    }

    let mut movements = Vec::with_capacity(ids.len());
    for (index, (token_id, amount)) in ids.into_iter().zip(values).enumerate() {
        movements.push(base_log_movement(
            block,
            transaction,
            log,
            AssetType::Erc1155,
            topic_address(&log.topics[2])?,
            topic_address(&log.topics[3])?,
            Some(token_id),
            amount,
            index as i32,
        )?);
    }

    Ok(movements)
}
