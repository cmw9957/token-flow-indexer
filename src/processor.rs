use crate::{
    db::{BlockRange, Store},
    error::{AppError, Result},
    extractor::{Extractor, RawBlock, RawLog, RawTransaction},
    models::{AssetMovement, BlockRecord, SyncCheckpoint, SyncStatus},
    proto::{Block, BlockRef, ExExNotification, ExExNotificationKind},
};

#[derive(Debug, Clone)]
pub struct Processor<S> {
    store: S,
    chain_id: i32,
    chain_name: String,
}

impl<S> Processor<S>
where
    S: Store,
{
    /// Purpose: notification Processor 생성
    /// Param:
    /// - `store`: DB store
    /// - `chain_id`: chain_id 값 (e.g. Ethereum : 1)
    /// - `chain_name`: chain_name 값 (e.g. Mainnet)
    pub fn new(store: S, chain_id: i32, chain_name: impl Into<String>) -> Self {
        Self {
            store,
            chain_id,
            chain_name: chain_name.into(),
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
        if notification.chain_id != self.chain_id as u64 {
            return Err(AppError::msg(format!(
                "notification chain_id {} does not match configured chain_id {}",
                notification.chain_id, self.chain_id
            )));
        }

        self.store
            .set_checkpoint_status(self.chain_id, SyncStatus::Syncing)
            .await?;

        let result = match ExExNotificationKind::try_from(notification.kind)
            .map_err(|error| AppError::with_source("invalid ExEx notification kind", error))?
        {
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

            self.store
                .apply_block(indexed_block.record, indexed_block.movements, checkpoint)
                .await?;

            if is_tip {
                return Ok(());
            }
        }

        self.store
            .save_checkpoint(tip_checkpoint(self.chain_id, tip_block)?)
            .await
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
                    last_indexed_hash: Some(format_hash(&fork_block.hash)?),
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

/// Purpose: proto 블록을 extractor 입력 모델로 변환
/// Param:
/// - `block`: 변환할 proto block
fn raw_block(block: Block) -> Result<RawBlock> {
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
                                data: format_bytes(&log.data),
                            })
                        })
                        .collect::<Result<Vec<_>>>()?,
                })
            })
            .collect::<Result<Vec<_>>>()?,
    })
}

/// Purpose: 필수 블록 참조 추출
/// Param:
/// - `block_ref`: 검사할 block_ref
/// - `name`: error message용 field name
fn required_block_ref(block_ref: Option<BlockRef>, name: &str) -> Result<BlockRef> {
    block_ref.ok_or_else(|| AppError::msg(format!("missing {name}")))
}

/// Purpose: 32바이트 해시를 0x hex 문자열로 변환
/// Param:
/// - `bytes`: 32-byte hash bytes
fn format_hash(bytes: &[u8]) -> Result<String> {
    if bytes.len() != 32 {
        return Err(AppError::msg(format!(
            "invalid block hash length: expected 32 bytes, got {}",
            bytes.len()
        )));
    }

    let mut out = String::with_capacity(66);
    out.push_str("0x");
    for byte in bytes {
        out.push(hex_char(byte >> 4));
        out.push(hex_char(byte & 0x0f));
    }
    Ok(out)
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

    let mut out = String::with_capacity(42);
    out.push_str("0x");
    for byte in bytes {
        out.push(hex_char(byte >> 4));
        out.push(hex_char(byte & 0x0f));
    }
    Ok(out)
}

/// Purpose: 바이트 배열을 0x hex 문자열로 변환
/// Param:
/// - `bytes`: bytes 값
fn format_bytes(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(2 + bytes.len() * 2);
    out.push_str("0x");
    for byte in bytes {
        out.push(hex_char(byte >> 4));
        out.push(hex_char(byte & 0x0f));
    }
    out
}

