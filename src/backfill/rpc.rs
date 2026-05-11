use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::{AppError, Result};

use super::RpcBackfillClient;

const MAX_RECEIPT_BATCH_SIZE: usize = 1_000;

impl RpcBackfillClient {
    /// Purpose: eth_getBlockByNumber 요청들을 JSON-RPC batch로 조회
    /// Param:
    /// - `self`: RpcBackfillClient
    /// - `from_block`: 시작 block_number
    /// - `to_block`: 종료 block_number
    pub(super) async fn fetch_rpc_blocks(
        &self,
        from_block: i64,
        to_block: i64,
    ) -> Result<Vec<RpcBlock>> {
        if from_block > to_block {
            return Err(AppError::msg(format!(
                "invalid backfill range: from_block {from_block} is greater than to_block {to_block}"
            )));
        }

        let requests = (from_block..=to_block)
            .enumerate()
            .map(|(index, block_number)| JsonRpcRequest {
                jsonrpc: "2.0",
                id: index as u64,
                method: "eth_getBlockByNumber",
                params: json!([format!("0x{block_number:x}"), true]),
            })
            .collect::<Vec<_>>();
        let responses = self.send_batch(requests, "block batch").await?;

        responses
            .into_iter()
            .enumerate()
            .map(|(index, response)| {
                let block_number = from_block + index as i64;
                let result = response.result.ok_or_else(|| {
                    AppError::msg(format!("backfill block {block_number} missing result"))
                })?;

                if result.is_null() {
                    return Err(AppError::msg(format!(
                        "backfill block {block_number} not found"
                    )));
                }

                serde_json::from_value(result).map_err(|error| {
                    AppError::with_source("failed to decode backfill block", error)
                })
            })
            .collect()
    }

    /// Purpose: 여러 블록의 transaction receipt를 JSON-RPC batch로 조회
    /// Param:
    /// - `self`: RpcBackfillClient
    /// - `blocks`: receipt 조회 대상 block 목록
    pub(super) async fn fetch_receipts_for_blocks(
        &self,
        blocks: &[RpcBlock],
    ) -> Result<Vec<Vec<RpcReceipt>>> {
        match self.fetch_block_receipts_batch(blocks).await {
            Ok(receipts) => {
                return Ok(receipts);
            }
            Err(error) => {
                eprintln!(
                    "eth_getBlockReceipts failed; falling back to transaction receipt batch: {error}"
                );
            }
        }

        let request_count = blocks
            .iter()
            .map(|block| block.transactions.len())
            .sum::<usize>();
        if request_count == 0 {
            return Ok(blocks.iter().map(|_| Vec::new()).collect());
        }

        let mut requests = Vec::with_capacity(request_count);
        let mut receipt_positions = Vec::with_capacity(request_count);
        let mut id = 0_u64;
        for (block_index, block) in blocks.iter().enumerate() {
            for transaction_index in 0..block.transactions.len() {
                requests.push(JsonRpcRequest {
                    jsonrpc: "2.0",
                    id,
                    method: "eth_getTransactionReceipt",
                    params: json!([block.transactions[transaction_index].hash]),
                });
                receipt_positions.push((block_index, transaction_index));
                id += 1;
            }
        }

        let mut receipts_by_block = blocks
            .iter()
            .map(|block| {
                let mut receipts = Vec::with_capacity(block.transactions.len());
                receipts.resize_with(block.transactions.len(), || None);
                receipts
            })
            .collect::<Vec<_>>();

        for request_chunk in requests.chunks(MAX_RECEIPT_BATCH_SIZE) {
            let responses = self
                .send_batch(request_chunk.to_vec(), "receipt batch")
                .await?;

            for response in responses {
                let (block_index, transaction_index) = receipt_positions
                    .get(response.id as usize)
                    .copied()
                    .ok_or_else(|| {
                        AppError::msg(format!(
                            "receipt response id {} is out of range",
                            response.id
                        ))
                    })?;

                let result = response
                    .result
                    .ok_or_else(|| AppError::msg("receipt response missing result"))?;
                let receipt = serde_json::from_value(result).map_err(|error| {
                    AppError::with_source("failed to decode backfill receipt", error)
                })?;
                receipts_by_block[block_index][transaction_index] = Some(receipt);
            }
        }

        receipts_by_block
            .into_iter()
            .map(|receipts| {
                receipts
                    .into_iter()
                    .map(|receipt| {
                        receipt.ok_or_else(|| AppError::msg("receipt response missing result"))
                    })
                    .collect()
            })
            .collect()
    }

