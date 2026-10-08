use super::*;
use crate::connection_sessions::{EventKind, Registry};
use tokio_tungstenite::{WebSocketStream, tungstenite::protocol::Role};

type TestSocket = WebSocketStream<tokio::io::DuplexStream>;
async fn receive(socket: &mut TestSocket) -> Value {
    serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap()
}
async fn fixture() -> (
    Session,
    TestSocket,
    tokio::task::JoinHandle<Result<(), &'static str>>,
) {
    let (client, server) = tokio::io::duplex(16384);
    let client = WebSocketStream::from_raw_socket(client, Role::Client, None).await;
    let server = WebSocketStream::from_raw_socket(server, Role::Server, None).await;
    let (updates, receiver) = async_channel::bounded(16);
    let (commands, requests) = async_channel::bounded(16);
    let worker = tokio::spawn(async move { run_session(client, &updates, &requests).await });
    (
        Session {
            updates: receiver,
            commands,
        },
        server,
        worker,
    )
}
async fn sync(socket: &mut TestSocket, sequence: u64) {
    let shell = receive(socket).await;
    assert_eq!(shell["tag"], "orchestration.subscribeShell");
    socket.send(Message::Text(json!({"_tag":"Chunk", "requestId":shell["id"], "values":[
        {"kind":"snapshot","snapshot":{"snapshotSequence":sequence,"projects":[],"threads":[]}},
        {"kind":"synchronized"}
    ]}).to_string().into())).await.unwrap();
    assert_eq!(receive(socket).await["_tag"], "Ack");
}

#[tokio::test]
async fn r14_a_two_live_sockets_keep_receipts_isolated_after_switch_and_disconnect() {
    let (office, mut office_server, office_worker) = fixture().await;
    let (lab, mut lab_server, lab_worker) = fixture().await;
    let mut registry = Registry::default();
    let office_owner = registry.insert("office-id".into(), office).unwrap();
    let lab_owner = registry.insert("lab-id".into(), lab).unwrap();
    sync(&mut office_server, 10).await;
    sync(&mut lab_server, 20).await;
    while !registry.iter().all(|(_, session)| session.ready()) {
        registry.next().await.unwrap();
    }
    registry.select("office-id").unwrap();
    let office_commands = registry.commands(&office_owner).unwrap();
    let mutation = tokio::spawn(async move {
        call(
            &office_commands,
            "orchestration.dispatchCommand",
            json!({
                "commandId":"office-stop", "type":"thread.turn.interrupt", "threadId":"same-id", "createdAt":"2026-01-01T00:00:00.000Z"
            }),
        )
        .await
    });
    let request = receive(&mut office_server).await;
    assert_eq!(request["tag"], "orchestration.dispatchCommand");
    assert_eq!(
        request["payload"],
        json!({"commandId":"office-stop", "type":"thread.turn.interrupt", "threadId":"same-id", "createdAt":"2026-01-01T00:00:00.000Z"})
    );
    registry.select("lab-id").unwrap();
    office_server.close(None).await.unwrap();
    assert!(
        mutation
            .await
            .unwrap()
            .unwrap_err()
            .contains("outcome is unknown")
    );
    assert_eq!(office_worker.await.unwrap(), Err("Server disconnected"));
    loop {
        let event = registry.next().await.unwrap();
        if event.owner.environment_id == "office-id" && matches!(event.kind, EventKind::Closed) {
            assert!(!event.selected);
            break;
        }
    }
    assert!(!registry.owns(&office_owner));
    assert!(registry.selected().unwrap().ready());
    assert_eq!(registry.selected().unwrap().shell.sequence, Some(20));
    let commands = registry.commands(&lab_owner).unwrap();
    let probe = tokio::spawn(async move {
        call(
            &commands,
            "orchestration.searchThreads",
            json!({"query":"lab-only"}),
        )
        .await
    });
    let request = receive(&mut lab_server).await;
    // No office stop, cancellation, selection or subscription crossed the socket.
    assert_eq!(request["tag"], "orchestration.searchThreads");
    assert_eq!(request["payload"], json!({"query":"lab-only"}));
    lab_server.send(Message::Text(json!({"_tag":"Exit","requestId":request["id"],"exit":{"_tag":"Success","value":{"matches":[]}}}).to_string().into())).await.unwrap();
    assert_eq!(probe.await.unwrap().unwrap(), json!({"matches":[]}));
    registry.remove("lab-id");
    lab_server.close(None).await.unwrap();
    assert_eq!(lab_worker.await.unwrap(), Err("Server disconnected"));
}
