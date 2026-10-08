use super::composer_parity::{provider_fixture, provider_fixture_value};
use super::*;

#[gpui_kit::test]
fn r14_c_fuzzy_skill_capture_refresh_send_rejection_retains_prompt(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            app.provider_catalog = vec![provider_fixture("codex", "Personal")];
        })
    });
    cx.simulate_input("$gfc");
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("refresh");
    };
    assert_eq!(tag, "server.refreshProviders");
    assert_eq!(payload, json!({"instanceId":"codex","cwd":"/tmp/fixture"}));
    let mut provider = provider_fixture_value("codex", "Personal");
    provider["workspaceSnapshots"] = json!([{"cwd":"/tmp/fixture","slashCommands":[],"skills":[{"name":"gh-fix-ci","path":"/tmp/fixture/ci.md","enabled":true}]}]);
    reply.try_send(Ok(json!({"providers":[provider]}))).unwrap();
    cx.run_until_parked();
    assert!(cx.debug_bounds("slash-$gh-fix-ci").is_some());
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
        panic!("send");
    };
    assert_eq!(tag, "orchestration.dispatchCommand");
    assert_eq!(
        payload["message"]["context"]["records"][0]["name"],
        "gh-fix-ci"
    );
    reply.try_send(Err("Rejected".into())).unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, cx| {
        assert_eq!(app.composer.read(cx).value().as_str(), "$gh-fix-ci ")
    });
}
