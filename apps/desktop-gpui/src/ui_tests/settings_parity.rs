//! Real palette/input dispatch and host RPC receipts, without live-device state.
use super::*;
use gpui_kit::component::{ActiveTheme, Theme};

fn call(
    requests: &Requests,
    expected: &str,
) -> (Value, async_channel::Sender<Result<Value, String>>) {
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("expected {expected}");
    };
    assert_eq!(tag, expected);
    (payload, reply)
}

#[gpui_kit::test]
fn palette_theme_and_zoom_keys_change_live_ui_and_reset(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.simulate_keystrokes("cmd-k");
    cx.simulate_input("Theme: Light");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    cx.update(|_, cx| {
        assert_eq!(Theme::global(cx).theme_name().as_ref(), "One Light");
        assert!(!cx.theme().is_dark());
    });
    assert!(!app.read_with(cx, |app, _| app.palette_open));
    cx.simulate_keystrokes("cmd-=");
    cx.run_until_parked();
    cx.update(|_, cx| {
        assert_eq!(settings_appearance::current(cx).zoom, 1);
        assert!(cx.theme().font_size > px(14.));
    });
    cx.simulate_keystrokes("cmd--");
    cx.simulate_keystrokes("cmd-=");
    cx.simulate_keystrokes("cmd-0");
    cx.run_until_parked();
    cx.update(|_, cx| {
        assert_eq!(settings_appearance::current(cx).zoom, 0);
        assert_eq!(cx.theme().font_size, px(14.));
    });
    assert!(requests.is_empty());
}

#[gpui_kit::test]
fn palette_search_selects_thread_and_targets_new_thread_project(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| {
        app.shell.projects.push(serde_json::from_value(json!({"id":"p2","title":"Second repo","workspaceRoot":"/tmp/second","defaultModelSelection":{"instanceId":"codex","model":"gpt-5.4"}})).unwrap());
        app.shell.threads.push(serde_json::from_value(json!({"id":"t2","projectId":"p2","title":"Recent task","updatedAt":"2026-01-02"})).unwrap());
        cx.notify();
    }));
    cx.simulate_keystrokes("cmd-k");
    cx.simulate_input("Recent task");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.selected_thread.clone())
            .as_deref(),
        Some("t2")
    );
    cx.simulate_keystrokes("cmd-k");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.provider
                .update(cx, |input, cx| input.set_value("", window, cx));
            app.model
                .update(cx, |input, cx| input.set_value("", window, cx));
        })
    });
    cx.simulate_input("New thread in Second");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(app.selected_project.as_deref(), Some("p2"));
        assert!(!app.pending_threads.is_empty(), "creation: {} settings={:?}", app.status, app.project_settings);
        assert_eq!(app.pending_threads.last().unwrap().thread.project_id, "p2");
    });
}

#[gpui_kit::test]
fn host_directory_browser_uses_reply_path_and_never_adds_on_open(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|window, cx| app.update(cx, |app, cx| app.open_add_project(window, cx)));
    cx.run_until_parked();
    let (browse, receipt) = call(&requests, "filesystem.browse");
    assert!(browse["partialPath"].as_str().is_some());
    assert!(requests.is_empty());
    receipt
        .send_blocking(Ok(json!({"parentPath":"/remote/repo","entries":[]})))
        .unwrap();
    cx.run_until_parked();
    let folder = cx.debug_bounds("project-use-folder").unwrap().center();
    cx.simulate_click(folder, gpui_kit::Modifiers::none());
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, cx| app.workspace.read(cx).value())
            .as_ref(),
        "/remote/repo"
    );
    assert!(requests.is_empty());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.title
                .update(cx, |input, cx| input.set_value("Remote repo", window, cx));
            app.create_project_from_fields(cx);
        })
    });
    cx.run_until_parked();
    let (create, receipt) = call(&requests, "orchestration.dispatchCommand");
    assert_eq!(create["workspaceRoot"], "/remote/repo");
    receipt.send_blocking(Ok(Value::Null)).unwrap();
}

