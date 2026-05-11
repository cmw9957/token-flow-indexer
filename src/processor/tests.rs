use std::sync::{Arc, Mutex};

use crate::{
    db::{BlockRange, Store, StoredBlock},
    error::{AppError, Result},
    models::{AssetMovement, BlockRecord, SyncCheckpoint, SyncStatus},
    proto::{Block, BlockRef, ExExNotification, ExExNotificationKind},
};

use super::{BackfillSource, Processor};

#[derive(Debug, Clone, Default)]
struct MockStore {
    state: Arc<Mutex<MockState>>,
}

#[derive(Debug, Clone, Default)]
struct MockBackfill {
    blocks: Arc<Mutex<Vec<Block>>>,
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

    async fn apply_blocks(
        &self,
        blocks: Vec<StoredBlock>,
        checkpoint: SyncCheckpoint,
    ) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        state
            .applied_blocks
            .extend(blocks.into_iter().map(|block| block.record));
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

impl BackfillSource for MockBackfill {
    async fn fetch_blocks(
        &self,
        _chain_id: i32,
        from_block: i64,
        to_block: i64,
    ) -> Result<Vec<Block>> {
        let blocks = self.blocks.lock().unwrap();
        (from_block..=to_block)
            .map(|block_number| {
                blocks
                    .iter()
                    .find(|block| block.number == block_number as u64)
                    .cloned()
                    .ok_or_else(|| {
                        AppError::msg(format!("missing mock backfill block {block_number}"))
                    })
            })
            .collect()
    }
}

#[tokio::test]
async fn committed_notification_applies_block_and_updates_checkpoint() {
    let store = MockStore::default();
    store.state.lock().unwrap().checkpoint = Some(SyncCheckpoint {
        chain_id: 1,
        last_indexed_block: Some(9),
        last_indexed_hash: Some(hex_bytes(0x22, 32)),
        status: SyncStatus::Idle,
    });
    let processor = Processor::new(store.clone(), MockBackfill::default(), 1, "ethereum", 50);

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
async fn committed_notification_backfills_gap_before_applying_stream_block() {
    let store = MockStore::default();
    store.state.lock().unwrap().checkpoint = Some(SyncCheckpoint {
        chain_id: 1,
        last_indexed_block: Some(9),
        last_indexed_hash: Some(hex_bytes(0x22, 32)),
        status: SyncStatus::Idle,
    });
    let backfill = MockBackfill::default();
    backfill
        .blocks
        .lock()
        .unwrap()
        .push(proto_block(10, 0x22, 0x10));
    let processor = Processor::new(store.clone(), backfill, 1, "ethereum", 50);

    processor
        .process_remote_notification(committed_notification(11, 0x10, 0x11))
        .await
        .unwrap();

    let state = store.state.lock().unwrap();
    assert_eq!(state.statuses, vec![SyncStatus::Syncing, SyncStatus::Idle]);
    assert_eq!(state.applied_blocks.len(), 2);
    assert_eq!(state.applied_blocks[0].block_number, 10);
    assert_eq!(state.applied_blocks[1].block_number, 11);
}

#[tokio::test]
async fn reverted_notification_deletes_old_range_and_moves_checkpoint_to_fork() {
    let store = MockStore::default();
    let processor = Processor::new(store.clone(), MockBackfill::default(), 1, "ethereum", 50);

    processor
        .process_remote_notification(ExExNotification {
            kind: ExExNotificationKind::ChainReverted as i32,
            old_range: Some(crate::proto::BlockRange {
                first: 10,
                last: 12,
            }),
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

fn committed_notification(number: u64, parent_hash_byte: u8, hash_byte: u8) -> ExExNotification {
    ExExNotification {
        kind: ExExNotificationKind::ChainCommitted as i32,
        old_range: None,
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
