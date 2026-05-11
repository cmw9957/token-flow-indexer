mod abi;
mod erc;
mod erc1155;
mod hex;
mod native;

use crate::{
    error::Result,
    models::{AssetMovement, AssetType, BlockRecord, IndexedBlock, SourceType},
};

use abi::is_zero_hex_or_decimal;
use erc::log_movements;
use hex::{normalize_address, normalize_hash};
use native::native_movement;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawBlock {
    pub chain_id: i32,
    pub block_number: i64,
    pub block_hash: String,
    pub parent_hash: String,
    pub block_timestamp: i64,
    pub transactions: Vec<RawTransaction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawTransaction {
    pub tx_hash: String,
    pub tx_index: i32,
    pub from_address: String,
    pub to_address: Option<String>,
    pub value_raw: String,
    pub logs: Vec<RawLog>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawLog {
    pub log_index: i32,
    pub contract_address: String,
    pub topics: Vec<String>,
    pub data: String,
}

pub struct Extractor;

impl Extractor {
    /// Purpose: 원시 블록에서 native/ERC token flow 추출
    /// Param:
    /// - `block`: 추출 대상 RawBlock
    pub fn extract_block(block: RawBlock) -> Result<IndexedBlock> {
        let mut movements = Vec::new();

        for transaction in &block.transactions {
            if !is_zero_hex_or_decimal(&transaction.value_raw) {
                movements.push(native_movement(&block, transaction)?);
            }

            for log in &transaction.logs {
                movements.extend(log_movements(&block, transaction, log)?);
            }
        }

        let record = BlockRecord {
            chain_id: block.chain_id,
            block_number: block.block_number,
            block_hash: normalize_hash(&block.block_hash)?,
            parent_hash: normalize_hash(&block.parent_hash)?,
            block_timestamp: block.block_timestamp,
            tx_count: block.transactions.len() as i32,
            movement_count: 0,
        };

        Ok(IndexedBlock::new(record, movements))
    }
}

/// Purpose: 로그 기반 자산 이동 공통 필드 조립
/// Param:
/// - `block`: log 포함 RawBlock
/// - `transaction`: log 포함 RawTransaction
/// - `log`: movement 원본 RawLog
/// - `asset_type`: 추출된 asset_type
/// - `from_address`: from_address 값
/// - `to_address`: to_address 값
/// - `token_id`: NFT 또는 ERC1155 token_id
/// - `amount_raw`: raw amount_raw
/// - `log_sub_index`: log 내부 log_sub_index
fn base_log_movement(
    block: &RawBlock,
    transaction: &RawTransaction,
    log: &RawLog,
    asset_type: AssetType,
    from_address: String,
    to_address: String,
    token_id: Option<String>,
    amount_raw: String,
    log_sub_index: i32,
) -> Result<AssetMovement> {
    Ok(AssetMovement {
        chain_id: block.chain_id,
        block_number: block.block_number,
        block_hash: normalize_hash(&block.block_hash)?,
        block_timestamp: block.block_timestamp,
        tx_hash: normalize_hash(&transaction.tx_hash)?,
        tx_index: transaction.tx_index,
        source_type: SourceType::Log,
        asset_type,
        token_address: Some(normalize_address(&log.contract_address)?),
        from_address,
        to_address: Some(to_address),
        token_id,
        amount_raw,
        log_index: Some(log.log_index),
        log_sub_index,
    })
}

#[cfg(test)]
mod tests {
    use crate::models::{AssetType, SourceType};

    use super::*;

    #[test]
    fn extract_block_emits_native_movement_from_transaction_value() {
        // native ETH 이동 추출 검증
        let block = RawBlock {
            chain_id: 1,
            block_number: 10,
            block_hash: format!("0x{}", "11".repeat(32)),
            parent_hash: format!("0x{}", "22".repeat(32)),
            block_timestamp: 1_700_000_000,
            transactions: vec![RawTransaction {
                tx_hash: format!("0x{}", "33".repeat(32)),
                tx_index: 0,
                from_address: format!("0x{}", "44".repeat(20)),
                to_address: Some(format!("0x{}", "55".repeat(20))),
                value_raw: "0x0a".to_owned(),
                logs: Vec::new(),
            }],
        };

        let indexed = Extractor::extract_block(block).unwrap();

        assert_eq!(indexed.record.movement_count, 1);
        assert_eq!(indexed.movements.len(), 1);
        assert_eq!(indexed.movements[0].source_type, SourceType::TxValue);
        assert_eq!(indexed.movements[0].asset_type, AssetType::Native);
        assert_eq!(indexed.movements[0].amount_raw, "10");
    }

    #[test]
    fn extract_block_emits_erc20_transfer_movement_from_log() {
        // ERC20 Transfer 로그 추출 검증
        let from_address = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let to_address = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let block = RawBlock {
            chain_id: 1,
            block_number: 10,
            block_hash: format!("0x{}", "11".repeat(32)),
            parent_hash: format!("0x{}", "22".repeat(32)),
            block_timestamp: 1_700_000_000,
            transactions: vec![RawTransaction {
                tx_hash: format!("0x{}", "33".repeat(32)),
                tx_index: 0,
                from_address: format!("0x{}", "44".repeat(20)),
                to_address: Some(format!("0x{}", "55".repeat(20))),
                value_raw: "0".to_owned(),
                logs: vec![RawLog {
                    log_index: 7,
                    contract_address: format!("0x{}", "66".repeat(20)),
                    topics: vec![
                        erc::TRANSFER_TOPIC.to_owned(),
                        format!("0x{}{}", "00".repeat(12), from_address),
                        format!("0x{}{}", "00".repeat(12), to_address),
                    ],
                    data: format!("0x{}05", "0".repeat(62)),
                }],
            }],
        };

        let indexed = Extractor::extract_block(block).unwrap();

        assert_eq!(indexed.record.movement_count, 1);
        assert_eq!(indexed.movements[0].source_type, SourceType::Log);
        assert_eq!(indexed.movements[0].asset_type, AssetType::Erc20);
        assert_eq!(
            indexed.movements[0].from_address,
            format!("0x{from_address}")
        );
        assert_eq!(
            indexed.movements[0].to_address,
            Some(format!("0x{to_address}"))
        );
        assert_eq!(indexed.movements[0].amount_raw, "5");
        assert_eq!(indexed.movements[0].log_index, Some(7));
    }
}
