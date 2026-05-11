use crate::{
    backfill::BackfillSource,
    chain_validation::{ensure_block_sequence, ensure_checkpoint_continuity},
    db::{BlockRange, Store, StoredBlock},
    error::{AppError, Result},
    extractor::Extractor,
    models::{IndexedBlock, SyncCheckpoint, SyncStatus},
    proto::{Block, BlockRef, ExExNotification, ExExNotificationKind},
    proto_convert::{block_ref_hash, format_hash, raw_block},
};

#[derive(Debug, Clone)]
pub struct Processor<S, B> {
    store: S,
    backfill: B,
    chain_id: i32,
    chain_name: String,
    backfill_chunk_size: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Gap {
    last_indexed_block: i64,
    first_new_block: i64,
    from_block: i64,
    to_block: i64,
}

impl<S, B> Processor<S, B>
where
    S: Store,
    B: BackfillSource,
{
    /// Purpose: notification Processor 생성
    /// Param:
    /// - `store`: DB store
    /// - `backfill`: gap backfill source
    /// - `chain_id`: chain_id 값 (e.g. Ethereum : 1)
    /// - `chain_name`: chain_name 값 (e.g. Mainnet)
    pub fn new(
        store: S,
        backfill: B,
        chain_id: i32,
        chain_name: impl Into<String>,
        backfill_chunk_size: usize,
    ) -> Self {
        Self {
            store,
            backfill,
            chain_id,
            chain_name: chain_name.into(),
            backfill_chunk_size,
        }
    }

    /// Purpose: 체인 메타데이터와 체크포인트 초기 상태 준비
    /// Param:
    /// - `self`: Processor.
    pub async fn initialize(&self) -> Result<()> {
        self.store
            .ensure_chain(self.chain_id, &self.chain_name)
            .await?;
        self.store
            .set_checkpoint_status(self.chain_id, SyncStatus::Idle)
            .await
    }

    /// Purpose: 원격 ExEx notification 종류별 처리
    /// Param:
    /// - `self`: Processor
    /// - `notification`: remote ExEx notification
    pub async fn process_remote_notification(&self, notification: ExExNotification) -> Result<()> {
        let notification_kind = ExExNotificationKind::try_from(notification.kind)
            .map_err(|error| AppError::with_source("invalid ExEx notification kind", error))?;
        let tip = notification
            .tip_block
            .as_ref()
            .map(|block| block.number.to_string())
            .unwrap_or_else(|| "none".to_owned());

        println!(
            "notification received: kind={} chain_id={} new_blocks={} tip={}",
            notification_kind.as_log_str(),
            notification.chain_id,
            notification.new_blocks.len(),
            tip
        );

        if notification.chain_id != self.chain_id as u64 {
            return Err(AppError::msg(format!(
                "notification chain_id {} does not match configured chain_id {}",
                notification.chain_id, self.chain_id
            )));
        }

        self.store
            .set_checkpoint_status(self.chain_id, SyncStatus::Syncing)
            .await?;

        let result = match notification_kind {
            ExExNotificationKind::Unknown => {
                Err(AppError::msg("received unknown ExEx notification kind"))
            }
            ExExNotificationKind::ChainCommitted => {
                self.apply_new_blocks(notification.new_blocks, notification.tip_block)
                    .await
            }
            ExExNotificationKind::ChainReorged => {
                self.reorg_to_new_blocks(
                    notification.old_range,
                    notification.new_blocks,
                    notification.tip_block,
                )
                .await
            }
            ExExNotificationKind::ChainReverted => {
                self.revert_to_fork(notification.old_range, notification.fork_block)
                    .await
            }
        };

        match result {
            Ok(()) => {
                self.store
                    .set_checkpoint_status(self.chain_id, SyncStatus::Idle)
                    .await
            }
            Err(error) => {
                let _ = self
                    .store
                    .set_checkpoint_status(self.chain_id, SyncStatus::Error)
                    .await;
                Err(error)
            }
        }
    }

    /// Purpose: 새 블록 목록을 검증 후 순서대로 인덱싱
    /// Param:
    /// - `self`: Processor
    /// - `new_blocks`: indexing 대상 new_blocks
    /// - `tip_block`: notification 기준 가장 끝 block
    async fn apply_new_blocks(
        &self,
        new_blocks: Vec<Block>,
        tip_block: Option<BlockRef>,
    ) -> Result<()> {
        let tip_block = required_block_ref(tip_block, "tip_block")?;
        self.backfill_gap_if_needed(&new_blocks).await?;
        self.ensure_contiguous(&new_blocks).await?;

        for block in new_blocks {
            if block.chain_id != self.chain_id as u64 {
                return Err(AppError::msg(format!(
                    "block chain_id {} does not match configured chain_id {}",
                    block.chain_id, self.chain_id
                )));
            }

            let indexed_block = Extractor::extract_block(raw_block(block)?)?;
            let is_tip = indexed_block.record.block_number == tip_block.number as i64;
            let checkpoint = SyncCheckpoint {
                chain_id: self.chain_id,
                last_indexed_block: Some(indexed_block.record.block_number),
                last_indexed_hash: Some(indexed_block.record.block_hash.clone()),
                status: SyncStatus::Syncing,
            };
            let chain_id = indexed_block.record.chain_id;
            let block_number = indexed_block.record.block_number;
            let tx_count = indexed_block.record.tx_count;
            let movement_count = indexed_block.record.movement_count;

            self.store
                .apply_block(indexed_block.record, indexed_block.movements, checkpoint)
                .await?;

            println!(
                "block indexed: chain_id={} block={} txs={} movements={}",
                chain_id, block_number, tx_count, movement_count
            );

            if is_tip {
                return Ok(());
            }
        }

        self.store
            .save_checkpoint(tip_checkpoint(self.chain_id, tip_block)?)
            .await
    }

    /// Purpose: gap 발생 시 RPC backfill로 누락 블록 보충
    /// Param:
    /// - `self`: Processor
    /// - `new_blocks`: gap 검사 대상 new_blocks
    async fn backfill_gap_if_needed(&self, new_blocks: &[Block]) -> Result<()> {
        let Some(gap) = self.detect_gap(new_blocks).await? else {
            return Ok(());
        };

        println!(
            "gap detected: checkpoint={} first_new_block={} backfill_range={}..{}",
            gap.last_indexed_block, gap.first_new_block, gap.from_block, gap.to_block
        );

        let mut from_block = gap.from_block;
        while from_block <= gap.to_block {
            let to_block = (from_block + self.backfill_chunk_size as i64 - 1).min(gap.to_block);
            let blocks = self
                .backfill
                .fetch_blocks(self.chain_id, from_block, to_block)
                .await?;
            self.apply_backfill_blocks(blocks).await?;
            from_block = to_block + 1;
        }

        Ok(())
    }

    /// Purpose: backfill 블록들을 한 트랜잭션으로 저장
    /// Param:
    /// - `self`: Processor
    /// - `blocks`: backfill로 조회한 연속 block 목록
    async fn apply_backfill_blocks(&self, blocks: Vec<Block>) -> Result<()> {
        if blocks.is_empty() {
            return Ok(());
        }

        for block in &blocks {
            if block.chain_id != self.chain_id as u64 {
                return Err(AppError::msg(format!(
                    "backfill block chain_id {} does not match configured chain_id {}",
                    block.chain_id, self.chain_id
                )));
            }
        }

        self.ensure_contiguous(&blocks).await?;
        ensure_block_sequence(&blocks)?;

        let indexed_blocks = extract_blocks_parallel(blocks).await?;
        let Some(last_block) = indexed_blocks.last() else {
            return Ok(());
        };

        let checkpoint = SyncCheckpoint {
            chain_id: self.chain_id,
            last_indexed_block: Some(last_block.record.block_number),
            last_indexed_hash: Some(last_block.record.block_hash.clone()),
            status: SyncStatus::Syncing,
        };

        let stored_blocks = indexed_blocks
            .into_iter()
            .map(|indexed_block| StoredBlock {
                record: indexed_block.record,
                movements: indexed_block.movements,
            })
            .collect::<Vec<_>>();

        let first_block_number = stored_blocks
            .first()
            .map(|block| block.record.block_number)
            .unwrap_or_default();
        let last_block_number = stored_blocks
            .last()
            .map(|block| block.record.block_number)
            .unwrap_or_default();
        let block_count = stored_blocks.len();
        let movement_count = stored_blocks
            .iter()
            .map(|block| block.record.movement_count)
            .sum::<i32>();
        let tx_count = stored_blocks
            .iter()
            .map(|block| block.record.tx_count)
            .sum::<i32>();

        self.store.apply_blocks(stored_blocks, checkpoint).await?;

        println!(
            "backfill blocks indexed: chain_id={} range={}..{} blocks={} txs={} movements={}",
            self.chain_id,
            first_block_number,
            last_block_number,
            block_count,
            tx_count,
            movement_count
        );

        Ok(())
    }

    /// Purpose: checkpoint와 첫 새 블록 사이 gap 계산
    /// Param:
    /// - `self`: Processor
    /// - `new_blocks`: gap 검사 대상 new_blocks
    async fn detect_gap(&self, new_blocks: &[Block]) -> Result<Option<Gap>> {
        let Some(first_block) = new_blocks.first() else {
            return Ok(None);
        };

        let Some(checkpoint) = self.store.load_checkpoint(self.chain_id).await? else {
            return Ok(None);
        };

        let Some(last_indexed_block) = checkpoint.last_indexed_block else {
            return Ok(None);
        };

        let first_new_block = i64::try_from(first_block.number)
            .map_err(|error| AppError::with_source("block number does not fit in i64", error))?;
        let expected_next = last_indexed_block + 1;

        if first_new_block <= expected_next {
            return Ok(None);
        }

        Ok(Some(Gap {
            last_indexed_block,
            first_new_block,
            from_block: expected_next,
            to_block: first_new_block - 1,
        }))
    }

    /// Purpose: 체크포인트와 첫 새 블록의 연속성 확인
    /// Param:
    /// - `self`: Processor
    /// - `new_blocks`: 연속성 검증 대상 new_blocks
    async fn ensure_contiguous(&self, new_blocks: &[Block]) -> Result<()> {
        let Some(first_block) = new_blocks.first() else {
            return Ok(());
        };

        let Some(checkpoint) = self.store.load_checkpoint(self.chain_id).await? else {
            return Ok(());
        };

        let Some(last_indexed_block) = checkpoint.last_indexed_block else {
            return Ok(());
        };
        let Some(last_indexed_hash) = checkpoint.last_indexed_hash else {
            return Ok(());
        };

        ensure_checkpoint_continuity(last_indexed_block, &last_indexed_hash, first_block)
    }

    /// Purpose: reorg 대상 블록 삭제 후 새 체인 블록 인덱싱
    /// Param:
    /// - `self`: Processor
    /// - `old_range`: 삭제 대상 old_range
    /// - `new_blocks`: indexing 대상 new_blocks
    /// - `tip_block`: 새 chain의 tip_block
    async fn reorg_to_new_blocks(
        &self,
        old_range: Option<crate::proto::BlockRange>,
        new_blocks: Vec<Block>,
        tip_block: Option<BlockRef>,
    ) -> Result<()> {
        let old_range = old_range.ok_or_else(|| AppError::msg("missing old_range"))?;
        self.store
            .revert_blocks(
                self.chain_id,
                BlockRange::new(old_range.first as i64, old_range.last as i64)?,
                SyncCheckpoint {
                    chain_id: self.chain_id,
                    last_indexed_block: None,
                    last_indexed_hash: None,
                    status: SyncStatus::Syncing,
                },
            )
            .await?;

        self.apply_new_blocks(new_blocks, tip_block).await
    }

    /// Purpose: revert 범위 삭제 후 fork 블록으로 체크포인트 이동
    /// Param:
    /// - `self`: Processor
    /// - `old_range`: 삭제 대상 old_range
    /// - `fork_block`: revert 기준 fork_block
    async fn revert_to_fork(
        &self,
        old_range: Option<crate::proto::BlockRange>,
        fork_block: Option<BlockRef>,
    ) -> Result<()> {
        let fork_block = required_block_ref(fork_block, "fork_block")?;
        let old_range = old_range.ok_or_else(|| AppError::msg("missing old_range"))?;

        self.store
            .revert_blocks(
                self.chain_id,
                BlockRange::new(old_range.first as i64, old_range.last as i64)?,
                SyncCheckpoint {
                    chain_id: self.chain_id,
                    last_indexed_block: Some(fork_block.number as i64),
                    last_indexed_hash: Some(block_ref_hash(&fork_block)?),
                    status: SyncStatus::Syncing,
                },
            )
            .await
    }
}

/// Purpose: tip 블록 기준 체크포인트 생성
/// Param:
/// - `chain_id`: chain_id 값
/// - `tip_block`: checkpoint 기준 tip_block
fn tip_checkpoint(chain_id: i32, tip_block: BlockRef) -> Result<SyncCheckpoint> {
    Ok(SyncCheckpoint {
        chain_id,
        last_indexed_block: Some(tip_block.number as i64),
        last_indexed_hash: Some(format_hash(&tip_block.hash)?),
        status: SyncStatus::Syncing,
    })
}

/// Purpose: 여러 block의 extract 작업을 병렬 실행
/// Param:
/// - `blocks`: extract 대상 block 목록
async fn extract_blocks_parallel(blocks: Vec<Block>) -> Result<Vec<IndexedBlock>> {
    let mut tasks = tokio::task::JoinSet::new();

    for block in blocks {
        tasks.spawn_blocking(move || Extractor::extract_block(raw_block(block)?));
    }

    let mut indexed_blocks = Vec::new();
    while let Some(result) = tasks.join_next().await {
        let indexed_block = result
            .map_err(|error| AppError::with_source("backfill extract task failed", error))??;
        indexed_blocks.push(indexed_block);
    }

    indexed_blocks.sort_by_key(|block| block.record.block_number);
    Ok(indexed_blocks)
}

/// Purpose: 필수 블록 참조 추출
/// Param:
/// - `block_ref`: 검사할 block_ref
/// - `name`: error message용 field name
fn required_block_ref(block_ref: Option<BlockRef>, name: &str) -> Result<BlockRef> {
    block_ref.ok_or_else(|| AppError::msg(format!("missing {name}")))
}

impl ExExNotificationKind {
    /// Purpose: notification kind 로그 문자열 반환
    /// Param:
    /// - `self`: 출력할 ExExNotificationKind
    fn as_log_str(self) -> &'static str {
        match self {
            Self::Unknown => "UNKNOWN",
            Self::ChainCommitted => "CHAIN_COMMITTED",
            Self::ChainReorged => "CHAIN_REORGED",
            Self::ChainReverted => "CHAIN_REVERTED",
        }
    }
}

#[cfg(test)]
mod tests;
