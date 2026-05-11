use crate::{
    error::{AppError, Result},
    hex::{decode_bytes, decode_fixed_bytes, parse_u64},
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
        number: parse_u64(&block.number)?,
        hash: decode_fixed_bytes(&block.hash, 32)?,
        parent_hash: decode_fixed_bytes(&block.parent_hash, 32)?,
        timestamp: parse_u64(&block.timestamp)?,
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
        hash: decode_fixed_bytes(&transaction.hash, 32)?,
        index: u32::try_from(parse_u64(&transaction.transaction_index)?).map_err(|error| {
            AppError::with_source("transaction index does not fit in u32", error)
        })?,
        from: decode_fixed_bytes(&transaction.from, 20)?,
        to: transaction
            .to
            .as_deref()
            .map(|to| decode_fixed_bytes(to, 20))
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
        index: u32::try_from(parse_u64(&log.log_index)?)
            .map_err(|error| AppError::with_source("log index does not fit in u32", error))?,
        contract_address: decode_fixed_bytes(&log.address, 20)?,
        topics: log
            .topics
            .iter()
            .map(|topic| decode_fixed_bytes(topic, 32))
            .collect::<Result<Vec<_>>>()?,
        data: decode_bytes(&log.data)?,
    })
}
