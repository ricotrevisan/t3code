use super::*;
use crate::shell::{ShellItem, Snapshot};

#[tokio::test]
async fn r14_a_busy_selected_environment_cannot_starve_background_snapshots() {
    let mut registry = Registry::default();
    let mut sources = Vec::new();
    let mut command_receivers = Vec::new();
    for id in ["a-office", "z-lab"] {
        let (updates, receiver) = async_channel::bounded(16);
        let (commands, requests) = async_channel::bounded(16);
        registry
            .insert(
                id.into(),
                Session {
                    updates: receiver,
                    commands,
                },
            )
            .unwrap();
        sources.push(updates);
        command_receivers.push(requests);
    }
    registry.select("a-office").unwrap();
    for sequence in 1..=16 {
        sources[0]
            .send(Update::Shell(Box::new(ShellItem::Snapshot {
                snapshot: Snapshot {
                    snapshot_sequence: sequence,
                    projects: vec![],
                    threads: vec![],
                },
            })))
            .await
            .unwrap();
    }
    sources[1]
        .send(Update::Shell(Box::new(ShellItem::Snapshot {
            snapshot: Snapshot {
                snapshot_sequence: 99,
                projects: vec![],
                threads: vec![],
            },
        })))
        .await
        .unwrap();
    registry.next().await.unwrap();
    let event = registry.next().await.unwrap();
    assert_eq!(event.owner.environment_id, "z-lab");
    assert!(!event.selected);
    assert_eq!(registry.get("z-lab").unwrap().shell.sequence, Some(99));
    assert_eq!(registry.selected().unwrap().shell.sequence, Some(1));
}

#[tokio::test]
async fn r14_a_entity_receive_does_not_borrow_registry_and_stale_delivery_is_rejected() {
    let mut registry = Registry::default();
    let (updates, receiver) = async_channel::bounded(4);
    let (commands, _requests) = async_channel::bounded(4);
    registry
        .insert(
            "office".into(),
            Session {
                updates: receiver,
                commands,
            },
        )
        .unwrap();
    let receive = registry.receive();
    // The owned future permits selection while waiting, as GPUI requires.
    registry.select("office").unwrap();
    updates
        .send(Update::Error("late old session error".into()))
        .await
        .unwrap();
    let incoming = receive.await.unwrap();
    registry.remove("office");
    let (_updates, receiver) = async_channel::bounded(4);
    let (commands, _new_requests) = async_channel::bounded(4);
    let replacement = registry
        .insert(
            "office".into(),
            Session {
                updates: receiver,
                commands,
            },
        )
        .unwrap();
    registry.select("office").unwrap();
    assert!(registry.apply(incoming).is_none());
    assert!(registry.owns(&replacement));
    assert_eq!(registry.selected().unwrap().phase, Phase::Connecting);
}
