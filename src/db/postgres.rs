use sqlx::{PgPool, Postgres, QueryBuilder, Row};

use crate::{
    db::{BlockRange, Store, StoredBlock},
    error::{AppError, Result},
    models::{AssetMovement, BlockRecord, SyncCheckpoint, SyncStatus},
};

const MAX_MOVEMENT_ROWS_PER_INSERT: usize = 4_000;

#[derive(Debug, Clone)]
pub struct PostgresStore {
    pool: PgPool,
}

impl PostgresStore {
    /// 기능: PostgreSQL 연결 풀 생성.
    /// 파라미터:
    /// - `database_url`: PostgreSQL database_url.
    pub async fn connect(database_url: &str) -> Result<Self> {
        let pool = PgPool::connect(database_url)
            .await
            .map_err(|error| AppError::with_source("failed to connect to postgres", error))?;

        Ok(Self { pool })
    }
}

impl Store for PostgresStore {
    /// 기능: 체인 메타데이터 upsert.
    /// 파라미터:
    /// - `self`: 현재 PostgresStore.
    /// - `chain_id`: chain_id 값.
    /// - `name`: chain name 값.
    async fn ensure_chain(&self, chain_id: i32, name: &str) -> Result<()> {
        sqlx::query(
            r#"
            insert into chains (chain_id, name)
            values ($1, $2)
            on conflict (chain_id) do update
                set name = excluded.name
            "#,
        )
        .bind(chain_id)
        .bind(name)
        .execute(&self.pool)
        .await
        .map_err(|error| AppError::with_source("failed to ensure chain row", error))?;

        Ok(())
    }

    /// 기능: 현재 인덱서 체크포인트 조회.
    /// 파라미터:
    /// - `self`: 현재 PostgresStore.
    /// - `chain_id`: chain_id 값.
    async fn load_checkpoint(&self, chain_id: i32) -> Result<Option<SyncCheckpoint>> {
        let row = sqlx::query(
            r#"
            select chain_id, last_indexed_block, last_indexed_hash, status
            from sync_checkpoints
            where chain_id = $1
            "#,
        )
        .bind(chain_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| AppError::with_source("failed to load sync checkpoint", error))?;

        row.map(|row| {
            let status: String = row.get("status");

            Ok(SyncCheckpoint {
                chain_id: row.get("chain_id"),
                last_indexed_block: row.get("last_indexed_block"),
                last_indexed_hash: row.get("last_indexed_hash"),
                status: status.parse().map_err(|error| {
                    AppError::with_source("failed to parse sync checkpoint status", error)
                })?,
            })
        })
        .transpose()
    }

    /// 기능: 체크포인트 상태만 upsert.
    /// 파라미터:
    /// - `self`: 현재 PostgresStore.
    /// - `chain_id`: chain_id 값.
    /// - `status`: 저장할 status.
    async fn set_checkpoint_status(&self, chain_id: i32, status: SyncStatus) -> Result<()> {
        sqlx::query(
            r#"
            insert into sync_checkpoints (
                chain_id,
                status,
                updated_at
            )
            values ($1, $2, now())
            on conflict (chain_id) do update
                set status = excluded.status,
                    updated_at = now()
            "#,
        )
        .bind(chain_id)
        .bind(status.as_str())
        .execute(&self.pool)
        .await
        .map_err(|error| AppError::with_source("failed to set checkpoint status", error))?;

        Ok(())
    }

    /// 기능: 체크포인트를 단독 트랜잭션으로 저장.
    /// 파라미터:
    /// - `self`: 현재 PostgresStore.
    /// - `checkpoint`: 저장할 checkpoint.
    async fn save_checkpoint(&self, checkpoint: SyncCheckpoint) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(|error| {
            AppError::with_source("failed to begin checkpoint transaction", error)
        })?;

        upsert_checkpoint(&mut tx, &checkpoint).await?;

        tx.commit().await.map_err(|error| {
            AppError::with_source("failed to commit checkpoint transaction", error)
        })?;

