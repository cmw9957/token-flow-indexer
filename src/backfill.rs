use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    error::{AppError, Result},
    proto::{Block, Log, Transaction},
};

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
}

impl BackfillSource for RpcBackfillClient {
    async fn fetch_block(&self, chain_id: i32, block_number: i64) -> Result<Block> {
        let block = self.fetch_rpc_block(block_number).await?;
        let receipts = self.fetch_receipts(&block.transactions).await?;

        rpc_block_to_proto(chain_id, block, receipts)
    }
}

impl RpcBackfillClient {
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

        let mut responses = self
            .http
            .post(&self.rpc_url)
            .json(&requests)
            .send()
            .await
            .map_err(|error| AppError::with_source("failed to send receipt batch request", error))?
            .error_for_status()
            .map_err(|error| AppError::with_source("receipt batch request failed", error))?
            .json::<Vec<JsonRpcResponse>>()
            .await
            .map_err(|error| {
                AppError::with_source("failed to decode receipt batch response", error)
            })?;

        responses.sort_by_key(|response| response.id);

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

fn hex_u64(value: &str) -> Result<u64> {
    u64::from_str_radix(strip_0x(value)?, 16)
        .map_err(|error| AppError::with_source("failed to parse hex u64", error))
}

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

fn strip_0x(value: &str) -> Result<&str> {
    value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .ok_or_else(|| AppError::msg(format!("hex value must start with 0x: {value:?}")))
}

#[derive(Debug, Serialize)]
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
