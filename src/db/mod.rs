use crate::{
    error::Result,
    models::{AssetMovement, BlockRecord, SyncCheckpoint, SyncStatus},
};

pub mod postgres;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockRange {
    pub from_block: i64,
    pub to_block: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredBlock {
    pub record: BlockRecord,
    pub movements: Vec<AssetMovement>,
}

impl BlockRange {
    /// 기능: 유효한 블록 범위 생성.
    /// 파라미터:
    /// - `from_block`: 시작 from_block.
    /// - `to_block`: 종료 to_block.
    pub fn new(from_block: i64, to_block: i64) -> Result<Self> {
        if from_block > to_block {
            return Err(format!(
                "invalid block range: from_block {from_block} is greater than to_block {to_block}"
            )
            .into());
        }

        Ok(Self {
            from_block,
            to_block,
        })
    }
}

pub trait Store {
    /// 기능: 체인 메타데이터 존재 보장.
    /// 파라미터:
    /// - `self`: 현재 Store.
    /// - `chain_id`: chain_id 값.
    /// - `name`: chain name 값.
    async fn ensure_chain(&self, chain_id: i32, name: &str) -> Result<()>;

    /// 기능: 인덱서 체크포인트 조회.
    /// 파라미터:
    /// - `self`: 현재 Store.
    /// - `chain_id`: chain_id 값.
    async fn load_checkpoint(&self, chain_id: i32) -> Result<Option<SyncCheckpoint>>;

    /// 기능: 인덱서 체크포인트 상태 갱신.
    /// 파라미터:
    /// - `self`: 현재 Store.
    /// - `chain_id`: chain_id 값.
    /// - `status`: 저장할 status.
    async fn set_checkpoint_status(&self, chain_id: i32, status: SyncStatus) -> Result<()>;

    /// 기능: 인덱서 체크포인트 저장.
    /// 파라미터:
    /// - `self`: 현재 Store.
    /// - `checkpoint`: 저장할 checkpoint.
    async fn save_checkpoint(&self, checkpoint: SyncCheckpoint) -> Result<()>;

    /// 기능: 블록과 자산 이동을 원자적으로 저장.
    /// 파라미터:
    /// - `self`: 현재 Store.
    /// - `block`: 저장할 block.
    /// - `movements`: 저장할 movements.
    /// - `checkpoint`: 저장 후 갱신할 checkpoint.
    async fn apply_block(
        &self,
        block: BlockRecord,
        movements: Vec<AssetMovement>,
        checkpoint: SyncCheckpoint,
    ) -> Result<()>;

    /// 기능: 여러 블록과 자산 이동을 원자적으로 저장.
    /// 파라미터:
    /// - `self`: 현재 Store.
    /// - `blocks`: 저장할 block과 movement 묶음.
    /// - `checkpoint`: 저장 후 갱신할 checkpoint.
    async fn apply_blocks(
        &self,
        blocks: Vec<StoredBlock>,
        checkpoint: SyncCheckpoint,
    ) -> Result<()>;

    /// 기능: 지정 범위 블록과 자산 이동을 되돌림.
    /// 파라미터:
    /// - `self`: 현재 Store.
    /// - `chain_id`: chain_id 값.
    /// - `range`: 되돌릴 range.
    /// - `checkpoint`: 되돌린 후 저장할 checkpoint.
    async fn revert_blocks(
        &self,
        chain_id: i32,
        range: BlockRange,
        checkpoint: SyncCheckpoint,
    ) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_range_accepts_valid_inclusive_range() {
        // 정상 블록 범위 검증
        let range = BlockRange::new(10, 12).unwrap();

        assert_eq!(range.from_block, 10);
        assert_eq!(range.to_block, 12);
    }

    #[test]
    fn block_range_accepts_single_block_range() {
        // 단일 블록 범위 검증
        let range = BlockRange::new(10, 10).unwrap();

        assert_eq!(range.from_block, 10);
        assert_eq!(range.to_block, 10);
    }

    #[test]
    fn block_range_rejects_inverted_range() {
        // 역전된 블록 범위 거부 검증
        let error = BlockRange::new(12, 10).unwrap_err();

        assert!(error.to_string().contains("invalid block range"));
    }
}
