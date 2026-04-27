use std::{env, time::Duration};

const DEFAULT_CHAIN_NAME: &str = "ethereum";
const DEFAULT_RECONNECT_DELAY_SECS: u64 = 3;

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub exex_endpoint: String,
    pub backfill_rpc_url: String,
    pub chain_id: i32,
    pub chain_name: String,
    pub reconnect_delay: Duration,
}

impl Config {
    /// Purpose: 환경변수 기반 설정 생성
    /// Param: None
    pub fn from_env() -> Result<Self, ConfigError> {
        let database_url = required_env("DATABASE_URL")?;
        let exex_endpoint = required_env("EXEX_INDEXER_GRPC_ENDPOINT")?;
        let backfill_rpc_url = required_env("BACKFILL_RPC_URL")?;
        let chain_id = parse_required_env("CHAIN_ID")?;
        let chain_name = env::var("CHAIN_NAME").unwrap_or_else(|_| DEFAULT_CHAIN_NAME.into());
        let reconnect_delay_secs =
            parse_optional_env("EXEX_RECONNECT_DELAY_SECS", DEFAULT_RECONNECT_DELAY_SECS)?;

        Ok(Self {
            database_url,
            exex_endpoint,
            backfill_rpc_url,
            chain_id,
            chain_name,
            reconnect_delay: Duration::from_secs(reconnect_delay_secs),
        })
    }
}

#[derive(Debug)]
pub enum ConfigError {
    MissingEnv {
        name: &'static str,
    },
    InvalidEnv {
        name: &'static str,
        value: String,
        message: String,
    },
}

impl std::fmt::Display for ConfigError {
    /// Purpose: 설정 에러 출력 문자열 작성
    /// Param:
    /// - `self`: 출력할 ConfigError
    /// - `f`: fmt formatter
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingEnv { name } => write!(f, "missing required environment variable {name}"),
            Self::InvalidEnv {
                name,
                value,
                message,
            } => {
                write!(
                    f,
                    "invalid environment variable {name}={value:?}: {message}"
                )
            }
        }
    }
}

impl std::error::Error for ConfigError {}

/// Purpose: 필수 환경변수 조회
/// Param:
/// - `name`: env var name
fn required_env(name: &'static str) -> Result<String, ConfigError> {
    env::var(name).map_err(|_| ConfigError::MissingEnv { name })
}

/// Purpose: 필수 환경변수 조회 후 지정 타입으로 변환
/// Param:
/// - `name`: env var name
fn parse_required_env<T>(name: &'static str) -> Result<T, ConfigError>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    let value = required_env(name)?;
    parse_env_value(name, value)
}

/// Purpose: 선택 환경변수 조회 후 지정 타입으로 변환
/// Param:
/// - `name`: env var name
/// - `default`: env var 미설정 시 default
fn parse_optional_env<T>(name: &'static str, default: T) -> Result<T, ConfigError>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    match env::var(name) {
        Ok(value) => parse_env_value(name, value),
        Err(env::VarError::NotPresent) => Ok(default),
        Err(error) => Err(ConfigError::InvalidEnv {
            name,
            value: String::new(),
            message: error.to_string(),
        }),
    }
}

/// Purpose: 환경변수 문자열을 지정 타입으로 변환
/// Param:
/// - `name`: env var name
/// - `value`: 변환할 env var value
fn parse_env_value<T>(name: &'static str, value: String) -> Result<T, ConfigError>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    value
        .parse()
        .map_err(|error: T::Err| ConfigError::InvalidEnv {
            name,
            value,
            message: error.to_string(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_env_value_converts_to_target_type() {
        // 타입 변환 검증
        let value = parse_env_value::<i32>("CHAIN_ID", "1".to_owned()).unwrap();

        assert_eq!(value, 1);
    }

    #[test]
    fn parse_env_value_reports_invalid_input() {
        // 잘못된 환경변수 검증
        let error = parse_env_value::<i32>("CHAIN_ID", "mainnet".to_owned()).unwrap_err();

        assert!(matches!(
            error,
            ConfigError::InvalidEnv {
                name: "CHAIN_ID",
                ..
            }
        ));
        assert!(error.to_string().contains("CHAIN_ID"));
        assert!(error.to_string().contains("mainnet"));
    }

    #[test]
    fn config_error_formats_missing_required_env() {
        // 필수 환경변수 누락 메시지 검증
        let error = ConfigError::MissingEnv {
            name: "DATABASE_URL",
        };

        assert_eq!(
            error.to_string(),
            "missing required environment variable DATABASE_URL"
        );
    }
}
