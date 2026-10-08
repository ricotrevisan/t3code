use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn accept_ticket(listener: &tokio::net::TcpListener, ticket: &str) {
    let (mut stream, _) = listener.accept().await.unwrap();
    let mut request = Vec::new();
    while !request.ends_with(b"\r\n\r\n") {
        let mut byte = [0];
        stream.read_exact(&mut byte).await.unwrap();
        request.push(byte[0]);
        assert!(request.len() < 8192);
    }
    let text = String::from_utf8(request).unwrap().to_lowercase();
    assert!(text.starts_with("post /api/auth/websocket-ticket "));
    assert!(text.contains("authorization: bearer test-token"));
    let body = json!({"ticket":ticket,"expiresAt":"2099-01-01T00:00:00Z"}).to_string();
    stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
}

async fn read(socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>) -> Value {
    serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap()
}

async fn emit(
    socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    value: Value,
) {
    socket
        .send(Message::Text(value.to_string().into()))
        .await
        .unwrap();
}

#[tokio::test]
#[allow(clippy::result_large_err)] // tungstenite fixes the HTTP callback error type.
async fn reconnect_uses_fresh_ticket_restores_selection_and_never_replays_uncertain_mutation() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (second_finished, finished) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        for round in 1..=2 {
            let ticket = format!("ticket-{round}");
            accept_ticket(&listener, &ticket).await;
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_hdr_async(
                stream,
                |request: &tokio_tungstenite::tungstenite::handshake::server::Request, response| {
                    assert!(
                        request
                            .uri()
                            .to_string()
                            .contains(&format!("wsTicket={ticket}"))
                    );
                    Ok(response)
                },
            )
            .await
            .unwrap();
            assert_eq!(
                read(&mut socket).await["tag"],
                "orchestration.subscribeShell"
            );
            if round == 2 {
                let restored = read(&mut socket).await;
                assert_eq!(restored["tag"], "orchestration.subscribeThread");
                assert_eq!(restored["payload"]["threadId"], "t");
            }
            emit(&mut socket,json!({"_tag":"Chunk","requestId":"1","values":[{"kind":"snapshot","snapshot":{"snapshotSequence":round,"projects":[],"threads":[]}},{"kind":"synchronized"}]})).await;
            assert_eq!(read(&mut socket).await["_tag"], "Ack");
            if round == 1 {
                let selected = read(&mut socket).await;
                assert_eq!(selected["tag"], "orchestration.subscribeThread");
            }
            emit(&mut socket,json!({"_tag":"Chunk","requestId":"2","values":[{"kind":"snapshot","snapshot":{"snapshotSequence":round,"thread":{"id":"t","messages":[]}}},{"kind":"synchronized"}]})).await;
            assert_eq!(read(&mut socket).await["_tag"], "Ack");
            if round == 1 {
                let mutation = read(&mut socket).await;
                assert_eq!(mutation["tag"], "orchestration.dispatchCommand");
                assert_eq!(mutation["payload"]["commandId"], "uncertain-command");
                // Deliberately lose the receipt: repeating it would create an unsafe UI retry.
                socket.close(None).await.unwrap();
            } else {
                let probe = read(&mut socket).await;
                // The first unary after reconnection must be the explicit probe, not the lost mutation.
                assert_eq!(probe["tag"], "orchestration.searchThreads");
                emit(&mut socket,json!({"_tag":"Exit","requestId":probe["id"],"exit":{"_tag":"Success","value":{"matches":[]}}})).await;
                second_finished.send(()).unwrap();
                return;
            }
        }
    });
    let (sender, updates) = async_channel::bounded(32);
    let (commands, requests) = async_channel::bounded(16);
    let options =
        ConnectionOptions::new(&format!("http://{address}"), "test-token".into()).unwrap();
    let driver = tokio::spawn(async move {
        supervise(options, &sender, &requests).await;
    });
    while !matches!(
        updates.recv().await.unwrap(),
        Update::Shell(item) if matches!(*item,ShellItem::Synchronized)
    ) {}
    commands
        .send(Command::SelectThread(Some("t".into())))
        .await
        .unwrap();
    while !matches!(updates.recv().await.unwrap(),Update::Thread { item,.. } if item["kind"]=="synchronized")
    {
    }
    let outcome = call(
        &commands,
        "orchestration.dispatchCommand",
        json!({"commandId":"uncertain-command"}),
    )
    .await;
    assert!(outcome.unwrap_err().contains("outcome is unknown"));
    let mut reset = false;
    let mut reconnect_phase = false;
    let mut second_sync = false;
    while !second_sync {
        match updates.recv().await.unwrap() {
            Update::Resynchronizing => reset = true,
            Update::Phase(Phase::Reconnecting {
                attempt, retry_in, ..
            }) => {
                assert_eq!(attempt, 1);
                assert!(retry_in <= Duration::from_millis(250));
                reconnect_phase = true;
                commands.send(Command::RetryNow).await.unwrap();
            }
            Update::Thread { thread_id, item } if item["kind"] == "synchronized" => {
                assert_eq!(thread_id, "t");
                second_sync = true;
            }
            _ => {}
        }
    }
    assert!(
        reset,
        "UI must discard stale projection before replacement snapshots"
    );
    assert!(
        reconnect_phase,
        "typed backoff phase precedes the fresh connection"
    );
    assert_eq!(
        call(
            &commands,
            "orchestration.searchThreads",
            json!({"query":"probe"})
        )
        .await
        .unwrap(),
        json!({"matches":[]})
    );
    finished.await.unwrap();
    driver.abort();
    server.await.unwrap();
}

#[tokio::test]
async fn disconnected_commands_fail_without_being_requeued_and_selection_is_remembered() {
    let mut selected = None;
    let mut subscriptions = Subscriptions::new();
    reject_disconnected(
        Command::SelectThread(Some("t".into())),
        &mut selected,
        &mut subscriptions,
    )
    .await;
    assert_eq!(selected.as_deref(), Some("t"));
    let (reply, result) = async_channel::bounded(1);
    reject_disconnected(
        Command::Call {
            tag: "orchestration.dispatchCommand".into(),
            payload: json!({}),
            reply,
        },
        &mut selected,
        &mut subscriptions,
    )
    .await;
    assert!(
        result
            .recv()
            .await
            .unwrap()
            .unwrap_err()
            .contains("not sent")
    );
    reject_disconnected(
        Command::SelectThread(None),
        &mut selected,
        &mut subscriptions,
    )
    .await;
    assert!(selected.is_none());
}
