use std::time::Duration;

use tokio::time::sleep;

use crate::{
    error::{AppError, Result},
    proto::{ExExNotification, SubscribeRequest, remote_indexer_client::RemoteIndexerClient},
};

#[derive(Debug, Clone)]
pub struct RemoteSubscriber {
    endpoint: String,
    reconnect_delay: Duration,
}

impl RemoteSubscriber {
    /// Purpose: 원격 ExEx 구독자 생성
    /// Param:
    /// - `endpoint`: remote ExEx gRPC endpoint
    /// - `reconnect_delay`: reconnect delay
    pub fn new(endpoint: impl Into<String>, reconnect_delay: Duration) -> Self {
        Self {
            endpoint: endpoint.into(),
            reconnect_delay,
        }
    }

    /// Purpose: 원격 ExEx 스트림을 계속 구독하며 끊기면 재연결
    /// Param:
    /// - `self`: RemoteSubscriber
    /// - `handle_notification`: notification handler
    pub async fn run<F, Fut>(&self, mut handle_notification: F) -> Result<()>
    where
        F: FnMut(ExExNotification) -> Fut,
        Fut: Future<Output = Result<()>>,
    {
        loop {
            if let Err(error) = self.subscribe_once(&mut handle_notification).await {
                eprintln!("remote subscription failed: {error}");
            }

            sleep(self.reconnect_delay).await;
        }
    }

    /// Purpose: 원격 ExEx에 한 번 연결하고 스트림 종료까지 notification 처리
    /// Param:
    /// - `self`: RemoteSubscriber
    /// - `handle_notification`: notification handler
    async fn subscribe_once<F, Fut>(&self, handle_notification: &mut F) -> Result<()>
    where
        F: FnMut(ExExNotification) -> Fut,
        Fut: Future<Output = Result<()>>,
    {
        let mut client = RemoteIndexerClient::connect(self.endpoint.clone())
            .await
            .map_err(|error| AppError::with_source("failed to connect to remote ExEx", error))?
            .max_encoding_message_size(usize::MAX)
            .max_decoding_message_size(usize::MAX);

        let mut stream = client
            .subscribe(SubscribeRequest {})
            .await
            .map_err(|error| AppError::with_source("failed to subscribe to remote ExEx", error))?
            .into_inner();

        while let Some(notification) = stream
            .message()
            .await
            .map_err(|error| AppError::with_source("failed to read remote ExEx stream", error))?
        {
            handle_notification(notification).await?;
        }

        Err(AppError::msg("remote ExEx stream ended"))
    }
}
