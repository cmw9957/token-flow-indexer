# token-flow-indexer

`token-flow-indexer`는 reth ExEx에서 전달되는 블록 알림을 gRPC로 구독하고, 블록/트랜잭션/토큰 이동 정보를 PostgreSQL에 저장하는 Rust 인덱서입니다.

## 구성

- 루트 바이너리: `cargo run`으로 실행하는 인덱서입니다.
- `migrations/0001_init.sql`: PostgreSQL 테이블과 인덱스 생성 SQL입니다.

## 파일 트리

```text
token-flow-indexer/
├── Cargo.toml
├── README.md
├── migrations/
│   └── 0001_init.sql        # PostgreSQL schema, indexes
└── src/
    ├── main.rs              # 설정 로드, DB/RPC/client 조립, 구독 시작
    ├── config.rs            # 환경변수 파싱
    ├── remote.rs            # 원격 ExEx gRPC subscription/reconnect
    ├── processor.rs         # notification 처리, gap/reorg/revert orchestration
    ├── backfill.rs          # JSON-RPC 기반 gap backfill
    ├── extractor.rs         # native/ERC20/ERC721/ERC1155 movement 추출
    ├── models.rs            # DB 저장 모델과 enum
    ├── proto.rs             # protobuf/gRPC 타입
    ├── error.rs             # 공통 AppError/Result
    └── db/
        ├── mod.rs           # Store trait
        └── postgres.rs      # PostgreSQL Store 구현
```

## 요구 사항

- Rust 1.85 이상
- PostgreSQL 14 이상
- `psql`, `createdb` CLI
- 접근 가능한 `RemoteIndexer` gRPC endpoint

Rust edition 2024를 사용하므로 Rust 1.85 이상이 필요합니다. 먼저 설치 여부를 확인합니다.

```bash
rustc --version
cargo --version
```

Rust가 없다면 `rustup`으로 설치합니다.

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

설치가 끝나면 현재 shell에 Cargo 환경을 반영합니다.

```bash
source "$HOME/.cargo/env"
```

stable toolchain을 사용하도록 설정합니다.

```bash
rustup default stable
rustup update stable
```

다시 버전을 확인합니다.

```bash
rustc --version
cargo --version
```

PostgreSQL은 로컬 DB를 사용할 경우 설치되어 있어야 합니다.

macOS Homebrew 예시:

```bash
brew install postgresql@18
brew services start postgresql@18
```

설치 확인:

```bash
psql --version
createdb --version
```

## 데이터베이스 준비

PostgreSQL 데이터베이스를 만들고 마이그레이션을 적용합니다.

```bash
createdb token_flow_indexer
export DATABASE_URL='postgres:///token_flow_indexer'
psql "$DATABASE_URL" -f migrations/0001_init.sql
```

이미 사용할 데이터베이스가 있다면 `DATABASE_URL`에 맞는 DB에 `migrations/0001_init.sql`만 적용하면 됩니다.

## 환경변수

루트 인덱서를 실행하기 전에 다음 환경변수를 설정합니다.

```bash
export DATABASE_URL='postgres:///token_flow_indexer'
export CHAIN_ID=1
export CHAIN_NAME=ethereum
export EXEX_INDEXER_GRPC_ENDPOINT='<REMOTE_INDEXER_ENDPOINT>'
export BACKFILL_RPC_URL='<RPC_ENDPOINT>'
export EXEX_RECONNECT_DELAY_SECS=3
export BACKFILL_CHUNK_SIZE=50
```

필수 값:

- `DATABASE_URL`: PostgreSQL 연결 문자열
- `CHAIN_ID`: 인덱싱할 체인 ID. Ethereum mainnet은 `1`입니다.
- `EXEX_INDEXER_GRPC_ENDPOINT`: reth ExEx gRPC endpoint
- `BACKFILL_RPC_URL`: gap 발생 시 누락 블록을 조회할 JSON-RPC endpoint

선택 값:

- `CHAIN_NAME`: 체인 이름입니다. 기본값은 `ethereum`입니다.
- `EXEX_RECONNECT_DELAY_SECS`: gRPC 연결 실패 후 재시도 대기 시간입니다. 기본값은 `3`초입니다.
- `BACKFILL_CHUNK_SIZE`: gap backfill 때 한 번에 조회/저장할 블록 수입니다. 기본값은 `50`입니다.

## 인덱서 실행

```bash
cargo run
```

인덱서는 `EXEX_INDEXER_GRPC_ENDPOINT`에 연결해 ExEx 알림 스트림을 구독합니다. 연결이 끊기면 `EXEX_RECONNECT_DELAY_SECS` 간격으로 재연결을 시도합니다.

checkpoint와 새 notification 사이에 gap이 있으면 `BACKFILL_RPC_URL`로 누락 블록을 조회해 먼저 저장한 뒤 stream block 처리를 이어갑니다.

로컬 DB를 쓰는 외부 실행 예시는 다음과 같습니다.

```bash
createdb token_flow_indexer
export DATABASE_URL='postgres:///token_flow_indexer'
psql "$DATABASE_URL" -f migrations/0001_init.sql

export CHAIN_ID=1
export CHAIN_NAME=ethereum
export EXEX_INDEXER_GRPC_ENDPOINT='<REMOTE_INDEXER_ENDPOINT>'
export BACKFILL_RPC_URL='<RPC_ENDPOINT>'

cargo run
```

## 문제 해결

`missing required environment variable DATABASE_URL`:

`DATABASE_URL` 환경변수가 설정되지 않았습니다.

`missing required environment variable CHAIN_ID`:

`CHAIN_ID` 환경변수가 설정되지 않았습니다.

`missing required environment variable EXEX_INDEXER_GRPC_ENDPOINT`:

`EXEX_INDEXER_GRPC_ENDPOINT` 환경변수가 설정되지 않았습니다.

`missing required environment variable BACKFILL_RPC_URL`:

`BACKFILL_RPC_URL` 환경변수가 설정되지 않았습니다.

`failed to connect to postgres`:

PostgreSQL이 실행 중인지, `DATABASE_URL`이 올바른지, 마이그레이션 대상 DB가 존재하는지 확인합니다.

`failed to connect to remote ExEx`:

reth ExEx gRPC 서버가 실행 중인지, `EXEX_INDEXER_GRPC_ENDPOINT` 값이 서버 주소와 일치하는지 확인합니다.
