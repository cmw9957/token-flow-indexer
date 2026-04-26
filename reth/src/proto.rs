//! gRPC types for the remote ExEx indexer.

#[derive(Clone, PartialEq, ::prost::Message)]
/// Empty request used to subscribe to remote ExEx notifications.
pub struct SubscribeRequest {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
/// ExEx notification variant.
pub enum ExExNotificationKind {
    /// Unknown notification kind.
    Unknown = 0,
    /// Canonical chain commit.
    ChainCommitted = 1,
    /// Canonical chain reorg.
    ChainReorged = 2,
    /// Canonical chain revert.
    ChainReverted = 3,
}

#[derive(Clone, PartialEq, ::prost::Message)]
/// Block number and hash pair.
pub struct BlockRef {
    /// Block number.
    #[prost(uint64, tag = "1")]
    pub number: u64,
    /// Block hash.
    #[prost(bytes = "vec", tag = "2")]
    pub hash: Vec<u8>,
}

#[derive(Clone, PartialEq, ::prost::Message)]
/// Inclusive block number range.
pub struct BlockRange {
    /// First block number in the range.
    #[prost(uint64, tag = "1")]
    pub first: u64,
    /// Last block number in the range.
    #[prost(uint64, tag = "2")]
    pub last: u64,
}

#[derive(Clone, PartialEq, ::prost::Message)]
/// ExEx notification metadata for external indexers.
pub struct ExExNotification {
    /// Notification kind.
    #[prost(enumeration = "ExExNotificationKind", tag = "1")]
    pub kind: i32,
    /// Reverted or replaced range, if any.
    #[prost(message, optional, tag = "2")]
    pub old_range: Option<BlockRange>,
    /// Committed replacement range, if any.
    #[prost(message, optional, tag = "3")]
    pub new_range: Option<BlockRange>,
    /// Fork block for reverted chains.
    #[prost(message, optional, tag = "4")]
    pub fork_block: Option<BlockRef>,
    /// New canonical tip for committed chains.
    #[prost(message, optional, tag = "5")]
    pub tip_block: Option<BlockRef>,
    /// Blocks committed as part of this notification.
    #[prost(message, repeated, tag = "6")]
    pub new_blocks: Vec<Block>,
    /// Chain id for this notification.
    #[prost(uint64, tag = "7")]
    pub chain_id: u64,
}

#[derive(Clone, PartialEq, ::prost::Message)]
/// Block data required by external token-flow indexers.
pub struct Block {
    /// Block number.
    #[prost(uint64, tag = "1")]
    pub number: u64,
    /// Block hash.
    #[prost(bytes = "vec", tag = "2")]
    pub hash: Vec<u8>,
    /// Parent block hash.
    #[prost(bytes = "vec", tag = "3")]
    pub parent_hash: Vec<u8>,
    /// Block timestamp.
    #[prost(uint64, tag = "4")]
    pub timestamp: u64,
    /// Transactions included in the block.
    #[prost(message, repeated, tag = "5")]
    pub transactions: Vec<Transaction>,
    /// Chain id copied onto the block context.
    #[prost(uint64, tag = "6")]
    pub chain_id: u64,
}

#[derive(Clone, PartialEq, ::prost::Message)]
/// Transaction data required by external token-flow indexers.
pub struct Transaction {
    /// Transaction hash.
    #[prost(bytes = "vec", tag = "1")]
    pub hash: Vec<u8>,
    /// Transaction index within the block.
    #[prost(uint32, tag = "2")]
    pub index: u32,
    /// Recovered sender address.
    #[prost(bytes = "vec", tag = "3")]
    pub from: Vec<u8>,
    /// Recipient address. Empty for contract creation.
    #[prost(bytes = "vec", optional, tag = "4")]
    pub to: Option<Vec<u8>>,
    /// Native value as a base-10 integer string.
    #[prost(string, tag = "5")]
    pub value_raw: String,
    /// Receipt logs emitted by this transaction.
    #[prost(message, repeated, tag = "6")]
    pub logs: Vec<Log>,
}

#[derive(Clone, PartialEq, ::prost::Message)]
/// Receipt log data required by external token-flow indexers.
pub struct Log {
    /// Log index within the block.
    #[prost(uint32, tag = "1")]
    pub index: u32,
    /// Contract address that emitted the log.
    #[prost(bytes = "vec", tag = "2")]
    pub contract_address: Vec<u8>,
    /// Indexed log topics.
    #[prost(bytes = "vec", repeated, tag = "3")]
    pub topics: Vec<Vec<u8>>,
    /// Unindexed log data.
    #[prost(bytes = "vec", tag = "4")]
    pub data: Vec<u8>,
}

/// Remote indexer gRPC client.
pub mod remote_indexer_client {
    use super::{ExExNotification, SubscribeRequest};
    use tonic::codegen::*;

    #[derive(Debug, Clone)]
    /// Client for the remote ExEx indexer service.
    pub struct RemoteIndexerClient<T> {
        inner: tonic::client::Grpc<T>,
    }

    impl RemoteIndexerClient<tonic::transport::Channel> {
        /// Connects to a remote ExEx indexer endpoint.
        pub async fn connect<D>(dst: D) -> Result<Self, tonic::transport::Error>
        where
            D: TryInto<tonic::transport::Endpoint>,
            D::Error: Into<StdError>,
        {
            let conn = tonic::transport::Endpoint::new(dst)?.connect().await?;
            Ok(Self::new(conn))
        }
    }

