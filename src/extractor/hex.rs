use crate::error::{AppError, Result};

/// Purpose: indexed address topic에서 주소 추출
/// Param:
/// - `topic`: 32-byte topic
pub(super) fn topic_address(topic: &str) -> Result<String> {
    let hex = strip_0x(topic)?;
    if hex.len() != 64 {
        return Err(AppError::msg("indexed address topic must be 32 bytes"));
    }

    normalize_address(&format!("0x{}", &hex[24..]))
}

/// Purpose: indexed uint topic을 decimal 문자열로 변환
/// Param:
/// - `topic`: 32-byte topic
pub(super) fn topic_uint(topic: &str) -> Result<String> {
    let hex = strip_0x(topic)?;
    if hex.len() != 64 {
        return Err(AppError::msg("indexed uint topic must be 32 bytes"));
    }

    super::abi::hex_word_to_decimal(hex)
}

/// Purpose: 해시 hex 문자열 정규화
/// Param:
/// - `value`: 정규화할 hash value
pub(super) fn normalize_hash(value: &str) -> Result<String> {
    normalize_hex(value, 64, "hash")
}

/// Purpose: 주소 hex 문자열 정규화
/// Param:
/// - `value`: 정규화할 address value
pub(super) fn normalize_address(value: &str) -> Result<String> {
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
pub(super) fn strip_0x(value: &str) -> Result<&str> {
    value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .ok_or_else(|| AppError::msg(format!("hex value must start with 0x: {value:?}")))
}

/// Purpose: optional 0x prefix 제거
/// Param:
/// - `value`: 0x prefix가 있거나 없는 hex value
pub(super) fn strip_optional_0x(value: &str) -> &str {
    value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or(value)
}
