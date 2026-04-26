use crate::{
    error::{AppError, Result},
    models::{AssetMovement, AssetType, BlockRecord, SourceType},
    processor::IndexedBlock,
};

const TRANSFER_TOPIC: &str = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";
const TRANSFER_SINGLE_TOPIC: &str =
    "0xc3d58168c5ae7397731d063d5bbf3d601eaf52b74f8d4c987e20ee57f798f";
const TRANSFER_BATCH_TOPIC: &str =
    "0x4a39dc06d4c0dbc64b70a9e5e3c6760d4e5f0d5e4f7f2d1e1c4e481e7fdef7";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawBlock {
    pub chain_id: i32,
    pub block_number: i64,
    pub block_hash: String,
    pub parent_hash: String,
    pub block_timestamp: String,
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

/// Purpose: 트랜잭션 value 기반 native 자산 이동 생성
/// Param:
/// - `block`: transaction 포함 RawBlock
/// - `transaction`: native value 포함 RawTransaction
fn native_movement(block: &RawBlock, transaction: &RawTransaction) -> Result<AssetMovement> {
    Ok(AssetMovement {
        chain_id: block.chain_id,
        block_number: block.block_number,
        block_hash: normalize_hash(&block.block_hash)?,
        block_timestamp: block.block_timestamp.clone(),
        tx_hash: normalize_hash(&transaction.tx_hash)?,
        tx_index: transaction.tx_index,
        source_type: SourceType::TxValue,
        asset_type: AssetType::Native,
        token_address: None,
        from_address: normalize_address(&transaction.from_address)?,
        to_address: transaction
            .to_address
            .as_deref()
            .map(normalize_address)
            .transpose()?,
        token_id: None,
        amount_raw: normalize_uint(&transaction.value_raw)?,
        log_index: None,
        log_sub_index: 0,
    })
}

/// Purpose: 로그 topic 기준 ERC 자산 이동 추출
/// Param:
/// - `block`: log 포함 RawBlock
/// - `transaction`: log 포함 RawTransaction
/// - `log`: 분석할 RawLog
fn log_movements(
    block: &RawBlock,
    transaction: &RawTransaction,
    log: &RawLog,
) -> Result<Vec<AssetMovement>> {
    let Some(topic0) = log.topics.first() else {
        return Ok(Vec::new());
    };

    match topic0.to_ascii_lowercase().as_str() {
        TRANSFER_TOPIC => erc_transfer_movement(block, transaction, log)
            .map(|movement| movement.map_or_else(Vec::new, |movement| vec![movement])),
        TRANSFER_SINGLE_TOPIC => erc1155_single_movement(block, transaction, log)
            .map(|movement| movement.map_or_else(Vec::new, |movement| vec![movement])),
        TRANSFER_BATCH_TOPIC => erc1155_batch_movements(block, transaction, log),
        _ => Ok(Vec::new()),
    }
}

/// Purpose: ERC20/ERC721 Transfer 로그를 자산 이동으로 변환
/// Param:
/// - `block`: log 포함 RawBlock
/// - `transaction`: log 포함 RawTransaction
/// - `log`: Transfer RawLog
fn erc_transfer_movement(
    block: &RawBlock,
    transaction: &RawTransaction,
    log: &RawLog,
) -> Result<Option<AssetMovement>> {
    if log.topics.len() == 3 {
        return Ok(Some(base_log_movement(
            block,
            transaction,
            log,
            AssetType::Erc20,
            topic_address(&log.topics[1])?,
            topic_address(&log.topics[2])?,
            None,
            normalize_uint(&log.data)?,
            0,
        )?));
    }

    if log.topics.len() == 4 {
        return Ok(Some(base_log_movement(
            block,
            transaction,
            log,
            AssetType::Erc721,
            topic_address(&log.topics[1])?,
            topic_address(&log.topics[2])?,
            Some(topic_uint(&log.topics[3])?),
            "1".to_owned(),
            0,
        )?));
    }

    Ok(None)
}

/// Purpose: ERC1155 TransferSingle 로그를 자산 이동으로 변환
/// Param:
/// - `block`: log 포함 RawBlock
/// - `transaction`: log 포함 RawTransaction
/// - `log`: TransferSingle RawLog
fn erc1155_single_movement(
    block: &RawBlock,
    transaction: &RawTransaction,
    log: &RawLog,
) -> Result<Option<AssetMovement>> {
    if log.topics.len() != 4 {
        return Ok(None);
    }

    let words = data_words(&log.data)?;
    if words.len() != 2 {
        return Ok(None);
    }

    Ok(Some(base_log_movement(
        block,
        transaction,
        log,
        AssetType::Erc1155,
        topic_address(&log.topics[2])?,
        topic_address(&log.topics[3])?,
        Some(hex_word_to_decimal(&words[0])?),
        hex_word_to_decimal(&words[1])?,
        0,
    )?))
}

/// Purpose: ERC1155 TransferBatch 로그를 자산 이동 목록으로 변환
/// Param:
/// - `block`: log 포함 RawBlock
/// - `transaction`: log 포함 RawTransaction
/// - `log`: TransferBatch RawLog
fn erc1155_batch_movements(
    block: &RawBlock,
    transaction: &RawTransaction,
    log: &RawLog,
) -> Result<Vec<AssetMovement>> {
    if log.topics.len() != 4 {
        return Ok(Vec::new());
    }

    let words = data_words(&log.data)?;
    let (ids, values) = decode_two_uint_arrays(&words)?;
    if ids.len() != values.len() {
        return Ok(Vec::new());
    }

    let mut movements = Vec::with_capacity(ids.len());
    for (index, (token_id, amount)) in ids.into_iter().zip(values).enumerate() {
        movements.push(base_log_movement(
            block,
            transaction,
            log,
            AssetType::Erc1155,
            topic_address(&log.topics[2])?,
            topic_address(&log.topics[3])?,
            Some(token_id),
            amount,
            index as i32,
        )?);
    }

    Ok(movements)
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
        block_timestamp: block.block_timestamp.clone(),
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

/// Purpose: ABI encoded uint256 배열 두 개 디코딩
/// Param:
/// - `words`: 32-byte hex words
fn decode_two_uint_arrays(words: &[String]) -> Result<(Vec<String>, Vec<String>)> {
    if words.len() < 2 {
        return Ok((Vec::new(), Vec::new()));
    }

    let first_offset = hex_word_to_usize(&words[0])? / 32;
    let second_offset = hex_word_to_usize(&words[1])? / 32;

    let first = decode_uint_array_at(words, first_offset)?;
    let second = decode_uint_array_at(words, second_offset)?;

    Ok((first, second))
}

/// Purpose: 지정 offset의 ABI encoded uint256 배열 디코딩
/// Param:
/// - `words`: 32-byte hex words
/// - `offset_words`: array 시작 offset_words
fn decode_uint_array_at(words: &[String], offset_words: usize) -> Result<Vec<String>> {
    let Some(length_word) = words.get(offset_words) else {
        return Ok(Vec::new());
    };

    let length = hex_word_to_usize(length_word)?;
    let mut values = Vec::with_capacity(length);

    for index in 0..length {
        let Some(word) = words.get(offset_words + 1 + index) else {
            return Ok(Vec::new());
        };
        values.push(hex_word_to_decimal(word)?);
    }

    Ok(values)
}

/// Purpose: 로그 data를 32바이트 hex word 목록으로 분할
/// Param:
/// - `data`: 0x prefix log data
fn data_words(data: &str) -> Result<Vec<String>> {
    let hex = strip_0x(data)?;
    if hex.is_empty() {
        return Ok(Vec::new());
    }
    if hex.len() % 64 != 0 {
        return Err(AppError::msg(
            "log data length is not a multiple of 32 bytes",
        ));
    }

    Ok(hex
        .as_bytes()
        .chunks(64)
        .map(|chunk| String::from_utf8_lossy(chunk).into_owned())
        .collect())
}

/// Purpose: indexed address topic에서 주소 추출
/// Param:
/// - `topic`: 32-byte topic
fn topic_address(topic: &str) -> Result<String> {
    let hex = strip_0x(topic)?;
    if hex.len() != 64 {
        return Err(AppError::msg("indexed address topic must be 32 bytes"));
    }

    normalize_address(&format!("0x{}", &hex[24..]))
}

/// Purpose: indexed uint topic을 decimal 문자열로 변환
/// Param:
/// - `topic`: 32-byte topic
fn topic_uint(topic: &str) -> Result<String> {
    let hex = strip_0x(topic)?;
    if hex.len() != 64 {
        return Err(AppError::msg("indexed uint topic must be 32 bytes"));
    }

    hex_word_to_decimal(hex)
}

/// Purpose: hex 또는 decimal uint 문자열 정규화
/// Param:
/// - `value`: 정규화할 uint value
fn normalize_uint(value: &str) -> Result<String> {
    if value.starts_with("0x") || value.starts_with("0X") {
        return hex_word_to_decimal(strip_0x(value)?);
    }

    if value.chars().all(|char| char.is_ascii_digit()) {
        return Ok(trim_decimal_zeros(value));
    }

    Err(AppError::msg(format!("invalid uint value {value:?}")))
}

/// Purpose: hex word를 usize로 변환
/// Param:
/// - `word`: hex word
fn hex_word_to_usize(word: &str) -> Result<usize> {
    usize::from_str_radix(strip_0x(word)?, 16)
        .map_err(|error| AppError::with_source("failed to parse uint word as usize", error))
}

/// Purpose: hex uint를 decimal 문자열로 변환
/// Param:
/// - `hex`: hex uint
fn hex_word_to_decimal(hex: &str) -> Result<String> {
    let hex = strip_0x(hex)?;
    if hex.is_empty() {
        return Ok("0".to_owned());
    }
    if !hex.chars().all(|char| char.is_ascii_hexdigit()) {
        return Err(AppError::msg(format!("invalid hex uint {hex:?}")));
    }

    let mut decimal = String::from("0");
    for digit in hex.chars() {
        decimal = decimal_mul_small(&decimal, 16);
        decimal = decimal_add_small(&decimal, digit.to_digit(16).expect("hex digit") as u8);
    }

    Ok(trim_decimal_zeros(&decimal))
}

/// Purpose: decimal 문자열에 작은 정수 곱셈 적용
/// Param:
/// - `decimal`: decimal 값
/// - `multiplier`: multiplier 값
fn decimal_mul_small(decimal: &str, multiplier: u8) -> String {
    let mut carry = 0u16;
    let mut digits = Vec::with_capacity(decimal.len() + 1);

    for byte in decimal.bytes().rev() {
        let value = ((byte - b'0') as u16 * multiplier as u16) + carry;
        digits.push((value % 10) as u8 + b'0');
        carry = value / 10;
    }

    while carry > 0 {
        digits.push((carry % 10) as u8 + b'0');
        carry /= 10;
    }

    digits.reverse();
    String::from_utf8(digits).expect("decimal digits")
}

/// Purpose: decimal 문자열에 작은 정수 덧셈 적용
/// Param:
/// - `decimal`: decimal 값
/// - `addend`: addend 값
fn decimal_add_small(decimal: &str, addend: u8) -> String {
    let mut carry = addend as u16;
    let mut digits = Vec::with_capacity(decimal.len() + 1);

    for byte in decimal.bytes().rev() {
        let value = (byte - b'0') as u16 + carry;
        digits.push((value % 10) as u8 + b'0');
        carry = value / 10;
    }

    while carry > 0 {
        digits.push((carry % 10) as u8 + b'0');
        carry /= 10;
    }

    digits.reverse();
    String::from_utf8(digits).expect("decimal digits")
}

/// Purpose: hex 또는 decimal 문자열의 0 값 여부 확인
/// Param:
/// - `value`: 확인할 uint value
fn is_zero_hex_or_decimal(value: &str) -> bool {
    normalize_uint(value).map_or(false, |value| value == "0")
}

/// Purpose: 해시 hex 문자열 정규화
/// Param:
/// - `value`: 정규화할 hash value
fn normalize_hash(value: &str) -> Result<String> {
    normalize_hex(value, 64, "hash")
}

/// Purpose: 주소 hex 문자열 정규화
/// Param:
/// - `value`: 정규화할 address value
fn normalize_address(value: &str) -> Result<String> {
    normalize_hex(value, 40, "address")
}

/// Purpose: 0x hex 문자열 길이와 문자 검증 후 소문자 정규화
/// Param:
/// - `value`: 정규화할 hex value
/// - `expected_len`: 0x 제외 expected_len
/// - `name`: error message용 name
fn normalize_hex(value: &str, expected_len: usize, name: &str) -> Result<String> {
    let hex = strip_0x(value)?;
    if hex.len() != expected_len {
        return Err(AppError::msg(format!(
            "invalid {name} length: expected {expected_len} hex chars, got {}",
            hex.len()
        )));
    }
    if !hex.chars().all(|char| char.is_ascii_hexdigit()) {
        return Err(AppError::msg(format!("invalid {name} hex value {value:?}")));
    }

    Ok(format!("0x{}", hex.to_ascii_lowercase()))
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

/// Purpose: decimal 문자열의 앞쪽 0 제거
/// Param:
/// - `value`: 정리할 decimal value
fn trim_decimal_zeros(value: &str) -> String {
    let trimmed = value.trim_start_matches('0');
    if trimmed.is_empty() {
        "0".to_owned()
    } else {
        trimmed.to_owned()
    }
}
