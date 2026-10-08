//! Wire regression tests use local HTTP/WebSocket peers, never a T3 server.
use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn intw_upload_posts_raw_bytes_to_signed_url() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut header = Vec::new();
        while !header.ends_with(b"\r\n\r\n") {
            header.push(stream.read_u8().await.unwrap());
            assert!(header.len() < 8192);
        }
        let header = String::from_utf8(header).unwrap();
        assert!(header.starts_with("POST /api/attachments/upload/claims.signature HTTP/1.1\r\n"));
        assert!(header.contains("content-type: image/png\r\n"));
        assert!(header.contains("content-length: 4\r\n"));
        assert!(!header.to_lowercase().contains("authorization:"));
        let mut body = [0; 4];
        stream.read_exact(&mut body).await.unwrap();
        assert_eq!(body, [0, 255, 13, 10]);
        stream
            .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
    });
    upload_body(
        &format!("http://{address}/api/attachments/upload/claims.signature"),
        "image/png",
        vec![0, 255, 13, 10],
    )
    .await
    .unwrap();
    server.await.unwrap();
}

#[test]
fn intw_terminal_chunks_preserve_scalar_boundaries_and_paste_wrappers() {
    for data in [
        String::new(),
        "a".repeat(65_536),
        format!(
            "\x1b[200~{}😀{}\x1b[201~",
            "a".repeat(65_529),
            "😀".repeat(65_537)
        ),
    ] {
        let chunks = terminal_chunks(&data);
        assert!(
            chunks
                .iter()
                .all(|chunk| !chunk.is_empty() && chunk.encode_utf16().count() <= 65_536)
        );
        assert_eq!(chunks.concat(), data);
    }
}

async fn read(socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>) -> Value {
    serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap()
}

#[tokio::test]
async fn intw_terminal_transport_serializes_chunks_and_aggregates_all_receipts() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let paste = format!("\x1b[200~{}\x1b[201~", "😀".repeat(65_536));
    let expected = paste.clone();
    let (done, finished) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        assert_eq!(
            read(&mut socket).await["tag"],
            "orchestration.subscribeShell"
        );
        socket
            .send(Message::Text(
                json!({"_tag":"Chunk","requestId":"1","values":[{"kind":"synchronized"}]})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        assert_eq!(read(&mut socket).await["_tag"], "Ack");
        let mut combined = String::new();
        let mut ids = Vec::new();
        for _ in terminal_chunks(&expected) {
            let request = read(&mut socket).await;
            assert_eq!(request["tag"], "terminal.write");
            assert_eq!(request["payload"]["threadId"], "t");
            assert_eq!(request["payload"]["terminalId"], "pty");
            let data = request["payload"]["data"].as_str().unwrap();
            assert!(!data.is_empty() && data.encode_utf16().count() <= 65_536);
            combined.push_str(data);
            ids.push(request["id"].clone());
        }
        assert_eq!(combined, expected);
        let next = read(&mut socket).await;
        assert_eq!(next["payload"]["data"], "later");
        // Even out-of-order exits must not drop an earlier chunk's error.
        for (index, id) in ids.iter().enumerate().rev() {
            let exit = if index == 0 {
                json!({"_tag":"Failure","cause":[{"error":{"_tag":"TerminalError","message":"first chunk failed"}}]})
            } else {
                json!({"_tag":"Success","value":null})
            };
            socket
                .send(Message::Text(
                    json!({"_tag":"Exit","requestId":id,"exit":exit})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
        }
        socket.send(Message::Text(json!({"_tag":"Exit","requestId":next["id"],"exit":{"_tag":"Success","value":null}}).to_string().into())).await.unwrap();
        finished.await.unwrap();
        socket.close(None).await.unwrap();
    });
    let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}"))
        .await
        .unwrap();
    let (updates, incoming) = async_channel::bounded(32);
    let (commands, requests) = async_channel::bounded(4);
    let client = tokio::spawn(async move { run_session(socket, &updates, &requests).await });
    while !matches!(incoming.recv().await.unwrap(), Update::Shell(item) if matches!(*item, ShellItem::Synchronized))
    {
    }
    let (reply, receipt) = async_channel::bounded(1);
    commands
        .send(Command::Call {
            tag: "terminal.write".into(),
            payload: json!({"threadId":"t","terminalId":"pty","data":paste}),
            reply,
        })
        .await
        .unwrap();
    let (empty_reply, empty_receipt) = async_channel::bounded(1);
    commands
        .send(Command::Call {
            tag: "terminal.write".into(),
            payload: json!({"threadId":"t","terminalId":"pty","data":""}),
            reply: empty_reply,
        })
        .await
        .unwrap();
    let (next_reply, next_receipt) = async_channel::bounded(1);
    commands
        .send(Command::Call {
            tag: "terminal.write".into(),
            payload: json!({"threadId":"t","terminalId":"pty","data":"later"}),
            reply: next_reply,
        })
        .await
        .unwrap();
    assert_eq!(empty_receipt.recv().await.unwrap(), Ok(Value::Null));
    assert!(
        receipt
            .recv()
            .await
            .unwrap()
            .unwrap_err()
            .contains("first chunk failed")
    );
    assert_eq!(next_receipt.recv().await.unwrap(), Ok(Value::Null));
    done.send(()).unwrap();
    drop(commands);
    let _ = client.await.unwrap();
    server.await.unwrap();
}

#[test]
fn intw_auth_subscription_is_a_supported_stream() {
    assert!(supported_subscription("provider.auth.subscribe"));
}