#[gpui_kit::test]
fn checked_in_action_import_preserves_icon_preview_and_authoritative_settings(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    cx.simulate_resize(size(px(1600.), px(1600.)));
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.inspector_open = true;
            app.inspector_section = Some(InspectorSection::ProjectActions);
            app.right_tab = RightTab::Inspector;
            app.project_settings = Some(json!({"defaultProjectScripts":[]}));
            cx.notify();
        })
    });
    cx.run_until_parked();
    let read = cx.debug_bounds("actions-read-t3-json").unwrap().center();
    cx.simulate_click(read, gpui_kit::Modifiers::none());
    cx.run_until_parked();
    let (payload, receipt) = call(&requests, "projects.readFile");
    assert_eq!(
        payload,
        json!({"cwd":"/tmp/fixture","relativePath":"t3.json"})
    );
    receipt.send_blocking(Ok(json!({"contents":r#"{"scripts":[{"name":"Build","command":"vp build","icon":"build","runOnWorktreeCreate":true,"async":false,"previewUrl":"http://localhost:3000","autoOpenPreview":true}]}"#, "truncated":false}))).unwrap();
    cx.run_until_parked();
    let import = cx.debug_bounds("action-import-0").unwrap().center();
    cx.simulate_click(import, gpui_kit::Modifiers::none());
    cx.run_until_parked();
    let (_, settings) = call(&requests, "server.getSettings");
    settings.send_blocking(Ok(json!({"projectSettingsOverrides":{"p":{"defaultModelSelection":{"instanceId":"other","model":"saved"},"defaultProjectScripts":[]}}}))).unwrap();
    cx.run_until_parked();
    let (write, receipt) = call(&requests, "server.updateSettings");
    let project = &write["patch"]["projectSettingsOverrides"]["p"];
    assert_eq!(project["defaultModelSelection"]["model"], "saved");
    let script = &project["defaultProjectScripts"][0];
    assert_eq!(script["icon"], "build");
    assert_eq!(script["async"], false);
    assert_eq!(script["previewUrl"], "http://localhost:3000");
    assert_eq!(script["autoOpenPreview"], true);
    receipt
        .send_blocking(Ok(json!({"projectSettingsOverrides":{"p":project}})))
        .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn provider_settings_fetch_descriptors_then_only_offer_needed_auth(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.simulate_keystrokes("cmd-,");
    cx.run_until_parked();
    let (_, receipt) = call(&requests, "server.getConfig");
    receipt.send_blocking(Ok(json!({"providers":[
        {"instanceId":"login","driver":"custom","enabled":true,"installed":true,"status":"error","auth":{"status":"unauthenticated"},"setup":{"canAuthenticate":true},"version":"1.2","models":[]},
        {"instanceId":"ready","driver":"custom","enabled":true,"installed":true,"status":"ready","auth":{"status":"authenticated"},"setup":{"canAuthenticate":true},"models":[]},
        {"instanceId":"unsupported","driver":"custom","enabled":true,"installed":true,"status":"error","auth":{"status":"unauthenticated"},"models":[]}
    ]}))).unwrap();
    cx.run_until_parked();
    let (_, settings) = call(&requests, "server.getSettings");
    settings
        .send_blocking(Ok(json!({"providerInstances":{}})))
        .unwrap();
    cx.run_until_parked();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.settings_query
                .update(cx, |input, cx| input.set_value("providers", window, cx))
        })
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("provider-signin-ready").is_none());
    assert!(cx.debug_bounds("provider-signin-unsupported").is_none());
    let login = cx.debug_bounds("provider-signin-login").unwrap().center();
    cx.simulate_click(login, gpui_kit::Modifiers::none());
    cx.run_until_parked();
    let (auth, receipt) = call(&requests, "provider.auth.start");
    assert_eq!(auth["instanceId"], "login");
    receipt
        .send_blocking(Err("Authentication unavailable for this test".into()))
        .unwrap();
}
