//! A blocked HTTP response proves Upload cancellation closes the socket, not only UI state.
use super::*;
use tokio::io::AsyncReadExt;

#[tokio::test]
async fn r14_int_upload_receipt_cancellation_closes_http_socket() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (started, observed) = tokio::sync::oneshot::channel();
    let http = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut header = Vec::new();
        while !header.ends_with(b"\r\n\r\n") {
            header.push(stream.read_u8().await.unwrap());
            assert!(header.len() < 8192);
        }
        assert!(
            String::from_utf8(header)
                .unwrap()
                .starts_with("POST /signed HTTP/1.1")
        );
        let mut body = [0; 4];
        stream.read_exact(&mut body).await.unwrap();
        assert_eq!(body, [1, 2, 3, 4]);
        started.send(()).unwrap();
        let mut byte = [0];
        assert_eq!(
            stream.read(&mut byte).await.unwrap(),
            0,
            "aborted request closes blocked response socket"
        );
    });
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    let socket = tokio_tungstenite::WebSocketStream::from_raw_socket(
        client_io,
        tokio_tungstenite::tungstenite::protocol::Role::Client,
        None,
    )
    .await;
    let mut peer = tokio_tungstenite::WebSocketStream::from_raw_socket(
        server_io,
        tokio_tungstenite::tungstenite::protocol::Role::Server,
        None,
    )
    .await;
    let (updates, incoming) = async_channel::bounded(8);
    let (commands, requests) = async_channel::bounded(8);
    let worker = tokio::spawn(async move { run_session(socket, &updates, &requests).await });
    let request: Value =
        serde_json::from_str(peer.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
    assert_eq!(request["tag"], "orchestration.subscribeShell");
    peer.send(Message::Text(
        json!({"_tag":"Chunk","requestId":request["id"],"values":[{"kind":"synchronized"}]})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    while !matches!(incoming.recv().await.unwrap(), Update::Shell(item) if matches!(*item, ShellItem::Synchronized))
    {
    }
    let (reply, receipt) = async_channel::bounded(1);
    commands
        .send(Command::Upload {
            url: format!("http://{address}/signed"),
            content_type: "application/octet-stream".into(),
            bytes: vec![1, 2, 3, 4],
            reply,
        })
        .await
        .unwrap();
    observed.await.unwrap();
    drop(receipt);
    http.await.unwrap();
    peer.close(None).await.unwrap();
    drop(commands);
    let _ = worker.await.unwrap();
}

#[test]
fn r14_int_every_feature_subscription_is_allowlisted() {
    for tag in [
        "subscribeVcsStatus",
        "subscribeTerminalMetadata",
        "subscribeProjectClones",
        "subscribeServerConfig",
        "subscribeWorktreeSetup",
        "orchestration.subscribeThread",
        "terminal.attach",
        "provider.auth.subscribe",
    ] {
        assert!(supported_subscription(tag), "{tag}");
    }
    assert!(!supported_subscription("pullRequests.comment"));
}
