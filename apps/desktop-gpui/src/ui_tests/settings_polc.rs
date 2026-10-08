//! Real-window polish regressions: controls retain live behavior and distinct scopes.
use super::*;

#[gpui_kit::test]
fn settings_polc_single_send_control_changes_composer_dispatch(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.settings_open = true;
            app.settings_query
                .update(cx, |input, cx| input.set_value("send", window, cx));
            cx.notify();
        })
    });
    cx.run_until_parked();
    let choice = cx.debug_bounds("send-shortcut-mod-enter").unwrap();
    cx.simulate_click(choice.center(), Default::default());
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.send_shortcut()),
        drafts::SendShortcut::ModEnter
    );
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.settings_open = false;
            app.composer.update(cx, |input, cx| {
                input.set_value("Keep this draft", window, cx);
                input.focus(window, cx);
            });
            cx.notify();
        })
    });
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, cx| {
        app.composer.read(cx).value().contains("Keep this draft")
    }));
    assert!(app.read_with(cx, |app, cx| app.composer.read(cx).value().contains('\n')));
    assert_eq!(app.read_with(cx, |app, _| app.pending_actions), 0);
}

#[gpui_kit::test]
fn settings_polc_confirmation_switch_roundtrip(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            theme::update(window, cx, |p| p.confirm_archive = false).unwrap();
            app.settings_open = true;
            app.settings_query
                .update(cx, |input, cx| input.set_value("confirmations", window, cx));
            cx.notify();
        })
    });
    cx.run_until_parked();
    for expected in [true, false] {
        let row = cx.debug_bounds("settings-confirmation-0").unwrap();
        // The leading track is the actual Kit switch, not an extra toggle handler.
        cx.simulate_click(
            point(row.left() + px(14.), row.center().y),
            Default::default(),
        );
        cx.run_until_parked();
        assert_eq!(
            app.read_with(cx, |app, cx| app
                .confirmation_required("thread.archive", cx)),
            expected
        );
    }
}

#[gpui_kit::test]
fn settings_polc_selected_project_add_opens_fresh_scoped_form_and_cancel_restores(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.reveal_inspector(InspectorSection::Project, window, cx);
            cx.notify();
        })
    });
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("add-project").is_none(),
        "selected-project editing must not submit another project"
    );
    assert!(
        cx.debug_bounds("project-browse").is_none(),
        "a selected project's root is not editable"
    );
    let add = cx.debug_bounds("project-start-add").unwrap();
    let remove = cx.debug_bounds("remove-project").unwrap();
    assert!(add.size.height <= px(28.));
    assert_eq!(add.center().y, remove.center().y);
    cx.simulate_click(add.center(), Default::default());
    cx.run_until_parked();
    let connection::Command::Call { tag, reply, .. } = requests.try_recv().unwrap() else {
        panic!("expected folder browse");
    };
    assert_eq!(tag, "filesystem.browse");
    assert!(requests.is_empty(), "opening Add must not create a project");
    app.read_with(cx, |app, cx| {
        assert!(app.directory_browser.adding);
        assert!(app.title.read(cx).value().is_empty());
        assert!(app.workspace.read(cx).value().is_empty());
        assert_eq!(app.selected_project.as_deref(), Some("p"));
    });
    reply
        .send_blocking(Ok(json!({"parentPath":"/remote","entries":[]})))
        .unwrap();
    cx.run_until_parked();
    let browse = cx.debug_bounds("project-browse").unwrap();
    assert!(
        browse.size.width < px(160.),
        "browse is an intrinsic-width leading control, not a full-width form button"
    );
    assert!(cx.debug_bounds("rename-project").is_none());
    let cancel = cx.debug_bounds("project-cancel-add").unwrap();
    cx.simulate_click(cancel.center(), Default::default());
    cx.run_until_parked();
    app.read_with(cx, |app, cx| {
        assert!(!app.directory_browser.adding);
        assert_eq!(app.title.read(cx).value().as_ref(), "Fixture");
        assert_eq!(app.workspace.read(cx).value().as_ref(), "/tmp/fixture");
    });
    assert!(cx.debug_bounds("rename-project").is_some());
    assert!(cx.debug_bounds("add-project").is_none());
    assert!(requests.is_empty());
}
