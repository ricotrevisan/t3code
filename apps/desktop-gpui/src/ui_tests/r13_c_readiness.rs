use super::composer_parity::provider_fixture;
use super::*;

#[gpui_kit::test]
fn r13_c_compact_slash_availability_follows_typed_sync_state(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            app.provider_catalog = vec![provider_fixture("codex", "Personal")];
        })
    });
    cx.simulate_input("/");
    cx.run_until_parked();
    let connection::Command::Call { tag, reply, .. } = requests.try_recv().unwrap() else {
        panic!("catalog");
    };
    assert_eq!(tag, "server.refreshProviders");
    reply.try_send(Ok(json!({"providers":[super::composer_parity::provider_fixture_value("codex", "Personal")]}))).unwrap();
    cx.run_until_parked();
    assert!(cx.debug_bounds("slash-/compact").is_some());
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.apply_connection_phase(connection::Phase::Syncing, cx);
            assert!(app.commands.is_some());
        })
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("slash-/compact").is_none());
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.apply_shell_item(shell::ShellItem::Synchronized, cx)
                .unwrap();
            assert!(app.connection_ready());
            cx.notify();
        })
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("slash-/compact").is_some());
    assert!(requests.try_recv().is_err());
}

#[gpui_kit::test]
fn r13_c_compact_reconnect_retains_sender_but_blocks_until_synchronized(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.provider_catalog = vec![provider_fixture("codex", "Personal")];
            app.composer
                .update(cx, |input, cx| input.set_value("Keep draft", window, cx));
            app.attachments.push(json!({"id":"owned"}));
            for phase in [
                connection::Phase::Offline,
                connection::Phase::Syncing,
                connection::Phase::Reconnecting {
                    attempt: 1,
                    retry_in: std::time::Duration::from_secs(1),
                    reason: "Lost stream".into(),
                },
            ] {
                app.connection_phase = phase;
                assert!(app.commands.is_some());
                assert_eq!(
                    app.compact_disabled_reason(cx),
                    Some("Reconnect before compacting")
                );
                app.compact_context(cx);
            }
            app.connection_phase = connection::Phase::Ready;
            app.shell.synchronized = false;
            assert_eq!(
                app.compact_disabled_reason(cx),
                Some("Reconnect before compacting")
            );
            app.compact_context(cx);
        })
    });
    cx.run_until_parked();
    assert!(requests.try_recv().is_err());
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.shell.synchronized = true;
            assert_eq!(app.compact_disabled_reason(cx), None);
            app.compact_context(cx);
        })
    });
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("compact call");
    };
    assert_eq!(tag, "orchestration.dispatchCommand");
    assert_eq!(payload["type"], "thread.turn.start");
    assert_eq!(payload["threadId"], "t");
    assert_eq!(payload["message"]["text"], "/compact");
    assert_eq!(payload["message"]["attachments"], json!([]));
    reply
        .try_send(Err("Provider rejected compaction".into()))
        .unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, cx| {
        assert_eq!(app.composer.read(cx).value().as_str(), "Keep draft");
        assert_eq!(app.attachments, vec![json!({"id":"owned"})]);
        assert_eq!(app.pending_actions, 0);
    });
}
