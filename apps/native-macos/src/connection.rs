//! Bearer authorization and Effect RPC shell subscription on a dedicated runtime.
//! Credentials stay off argv, logs and persisted app state.
use crate::protocol::{self, ClientMessage, RequestId, RpcExit, ServerMessage};
use crate::shell::ShellItem;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;
use url::Url;

pub enum Update {
    Status(String),
    Shell(ShellItem),
}

pub struct ConnectionOptions {
    base: Url,
    token: String,
}

impl ConnectionOptions {
    pub fn new(base: &str, token: String) -> Result<Self, &'static str> {
        let base = Url::parse(base).map_err(|_| "Invalid server URL")?;
        if !matches!(base.scheme(), "http" | "https")
            || base.host_str().is_none()
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
        {
            return Err("Use an HTTP(S) origin without credentials, query or fragment");
        }
        if base.path() != "/" {
            return Err("Path-prefixed server URLs are not supported yet");
        }
        let loopback = matches!(base.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        if base.scheme() == "http" && !loopback {
            return Err("Remote servers require HTTPS");
        }
        if token.trim().is_empty() {
            return Err("A bearer access token is required");
        }
        Ok(Self { base, token })
    }
}

#[derive(Deserialize)]
struct Ticket {
    ticket: String,
}

pub fn start(options: ConnectionOptions) -> async_channel::Receiver<Update> {
    // A bounded channel prevents a blocked UI from accumulating arbitrary history.
    let (sender, receiver) = async_channel::bounded(64);
    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(_) => {
                let _ = sender
                    .send_blocking(Update::Status("Could not start connection runtime".into()));
                return;
            }
        };
        runtime.block_on(async move {
            let work = async {
                let _ = sender.send(Update::Status("Authorizing…".into())).await;
                let client = reqwest::Client::builder().timeout(Duration::from_secs(15))
                    .redirect(reqwest::redirect::Policy::none()).build().map_err(|_| "Could not create HTTP client")?;
                let response = client.post(options.base.join("api/auth/websocket-ticket").map_err(|_| "Invalid ticket endpoint")?)
                    .bearer_auth(&options.token).send().await.map_err(|_| "Could not reach authorization endpoint")?;
                if !response.status().is_success() { return Err("Authorization rejected; check token and scopes"); }
                let ticket: Ticket = response.json().await.map_err(|_| "Invalid authorization response")?;
                let mut socket_url = options.base.clone();
                let scheme = if socket_url.scheme() == "https" { "wss" } else { "ws" };
                socket_url.set_scheme(scheme).map_err(|_| "Invalid WebSocket scheme")?;
                socket_url.set_path("/ws");
                socket_url.query_pairs_mut().append_pair("wsTicket", &ticket.ticket).append_pair("orchestrationProtocol", "1")
                    .append_pair("clientSurface", "desktop")
                    .append_pair("clientDeviceType", "desktop")
                    .append_pair("clientOs", "macos")
                    .append_pair("clientAppVersion", env!("CARGO_PKG_VERSION"))
                    .append_pair("connectionMethod", "direct");
                let (socket, _) = tokio::time::timeout(Duration::from_secs(15), tokio_tungstenite::connect_async(socket_url.as_str()))
                    .await.map_err(|_| "WebSocket connection timed out")?
                    .map_err(|_| "WebSocket connection rejected")?;
                let _ = sender.send(Update::Status("Loading projects and threads…".into())).await;
                run_socket(socket, &sender).await
            };
            // Dropping the UI receiver cancels both authorization and the socket worker.
            tokio::select! {
                result = work => { if let Err(message) = result { let _ = sender.send(Update::Status(message.into())).await; } }
                _ = sender.closed() => {}
            }
        });
    });
    receiver
}

async fn send<S>(
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
    message: ClientMessage,
) -> Result<(), &'static str>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let text = serde_json::to_string(&message).map_err(|_| "Could not encode RPC message")?;
    socket
        .send(Message::Text(text.into()))
        .await
        .map_err(|_| "Connection closed while sending")
}

