#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SubscribeRequest {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum ExExNotificationKind {
    Unknown = 0,
    ChainCommitted = 1,
    ChainReorged = 2,
    ChainReverted = 3,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct BlockRef {
    #[prost(uint64, tag = "1")]
    pub number: u64,
    #[prost(bytes = "vec", tag = "2")]
    pub hash: Vec<u8>,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct BlockRange {
    #[prost(uint64, tag = "1")]
    pub first: u64,
    #[prost(uint64, tag = "2")]
    pub last: u64,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct ExExNotification {
    #[prost(enumeration = "ExExNotificationKind", tag = "1")]
    pub kind: i32,
    #[prost(message, optional, tag = "2")]
    pub old_range: Option<BlockRange>,
    #[prost(message, optional, tag = "3")]
    pub new_range: Option<BlockRange>,
    #[prost(message, optional, tag = "4")]
    pub fork_block: Option<BlockRef>,
    #[prost(message, optional, tag = "5")]
    pub tip_block: Option<BlockRef>,
    #[prost(message, repeated, tag = "6")]
    pub new_blocks: Vec<Block>,
    #[prost(uint64, tag = "7")]
    pub chain_id: u64,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Block {
    #[prost(uint64, tag = "1")]
    pub number: u64,
    #[prost(bytes = "vec", tag = "2")]
    pub hash: Vec<u8>,
    #[prost(bytes = "vec", tag = "3")]
    pub parent_hash: Vec<u8>,
    #[prost(uint64, tag = "4")]
    pub timestamp: u64,
    #[prost(message, repeated, tag = "5")]
    pub transactions: Vec<Transaction>,
    #[prost(uint64, tag = "6")]
    pub chain_id: u64,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Transaction {
    #[prost(bytes = "vec", tag = "1")]
    pub hash: Vec<u8>,
    #[prost(uint32, tag = "2")]
    pub index: u32,
    #[prost(bytes = "vec", tag = "3")]
    pub from: Vec<u8>,
    #[prost(bytes = "vec", optional, tag = "4")]
    pub to: Option<Vec<u8>>,
    #[prost(string, tag = "5")]
    pub value_raw: String,
    #[prost(message, repeated, tag = "6")]
    pub logs: Vec<Log>,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Log {
    #[prost(uint32, tag = "1")]
    pub index: u32,
    #[prost(bytes = "vec", tag = "2")]
    pub contract_address: Vec<u8>,
    #[prost(bytes = "vec", repeated, tag = "3")]
    pub topics: Vec<Vec<u8>>,
    #[prost(bytes = "vec", tag = "4")]
    pub data: Vec<u8>,
}

pub mod remote_indexer_client {
    use super::{ExExNotification, SubscribeRequest};
    use tonic::codegen::*;

    #[derive(Debug, Clone)]
    pub struct RemoteIndexerClient<T> {
        inner: tonic::client::Grpc<T>,
    }

    impl RemoteIndexerClient<tonic::transport::Channel> {
        /// Purpose: gRPC 채널 생성 후 RemoteIndexer 클라이언트 연결
        /// Param:
        /// - `dst`: gRPC endpoint dst
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
        /// Purpose: 기존 gRPC 서비스로 RemoteIndexer 클라이언트 생성
        /// Param:
        /// - `inner`: 내부 gRPC service
        pub fn new(inner: T) -> Self {
            Self {
                inner: tonic::client::Grpc::new(inner),
            }
        }

        /// Purpose: 최대 디코딩 메시지 크기 설정
        /// Param:
        /// - `self`: 현재 RemoteIndexerClient
        /// - `limit`: max decoding size
        pub fn max_decoding_message_size(mut self, limit: usize) -> Self {
            self.inner = self.inner.max_decoding_message_size(limit);
            self
        }

        /// Purpose: 최대 인코딩 메시지 크기 설정
        /// Param:
        /// - `self`: 현재 RemoteIndexerClient
        /// - `limit`: max encoding size
        pub fn max_encoding_message_size(mut self, limit: usize) -> Self {
            self.inner = self.inner.max_encoding_message_size(limit);
            self
        }

        /// Purpose: 원격 ExEx notification 스트림 구독
        /// Param:
        /// - `self`: 현재 RemoteIndexerClient
        /// - `request`: SubscribeRequest
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
            self.inner
                .server_streaming(request.into_request(), path, codec)
                .await
        }
    }
}

#[cfg(test)]
mod tests {
    use prost::Message;

    use super::*;

    #[test]
    fn notification_kind_converts_from_wire_value() {
        // notification kind wire value 검증
        assert_eq!(
            ExExNotificationKind::try_from(1).unwrap(),
            ExExNotificationKind::ChainCommitted
        );
        assert!(ExExNotificationKind::try_from(99).is_err());
    }

    #[test]
    fn exex_notification_round_trips_through_protobuf_encoding() {
        // protobuf encode/decode 검증
        let notification = ExExNotification {
            kind: ExExNotificationKind::ChainCommitted as i32,
            old_range: None,
            new_range: Some(BlockRange {
                first: 10,
                last: 10,
            }),
            fork_block: None,
            tip_block: Some(BlockRef {
                number: 10,
                hash: vec![1; 32],
            }),
            new_blocks: vec![Block {
                number: 10,
                hash: vec![1; 32],
                parent_hash: vec![2; 32],
                timestamp: 1_700_000_000,
                transactions: vec![Transaction {
                    hash: vec![3; 32],
                    index: 0,
                    from: vec![4; 20],
                    to: Some(vec![5; 20]),
                    value_raw: "1".to_owned(),
                    logs: vec![Log {
                        index: 0,
                        contract_address: vec![6; 20],
                        topics: vec![vec![7; 32]],
                        data: vec![8, 9],
                    }],
                }],
                chain_id: 1,
            }],
            chain_id: 1,
        };

        let mut bytes = Vec::new();
        notification.encode(&mut bytes).unwrap();
        let decoded = ExExNotification::decode(bytes.as_slice()).unwrap();

        assert_eq!(decoded, notification);
    }
}
