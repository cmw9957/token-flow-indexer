use crate::error::{AppError, Result};

/// Purpose: 0x prefix 제거
/// Param:
/// - `value`: 0x prefix 필요 value
pub fn strip_0x(value: &str) -> Result<&str> {
    value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .ok_or_else(|| AppError::msg(format!("hex value must start with 0x: {value:?}")))
}

/// Purpose: optional 0x prefix 제거
/// Param:
/// - `value`: 0x prefix가 있거나 없는 hex value
pub fn strip_optional_0x(value: &str) -> &str {
    value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or(value)
}

/// Purpose: bytes를 0x prefix hex 문자열로 변환
/// Param:
/// - `bytes`: 변환할 bytes
pub fn encode_prefixed(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(2 + bytes.len() * 2);
    out.push_str("0x");
    for byte in bytes {
        out.push(hex_char(byte >> 4));
        out.push(hex_char(byte & 0x0f));
    }
    out
}

/// Purpose: 0x hex 문자열을 byte 배열로 변환
/// Param:
/// - `value`: 변환할 0x hex 문자열
pub fn decode_bytes(value: &str) -> Result<Vec<u8>> {
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

/// Purpose: 0x hex 문자열을 고정 길이 byte 배열로 변환
/// Param:
/// - `value`: 변환할 0x hex 문자열
/// - `expected_len`: 기대 byte 길이
pub fn decode_fixed_bytes(value: &str, expected_len: usize) -> Result<Vec<u8>> {
    let bytes = decode_bytes(value)?;
    if bytes.len() != expected_len {
        return Err(AppError::msg(format!(
            "invalid hex byte length: expected {expected_len}, got {}",
            bytes.len()
        )));
    }
    Ok(bytes)
}

/// Purpose: 0x hex 문자열을 u64로 변환
/// Param:
/// - `value`: 변환할 0x hex 문자열
pub fn parse_u64(value: &str) -> Result<u64> {
    u64::from_str_radix(strip_0x(value)?, 16)
        .map_err(|error| AppError::with_source("failed to parse hex u64", error))
}

/// Purpose: 4비트 값을 hex 문자로 변환
/// Param:
/// - `value`: 0~15 value
fn hex_char(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        10..=15 => (b'a' + value - 10) as char,
        _ => unreachable!("nibble is always <= 15"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_fixed_bytes_decodes_prefixed_hex() {
        // fixed hex bytes 변환 검증
        assert_eq!(decode_fixed_bytes("0x0102", 2).unwrap(), vec![1, 2]);
    }

    #[test]
    fn parse_u64_decodes_prefixed_hex() {
        // hex u64 변환 검증
        assert_eq!(parse_u64("0x0a").unwrap(), 10);
    }
}
