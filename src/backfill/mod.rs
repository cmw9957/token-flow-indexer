mod convert;
mod rpc;

use crate::{error::Result, proto::Block};

#[derive(Debug, Clone)]
pub struct RpcBackfillClient {
    pub(super) rpc_url: String,
    pub(super) http: reqwest::Client,
}

impl RpcBackfillClient {
    /// Purpose: JSON-RPC backfill client 생성
    /// Param:
    /// - `rpc_url`: JSON-RPC endpoint
    pub fn new(rpc_url: impl Into<String>) -> Self {
        Self {
            rpc_url: rpc_url.into(),
            http: reqwest::Client::new(),
        }
    }
}

pub trait BackfillSource {
    /// Purpose: 누락 블록 범위를 조회
    /// Param:
    /// - `chain_id`: chain_id 값
    /// - `from_block`: 시작 block_number
    /// - `to_block`: 종료 block_number
    async fn fetch_blocks(
        &self,
        chain_id: i32,
        from_block: i64,
        to_block: i64,
    ) -> Result<Vec<Block>>;
}

impl BackfillSource for RpcBackfillClient {
    /// Purpose: JSON-RPC batch로 block과 receipt를 조회해 proto block 목록으로 변환
    /// Param:
    /// - `chain_id`: chain_id 값
    /// - `from_block`: 시작 block_number
    /// - `to_block`: 종료 block_number
    async fn fetch_blocks(
        &self,
        chain_id: i32,
        from_block: i64,
        to_block: i64,
    ) -> Result<Vec<Block>> {
        let blocks = self.fetch_rpc_blocks(from_block, to_block).await?;
        let receipts_by_block = self.fetch_receipts_grouped_by_block(&blocks).await?;

        blocks
            .into_iter()
            .zip(receipts_by_block)
            .map(|(block, receipts)| convert::rpc_block_to_proto(chain_id, block, receipts))
            .collect()
    }
}
