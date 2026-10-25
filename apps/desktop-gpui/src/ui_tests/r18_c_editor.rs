use super::*;
use t3code_desktop_gpui::{draft_context::DraftContextRecord, mentions};

#[gpui_kit::test]
fn r18_c_removed_context_undo_restores_token_payload_and_rejected_turn(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let record =
        DraftContextRecord::terminal("Build", "term", "Build", 1, 2, "captured output").unwrap();
    let expected = record.value().clone();
    let id = record.id().to_owned();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.provider_catalog = vec![super::composer_parity::provider_fixture(
                "codex", "Personal",
            )];
            let owner = app.environment_scope.clone();
            let thread = app.selected_thread.clone().unwrap();
            app.intake_draft_context(&owner, &thread, record, window, cx)
                .unwrap();
            app.remove_draft_context(&id, window, cx);
            assert!(app.composer.read(cx).tokens().is_empty());
        })
    });
    cx.simulate_keystrokes("cmd-z");
    cx.run_until_parked();
    app.read_with(cx, |app, cx| {
        assert_eq!(app.composer.read(cx).tokens().len(), 1);
        assert!(app.composer.read(cx).value().contains(&id));
    });
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("turn")
    };
    assert_eq!(tag, "orchestration.dispatchCommand");
    assert_eq!(payload["type"], "thread.turn.start");
    assert_eq!(payload["message"]["context"]["records"], json!([expected]));
    reply.try_send(Err("Denied".into())).unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, cx| {
        assert!(app.composer.read(cx).value().contains(&id));
        assert_eq!(app.draft_context_records()[0].value(), &expected);
    });
}

#[gpui_kit::test]
fn r18_c_context_capture_undo_keeps_preexisting_typing_history(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    cx.simulate_input("Existing prompt");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let owner = app.environment_scope.clone();
            let thread = app.selected_thread.clone().unwrap();
            let record =
                DraftContextRecord::terminal("Build", "term", "Build", 1, 1, "output").unwrap();
            app.intake_draft_context(&owner, &thread, record, window, cx)
                .unwrap();
        })
    });
    // Trailing delimiter, chip, leading delimiter, then the original typing.
    cx.simulate_keystrokes("cmd-z cmd-z cmd-z");
    app.read_with(cx, |app, cx| {
        assert_eq!(app.composer.read(cx).value().as_str(), "Existing prompt");
        assert!(app.composer.read(cx).tokens().is_empty());
    });
    cx.simulate_keystrokes("cmd-z");
    app.read_with(cx, |app, cx| {
        assert!(app.composer.read(cx).value().is_empty())
    });
}

#[gpui_kit::test]
fn r18_c_plain_reference_paste_is_atomic_chip_and_preserves_undo(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let source = mentions::file_link("src/日本語.rs");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.provider_catalog = vec![super::composer_parity::provider_fixture(
                "codex", "Personal",
            )];
            app.composer
                .update(cx, |input, cx| input.replace("Review ", window, cx));
            cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(source.clone()));
        })
    });
    cx.simulate_keystrokes("cmd-v");
    cx.run_until_parked();
    app.read_with(cx, |app, cx| {
        assert_eq!(app.composer.read(cx).tokens().len(), 1);
        assert_eq!(
            app.composer.read(cx).value().as_str(),
            format!("Review {source}")
        );
    });
    cx.simulate_keystrokes("cmd-z");
    app.read_with(cx, |app, cx| {
        assert_eq!(app.composer.read(cx).value().as_str(), "Review ")
    });
    cx.simulate_keystrokes("cmd-shift-z enter");
    cx.run_until_parked();
    let connection::Command::Call { payload, reply, .. } = requests.try_recv().unwrap() else {
        panic!("turn")
    };
    assert_eq!(
        payload["message"]["context"]["records"][0]["path"],
        "src/日本語.rs"
    );
    reply.try_send(Err("Keep prompt".into())).unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn r18_c_structured_paste_undo_retains_surrounding_chip_and_source_payload(
    cx: &mut TestAppContext,
) {
    let (app, cx, _) = harness(cx);
    let record =
        DraftContextRecord::terminal("Copied", "term", "Build", 1, 1, "copied output").unwrap();
    let original = record.id().to_owned();
    let prefix = mentions::file_link("src/main.rs");
    cx.update(|window, cx| app.update(cx, |app, cx| {
        app.composer.update(cx, |input, cx| {
            input.set_value(crate::composer_state::prompt_content(&prefix), window, cx);
            input.set_selected_range(prefix.len()..prefix.len(), cx);
        });
        let clipboard = gpui_kit::ClipboardItem::new_string_with_json_metadata(record.reference(), json!({"version":1,"source":{"environmentId":"other","threadId":"other"},"records":[record.value()]}));
        assert!(app.paste_context_fragment(&clipboard, window, cx));
        assert_eq!(app.composer.read(cx).tokens().len(), 2);
        assert_ne!(app.draft_context_records()[0].id(), original);
    }));
    cx.simulate_keystrokes("cmd-z");
    app.read_with(cx, |app, cx| {
        assert_eq!(app.composer.read(cx).value().as_str(), prefix);
        assert_eq!(app.composer.read(cx).tokens().len(), 1);
        assert_eq!(
            app.draft_context_records()[0].value()["text"],
            "copied output"
        );
    });
    cx.simulate_keystrokes("cmd-shift-z");
    app.read_with(cx, |app, cx| {
        assert_eq!(app.composer.read(cx).tokens().len(), 2)
    });
}

#[gpui_kit::test]
fn r18_c_structured_paste_rejects_missing_and_duplicate_records_without_mutation(
    cx: &mut TestAppContext,
) {
    let (app, cx, _) = harness(cx);
    let record = DraftContextRecord::terminal("Copied", "term", "Build", 1, 1, "output").unwrap();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.composer
                .update(cx, |input, cx| input.set_value("untouched", window, cx));
            for records in [json!([]), json!([record.value(), record.value()])] {
                let clipboard = gpui_kit::ClipboardItem::new_string_with_json_metadata(
                    record.reference(),
                    json!({"version":1,"source":{"environmentId":"source"},"records":records}),
                );
                assert!(app.paste_context_fragment(&clipboard, window, cx));
                assert_eq!(app.composer.read(cx).value().as_str(), "untouched");
                assert!(app.draft_context_records().is_empty());
            }
        })
    });
}
