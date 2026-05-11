use crate::{
    error::{AppError, Result},
    extractor::hex::{strip_0x, strip_optional_0x},
};

/// Purpose: ABI encoded uint256 배열 두 개 디코딩
/// Param:
/// - `words`: 32-byte hex words
pub(super) fn decode_two_uint_arrays(words: &[String]) -> Result<(Vec<String>, Vec<String>)> {
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
pub(super) fn data_words(data: &str) -> Result<Vec<String>> {
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

/// Purpose: hex 또는 decimal uint 문자열 정규화
/// Param:
/// - `value`: 정규화할 uint value
pub(super) fn normalize_uint(value: &str) -> Result<String> {
    if value.starts_with("0x") || value.starts_with("0X") {
        return hex_word_to_decimal(strip_0x(value)?);
    }

    if value.len() == 64 && value.chars().all(|char| char.is_ascii_hexdigit()) {
        return hex_word_to_decimal(value);
    }

    if value.chars().all(|char| char.is_ascii_digit()) {
        return Ok(trim_decimal_zeros(value));
    }

    if value.chars().all(|char| char.is_ascii_hexdigit()) {
        return hex_word_to_decimal(value);
    }

    Err(AppError::msg(format!("invalid uint value {value:?}")))
}

/// Purpose: hex word를 usize로 변환
/// Param:
/// - `word`: hex word
fn hex_word_to_usize(word: &str) -> Result<usize> {
    usize::from_str_radix(strip_optional_0x(word), 16)
        .map_err(|error| AppError::with_source("failed to parse uint word as usize", error))
}

/// Purpose: hex uint를 decimal 문자열로 변환
/// Param:
/// - `hex`: hex uint
pub(super) fn hex_word_to_decimal(hex: &str) -> Result<String> {
    let hex = strip_optional_0x(hex);
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
pub(super) fn is_zero_hex_or_decimal(value: &str) -> bool {
    normalize_uint(value).map_or(false, |value| value == "0")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_uint_accepts_abi_word_without_0x() {
        // 0x 없는 ABI uint word 검증
        let value = "00000000000000000000000000000000000000000000000000000000d6343529";

        assert_eq!(normalize_uint(value).unwrap(), "3593745705");
    }

    #[test]
    fn normalize_uint_keeps_decimal_values_decimal() {
        // decimal uint 정규화 검증
        assert_eq!(normalize_uint("000123").unwrap(), "123");
    }

    #[test]
    fn hex_word_to_decimal_accepts_optional_0x() {
        // optional 0x hex 변환 검증
        assert_eq!(hex_word_to_decimal("0x0a").unwrap(), "10");
        assert_eq!(hex_word_to_decimal("0a").unwrap(), "10");
    }
}
