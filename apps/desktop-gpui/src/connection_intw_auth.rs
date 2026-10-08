use super::*;

#[tokio::test]
async fn intw_auth_stream_uses_instance_payload_and_delivers_phase_changes() {
    async fn read(socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>) -> Value {
        serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap()
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
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
        let request = read(&mut socket).await;
        assert_eq!(request["tag"], "provider.auth.subscribe");
        assert_eq!(request["payload"], json!({"instanceId":"login"}));
        for phase in ["waiting", "succeeded"] {
            socket.send(Message::Text(json!({"_tag":"Chunk","requestId":request["id"],"values":[{"instanceId":"login","phase":phase,"flowId":null,"authorizationUrl":null,"expiresAt":null,"message":null}]}).to_string().into())).await.unwrap();
            assert_eq!(read(&mut socket).await["_tag"], "Ack");
        }
        finished.await.unwrap();
        socket.close(None).await.unwrap();
    });
    let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}"))
        .await
        .unwrap();
    let (updates, incoming) = async_channel::bounded(32);
    let (commands, requests) = async_channel::bounded(1);
    let client = tokio::spawn(async move { run_session(socket, &updates, &requests).await });
    while !matches!(incoming.recv().await.unwrap(), Update::Shell(item) if matches!(*item, ShellItem::Synchronized))
    {
    }
    commands
        .send(Command::Subscribe {
            key: "auth:login".into(),
            tag: "provider.auth.subscribe".into(),
            payload: json!({"instanceId":"login"}),
        })
        .await
        .unwrap();
    for expected in ["waiting", "succeeded"] {
        let Update::Subscription { key, item } = incoming.recv().await.unwrap() else {
            panic!("expected auth update")
        };
        assert_eq!(key, "auth:login");
        assert_eq!(item.unwrap()["phase"], expected);
    }
    done.send(()).unwrap();
    let _ = client.await.unwrap();
    server.await.unwrap();
}
