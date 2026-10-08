use super::composer_parity::{provider_fixture, provider_fixture_value};
use super::*;

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let bounds = cx.debug_bounds(selector).unwrap();
    cx.simulate_click(bounds.center(), gpui_kit::Modifiers::none());
    cx.run_until_parked();
}

#[gpui_kit::test]
fn r13_c_optional_slash_skills_keep_dollar_intake_and_structured_transport(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.draft_store = None;
            app.provider_catalog = vec![provider_fixture("codex", "Personal")];
            app.open_settings("send", window, cx);
        })
    });
    cx.run_until_parked();
    click(cx, "skills-in-slash-menu");
    click(cx, "settings-close");
    app.read_with(cx, |app, _| assert!(!app.show_skills_in_slash_menu()));
    cx.simulate_input("/");
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("workspace catalog");
    };
    assert_eq!(tag, "server.refreshProviders");
    assert_eq!(payload, json!({"instanceId":"codex", "cwd":"/tmp/fixture"}));
    let mut provider = provider_fixture_value("codex", "Personal");
    provider["workspaceSnapshots"] = json!([{"cwd":"/tmp/fixture","slashCommands":[{"name":"ui","description":"Native command"}],"skills":[{"name":"ui","path":"/tmp/fixture/ui.md","enabled":true}]}]);
    reply.try_send(Ok(json!({"providers":[provider]}))).unwrap();
    cx.run_until_parked();
    assert!(cx.debug_bounds("slash-/skill:ui").is_none());
    assert!(
        cx.debug_bounds("slash-/ui").is_some(),
        "Hidden skills must not suppress native commands"
    );
    cx.simulate_keystrokes("backspace");
    cx.simulate_input("$");
    cx.run_until_parked();
    assert!(cx.debug_bounds("slash-$ui").is_some());
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("skill send");
    };
    assert_eq!(tag, "orchestration.dispatchCommand");
    assert_eq!(payload["message"]["context"]["records"][0]["kind"], "skill");
    assert_eq!(payload["message"]["context"]["records"][0]["name"], "ui");
    reply.try_send(Err("Skill rejected".into())).unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, cx| {
        assert_eq!(app.composer.read(cx).value().as_str(), "$ui ")
    });
}

#[gpui_kit::test]
fn r13_c_usage_builtin_routes_without_sending_literal_command(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            app.provider_catalog = vec![provider_fixture("codex", "Personal")];
        })
    });
    cx.simulate_input("/usage-limits");
    cx.run_until_parked();
    let connection::Command::Call { tag, reply, .. } = requests.try_recv().unwrap() else {
        panic!("workspace refresh");
    };
    assert_eq!(tag, "server.refreshProviders");
    let mut provider = provider_fixture_value("codex", "Personal");
    provider["slashCommands"] =
        json!([{"name":"usage-limits", "description":"View subscription usage"}]);
    reply.try_send(Ok(json!({"providers":[provider]}))).unwrap();
    cx.run_until_parked();
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(requests.try_recv().is_err());
    app.read_with(cx, |app, cx| {
        assert!(!app.settings_open);
        assert!(app.composer.read(cx).value().is_empty());
    });
    assert!(cx.debug_bounds("usage-limits-panel").is_some());
    click(cx, "usage-close");
    assert!(cx.debug_bounds("usage-limits-panel").is_none());
}
