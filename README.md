# token-flow-indexer

`token-flow-indexer`는 reth ExEx에서 전달되는 블록 알림을 gRPC로 구독하고, 블록/트랜잭션/토큰 이동 정보를 PostgreSQL에 저장하는 Rust 인덱서입니다.

## 구성

- 루트 바이너리: `cargo run`으로 실행하는 인덱서입니다.
- `migrations/0001_init.sql`: PostgreSQL 테이블과 인덱스 생성 SQL입니다.

## 요구 사항

- Rust 1.85 이상
- PostgreSQL 14 이상
- `psql`, `createdb` CLI
- 접근 가능한 `RemoteIndexer` gRPC endpoint

Rust edition 2024를 사용하므로 Rust 1.85 이상이 필요합니다.

```bash
rustc --version
cargo --version
```

Rust가 없다면 `rustup`으로 설치합니다.

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup default stable
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
psql postgres://localhost/token_flow_indexer -f migrations/0001_init.sql
```

이미 사용할 데이터베이스가 있다면 `DATABASE_URL`에 맞는 DB에 `migrations/0001_init.sql`만 적용하면 됩니다.

## 환경변수

루트 인덱서를 실행하기 전에 다음 환경변수를 설정합니다.

```bash
export DATABASE_URL=postgres://localhost/token_flow_indexer
export CHAIN_ID=1
export CHAIN_NAME=ethereum
export EXEX_INDEXER_GRPC_ENDPOINT='https://mev-dashboard.ddns.net:443'
export BACKFILL_RPC_URL='https://mev-dashboard.ddns.net/rpc'
export EXEX_RECONNECT_DELAY_SECS=3
```

필수 값:

- `DATABASE_URL`: PostgreSQL 연결 문자열
- `CHAIN_ID`: 인덱싱할 체인 ID. Ethereum mainnet은 `1`입니다.

선택 값:

- `CHAIN_NAME`: 체인 이름입니다. 기본값은 `ethereum`입니다.
- `EXEX_INDEXER_GRPC_ENDPOINT`: reth ExEx gRPC endpoint입니다. 외부 nginx 443 gRPC 프록시를 사용할 때는 `https://mev-dashboard.ddns.net:443`처럼 endpoint만 지정합니다. gRPC path인 `/exex.indexer.RemoteIndexer/Subscribe`는 코드에 고정되어 있어 URL에 붙이지 않습니다. 설정하지 않으면 기본값은 `http://[::1]:10000`입니다.
- `BACKFILL_RPC_URL`: gap 발생 시 누락 블록을 조회할 JSON-RPC endpoint입니다. 설정하지 않으면 기본값은 `https://mev-dashboard.ddns.net/rpc`입니다.
- `EXEX_RECONNECT_DELAY_SECS`: gRPC 연결 실패 후 재시도 대기 시간입니다. 기본값은 `3`초입니다.

## 인덱서 실행

```bash
cargo run
```

인덱서는 `EXEX_INDEXER_GRPC_ENDPOINT`에 연결해 ExEx 알림 스트림을 구독합니다. 연결이 끊기면 `EXEX_RECONNECT_DELAY_SECS` 간격으로 재연결을 시도합니다.

checkpoint와 새 notification 사이에 gap이 있으면 `BACKFILL_RPC_URL`로 누락 블록을 조회해 먼저 저장한 뒤 stream block 처리를 이어갑니다.

로컬 DB를 쓰는 외부 실행 예시는 다음과 같습니다.

```bash
createdb token_flow_indexer
psql postgres://localhost/token_flow_indexer -f migrations/0001_init.sql

export DATABASE_URL='postgres://localhost/token_flow_indexer'
export CHAIN_ID=1
export CHAIN_NAME=ethereum
export EXEX_INDEXER_GRPC_ENDPOINT='https://mev-dashboard.ddns.net:443'
export BACKFILL_RPC_URL='https://mev-dashboard.ddns.net/rpc'

cargo run
```

## 문제 해결

`missing required environment variable DATABASE_URL`:

`DATABASE_URL` 환경변수가 설정되지 않았습니다.

`missing required environment variable CHAIN_ID`:

`CHAIN_ID` 환경변수가 설정되지 않았습니다.

`failed to connect to postgres`:

PostgreSQL이 실행 중인지, `DATABASE_URL`이 올바른지, 마이그레이션 대상 DB가 존재하는지 확인합니다.

`failed to connect to remote ExEx`:

reth ExEx gRPC 서버가 실행 중인지, `EXEX_INDEXER_GRPC_ENDPOINT` 값이 서버 주소와 일치하는지 확인합니다.
