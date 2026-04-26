use std::{fmt, str::FromStr};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockRecord {
    pub chain_id: i32,
    pub block_number: i64,
    pub block_hash: String,
    pub parent_hash: String,
    pub block_timestamp: String,
    pub tx_count: i32,
    pub movement_count: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetMovement {
    pub chain_id: i32,
    pub block_number: i64,
    pub block_hash: String,
    pub block_timestamp: String,
    pub tx_hash: String,
    pub tx_index: i32,
    pub source_type: SourceType,
    pub asset_type: AssetType,
    pub token_address: Option<String>,
    pub from_address: String,
    pub to_address: Option<String>,
    pub token_id: Option<String>,
    pub amount_raw: String,
    pub log_index: Option<i32>,
    pub log_sub_index: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncCheckpoint {
    pub chain_id: i32,
    pub last_indexed_block: Option<i64>,
    pub last_indexed_hash: Option<String>,
    pub status: SyncStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceType {
    TxValue,
    Log,
}

impl SourceType {
    /// Purpose: SourceType을 DB 저장 문자열로 변환
    /// Param:
    /// - `self`: 변환할 SourceType
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TxValue => "TX_VALUE",
            Self::Log => "LOG",
        }
    }
}

impl fmt::Display for SourceType {
    /// Purpose: SourceType 출력 문자열 작성
    /// Param:
    /// - `self`: 출력할 SourceType
    /// - `f`: fmt formatter
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for SourceType {
    type Err = ParseEnumError;

    /// Purpose: DB 문자열을 SourceType으로 변환
    /// Param:
    /// - `value`: 변환할 DB value
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "TX_VALUE" => Ok(Self::TxValue),
            "LOG" => Ok(Self::Log),
            _ => Err(ParseEnumError::new("SourceType", value)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetType {
    Native,
    Erc20,
    Erc721,
    Erc1155,
    Unknown,
}

impl AssetType {
    /// Purpose: AssetType을 DB 저장 문자열로 변환
    /// Param:
    /// - `self`: 변환할 AssetType
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Native => "NATIVE",
            Self::Erc20 => "ERC20",
            Self::Erc721 => "ERC721",
            Self::Erc1155 => "ERC1155",
            Self::Unknown => "UNKNOWN",
        }
    }
}

impl fmt::Display for AssetType {
    /// Purpose: AssetType 출력 문자열 작성
    /// Param:
    /// - `self`: 출력할 AssetType
    /// - `f`: fmt formatter
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for AssetType {
    type Err = ParseEnumError;

    /// Purpose: DB 문자열을 AssetType으로 변환
    /// Param:
    /// - `value`: 변환할 DB value
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "NATIVE" => Ok(Self::Native),
            "ERC20" => Ok(Self::Erc20),
            "ERC721" => Ok(Self::Erc721),
            "ERC1155" => Ok(Self::Erc1155),
            "UNKNOWN" => Ok(Self::Unknown),
            _ => Err(ParseEnumError::new("AssetType", value)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncStatus {
    Idle,
    Syncing,
    Error,
}

impl SyncStatus {
    /// Purpose: SyncStatus를 DB 저장 문자열로 변환
    /// Param:
    /// - `self`: 변환할 SyncStatus
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "IDLE",
            Self::Syncing => "SYNCING",
            Self::Error => "ERROR",
        }
    }
}

impl fmt::Display for SyncStatus {
    /// Purpose: SyncStatus 출력 문자열 작성
    /// Param:
    /// - `self`: 출력할 SyncStatus
    /// - `f`: fmt formatter
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for SyncStatus {
    type Err = ParseEnumError;

    /// Purpose: DB 문자열을 SyncStatus로 변환
    /// Param:
    /// - `value`: 변환할 DB value
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "IDLE" => Ok(Self::Idle),
            "SYNCING" => Ok(Self::Syncing),
            "ERROR" => Ok(Self::Error),
            _ => Err(ParseEnumError::new("SyncStatus", value)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseEnumError {
    enum_name: &'static str,
    value: String,
}

impl ParseEnumError {
    /// Purpose: enum 파싱 에러 생성
    /// Param:
    /// - `enum_name`: parsing 대상 enum_name
    /// - `value`: parsing 실패 value
    fn new(enum_name: &'static str, value: &str) -> Self {
        Self {
            enum_name,
            value: value.to_owned(),
        }
    }
}

impl fmt::Display for ParseEnumError {
    /// Purpose: enum 파싱 에러 출력 문자열 작성.
    /// Param:
    /// - `self`: 출력할 ParseEnumError.
    /// - `f`: fmt formatter.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown {} value {:?}", self.enum_name, self.value)
    }
}

impl std::error::Error for ParseEnumError {}
