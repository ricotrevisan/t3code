use super::*;
use crate::connection;
use crate::shell::{ShellItem, Snapshot};
use serde_json::json;

fn fixture(
    registry: &mut Registry,
    id: &str,
) -> (
    Owner,
    async_channel::Sender<Update>,
    async_channel::Receiver<Command>,
) {
    let (updates, receiver) = async_channel::bounded(16);
    let (commands, requests) = async_channel::bounded(16);
    let owner = registry
        .insert(
            id.into(),
            Session {
                updates: receiver,
                commands,
            },
        )
        .unwrap();
    (owner, updates, requests)
}
async fn synchronize(
    registry: &mut Registry,
    updates: &async_channel::Sender<Update>,
    sequence: u64,
) {
    updates
        .send(Update::Shell(Box::new(ShellItem::Snapshot {
            snapshot: Snapshot {
                snapshot_sequence: sequence,
                projects: vec![],
                threads: vec![],
            },
        })))
        .await
        .unwrap();
    assert!(matches!(
        registry.next().await.unwrap().kind,
        EventKind::Shell
    ));
    updates
        .send(Update::Shell(Box::new(ShellItem::Synchronized)))
        .await
        .unwrap();
    registry.next().await.unwrap();
    // Readiness comes from the authoritative synchronized shell receipt.
    assert!(
        registry
            .iter()
            .any(|(_, session)| session.shell.sequence == Some(sequence) && session.ready())
    );
}

#[tokio::test]
async fn r14_a_multi_session_switch_keeps_background_projection_and_receipt_owner() {
    let mut registry = Registry::default();
    let (a, a_updates, a_requests) = fixture(&mut registry, "office");
    let (b, b_updates, b_requests) = fixture(&mut registry, "lab");
    synchronize(&mut registry, &a_updates, 10).await;
    synchronize(&mut registry, &b_updates, 30).await;
    registry.select("office").unwrap();
    let commands = registry.commands(&a).unwrap();
    let call = tokio::spawn(async move {
        connection::call(
            &commands,
            "orchestration.dispatchCommand",
            json!({"commandId":"owned-office", "type":"thread.turn.interrupt", "threadId":"same-id", "createdAt":"2026-01-01T00:00:00.000Z"}),
        )
        .await
    });
    let Command::Call {
        tag,
        payload,
        reply,
    } = a_requests.recv().await.unwrap()
    else {
        panic!("owned call")
    };
    assert_eq!(tag, "orchestration.dispatchCommand");
    assert_eq!(payload["threadId"], "same-id");
    assert_eq!(payload["commandId"], "owned-office");
    registry.select("lab").unwrap();
    assert!(!registry.selected_owns(&a));
    assert!(
        registry.owns(&a),
        "receipt still belongs to the background session"
    );
    assert!(
        b_requests.try_recv().is_err(),
        "selection never dispatches or replays mutations"
    );
    a_updates
        .send(Update::Shell(Box::new(ShellItem::ProjectUpserted {
            sequence: 11,
            project: crate::shell::Project {
                id: "p".into(),
                title: "Office project".into(),
                workspace_root: "/office".into(),
                repository_identity: None,
                default_model_selection: None,
                scripts: vec![],
            },
        })))
        .await
        .unwrap();
    let event = registry.next().await.unwrap();
    assert_eq!(event.owner, a);
    assert!(!event.selected);
    assert_eq!(
        registry.get("office").unwrap().shell.projects[0].workspace_root,
        "/office"
    );
    assert!(registry.selected().unwrap().shell.projects.is_empty());
    reply.send(Ok(json!({"accepted":true}))).await.unwrap();
    assert_eq!(call.await.unwrap().unwrap(), json!({"accepted":true}));
    assert_eq!(registry.select("office").unwrap(), a);
    assert_eq!(registry.selected().unwrap().shell.sequence, Some(11));
    assert!(registry.commands(&b).is_some());
}

