//! `WebSocketRequest.connector` replaces the built-in tungstenite connect.
#![cfg(feature = "ws")]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;

use aimux_core::AiMuxError;
use aimux_provider_utils::ws::{WebSocketRequest, WsConnection, WsConnector, ws_connect};

struct RefusingConnector(AtomicUsize);

#[async_trait]
impl WsConnector for RefusingConnector {
    async fn connect(&self, request: &WebSocketRequest) -> Result<WsConnection, AiMuxError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(AiMuxError::Other(format!(
            "custom connector: {}",
            request.url
        )))
    }
}

#[tokio::test]
async fn ws_connect_delegates_to_the_injected_connector() {
    let connector = Arc::new(RefusingConnector(AtomicUsize::new(0)));
    let request = WebSocketRequest {
        // Not connectable: only the injected connector can have produced the error.
        url: "wss://unreachable.invalid/realtime".to_string(),
        headers: Vec::new(),
        subprotocols: Vec::new(),
        abort_signal: None,
        timeout: None,
        connector: Some(connector.clone()),
    };

    let error = ws_connect(&request).await.err().expect("connector refuses");

    assert!(
        matches!(&error, AiMuxError::Other(m) if m == "custom connector: wss://unreachable.invalid/realtime"),
        "{error:?}"
    );
    assert_eq!(connector.0.load(Ordering::SeqCst), 1);
}
