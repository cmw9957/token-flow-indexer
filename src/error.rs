use std::{error::Error as StdError, fmt};

pub type Result<T> = std::result::Result<T, AppError>;

#[derive(Debug)]
pub struct AppError {
    message: String,
    source: Option<Box<dyn StdError + Send + Sync>>,
}

impl AppError {
    /// Purpose: 메시지만 가진 애플리케이션 에러 생성
    /// Param:
    /// - `message`: error message
    pub fn msg(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            source: None,
        }
    }

    /// Purpose: 원인 에러를 포함한 애플리케이션 에러 생성
    /// Param:
    /// - `message`: error message
    /// - `source`: source error
    pub fn with_source(
        message: impl Into<String>,
        source: impl StdError + Send + Sync + 'static,
    ) -> Self {
        Self {
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }
}

impl fmt::Display for AppError {
    /// Purpose: 애플리케이션 에러 출력 문자열 작성
    /// Param:
    /// - `self`: 출력할 AppError
    /// - `f`: fmt formatter
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl StdError for AppError {
    /// Purpose: 원인 에러 참조 반환
    /// Param:
    /// - `self`: source 조회 대상 AppError
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &(dyn StdError + 'static))
    }
}

impl From<String> for AppError {
    /// Purpose: 문자열 소유값을 애플리케이션 에러로 변환
    /// Param:
    /// - `message`: error message
    fn from(message: String) -> Self {
        Self::msg(message)
    }
}

impl From<&str> for AppError {
    /// Purpose: 문자열 참조를 애플리케이션 에러로 변환
    /// Param:
    /// - `message`: error message
    fn from(message: &str) -> Self {
        Self::msg(message)
    }
}