        Ok(())
    }

    /// 기능: 기존 동일 블록 삭제 후 블록, 자산 이동, 체크포인트 저장.
    /// 파라미터:
    /// - `self`: 현재 PostgresStore.
    /// - `block`: 저장할 block.
    /// - `movements`: 저장할 movements.
    /// - `checkpoint`: 저장 후 갱신할 checkpoint.
    async fn apply_block(
        &self,
        block: BlockRecord,
        movements: Vec<AssetMovement>,
        checkpoint: SyncCheckpoint,
    ) -> Result<()> {
        self.apply_blocks(
            vec![StoredBlock {
                record: block,
                movements,
            }],
            checkpoint,
        )
        .await
    }

    /// 기능: 기존 동일 블록 삭제 후 여러 블록, 자산 이동, 체크포인트 저장.
    /// 파라미터:
    /// - `self`: 현재 PostgresStore.
    /// - `blocks`: 저장할 block과 movement 묶음.
    /// - `checkpoint`: 저장 후 갱신할 checkpoint.
    async fn apply_blocks(
        &self,
        blocks: Vec<StoredBlock>,
        checkpoint: SyncCheckpoint,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(|error| {
            AppError::with_source("failed to begin apply block transaction", error)
        })?;

        if !blocks.is_empty() {
            validate_apply_blocks(&blocks, checkpoint.chain_id)?;
            delete_existing_blocks(&mut tx, checkpoint.chain_id, &blocks).await?;
            insert_blocks(&mut tx, &blocks).await?;
            insert_asset_movements(&mut tx, &blocks).await?;
        }

        upsert_checkpoint(&mut tx, &checkpoint).await?;

        tx.commit().await.map_err(|error| {
            AppError::with_source("failed to commit apply block transaction", error)
        })?;

        Ok(())
    }

    /// 기능: 지정 블록 범위 삭제 후 체크포인트 갱신.
    /// 파라미터:
    /// - `self`: 현재 PostgresStore.
    /// - `chain_id`: chain_id 값.
    /// - `range`: 삭제할 range.
    /// - `checkpoint`: 삭제 후 저장할 checkpoint.
    async fn revert_blocks(
        &self,
        chain_id: i32,
        range: BlockRange,
        checkpoint: SyncCheckpoint,
    ) -> Result<()> {
        let mut tx =
            self.pool.begin().await.map_err(|error| {
                AppError::with_source("failed to begin revert transaction", error)
            })?;

        sqlx::query(
            r#"
            delete from blocks
            where chain_id = $1
              and block_number >= $2
              and block_number <= $3
            "#,
        )
        .bind(chain_id)
        .bind(range.from_block)
        .bind(range.to_block)
        .execute(&mut *tx)
        .await
        .map_err(|error| AppError::with_source("failed to delete reverted blocks", error))?;

        upsert_checkpoint(&mut tx, &checkpoint).await?;

        tx.commit()
            .await
            .map_err(|error| AppError::with_source("failed to commit revert transaction", error))?;

        Ok(())
    }
}

/// 기능: apply_blocks 입력의 chain_id 일관성 검증.
/// 파라미터:
/// - `blocks`: 저장 대상 blocks.
/// - `chain_id`: checkpoint chain_id.
fn validate_apply_blocks(blocks: &[StoredBlock], chain_id: i32) -> Result<()> {
    for block in blocks {
        if block.record.chain_id != chain_id {
            return Err(AppError::msg(format!(
                "block chain_id {} does not match checkpoint chain_id {}",
                block.record.chain_id, chain_id
            )));
        }

        for movement in &block.movements {
            if movement.chain_id != chain_id {
                return Err(AppError::msg(format!(
                    "movement chain_id {} does not match checkpoint chain_id {}",
                    movement.chain_id, chain_id
                )));
            }
        }
    }

    Ok(())
}

/// 기능: 기존 동일 블록 삭제.
/// 파라미터:
/// - `tx`: 실행 중인 transaction.
/// - `chain_id`: chain_id 값.
/// - `blocks`: 삭제 대상 block 목록.
async fn delete_existing_blocks(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    chain_id: i32,
    blocks: &[StoredBlock],
) -> Result<()> {
    let block_numbers = blocks
        .iter()
        .map(|block| block.record.block_number)
        .collect::<Vec<_>>();

    sqlx::query(
        r#"
        delete from blocks
        where chain_id = $1 and block_number = any($2)
        "#,
    )
    .bind(chain_id)
    .bind(&block_numbers)
    .execute(&mut **tx)
    .await
    .map_err(|error| AppError::with_source("failed to delete existing blocks", error))?;

    Ok(())
}