/// Purpose: 4비트 값을 hex 문자로 변환
/// Param:
/// - `value`: 0~15 value
fn hex_char(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        10..=15 => (b'a' + value - 10) as char,
        _ => unreachable!("nibble is always <= 15"),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedBlock {
    pub record: BlockRecord,
    pub movements: Vec<AssetMovement>,
}

impl IndexedBlock {
    /// Purpose: 블록 레코드와 자산 이동 목록으로 인덱싱 결과 생성
    /// Param:
    /// - `record`: block record
    /// - `movements`: asset movements
    pub fn new(mut record: BlockRecord, movements: Vec<AssetMovement>) -> Self {
        record.movement_count = movements.len() as i32;
        Self { record, movements }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Debug, Clone, Default)]
    struct MockStore {
        state: Arc<Mutex<MockState>>,
    }

    #[derive(Debug, Default)]
    struct MockState {
        chains: Vec<(i32, String)>,
        statuses: Vec<SyncStatus>,
        checkpoint: Option<SyncCheckpoint>,
        applied_blocks: Vec<BlockRecord>,
        reverted_ranges: Vec<BlockRange>,
    }

    impl Store for MockStore {
        async fn ensure_chain(&self, chain_id: i32, name: &str) -> Result<()> {
            self.state
                .lock()
                .unwrap()
                .chains
                .push((chain_id, name.to_owned()));
            Ok(())
        }

        async fn load_checkpoint(&self, _chain_id: i32) -> Result<Option<SyncCheckpoint>> {
            Ok(self.state.lock().unwrap().checkpoint.clone())
        }

        async fn set_checkpoint_status(&self, _chain_id: i32, status: SyncStatus) -> Result<()> {
            self.state.lock().unwrap().statuses.push(status);
            Ok(())
        }

        async fn save_checkpoint(&self, checkpoint: SyncCheckpoint) -> Result<()> {
            self.state.lock().unwrap().checkpoint = Some(checkpoint);
            Ok(())
        }

        async fn apply_block(
            &self,
            block: BlockRecord,
            _movements: Vec<AssetMovement>,
            checkpoint: SyncCheckpoint,
        ) -> Result<()> {
            let mut state = self.state.lock().unwrap();
            state.applied_blocks.push(block);
            state.checkpoint = Some(checkpoint);
            Ok(())
        }

        async fn revert_blocks(
            &self,
            _chain_id: i32,
            range: BlockRange,
            checkpoint: SyncCheckpoint,
        ) -> Result<()> {
            let mut state = self.state.lock().unwrap();
            state.reverted_ranges.push(range);
            state.checkpoint = Some(checkpoint);
            Ok(())
        }
    }

    #[tokio::test]
    async fn initialize_ensures_chain_and_sets_idle_status() {
        // 초기화 상태 갱신 검증
        let store = MockStore::default();
        let processor = Processor::new(store.clone(), 1, "ethereum");

        processor.initialize().await.unwrap();

        let state = store.state.lock().unwrap();
        assert_eq!(state.chains, vec![(1, "ethereum".to_owned())]);
        assert_eq!(state.statuses, vec![SyncStatus::Idle]);
    }

    #[tokio::test]
    async fn committed_notification_applies_block_and_updates_checkpoint() {
        // committed 블록 적용 검증
        let store = MockStore::default();
        store.state.lock().unwrap().checkpoint = Some(SyncCheckpoint {
            chain_id: 1,
            last_indexed_block: Some(9),
            last_indexed_hash: Some(hex_bytes(0x22, 32)),
            status: SyncStatus::Idle,
        });
        let processor = Processor::new(store.clone(), 1, "ethereum");

        processor
            .process_remote_notification(committed_notification(10, 0x22, 0x11))
            .await
            .unwrap();

        let state = store.state.lock().unwrap();
        assert_eq!(state.statuses, vec![SyncStatus::Syncing, SyncStatus::Idle]);
        assert_eq!(state.applied_blocks.len(), 1);
        assert_eq!(state.applied_blocks[0].block_number, 10);
        assert_eq!(
            state.checkpoint.as_ref().unwrap().last_indexed_block,
            Some(10)
        );
        assert_eq!(
            state.checkpoint.as_ref().unwrap().last_indexed_hash,
            Some(hex_bytes(0x11, 32))
        );
    }

    #[tokio::test]
    async fn committed_notification_reports_gap_and_sets_error_status() {
        // gap 감지 검증
        let store = MockStore::default();
        store.state.lock().unwrap().checkpoint = Some(SyncCheckpoint {
            chain_id: 1,
            last_indexed_block: Some(9),
            last_indexed_hash: Some(hex_bytes(0x22, 32)),
            status: SyncStatus::Idle,
        });
        let processor = Processor::new(store.clone(), 1, "ethereum");

        let error = processor
            .process_remote_notification(committed_notification(11, 0x22, 0x11))
            .await
            .unwrap_err();

        assert!(error.to_string().contains("gap detected"));
        let state = store.state.lock().unwrap();
        assert_eq!(state.statuses, vec![SyncStatus::Syncing, SyncStatus::Error]);
        assert!(state.applied_blocks.is_empty());
    }

    #[tokio::test]
    async fn reverted_notification_deletes_old_range_and_moves_checkpoint_to_fork() {
        // reverted checkpoint 이동 검증
        let store = MockStore::default();
        let processor = Processor::new(store.clone(), 1, "ethereum");

        processor
            .process_remote_notification(ExExNotification {
                kind: ExExNotificationKind::ChainReverted as i32,
                old_range: Some(crate::proto::BlockRange {
                    first: 10,
                    last: 12,
                }),
                new_range: None,
                fork_block: Some(BlockRef {
                    number: 9,
                    hash: vec![0x99; 32],
                }),
                tip_block: None,
                new_blocks: Vec::new(),
                chain_id: 1,
            })
            .await
            .unwrap();

        let state = store.state.lock().unwrap();
        assert_eq!(
            state.reverted_ranges,
            vec![BlockRange {
                from_block: 10,
                to_block: 12,
            }]
        );
        assert_eq!(
            state.checkpoint.as_ref().unwrap().last_indexed_block,
            Some(9)
        );
        assert_eq!(
            state.checkpoint.as_ref().unwrap().last_indexed_hash,
            Some(hex_bytes(0x99, 32))
        );
    }

    #[test]
    fn raw_block_formats_proto_bytes_for_extractor() {
        // proto block 변환 검증
        let raw = raw_block(proto_block(10, 0x22, 0x11)).unwrap();

        assert_eq!(raw.block_number, 10);
        assert_eq!(raw.block_hash, hex_bytes(0x11, 32));
        assert_eq!(raw.parent_hash, hex_bytes(0x22, 32));
        assert_eq!(raw.transactions[0].from_address, hex_bytes(0x44, 20));
    }

    fn committed_notification(
        number: u64,
        parent_hash_byte: u8,
        hash_byte: u8,
    ) -> ExExNotification {
        ExExNotification {
            kind: ExExNotificationKind::ChainCommitted as i32,
            old_range: None,
            new_range: Some(crate::proto::BlockRange {
                first: number,
                last: number,
            }),
            fork_block: None,
            tip_block: Some(BlockRef {
                number,
                hash: vec![hash_byte; 32],
            }),
            new_blocks: vec![proto_block(number, parent_hash_byte, hash_byte)],
            chain_id: 1,
        }
    }

    fn proto_block(number: u64, parent_hash_byte: u8, hash_byte: u8) -> Block {
        Block {
            number,
            hash: vec![hash_byte; 32],
            parent_hash: vec![parent_hash_byte; 32],
            timestamp: 1_700_000_000,
            chain_id: 1,
            transactions: vec![crate::proto::Transaction {
                hash: vec![0x33; 32],
                index: 0,
                from: vec![0x44; 20],
                to: Some(vec![0x55; 20]),
                value_raw: "0".to_owned(),
                logs: Vec::new(),
            }],
        }
    }

    fn hex_bytes(byte: u8, len: usize) -> String {
        let mut out = String::from("0x");
        for _ in 0..len {
            out.push_str(&format!("{byte:02x}"));
        }
        out
    }
}
