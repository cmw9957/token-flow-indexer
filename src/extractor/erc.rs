use crate::{
    error::Result,
    models::{AssetMovement, AssetType},
};

use super::{
    RawBlock, RawLog, RawTransaction,
    abi::normalize_uint,
    build_log_movement,
    erc1155::{erc1155_batch_movements, erc1155_single_movement},
    normalize::topic_address,
};

pub(super) const TRANSFER_TOPIC: &str =
    "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";
const TRANSFER_SINGLE_TOPIC: &str =
    "0xc3d58168c5ae7397731d063d5bbf3d601eaf52b74f8d4c987e20ee57f798f";
const TRANSFER_BATCH_TOPIC: &str =
    "0x4a39dc06d4c0dbc64b70a9e5e3c6760d4e5f0d5e4f7f2d1e1c4e481e7fdef7";

/// Purpose: 로그 topic 기준 ERC 자산 이동 추출
/// Param:
/// - `block`: log 포함 RawBlock
/// - `transaction`: log 포함 RawTransaction
/// - `log`: 분석할 RawLog
pub(super) fn log_movements(
    block: &RawBlock,
    transaction: &RawTransaction,
    log: &RawLog,
) -> Result<Vec<AssetMovement>> {
    let Some(topic0) = log.topics.first() else {
        return Ok(Vec::new());
    };

    match topic0.to_ascii_lowercase().as_str() {
        TRANSFER_TOPIC => erc_transfer_movement(block, transaction, log)
            .map(|movement| movement.map_or_else(Vec::new, |movement| vec![movement])),
        TRANSFER_SINGLE_TOPIC => erc1155_single_movement(block, transaction, log)
            .map(|movement| movement.map_or_else(Vec::new, |movement| vec![movement])),
        TRANSFER_BATCH_TOPIC => erc1155_batch_movements(block, transaction, log),
        _ => Ok(Vec::new()),
    }
}

/// Purpose: ERC20/ERC721 Transfer 로그를 자산 이동으로 변환
/// Param:
/// - `block`: log 포함 RawBlock
/// - `transaction`: log 포함 RawTransaction
/// - `log`: Transfer RawLog
fn erc_transfer_movement(
    block: &RawBlock,
    transaction: &RawTransaction,
    log: &RawLog,
) -> Result<Option<AssetMovement>> {
    if log.topics.len() == 3 {
        return Ok(Some(build_log_movement(
            block,
            transaction,
            log,
            AssetType::Erc20,
            topic_address(&log.topics[1])?,
            topic_address(&log.topics[2])?,
            None,
            normalize_uint(&log.data)?,
            0,
        )?));
    }

    if log.topics.len() == 4 {
        return Ok(Some(build_log_movement(
            block,
            transaction,
            log,
            AssetType::Erc721,
            topic_address(&log.topics[1])?,
            topic_address(&log.topics[2])?,
            Some(super::normalize::topic_uint(&log.topics[3])?),
            "1".to_owned(),
            0,
        )?));
    }

    Ok(None)
}