/// 기능: 여러 블록을 bulk insert.
/// 파라미터:
/// - `tx`: 실행 중인 transaction.
/// - `blocks`: 저장 대상 block 목록.
async fn insert_blocks(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    blocks: &[StoredBlock],
) -> Result<()> {
    let mut builder = QueryBuilder::<Postgres>::new(
        r#"
        insert into blocks (
            chain_id,
            block_number,
            block_hash,
            parent_hash,
            block_timestamp,
            tx_count,
            movement_count,
            indexed_at
        )
        "#,
    );

    builder.push_values(blocks, |mut row, block| {
        row.push_bind(block.record.chain_id)
            .push_bind(block.record.block_number)
            .push_bind(&block.record.block_hash)
            .push_bind(&block.record.parent_hash)
            .push("to_timestamp(")
            .push_bind(&block.record.block_timestamp)
            .push("::double precision)")
            .push_bind(block.record.tx_count)
            .push_bind(block.record.movement_count)
            .push("now()");
    });

    builder
        .build()
        .execute(&mut **tx)
        .await
        .map_err(|error| AppError::with_source("failed to insert blocks", error))?;

    Ok(())
}

/// 기능: 여러 블록의 asset_movements를 bulk insert.
/// 파라미터:
/// - `tx`: 실행 중인 transaction.
/// - `blocks`: 저장 대상 block 목록.
async fn insert_asset_movements(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    blocks: &[StoredBlock],
) -> Result<()> {
    let movements = blocks
        .iter()
        .flat_map(|block| block.movements.iter())
        .collect::<Vec<_>>();

    for chunk in movements.chunks(MAX_MOVEMENT_ROWS_PER_INSERT) {
        let mut builder = QueryBuilder::<Postgres>::new(
            r#"
            insert into asset_movements (
                chain_id,
                block_number,
                block_hash,
                block_timestamp,
                tx_hash,
                tx_index,
                source_type,
                asset_type,
                token_address,
                from_address,
                to_address,
                token_id,
                amount_raw,
                log_index,
                log_sub_index,
                created_at
            )
            "#,
        );

        builder.push_values(chunk, |mut row, movement| {
            row.push_bind(movement.chain_id)
                .push_bind(movement.block_number)
                .push_bind(&movement.block_hash)
                .push("to_timestamp(")
                .push_bind(&movement.block_timestamp)
                .push("::double precision)")
                .push_bind(&movement.tx_hash)
                .push_bind(movement.tx_index)
                .push_bind(movement.source_type.as_str())
                .push_bind(movement.asset_type.as_str())
                .push_bind(&movement.token_address)
                .push_bind(&movement.from_address)
                .push_bind(&movement.to_address)
                .push_bind(&movement.token_id)
                .push_bind(&movement.amount_raw)
                .push_bind(movement.log_index)
                .push_bind(movement.log_sub_index)
                .push("now()");
        });

        builder.push(" on conflict do nothing");
        builder
            .build()
            .execute(&mut **tx)
            .await
            .map_err(|error| AppError::with_source("failed to insert asset movements", error))?;
    }

    Ok(())
}

/// 기능: 체크포인트 insert 또는 update.
/// 파라미터:
/// - `tx`: checkpoint 저장용 tx.
/// - `checkpoint`: 저장할 checkpoint.
async fn upsert_checkpoint(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    checkpoint: &SyncCheckpoint,
) -> Result<()> {
    sqlx::query(
        r#"
        insert into sync_checkpoints (
            chain_id,
            last_indexed_block,
            last_indexed_hash,
            status,
            updated_at
        )
        values ($1, $2, $3, $4, now())
        on conflict (chain_id) do update
            set last_indexed_block = excluded.last_indexed_block,
                last_indexed_hash = excluded.last_indexed_hash,
                status = excluded.status,
                updated_at = now()
        "#,
    )
    .bind(checkpoint.chain_id)
    .bind(checkpoint.last_indexed_block)
    .bind(&checkpoint.last_indexed_hash)
    .bind(checkpoint.status.as_str())
    .execute(&mut **tx)
    .await
    .map_err(|error| AppError::with_source("failed to upsert sync checkpoint", error))?;

    Ok(())
}
