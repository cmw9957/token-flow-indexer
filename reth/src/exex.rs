//! Remote ExEx indexer server.

use alloy_consensus::{Transaction as _, TxReceipt};
use futures::TryStreamExt;
use reth_chainspec::EthChainSpec;
use reth_ethereum::{
    cli::interface::Cli,
    exex::{ExExContext, ExExEvent, ExExNotification as RethExExNotification},
    node::{
        api::{FullNodeComponents, NodeTypes},
        EthereumNode,
    },
    EthPrimitives,
};
use reth_execution_types::Chain;
use reth_exex_indexer::proto::{
    self,
    remote_indexer_server::{RemoteIndexer, RemoteIndexerServer},
};
use reth_primitives_traits::RecoveredBlock;
use std::{net::SocketAddr, sync::Arc};
use tokio::sync::{broadcast, mpsc};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{transport::Server, Request, Response, Status};
use tracing::info;

const DEFAULT_GRPC_ADDR: &str = "[::1]:10000";

#[derive(Debug)]
struct IndexerService {
    notifications: Arc<broadcast::Sender<proto::ExExNotification>>,
}

#[tonic::async_trait]
impl RemoteIndexer for IndexerService {
    type SubscribeStream = ReceiverStream<Result<proto::ExExNotification, Status>>;