#[tokio::test]
async fn r14_a_background_reconnect_invalidates_only_its_receipts_and_projection() {
    let mut registry = Registry::default();
    let (a, a_updates, a_requests) = fixture(&mut registry, "office");
    let (b, b_updates, _b_requests) = fixture(&mut registry, "lab");
    synchronize(&mut registry, &a_updates, 4).await;
    synchronize(&mut registry, &b_updates, 20).await;
    registry.select_thread(&a, Some("same-id".into())).unwrap();
    assert!(
        matches!(a_requests.recv().await.unwrap(), Command::SelectThread(Some(id)) if id == "same-id")
    );
    registry.select("lab").unwrap();
    a_updates.send(Update::Resynchronizing).await.unwrap();
    let event = registry.next().await.unwrap();
    assert!(!event.selected);
    assert!(matches!(event.kind, EventKind::Resynchronizing));
    assert!(!registry.owns(&a));
    assert!(registry.commands(&a).is_none());
    assert_eq!(registry.get("office").unwrap().shell.sequence, None);
    assert!(!registry.get("office").unwrap().ready());
    assert_eq!(
        registry.get("office").unwrap().selected_thread.as_deref(),
        Some("same-id")
    );
    assert!(registry.owns(&b));
    assert!(registry.selected().unwrap().ready());
    assert_eq!(registry.selected().unwrap().shell.sequence, Some(20));
    synchronize(&mut registry, &a_updates, 5).await;
    let renewed = registry.select("office").unwrap();
    assert_ne!(renewed.generation, a.generation);
    assert_eq!(renewed.instance, a.instance);
    assert!(registry.selected().unwrap().ready());
    assert!(
        a_requests.try_recv().is_err(),
        "registry must not replay uncertain operations on reconnect"
    );
}

#[tokio::test]
async fn r14_a_removed_session_closes_cloned_sender_and_replacement_rejects_old_owner() {
    let mut registry = Registry::default();
    let (old, updates, _old_requests) = fixture(&mut registry, "office");
    let retained = registry.commands(&old).unwrap();
    registry.select("office").unwrap();
    assert!(registry.remove("office"));
    assert!(retained.is_closed());
    assert!(registry.selected().is_none());
    assert!(
        updates
            .try_send(Update::Status("late update".into()))
            .is_err()
    );
    let (new, updates, _new_requests) = fixture(&mut registry, "office");
    assert_ne!(old.instance, new.instance);
    assert!(!registry.owns(&old));
    assert!(registry.owns(&new));
    drop(updates);
    assert!(matches!(
        registry.next().await.unwrap().kind,
        EventKind::Closed
    ));
    assert!(!registry.owns(&new));
    assert!(
        registry.next().await.is_none(),
        "closed receivers must not spin"
    );
}

#[tokio::test]
async fn r14_a_projection_failure_isolated_and_thread_selection_backpressure_preserves_owner() {
    let mut registry = Registry::default();
    let (a, updates, _requests) = fixture(&mut registry, "office");
    let (b, b_updates, _b_requests) = fixture(&mut registry, "lab");
    synchronize(&mut registry, &b_updates, 1).await;
    registry.select("lab").unwrap();
    let commands = registry.commands(&a).unwrap();
    for _ in 0..16 {
        commands.try_send(Command::RetryNow).unwrap();
    }
    assert!(registry.select_thread(&a, Some("not-sent".into())).is_err());
    assert!(registry.get("office").unwrap().selected_thread.is_none());
    updates
        .send(Update::Shell(Box::new(ShellItem::Synchronized)))
        .await
        .unwrap();
    assert!(matches!(
        registry.next().await.unwrap().kind,
        EventKind::Error(_)
    ));
    assert!(matches!(
        registry.get("office").unwrap().phase,
        Phase::Error(_)
    ));
    assert!(commands.is_closed());
    assert!(!registry.owns(&a));
    assert!(registry.owns(&b));
    assert!(registry.selected().unwrap().ready());
}
