//! Cross-lane acceptance drives owned action receipts and actual shell entry points.
use super::*;

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
        panic!("expected {expected}")
    };
    assert_eq!(tag, expected);
    (payload, reply)
}

#[gpui_kit::test]
fn r14_int_preview_requires_accepted_write_and_current_owner(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let scope = "http://127.0.0.1:3773".to_owned();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.server
                .update(cx, |input, cx| input.set_value(scope.clone(), window, cx));
        })
    });
    for outcome in [
        "open-failure",
        "write-failure",
        "generation",
        "scope",
        "thread",
        "accepted",
    ] {
        cx.update(|_, cx| app.update(cx, |app, cx| {
            app.environment_scope = scope.clone();
            app.project_settings = Some(json!({"projectSettingsFolded":true,"defaultProjectScripts":[{"id":"preview","name":"Preview","command":"run preview","icon":"play","previewUrl":"http://localhost:4321/path?q=1#section","autoOpenPreview":true}]}));
            app.selected_thread = Some("t".into());
            app.run_project_action("preview", cx);
        }));
        cx.run_until_parked();
        assert_eq!(
            app.read_with(cx, |app, cx| app.browser_panel.read(cx).tabs.entries.len()),
            0
        );
        let (open, receipt) = call(&requests, "terminal.open");
        let key = open["terminalId"].as_str().unwrap();
        assert_eq!(open["threadId"], "t");
        if outcome == "open-failure" {
            receipt.send_blocking(Err("open denied".into())).unwrap();
            cx.run_until_parked();
            assert!(requests.is_empty());
            continue;
        }
        receipt.send_blocking(Ok(json!({"threadId":"t","terminalId":key,"status":"running","history":"","sequence":0}))).unwrap();
        cx.run_until_parked();
        let connection::Command::Subscribe { tag, payload, .. } = requests.try_recv().unwrap()
        else {
            panic!("attach")
        };
        assert_eq!(tag, "terminal.attach");
        assert_eq!(
            payload,
            json!({"threadId":"t","terminalId":key,"restartIfNotRunning":false})
        );
        let (write, receipt) = call(&requests, "terminal.write");
        assert_eq!(
            write,
            json!({"threadId":"t","terminalId":key,"data":"run preview\r"})
        );
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                match outcome {
                    "generation" => app.generation += 1,
                    "scope" => app.environment_scope = "http://other.test".into(),
                    "thread" => app.selected_thread = Some("other".into()),
                    _ => {}
                }
                cx.notify();
            })
        });
        receipt
            .send_blocking(if outcome == "write-failure" {
                Err("write denied".into())
            } else {
                Ok(Value::Null)
            })
            .unwrap();
        cx.run_until_parked();
        let count = app.read_with(cx, |app, cx| app.browser_panel.read(cx).tabs.entries.len());
        assert_eq!(
            count,
            usize::from(outcome == "accepted"),
            "{outcome}: {:?}",
            app.read_with(cx, |app, cx| (
                app.browser_panel.read(cx).error.clone(),
                app.selected_project.clone(),
                app.selected_thread.clone(),
                app.environment_scope.clone()
            ))
        );
        if outcome == "accepted" {
            assert!(cx.debug_bounds("browser-tab-1").is_some());
            let inspector = cx.debug_bounds("right-tab-inspector").unwrap();
            cx.simulate_click(inspector.center(), Default::default());
            cx.run_until_parked();
            assert!(!app.read_with(cx, |app, cx| app.browser_panel.read(cx).browser_active()));
            assert!(cx.debug_bounds("section-project").is_some());
        }
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.action_runs.clear();
                app.selected_thread = Some("t".into());
                cx.notify();
            })
        });
    }
}

#[gpui_kit::test]
fn r14_int_pr_tab_and_browser_reverse_are_reachable(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    let add = cx
        .debug_bounds("browser-add")
        .expect("browser reachable in shell");
    cx.simulate_click(add.center(), Default::default());
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, cx| app.browser_panel.read(cx).browser_active()));
    assert!(cx.debug_bounds("section-project").is_none());
    let prs = cx.debug_bounds("right-tab-pull-requests").unwrap();
    cx.simulate_click(prs.center(), Default::default());
    cx.run_until_parked();
    assert!(!app.read_with(cx, |app, cx| app.browser_panel.read(cx).browser_active()));
    assert!(cx.debug_bounds("pull-requests-panel").is_some());
    let (_, receipt) = call(&requests, "pullRequests.list");
    receipt
        .send_blocking(Err("provider unavailable".into()))
        .unwrap();
    cx.run_until_parked();
    let add = cx
        .debug_bounds("browser-add")
        .expect("browser also reachable from PRs");
    cx.simulate_click(add.center(), Default::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("pull-requests-panel").is_none());
    let changes = cx.debug_bounds("right-tab-changes").unwrap();
    cx.simulate_click(changes.center(), Default::default());
    cx.run_until_parked();
    assert_eq!(app.read_with(cx, |app, _| app.right_tab), RightTab::Changes);
    assert!(!app.read_with(cx, |app, cx| app.browser_panel.read(cx).browser_active()));
    let browser = cx.debug_bounds("browser-tab-1").unwrap();
    cx.simulate_click(browser.center(), Default::default());
    cx.run_until_parked();
    cx.update(|_, cx| app.update(cx, |app, cx| app.view_git_diff("working-tree", cx)));
    cx.run_until_parked();
    assert!(!app.read_with(cx, |app, cx| app.browser_panel.read(cx).browser_active()));
    let (_, receipt) = call(&requests, "review.getDiffPreview");
    receipt
        .send_blocking(Err("diff unavailable".into()))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(app.read_with(cx, |app, _| app.right_tab), RightTab::Changes);
}
