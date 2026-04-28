mod backfill;
mod config;
mod db;
mod error;
mod extractor;
mod models;
mod processor;
mod proto;
mod remote;

use std::sync::Arc;

use backfill::RpcBackfillClient;
use config::Config;
use db::postgres::PostgresStore;
use error::{AppError, Result};
use processor::Processor;
use remote::RemoteSubscriber;

#[tokio::main]
/// Purpose: 설정 로드, DB 연결, 원격 ExEx 구독 시작
/// Param: None
async fn main() -> Result<()> {
    let config = Config::from_env()
        .map_err(|error| AppError::with_source("failed to load configuration", error))?;

    println!(
        "indexer starting: chain_id={} chain_name={} endpoint={}",
        config.chain_id, config.chain_name, config.exex_endpoint
    );

    let store = PostgresStore::connect(&config.database_url).await?;
    let backfill = RpcBackfillClient::new(config.backfill_rpc_url);
    let processor = Arc::new(Processor::new(
        store,
        backfill,
        config.chain_id,
        config.chain_name,
        config.backfill_chunk_size,
    ));
    processor.initialize().await?;

    let subscriber = RemoteSubscriber::new(config.exex_endpoint, config.reconnect_delay);
    subscriber
        .run(|payload| {
            let processor = Arc::clone(&processor);
            async move { processor.process_remote_notification(payload).await }
        })
        .await
}
