use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    error::{AppError, Result},
    proto::{Block, Log, Transaction},
};

const MAX_RECEIPT_BATCH_SIZE: usize = 1_000;

#[derive(Debug, Clone)]
pub struct RpcBackfillClient {
    rpc_url: String,
    http: reqwest::Client,
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
    /// Purpose: 누락 블록 하나를 조회
    /// Param:
    /// - `chain_id`: chain_id 값
    /// - `block_number`: 조회할 block_number
    async fn fetch_block(&self, chain_id: i32, block_number: i64) -> Result<Block>;

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
    ) -> Result<Vec<Block>> {
        if from_block > to_block {
            return Err(AppError::msg(format!(
                "invalid backfill range: from_block {from_block} is greater than to_block {to_block}"
            )));
        }

        let mut blocks = Vec::new();
        for block_number in from_block..=to_block {
            blocks.push(self.fetch_block(chain_id, block_number).await?);
        }
        Ok(blocks)
    }
}

impl BackfillSource for RpcBackfillClient {
    /// Purpose: JSON-RPC에서 block과 receipt를 조회해 proto block으로 변환
    /// Param:
    /// - `chain_id`: chain_id 값
    /// - `block_number`: 조회할 block_number
    async fn fetch_block(&self, chain_id: i32, block_number: i64) -> Result<Block> {
        let block = self.fetch_rpc_block(block_number).await?;
        let receipts = self.fetch_receipts(&block.transactions).await?;

        rpc_block_to_proto(chain_id, block, receipts)
    }

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
        let receipts_by_block = self.fetch_receipts_for_blocks(&blocks).await?;

        blocks
            .into_iter()
            .zip(receipts_by_block)
            .map(|(block, receipts)| rpc_block_to_proto(chain_id, block, receipts))
            .collect()
    }
}