    impl<T> RemoteIndexerClient<T>
    where
        T: tonic::client::GrpcService<tonic::body::Body>,
        T::Error: Into<StdError>,
        T::ResponseBody: Body<Data = Bytes> + Send + 'static,
        <T::ResponseBody as Body>::Error: Into<StdError> + Send,
    {
        /// Creates a client from an inner gRPC service.
        pub fn new(inner: T) -> Self {
            Self { inner: tonic::client::Grpc::new(inner) }
        }

        /// Sets the maximum response decoding message size.
        pub fn max_decoding_message_size(mut self, limit: usize) -> Self {
            self.inner = self.inner.max_decoding_message_size(limit);
            self
        }

        /// Sets the maximum request encoding message size.
        pub fn max_encoding_message_size(mut self, limit: usize) -> Self {
            self.inner = self.inner.max_encoding_message_size(limit);
            self
        }

        /// Subscribes to remote ExEx notifications.
        pub async fn subscribe(
            &mut self,
            request: impl tonic::IntoRequest<SubscribeRequest>,
        ) -> Result<tonic::Response<tonic::codec::Streaming<ExExNotification>>, tonic::Status>
        {
            self.inner.ready().await.map_err(|error| {
                tonic::Status::unknown(format!("service was not ready: {}", error.into()))
            })?;

            let path =
                http::uri::PathAndQuery::from_static("/exex.indexer.RemoteIndexer/Subscribe");
            let codec = tonic_prost::ProstCodec::default();
            self.inner.server_streaming(request.into_request(), path, codec).await
        }
    }
}

/// Remote indexer gRPC server.
pub mod remote_indexer_server {
    use super::{ExExNotification, SubscribeRequest};
    use futures::Stream;
    use std::sync::Arc;
    use tonic::codegen::*;

    #[tonic::async_trait]
    /// Service implemented by remote ExEx indexer servers.
    pub trait RemoteIndexer: Send + Sync + 'static {
        /// Stream returned by `subscribe`.
        type SubscribeStream: Stream<Item = Result<ExExNotification, tonic::Status>>
            + Send
            + 'static;

        /// Subscribes a client to ExEx notifications.
        async fn subscribe(
            &self,
            request: tonic::Request<SubscribeRequest>,
        ) -> Result<tonic::Response<Self::SubscribeStream>, tonic::Status>;
    }

    #[derive(Debug)]
    /// Server for the remote ExEx indexer service.
    pub struct RemoteIndexerServer<T> {
        inner: Arc<T>,
    }

    impl<T> RemoteIndexerServer<T> {
        /// Creates a server around a `RemoteIndexer` implementation.
        pub fn new(inner: T) -> Self {
            Self { inner: Arc::new(inner) }
        }
    }

    impl<T> Clone for RemoteIndexerServer<T> {
        fn clone(&self) -> Self {
            Self { inner: self.inner.clone() }
        }
    }

    impl<T, B> Service<http::Request<B>> for RemoteIndexerServer<T>
    where
        T: RemoteIndexer,
        B: Body + Send + 'static,
        B::Error: Into<StdError> + Send + 'static,
    {
        type Response = http::Response<tonic::body::Body>;
        type Error = std::convert::Infallible;
        type Future = BoxFuture<Self::Response, Self::Error>;

        fn poll_ready(
            &mut self,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Ready(Ok(()))
        }

        fn call(&mut self, req: http::Request<B>) -> Self::Future {
            let inner = self.inner.clone();

            match req.uri().path() {
                "/exex.indexer.RemoteIndexer/Subscribe" => {
                    struct SubscribeSvc<T: RemoteIndexer>(pub Arc<T>);

                    impl<T: RemoteIndexer> tonic::server::ServerStreamingService<SubscribeRequest> for SubscribeSvc<T> {
                        type Response = ExExNotification;
                        type ResponseStream = T::SubscribeStream;
                        type Future =
                            BoxFuture<tonic::Response<Self::ResponseStream>, tonic::Status>;

                        fn call(
                            &mut self,
                            request: tonic::Request<SubscribeRequest>,
                        ) -> Self::Future {
                            let inner = self.0.clone();
                            Box::pin(async move { inner.subscribe(request).await })
                        }
                    }

                    let fut = async move {
                        let method = SubscribeSvc(inner);
                        let codec = tonic_prost::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec);
                        let res = grpc.server_streaming(method, req).await;
                        Ok(res)
                    };

                    Box::pin(fut)
                }
                _ => {
                    let fut = async move {
                        let mut response = http::Response::new(tonic::body::Body::empty());
                        *response.status_mut() = http::StatusCode::OK;
                        response.headers_mut().insert(
                            tonic::Status::GRPC_STATUS,
                            http::HeaderValue::from_static("12"),
                        );
                        response.headers_mut().insert(
                            http::header::CONTENT_TYPE,
                            http::HeaderValue::from_static("application/grpc"),
                        );
                        Ok(response)
                    };

                    Box::pin(fut)
                }
            }
        }
    }

    impl<T> tonic::server::NamedService for RemoteIndexerServer<T> {
        const NAME: &'static str = "exex.indexer.RemoteIndexer";
    }
}
