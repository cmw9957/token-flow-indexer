# token-flow-indexer

`token-flow-indexer`는 reth ExEx에서 전달되는 블록 알림을 gRPC로 구독하고, 블록/트랜잭션/토큰 이동 정보를 PostgreSQL에 저장하는 Rust 인덱서입니다.

## 구성

- 루트 바이너리: `cargo run`으로 실행하는 인덱서입니다.
- `reth/`: reth 노드에 붙여 실행하는 ExEx gRPC 서버 코드입니다. 기본 gRPC 주소는 `[::1]:10000`입니다.
- `migrations/0001_init.sql`: PostgreSQL 테이블과 인덱스 생성 SQL입니다.

## 요구 사항

- Rust toolchain
- PostgreSQL
- reth ExEx 서버 또는 호환되는 `RemoteIndexer` gRPC 서버

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
export EXEX_RECONNECT_DELAY_SECS=3
```

필수 값:

- `DATABASE_URL`: PostgreSQL 연결 문자열
- `CHAIN_ID`: 인덱싱할 체인 ID. Ethereum mainnet은 `1`입니다.

선택 값:

- `CHAIN_NAME`: 체인 이름입니다. 기본값은 `ethereum`입니다.
- `EXEX_INDEXER_GRPC_ENDPOINT`: reth ExEx gRPC endpoint입니다. 외부 nginx 443 gRPC 프록시를 사용할 때는 `https://mev-dashboard.ddns.net:443`처럼 endpoint만 지정합니다. gRPC path인 `/exex.indexer.RemoteIndexer/Subscribe`는 코드에 고정되어 있어 URL에 붙이지 않습니다. 설정하지 않으면 기본값은 `http://[::1]:10000`입니다.
- `EXEX_RECONNECT_DELAY_SECS`: gRPC 연결 실패 후 재시도 대기 시간입니다. 기본값은 `3`초입니다.

## 인덱서 실행

먼저 reth ExEx gRPC 서버가 떠 있어야 합니다. 서버가 준비된 뒤 루트에서 인덱서를 실행합니다.

```bash
cargo run
```

인덱서는 `EXEX_INDEXER_GRPC_ENDPOINT`에 연결해 ExEx 알림 스트림을 구독합니다. 연결이 끊기면 `EXEX_RECONNECT_DELAY_SECS` 간격으로 재연결을 시도합니다.

로컬 DB를 쓰는 외부 실행 예시는 다음과 같습니다.

```bash
createdb token_flow_indexer
psql postgres://localhost/token_flow_indexer -f migrations/0001_init.sql

export DATABASE_URL='postgres://localhost/token_flow_indexer'
export CHAIN_ID=1
export CHAIN_NAME=ethereum
export EXEX_INDEXER_GRPC_ENDPOINT='https://mev-dashboard.ddns.net:443'

cargo run
```

## reth ExEx 서버 실행

`reth/` 디렉터리의 `exex-indexer` 바이너리는 reth 노드에 설치되는 ExEx 서버입니다.

```bash
export EXEX_INDEXER_GRPC_ADDR='[::1]:10000'
```

이 크레이트는 reth 워크스페이스 안에서 실행되는 것을 전제로 합니다. reth 쪽 실행 옵션은 사용하는 reth 노드 설정과 데이터 디렉터리에 맞춰 지정해야 합니다.

기본 흐름은 다음과 같습니다.

1. reth ExEx 서버를 실행해 gRPC endpoint를 엽니다.
2. PostgreSQL DB와 스키마를 준비합니다.
3. 루트 `token-flow-indexer`를 `cargo run`으로 실행합니다.

## 동작 확인

빌드 확인:

```bash
cargo check
```

DB 적재 확인 예시:

```bash
psql "$DATABASE_URL" -c "select * from sync_checkpoints;"
psql "$DATABASE_URL" -c "select chain_id, block_number, tx_count, movement_count from blocks order by block_number desc limit 10;"
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