async fn run_socket<S>(
    mut socket: tokio_tungstenite::WebSocketStream<S>,
    sender: &async_channel::Sender<Update>,
) -> Result<(), &'static str>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    send(
        &mut socket,
        protocol::request(
            1,
            "orchestration.subscribeShell",
            json!({"requestCompletionMarker":true}),
        ),
    )
    .await?;
    let mut heartbeat = tokio::time::interval_at(
        tokio::time::Instant::now() + Duration::from_secs(5),
        Duration::from_secs(5),
    );
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut pong_received = true;
    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                if !pong_received { return Err("Server heartbeat timed out"); }
                pong_received = false;
                send(&mut socket, ClientMessage::Ping).await?;
            }
            incoming = socket.next() => {
                let incoming = incoming.ok_or("Server disconnected")?.map_err(|_| "WebSocket transport failed")?;
                let bytes = match incoming {
                    Message::Text(text) => text.as_bytes().to_vec(),
                    Message::Binary(bytes) => bytes.to_vec(),
                    Message::Close(_) => return Err("Server disconnected"),
                    Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => continue,
                };
                let messages = protocol::decode_frame(&bytes).map_err(|_| "Unsupported or malformed RPC message")?;
                for message in messages {
                    match message {
                        ServerMessage::Pong => pong_received = true,
                        ServerMessage::Chunk { request_id, values } => {
                            if request_id != RequestId::String("1".into()) { return Err("Unexpected RPC request ID"); }
                            if values.is_empty() { return Err("Empty RPC stream chunk"); }
                            for value in values {
                                let item: ShellItem = serde_json::from_value(value).map_err(|_| "Unsupported shell stream event")?;
                                sender.send(Update::Shell(item)).await.map_err(|_| "Client closed")?;
                            }
                            // Acknowledge only once the bounded consumer has accepted the full chunk.
                            send(&mut socket, ClientMessage::Ack { request_id }).await?;
                        }
                        ServerMessage::Exit { request_id, exit } => {
                            if request_id != RequestId::String("1".into()) { return Err("Unexpected RPC request ID"); }
                            match exit {
                                RpcExit::Success { value } => { drop(value); return Err("Shell subscription ended"); }
                                RpcExit::Failure { cause } => { drop(cause); return Err("Shell subscription failed; check authorization and server compatibility"); }
                            }
                        }
                        ServerMessage::Defect { defect } => { drop(defect); return Err("Server reported an RPC defect"); }
                        ServerMessage::ClientProtocolError { error } => { drop(error); return Err("Server rejected the RPC protocol"); }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_configuration_rejects_credential_leaks_and_unsafe_origins() {
        for base in [
            "http://remote.example",
            "https://user:password@host",
            "https://host/?token=x",
            "https://host/#secret",
            "file:///tmp/x",
            "https://host/prefix/",
        ] {
            assert!(
                ConnectionOptions::new(base, "token".into()).is_err(),
                "{base}"
            );
        }
        for base in [
            "http://127.0.0.1:3777",
            "http://localhost:3777",
            "https://host",
        ] {
            assert!(
                ConnectionOptions::new(base, "token".into()).is_ok(),
                "{base}"
            );
        }
        assert!(ConnectionOptions::new("https://host", "".into()).is_err());
    }

    #[tokio::test]
    async fn rejected_authorization_reports_failure_without_leaking_credentials() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).await.unwrap();
                request.push(byte[0]);
                assert!(request.len() < 8192);
            }
            stream
                .write_all(
                    b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
        });
        let receiver = start(
            ConnectionOptions::new(&format!("http://{address}"), "private-secret".into()).unwrap(),
        );
        let mut failed = false;
        while let Ok(update) = receiver.recv().await {
            match update {
                Update::Status(status) => {
                    assert!(!status.contains("private-secret"));
                    failed |= status.contains("Authorization rejected");
                }
                Update::Shell(_) => panic!("unauthorized connection delivered shell state"),
            }
        }
        assert!(failed);
        server.await.unwrap();
    }

    #[tokio::test]
    // tungstenite's handshake callback fixes the HTTP response error type.
    #[allow(clippy::result_large_err)]
    async fn bearer_ticket_is_exchanged_before_authenticated_websocket() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut http, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let mut byte = [0];
                http.read_exact(&mut byte).await.unwrap();
                request.push(byte[0]);
                if request.ends_with(b"\r\n\r\n") {
                    break;
                }
                assert!(request.len() < 8192);
            }
            let request = String::from_utf8(request).unwrap().to_lowercase();
            assert!(request.starts_with("post /api/auth/websocket-ticket "));
            assert!(request.contains("authorization: bearer test-secret"));
            let body = r#"{"ticket":"one-time-ticket","expiresAt":"2026-01-01T00:00:00Z"}"#;
            http.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).as_bytes()).await.unwrap();
            drop(http);
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_hdr_async(
                stream,
                |request: &tokio_tungstenite::tungstenite::handshake::server::Request, response| {
                    let uri = request.uri().to_string();
                    assert!(uri.starts_with("/ws?"));
                    assert!(uri.contains("wsTicket=one-time-ticket"));
                    assert!(uri.contains("orchestrationProtocol=1"));
                    assert!(uri.contains("clientSurface=desktop"));
                    assert!(!uri.contains("test-secret"));
                    Ok(response)
                },
            )
            .await
            .unwrap();
            let _ = socket.next().await.unwrap().unwrap();
            socket.send(Message::Text(json!({"_tag":"Chunk","requestId":"1","values":[{"kind":"snapshot","snapshot":{"snapshotSequence":0,"projects":[],"threads":[]}},{"kind":"synchronized"}]}).to_string().into())).await.unwrap();
            let ack = socket.next().await.unwrap().unwrap();
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(ack.to_text().unwrap()).unwrap()["_tag"],
                "Ack"
            );
            socket.close(None).await.unwrap();
        });
        let receiver = start(
            ConnectionOptions::new(&format!("http://{address}"), "test-secret".into()).unwrap(),
        );
        let mut shell = crate::shell::Shell::default();
        while let Ok(update) = receiver.recv().await {
            if let Update::Shell(item) = update {
                shell.apply(item).unwrap();
                if shell.synchronized {
                    break;
                }
            }
        }
        assert!(shell.synchronized);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn dropping_ui_receiver_cancels_pending_authorization() {
        use tokio::io::AsyncReadExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (accepted, request_started) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).await.unwrap();
                request.push(byte[0]);
                assert!(request.len() < 8192);
            }
            accepted.send(()).unwrap();
            // The HTTP response is intentionally withheld. Cancellation must close it.
            let mut byte = [0];
            assert_eq!(stream.read(&mut byte).await.unwrap(), 0);
        });
        let receiver =
            start(ConnectionOptions::new(&format!("http://{address}"), "token".into()).unwrap());
        request_started.await.unwrap();
        drop(receiver);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn socket_subscription_delivers_snapshot_and_acks_chunk() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let request: serde_json::Value =
                serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap())
                    .unwrap();
            assert_eq!(request["tag"], "orchestration.subscribeShell");
            assert_eq!(request["payload"], json!({"requestCompletionMarker":true}));
            socket.send(Message::Text(json!({"_tag":"Chunk","requestId":"1","values":[{"kind":"snapshot","snapshot":{"snapshotSequence":2,"projects":[{"id":"p","title":"Example","workspaceRoot":"/tmp/example"}],"threads":[]}},{"kind":"synchronized"}]}).to_string().into())).await.unwrap();
            let ack: serde_json::Value =
                serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap())
                    .unwrap();
            assert_eq!(ack, json!({"_tag":"Ack","requestId":"1"}));
            socket.close(None).await.unwrap();
        });
        let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}"))
            .await
            .unwrap();
        let (sender, receiver) = async_channel::bounded(4);
        let result = run_socket(socket, &sender).await;
        assert!(result.is_err());
        let mut shell = crate::shell::Shell::default();
        for _ in 0..2 {
            match receiver.recv().await.unwrap() {
                Update::Shell(item) => shell.apply(item).unwrap(),
                _ => panic!("expected shell item"),
            }
        }
        assert_eq!(shell.projects[0].title, "Example");
        assert!(shell.synchronized);
        server.await.unwrap();
    }
}
