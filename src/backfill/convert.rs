use crate::{
    error::{AppError, Result},
    proto::{Block, Log, Transaction},
};

use super::rpc::{RpcBlock, RpcLog, RpcReceipt, RpcTransaction};

/// Purpose: RPC block과 receipts를 proto Block으로 변환
/// Param:
/// - `chain_id`: chain_id 값
/// - `block`: RPC block
/// - `receipts`: block transaction 순서와 같은 receipts
pub(super) fn rpc_block_to_proto(
    chain_id: i32,
    block: RpcBlock,
    receipts: Vec<RpcReceipt>,
) -> Result<Block> {
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
