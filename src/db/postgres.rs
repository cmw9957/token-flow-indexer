use sqlx::{PgPool, Row};

use crate::{
    db::{BlockRange, Store},
    error::{AppError, Result},
    models::{AssetMovement, BlockRecord, SyncCheckpoint, SyncStatus},
};

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
        let mut tx = self.pool.begin().await.map_err(|error| {
            AppError::with_source("failed to begin apply block transaction", error)
        })?;

        sqlx::query(
            r#"
            delete from blocks
            where chain_id = $1 and block_number = $2
            "#,
        )
        .bind(block.chain_id)
        .bind(block.block_number)
        .execute(&mut *tx)
        .await
        .map_err(|error| AppError::with_source("failed to delete existing block", error))?;

        sqlx::query(
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
            values ($1, $2, $3, $4, to_timestamp($5::double precision), $6, $7, now())
            "#,
        )
        .bind(block.chain_id)
        .bind(block.block_number)
        .bind(&block.block_hash)
        .bind(&block.parent_hash)
        .bind(&block.block_timestamp)
        .bind(block.tx_count)
        .bind(block.movement_count)
        .execute(&mut *tx)
        .await
        .map_err(|error| AppError::with_source("failed to insert block", error))?;

        for movement in movements {
            sqlx::query(
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
                values (
                    $1,
                    $2,
                    $3,
                    to_timestamp($4::double precision),
                    $5,
                    $6,
                    $7,
                    $8,
                    $9,
                    $10,
                    $11,
                    $12::numeric,
                    $13::numeric,
                    $14,
                    $15,
                    now()
                )
                on conflict do nothing
                "#,
            )
            .bind(movement.chain_id)
            .bind(movement.block_number)
            .bind(&movement.block_hash)
            .bind(&movement.block_timestamp)
            .bind(&movement.tx_hash)
            .bind(movement.tx_index)
            .bind(movement.source_type.as_str())
            .bind(movement.asset_type.as_str())
            .bind(&movement.token_address)
            .bind(&movement.from_address)
            .bind(&movement.to_address)
            .bind(&movement.token_id)
            .bind(&movement.amount_raw)
            .bind(movement.log_index)
            .bind(movement.log_sub_index)
            .execute(&mut *tx)
            .await
            .map_err(|error| AppError::with_source("failed to insert asset movement", error))?;
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
