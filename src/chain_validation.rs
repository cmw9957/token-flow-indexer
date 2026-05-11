use crate::{
    error::{AppError, Result},
    proto::Block,
    proto_convert::format_hash,
};

/// Purpose: 체크포인트와 첫 새 블록의 연속성 확인
/// Param:
/// - `last_indexed_block`: checkpoint block number
/// - `last_indexed_hash`: checkpoint block hash
/// - `first_block`: 검증 대상 첫 block
pub fn ensure_checkpoint_continuity(
    last_indexed_block: i64,
    last_indexed_hash: &str,
    first_block: &Block,
) -> Result<()> {
    let expected_next = last_indexed_block + 1;
    let first_number = i64::try_from(first_block.number)
        .map_err(|error| AppError::with_source("block number does not fit in i64", error))?;
    let parent_hash = format_hash(&first_block.parent_hash)?;

    if first_number != expected_next {
        return Err(AppError::msg(format!(
            "gap detected: checkpoint is at block {last_indexed_block}, \
             but first new block is {first_number}"
        )));
    }

    if parent_hash != last_indexed_hash {
        return Err(AppError::msg(format!(
            "chain continuity mismatch at block {first_number}: \
             parent_hash {parent_hash} does not match checkpoint hash {last_indexed_hash}"
        )));
    }

    Ok(())
}

/// Purpose: 같은 batch 안의 block 번호와 parent hash 연속성 검증
/// Param:
/// - `blocks`: 순서 검증 대상 block 목록
pub fn ensure_block_sequence(blocks: &[Block]) -> Result<()> {
    for pair in blocks.windows(2) {
        let previous = &pair[0];
        let current = &pair[1];

        if current.number != previous.number + 1 {
            return Err(AppError::msg(format!(
                "non-contiguous block batch: block {} followed by {}",
                previous.number, current.number
            )));
        }

        if current.parent_hash != previous.hash {
            return Err(AppError::msg(format!(
                "block batch parent hash mismatch at block {}",
                current.number
            )));
        }
    }

    Ok(())
}