    /// Purpose: eth_getBlockReceipts로 여러 블록의 receipt 전체를 JSON-RPC batch 조회
    /// Param:
    /// - `self`: RpcBackfillClient
    /// - `blocks`: receipt 조회 대상 block 목록
    async fn fetch_block_receipts_batch(
        &self,
        blocks: &[RpcBlock],
    ) -> Result<Vec<Vec<RpcReceipt>>> {
        if blocks.is_empty() {
            return Ok(Vec::new());
        }

        let requests = blocks
            .iter()
            .enumerate()
            .map(|(index, block)| JsonRpcRequest {
                jsonrpc: "2.0",
                id: index as u64,
                method: "eth_getBlockReceipts",
                params: json!([block.number]),
            })
            .collect::<Vec<_>>();
        let responses = self.send_batch(requests, "block receipts batch").await?;

        responses
            .into_iter()
            .enumerate()
            .map(|(index, response)| {
                let result = response.result.ok_or_else(|| {
                    AppError::msg(format!(
                        "block receipts response missing result for block {}",
                        blocks[index].number
                    ))
                })?;

                let receipts =
                    serde_json::from_value::<Vec<RpcReceipt>>(result).map_err(|error| {
                        AppError::with_source("failed to decode block receipts", error)
                    })?;
                if receipts.len() != blocks[index].transactions.len() {
                    return Err(AppError::msg(format!(
                        "block receipts length {} does not match transaction count {} for block {}",
                        receipts.len(),
                        blocks[index].transactions.len(),
                        blocks[index].number
                    )));
                }

                Ok(receipts)
            })
            .collect()
    }

    /// Purpose: JSON-RPC batch 요청 전송 후 id 순서로 정렬된 response 반환
    /// Param:
    /// - `self`: RpcBackfillClient
    /// - `requests`: JSON-RPC request 목록
    /// - `label`: error message용 batch label
    async fn send_batch(
        &self,
        requests: Vec<JsonRpcRequest>,
        label: &'static str,
    ) -> Result<Vec<JsonRpcResponse>> {
        if requests.is_empty() {
            return Ok(Vec::new());
        }
        let mut request_ids = requests
            .iter()
            .map(|request| request.id)
            .collect::<Vec<_>>();
        request_ids.sort_unstable();

        let mut responses = self
            .http
            .post(&self.rpc_url)
            .json(&requests)
            .send()
            .await
            .map_err(|error| {
                AppError::with_source(format!("failed to send {label} request"), error)
            })?
            .error_for_status()
            .map_err(|error| AppError::with_source(format!("{label} request failed"), error))?
            .json::<Vec<JsonRpcResponse>>()
            .await
            .map_err(|error| {
                AppError::with_source(format!("failed to decode {label} response"), error)
            })?;

        responses.sort_by_key(|response| response.id);
        let response_ids = responses
            .iter()
            .map(|response| response.id)
            .collect::<Vec<_>>();
        if response_ids != request_ids {
            return Err(AppError::msg(format!(
                "{label} response ids do not match request ids"
            )));
        }

        for response in &responses {
            if let Some(error) = &response.error {
                return Err(AppError::msg(format!(
                    "{label} request failed: {}",
                    error.message
                )));
            }
        }

        Ok(responses)
    }
}

#[derive(Debug, Clone, Serialize)]
struct JsonRpcRequest {
    jsonrpc: &'static str,
    id: u64,
    method: &'static str,
    params: Value,
}

#[derive(Debug, Deserialize)]
struct JsonRpcResponse {
    id: u64,
    result: Option<Value>,
    error: Option<JsonRpcError>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcError {
    message: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RpcBlock {
    pub(super) number: String,
    pub(super) hash: String,
    pub(super) parent_hash: String,
    pub(super) timestamp: String,
    pub(super) transactions: Vec<RpcTransaction>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RpcTransaction {
    pub(super) hash: String,
    pub(super) transaction_index: String,
    pub(super) from: String,
    pub(super) to: Option<String>,
    pub(super) value: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct RpcReceipt {
    pub(super) logs: Vec<RpcLog>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RpcLog {
    pub(super) log_index: String,
    pub(super) address: String,
    pub(super) topics: Vec<String>,
    pub(super) data: String,
}
