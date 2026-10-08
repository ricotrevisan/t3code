use super::*;

#[gpui_kit::test]
fn r13_g_environment_picker_uses_saved_identity_and_preserves_dirty_checkout(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.server.update(cx, |input, cx| {
                input.set_value("https://office.example", window, cx)
            });
            app.save_environment(cx);
            app.server.update(cx, |input, cx| {
                input.set_value("https://lab.example", window, cx)
            });
            app.save_environment(cx);
            app.environment_scope = "https://office.example".into();
            app.perform("server.getConfig", json!({}), None, cx);
        })
    });
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("config request");
    };
    assert_eq!(tag, "server.getConfig");
    assert_eq!(payload, json!({}));
    let config = json!({"environment":{"environmentId":"office-id","label":"Office Mac"},"providers":[],"settings":{}});
    reply.send_blocking(Ok(config.clone())).unwrap();
    cx.run_until_parked();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.remember_environment_identity(&config, cx);
            assert_eq!(app.environment_choices(cx)[0].1, "Office Mac");
            app.file_editor.loaded(
                "/tmp/fixture".into(),
                files::Read {
                    relative_path: "notes.txt".into(),
                    contents: "saved".into(),
                    truncated: false,
                },
            );
            app.file_contents
                .update(cx, |input, cx| input.set_value("unsaved", window, cx));
            cx.notify();
        })
    });
    cx.run_until_parked();
    let bar = cx.debug_bounds("status-bar").unwrap();
    let chip = cx.debug_bounds("environment-chip").unwrap();
    let message = cx.debug_bounds("status-message").unwrap();
    assert!(chip.left() - bar.left() < px(48.));
    assert!(chip.right() <= message.left());
    let generation = app.read_with(cx, |app, _| app.generation);
    cx.simulate_click(chip.center(), Default::default());
    cx.run_until_parked();
    let choice = cx
        .debug_bounds("environment-choice-1")
        .expect("actual endpoint switcher opened");
    cx.simulate_click(choice.center(), Default::default());
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(app.generation, generation);
        assert_eq!(app.environment_scope, "https://office.example");
        assert_eq!(app.selected_thread.as_deref(), Some("t"));
        assert!(app.status.contains("Unsaved file edits"));
    });
    assert!(
        requests.try_recv().is_err(),
        "blocked switch cannot alter transport"
    );
}