impl RpcBackfillClient {
    /// Purpose: eth_getBlockByNumber 요청들을 JSON-RPC batch로 조회
    /// Param:
    /// - `self`: RpcBackfillClient
    /// - `from_block`: 시작 block_number
    /// - `to_block`: 종료 block_number
    async fn fetch_rpc_blocks(&self, from_block: i64, to_block: i64) -> Result<Vec<RpcBlock>> {
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

    /// Purpose: eth_getBlockByNumber로 transaction 포함 block 조회
    /// Param:
    /// - `self`: RpcBackfillClient
    /// - `block_number`: 조회할 block_number
    async fn fetch_rpc_block(&self, block_number: i64) -> Result<RpcBlock> {
        let result = self
            .send_single(
                "eth_getBlockByNumber",
                json!([format!("0x{block_number:x}"), true]),
            )
            .await?;

        if result.is_null() {
            return Err(AppError::msg(format!(
                "backfill block {block_number} not found"
            )));
        }

        serde_json::from_value(result)
            .map_err(|error| AppError::with_source("failed to decode backfill block", error))
    }

    /// Purpose: transaction 목록의 receipt를 batch JSON-RPC로 조회
    /// Param:
    /// - `self`: RpcBackfillClient
    /// - `transactions`: receipt 조회 대상 transaction 목록
    async fn fetch_receipts(&self, transactions: &[RpcTransaction]) -> Result<Vec<RpcReceipt>> {
        if transactions.is_empty() {
            return Ok(Vec::new());
        }

        let requests = transactions
            .iter()
            .enumerate()
            .map(|(index, transaction)| JsonRpcRequest {
                jsonrpc: "2.0",
                id: index as u64,
                method: "eth_getTransactionReceipt",
                params: json!([transaction.hash]),
            })
            .collect::<Vec<_>>();

        let responses = self.send_batch(requests, "receipt batch").await?;

        responses
            .into_iter()
            .map(|response| {
                if let Some(error) = response.error {
                    return Err(AppError::msg(format!(
                        "receipt request failed: {}",
                        error.message
                    )));
                }

                let result = response
                    .result
                    .ok_or_else(|| AppError::msg("receipt response missing result"))?;

                serde_json::from_value(result).map_err(|error| {
                    AppError::with_source("failed to decode backfill receipt", error)
                })
            })
            .collect()
    }

    /// Purpose: 여러 블록의 transaction receipt를 JSON-RPC batch로 조회
    /// Param:
    /// - `self`: RpcBackfillClient
    /// - `blocks`: receipt 조회 대상 block 목록
    async fn fetch_receipts_for_blocks(&self, blocks: &[RpcBlock]) -> Result<Vec<Vec<RpcReceipt>>> {
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

    /// Purpose: 단건 JSON-RPC 요청 전송 후 result 반환
    /// Param:
    /// - `self`: RpcBackfillClient
    /// - `method`: JSON-RPC method
    /// - `params`: JSON-RPC params
    async fn send_single(&self, method: &'static str, params: Value) -> Result<Value> {
        let response = self
            .http
            .post(&self.rpc_url)
            .json(&JsonRpcRequest {
                jsonrpc: "2.0",
                id: 1,
                method,
                params,
            })
            .send()
            .await
            .map_err(|error| AppError::with_source("failed to send JSON-RPC request", error))?
            .error_for_status()
            .map_err(|error| AppError::with_source("JSON-RPC request failed", error))?
            .json::<JsonRpcResponse>()
            .await
            .map_err(|error| AppError::with_source("failed to decode JSON-RPC response", error))?;

        if let Some(error) = response.error {
            return Err(AppError::msg(format!(
                "JSON-RPC request failed: {}",
                error.message
            )));
        }

        response
            .result
            .ok_or_else(|| AppError::msg("JSON-RPC response missing result"))
    }
}

/// Purpose: RPC block과 receipts를 proto Block으로 변환
/// Param:
/// - `chain_id`: chain_id 값
/// - `block`: RPC block
/// - `receipts`: block transaction 순서와 같은 receipts
fn rpc_block_to_proto(chain_id: i32, block: RpcBlock, receipts: Vec<RpcReceipt>) -> Result<Block> {
    if block.transactions.len() != receipts.len() {
        return Err(AppError::msg(format!(
            "backfill block has {} transactions but {} receipts",
            block.transactions.len(),
            receipts.len()
        )));
    }

    let transactions = block
        .transactions
        .into_iter()
        .zip(receipts)
        .map(|(transaction, receipt)| rpc_transaction_to_proto(transaction, receipt))
        .collect::<Result<Vec<_>>>()?;

    Ok(Block {
        number: hex_u64(&block.number)?,
        hash: hex_bytes_fixed(&block.hash, 32)?,
        parent_hash: hex_bytes_fixed(&block.parent_hash, 32)?,
        timestamp: hex_u64(&block.timestamp)?,
        transactions,
        chain_id: u64::try_from(chain_id)
            .map_err(|error| AppError::with_source("chain_id does not fit in u64", error))?,
    })
}

/// Purpose: RPC transaction과 receipt를 proto Transaction으로 변환
/// Param:
/// - `transaction`: RPC transaction
/// - `receipt`: transaction receipt
fn rpc_transaction_to_proto(
    transaction: RpcTransaction,
    receipt: RpcReceipt,
) -> Result<Transaction> {
    Ok(Transaction {
        hash: hex_bytes_fixed(&transaction.hash, 32)?,
        index: u32::try_from(hex_u64(&transaction.transaction_index)?).map_err(|error| {
            AppError::with_source("transaction index does not fit in u32", error)
        })?,
        from: hex_bytes_fixed(&transaction.from, 20)?,
        to: transaction
            .to
            .as_deref()
            .map(|to| hex_bytes_fixed(to, 20))
            .transpose()?,
        value_raw: transaction.value,
        logs: receipt
            .logs
            .into_iter()
            .map(rpc_log_to_proto)
            .collect::<Result<Vec<_>>>()?,
    })
}

/// Purpose: RPC log를 proto Log로 변환
/// Param:
/// - `log`: RPC log
fn rpc_log_to_proto(log: RpcLog) -> Result<Log> {
    Ok(Log {
        index: u32::try_from(hex_u64(&log.log_index)?)
            .map_err(|error| AppError::with_source("log index does not fit in u32", error))?,
        contract_address: hex_bytes_fixed(&log.address, 20)?,
        topics: log
            .topics
            .iter()
            .map(|topic| hex_bytes_fixed(topic, 32))
            .collect::<Result<Vec<_>>>()?,
        data: hex_bytes(&log.data)?,
    })
}

/// Purpose: 0x hex 문자열을 u64로 변환
/// Param:
/// - `value`: 변환할 0x hex 문자열
fn hex_u64(value: &str) -> Result<u64> {
    u64::from_str_radix(strip_0x(value)?, 16)
        .map_err(|error| AppError::with_source("failed to parse hex u64", error))
}

/// Purpose: 0x hex 문자열을 고정 길이 byte 배열로 변환
/// Param:
/// - `value`: 변환할 0x hex 문자열
/// - `expected_len`: 기대 byte 길이
fn hex_bytes_fixed(value: &str, expected_len: usize) -> Result<Vec<u8>> {
    let bytes = hex_bytes(value)?;
    if bytes.len() != expected_len {
        return Err(AppError::msg(format!(
            "invalid hex byte length: expected {expected_len}, got {}",
            bytes.len()
        )));
    }
    Ok(bytes)
}

/// Purpose: 0x hex 문자열을 byte 배열로 변환
/// Param:
/// - `value`: 변환할 0x hex 문자열
fn hex_bytes(value: &str) -> Result<Vec<u8>> {
    let hex = strip_0x(value)?;
    if hex.len() % 2 != 0 {
        return Err(AppError::msg("hex byte string has odd length"));
    }
    if !hex.chars().all(|char| char.is_ascii_hexdigit()) {
        return Err(AppError::msg(format!("invalid hex byte string {value:?}")));
    }

    hex.as_bytes()
        .chunks(2)
        .map(|chunk| {
            let byte = std::str::from_utf8(chunk)
                .map_err(|error| AppError::with_source("invalid hex byte utf8", error))?;
            u8::from_str_radix(byte, 16)
                .map_err(|error| AppError::with_source("failed to parse hex byte", error))
        })
        .collect()
}

/// Purpose: 0x prefix 제거
/// Param:
/// - `value`: 0x prefix 필요 value
fn strip_0x(value: &str) -> Result<&str> {
    value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .ok_or_else(|| AppError::msg(format!("hex value must start with 0x: {value:?}")))
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
struct RpcBlock {
    number: String,
    hash: String,
    parent_hash: String,
    timestamp: String,
    transactions: Vec<RpcTransaction>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RpcTransaction {
    hash: String,
    transaction_index: String,
    from: String,
    to: Option<String>,
    value: String,
}

#[derive(Debug, Deserialize)]
struct RpcReceipt {
    logs: Vec<RpcLog>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RpcLog {
    log_index: String,
    address: String,
    topics: Vec<String>,
    data: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_bytes_fixed_decodes_prefixed_hex() {
        // fixed hex bytes 변환 검증
        assert_eq!(hex_bytes_fixed("0x0102", 2).unwrap(), vec![1, 2]);
    }

    #[test]
    fn hex_u64_decodes_prefixed_hex() {
        // hex u64 변환 검증
        assert_eq!(hex_u64("0x0a").unwrap(), 10);
    }
}