    async fn subscribe(
        &self,
        _request: Request<proto::SubscribeRequest>,
    ) -> Result<Response<Self::SubscribeStream>, Status> {
        let (tx, rx) = mpsc::channel(1);
        let mut notifications = self.notifications.subscribe();

        tokio::spawn(async move {
            while let Ok(notification) = notifications.recv().await {
                if tx.send(Ok(notification)).await.is_err() {
                    break;
                }
            }
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }
}

async fn remote_indexer_exex<Node>(
    mut ctx: ExExContext<Node>,
    notifications: Arc<broadcast::Sender<proto::ExExNotification>>,
) -> eyre::Result<()>
where
    Node: FullNodeComponents<Types: NodeTypes<Primitives = EthPrimitives>>,
{
    let chain_id = ctx.config.chain.chain_id();

    while let Some(notification) = ctx.notifications.try_next().await? {
        match &notification {
            RethExExNotification::ChainCommitted { new } => {
                ctx.events.send(ExExEvent::FinishedHeight(new.tip().num_hash()))?;
                info!(range = ?new.range(), "forwarding committed chain");
            }
            RethExExNotification::ChainReorged { new, .. } => {
                ctx.events.send(ExExEvent::FinishedHeight(new.tip().num_hash()))?;
                info!(range = ?new.range(), "forwarding reorged chain");
            }
            RethExExNotification::ChainReverted { old } => {
                ctx.events.send(ExExEvent::FinishedHeight(old.fork_block()))?;
                info!(range = ?old.range(), "forwarding reverted chain");
            }
        }

        let _ = notifications.send(notification_to_proto(chain_id, &notification)?);
    }

    Ok(())
}

fn notification_to_proto(
    chain_id: u64,
    notification: &RethExExNotification,
) -> eyre::Result<proto::ExExNotification> {
    match notification {
        RethExExNotification::ChainCommitted { new } => Ok(proto::ExExNotification {
            kind: proto::ExExNotificationKind::ChainCommitted as i32,
            old_range: None,
            new_range: Some(block_range(new.range())),
            fork_block: None,
            tip_block: Some(block_ref(new.tip().num_hash())),
            new_blocks: chain_blocks(chain_id, new)?,
            chain_id,
        }),
        RethExExNotification::ChainReorged { old, new } => Ok(proto::ExExNotification {
            kind: proto::ExExNotificationKind::ChainReorged as i32,
            old_range: Some(block_range(old.range())),
            new_range: Some(block_range(new.range())),
            fork_block: None,
            tip_block: Some(block_ref(new.tip().num_hash())),
            new_blocks: chain_blocks(chain_id, new)?,
            chain_id,
        }),
        RethExExNotification::ChainReverted { old } => Ok(proto::ExExNotification {
            kind: proto::ExExNotificationKind::ChainReverted as i32,
            old_range: Some(block_range(old.range())),
            new_range: None,
            fork_block: Some({
                let fork_block = old.fork_block();
                proto::BlockRef {
                    number: fork_block.number,
                    hash: fork_block.hash.as_slice().to_vec(),
                }
            }),
            tip_block: None,
            new_blocks: Vec::new(),
            chain_id,
        }),
    }
}

fn chain_blocks(chain_id: u64, chain: &Chain<EthPrimitives>) -> eyre::Result<Vec<proto::Block>> {
    chain
        .blocks_and_receipts()
        .map(|(block, receipts)| block_to_proto(chain_id, block, receipts))
        .collect()
}

fn block_to_proto(
    chain_id: u64,
    block: &RecoveredBlock<<EthPrimitives as reth_primitives_traits::NodePrimitives>::Block>,
    receipts: &[<EthPrimitives as reth_primitives_traits::NodePrimitives>::Receipt],
) -> eyre::Result<proto::Block> {
    let header = block.header();
    let transactions_len = block.body().transactions().count();
    if transactions_len != receipts.len() {
        eyre::bail!(
            "block {} has {} transactions but {} receipts",
            header.number,
            transactions_len,
            receipts.len()
        );
    }

    let transactions = block
        .transactions_with_sender()
        .zip(receipts.iter())
        .enumerate()
        .scan(0usize, |next_log_index, (tx_index, ((sender, tx), receipt))| {
            let log_index_start = *next_log_index;
            *next_log_index += receipt.logs().len();
            Some(transaction_to_proto(tx_index, log_index_start, sender, tx, receipt))
        })
        .collect::<eyre::Result<Vec<_>>>()?;

    Ok(proto::Block {
        number: header.number,
        hash: block.hash().as_slice().to_vec(),
        parent_hash: header.parent_hash.as_slice().to_vec(),
        timestamp: header.timestamp,
        transactions,
        chain_id,
    })
}

fn transaction_to_proto(
    tx_index: usize,
    log_index_start: usize,
    sender: &alloy_primitives::Address,
    tx: &reth_ethereum::TransactionSigned,
    receipt: &reth_ethereum::Receipt,
) -> eyre::Result<proto::Transaction> {
    let logs = receipt
        .logs()
        .iter()
        .enumerate()
        .map(|(offset, log)| log_to_proto(log_index_start + offset, log))
        .collect::<eyre::Result<Vec<_>>>()?;

    Ok(proto::Transaction {
        hash: tx.tx_hash().as_slice().to_vec(),
        index: u32::try_from(tx_index)?,
        from: sender.as_slice().to_vec(),
        to: tx.to().map(|to| to.as_slice().to_vec()),
        value_raw: tx.value().to_string(),
        logs,
    })
}

fn log_to_proto(log_index: usize, log: &alloy_primitives::Log) -> eyre::Result<proto::Log> {
    Ok(proto::Log {
        index: u32::try_from(log_index)?,
        contract_address: log.address.as_slice().to_vec(),
        topics: log.data.topics().iter().map(|topic| topic.as_slice().to_vec()).collect(),
        data: log.data.data.to_vec(),
    })
}

fn block_range(range: std::ops::RangeInclusive<u64>) -> proto::BlockRange {
    proto::BlockRange { first: *range.start(), last: *range.end() }
}

fn block_ref(block: alloy_eips::BlockNumHash) -> proto::BlockRef {
    proto::BlockRef { number: block.number, hash: block.hash.as_slice().to_vec() }
}

fn main() -> eyre::Result<()> {
    Cli::parse_args().run(async move |builder, _| {
        let addr: SocketAddr = std::env::var("EXEX_INDEXER_GRPC_ADDR")
            .unwrap_or_else(|_| DEFAULT_GRPC_ADDR.to_string())
            .parse()?;
        let notifications = Arc::new(broadcast::channel(16).0);
        let service = IndexerService { notifications: notifications.clone() };

        let handle = builder
            .node(EthereumNode::default())
            .install_exex("remote-indexer", async move |ctx| {
                Ok(remote_indexer_exex(ctx, notifications))
            })
            .launch()
            .await?;

        handle.node.task_executor.spawn_critical_task("remote indexer gRPC server", async move {
            info!(%addr, "starting remote indexer gRPC server");
            Server::builder()
                .add_service(RemoteIndexerServer::new(service))
                .serve(addr)
                .await
                .expect("failed to start remote indexer gRPC server");
        });

        handle.wait_for_node_exit().await
    })?;

    Ok(())
}
