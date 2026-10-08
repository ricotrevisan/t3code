use super::*;
use gpui_kit::Modifiers;

#[gpui_kit::test]
fn conversation_plan_followup_retains_drafts_and_requests_the_correct_mode(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let mut item = snapshot(vec![], "idle");
            item["snapshot"]["thread"]["proposedPlans"] =
                json!([{"id":"p","planMarkdown":"# The plan\n\nMake it work"}]);
            app.apply_thread_update("t", item, cx).unwrap();
            app.connection_phase = connection::Phase::Ready;
            app.shell.synchronized = true;
            app.provider_catalog = vec![serde_json::from_value(json!({"instanceId":"codex","driver":"codex","enabled":true,"installed":true,"status":"ready","models":[]})).unwrap()];
            app.provider.update(cx, |input, cx| input.set_value("codex", window, cx));
            app.composer
                .update(cx, |input, cx| input.set_value("Keep my draft", window, cx));
            cx.notify();
        })
    });
    cx.run_until_parked();
    let bounds = cx.debug_bounds("plan-implement").unwrap();
    cx.simulate_click(bounds.center(), Modifiers::none());
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
        "Keep my draft"
    );
    assert!(requests.try_recv().is_err());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.composer
                .update(cx, |input, cx| input.set_value("", window, cx));
            cx.notify();
        })
    });
    cx.run_until_parked();
    let bounds = cx.debug_bounds("plan-refine").unwrap();
    cx.simulate_click(bounds.center(), Modifiers::none());
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap_or_else(|error| {
        panic!(
            "mode request {error}: {:?}",
            app.read_with(cx, |app, cx| (
                app.status.clone(),
                app.connection_ready(),
                app.pending_actions,
                app.composer.read(cx).value().to_string()
            ))
        )
    })
    else {
        panic!("mode request")
    };
    assert_eq!(tag, "orchestration.dispatchCommand");
    assert_eq!(payload["type"], "thread.interaction-mode.set");
    assert_eq!(payload["interactionMode"], "plan");
    assert_eq!(payload["threadId"], "t");
    assert!(app.read_with(cx, |app, cx| {
        app.composer
            .read(cx)
            .value()
            .starts_with("Refine the following plan:")
    }));
    reply.send_blocking(Ok(json!({}))).unwrap();
    cx.run_until_parked();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.composer
                .update(cx, |input, cx| input.set_value("", window, cx));
            cx.notify();
        })
    });
    cx.run_until_parked();
    let bounds = cx.debug_bounds("plan-implement").unwrap();
    cx.simulate_click(bounds.center(), Modifiers::none());
    cx.run_until_parked();
    let connection::Command::Call { payload, reply, .. } = requests.try_recv().unwrap() else {
        panic!("implementation mode request")
    };
    assert_eq!(payload["interactionMode"], "default");
    assert!(app.read_with(cx, |app, cx| {
        app.composer
            .read(cx)
            .value()
            .starts_with("Implement the following plan:")
    }));
    reply.send_blocking(Ok(json!({}))).unwrap();
    cx.run_until_parked();
}
