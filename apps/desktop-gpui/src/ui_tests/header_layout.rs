//! Real painted header geometry and clicks with both side panels open.
use super::*;

const CHECKOUT: &str = "/tmp/header-selected-checkout";

fn seed_header(app: &Entity<NativeApp>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.sidebar_open = true;
            app.inspector_open = true;
            app.shell.projects[0].title =
                "A very long project name that must not consume the thread title".into();
            app.shell.threads[0].branch = Some(
                "feature/a-very-long-branch-name-that-previously-hid-the-entire-thread-title"
                    .into(),
            );
            app.shell.threads[0].worktree_path = Some(CHECKOUT.into());
            app.detail.thread = Some(json!({
                "id":"t", "title":"Keep the selected thread title readable with both panels open",
                "worktreePath":CHECKOUT, "session":{"status":"interrupted"},
                "messages":[], "activities":[], "proposedPlans":[],
                "checkpoints":[{"files":[{"path":"src/example.rs","additions":12,"deletions":3}]}]
            }).into());
            // Checkout actions must not target this independently editable destination.
            app.workspace.update(cx, |input, cx| {
                input.set_value("/tmp/wrong-clone-destination", window, cx)
            });
            app.available_editors = vec!["zed".into()];
            cx.notify();
        })
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn header_readable_lanes_do_not_overlap_with_both_panels(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    seed_header(&app, cx);
    for width in [960., 1280., 960.] {
        cx.simulate_resize(size(px(width), px(1000.)));
        cx.run_until_parked();
        assert!(app.read_with(cx, |app, _| app.sidebar_open && app.inspector_open));
        let header = cx.debug_bounds("thread-header").unwrap();
        let project = cx.debug_bounds("header-project").unwrap();
        let title = cx.debug_bounds("header-title").unwrap();
        let branch = cx.debug_bounds("branch-selector").unwrap();
        let status = cx.debug_bounds("header-session").unwrap();
        let diff = cx.debug_bounds("header-diff").unwrap();
        assert!(
            project.size.width >= px(80.),
            "readable project: {project:?}"
        );
        assert!(
            title.size.width >= px(160.),
            "readable title at {width}: {title:?}"
        );
        assert!(branch.size.width >= px(160.), "readable branch: {branch:?}");
        assert!(
            status.size.width >= px(70.),
            "Interrupted must not clip: {status:?}"
        );
        assert!(project.right() <= title.left());
        assert!(
            title.bottom() <= branch.top(),
            "identity and metadata overlap"
        );
        assert!(branch.right() <= status.left(), "branch and state overlap");
        assert!(branch.bottom() <= diff.top() && status.bottom() <= diff.top());

        let mut painted = vec![project, title, branch, status, diff];
        for selector in [
            "open-editor",
            "project-actions-header",
            "push-pr",
            "toggle-inspector",
            "thread-menu",
        ] {
            let action = cx.debug_bounds(selector).expect(selector);
            let minimum_width = if selector == "push-pr" { px(20.) } else { px(40.) };
            assert!(action.size.width >= minimum_width && action.size.height >= px(20.));
            assert!(
                action.top() >= diff.bottom(),
                "{selector} overlaps metadata"
            );
            painted.push(action);
        }
        for (index, bounds) in painted.iter().enumerate() {
            assert!(
                bounds.left() >= header.left() && bounds.right() <= header.right(),
                "outside header at {width}: {bounds:?}"
            );
            assert!(bounds.top() >= header.top() && bounds.bottom() <= header.bottom());
            for other in &painted[index + 1..] {
                assert!(
                    bounds.right() <= other.left()
                        || other.right() <= bounds.left()
                        || bounds.bottom() <= other.top()
                        || other.bottom() <= bounds.top(),
                    "overlap at {width}: {bounds:?} / {other:?}"
                );
            }
        }
    }
}

#[gpui_kit::test]
fn narrow_header_actions_reach_selected_checkout_and_visible_panels(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    seed_header(&app, cx);
    cx.simulate_resize(size(px(960.), px(1000.)));
    cx.run_until_parked();

    let reply = open_branch_picker_at(cx, &requests, "branch-selector", CHECKOUT);
    let chip = cx.debug_bounds("branch-selector").unwrap();
    let picker = cx
        .debug_bounds("branch-picker")
        .expect("actual branch picker opens");
    assert!(picker.top() >= chip.bottom());
    reply
        .send_blocking(Ok(json!({"isRepo":true,"nextCursor":null,"refs":[]})))
        .unwrap();
    cx.run_until_parked();
    let manage = cx.debug_bounds("branch-picker-manage").unwrap();
    cx.simulate_click(manage.center(), Default::default());
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.inspector_section),
        Some(InspectorSection::Worktrees)
    );
    assert!(cx.debug_bounds("draft-checkout-update").is_some());
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("worktree manager must load refs");
    };
    assert_eq!(tag, "vcs.listRefs");
    assert_eq!(payload["cwd"], CHECKOUT);
    reply
        .send_blocking(Ok(json!({"isRepo":true,"nextCursor":null,"refs":[]})))
        .unwrap();
    cx.run_until_parked();

    let open = cx.debug_bounds("open-editor").unwrap();
    cx.simulate_click(open.center(), Default::default());
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("expected editor RPC");
    };
    assert_eq!(tag, "shell.openInEditor");
    assert_eq!(payload, json!({"cwd":CHECKOUT,"editor":"zed"}));
    reply.send_blocking(Ok(json!({}))).unwrap();
    cx.run_until_parked();

    let push = cx.debug_bounds("push-pr").unwrap();
    cx.simulate_click(push.center(), Default::default());
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("expected git status before any mutation");
    };
    assert_eq!(tag, "vcs.refreshStatus");
    assert_eq!(payload["cwd"], CHECKOUT);
    reply.send_blocking(Ok(json!({"isRepo":true,"refName":"feature","hasPrimaryRemote":true,"hasUpstream":true,"hasWorkingTreeChanges":true,"aheadCount":0,"behindCount":0,"isDefaultRef":false,"workingTree":{"files":[{"path":"a.rs"}]}}))).unwrap();
    cx.run_until_parked();
    let quick = cx.debug_bounds("git-quick-action").unwrap();
    cx.simulate_click(quick.center(), Default::default());
    cx.run_until_parked();
    let submit = cx.debug_bounds("git-form-submit").unwrap();
    cx.simulate_click(submit.center(), Default::default());
    cx.run_until_parked();
    let connection::Command::CallStreamLive {
        tag,
        payload,
        events,
    } = requests.try_recv().unwrap()
    else {
        panic!("expected stacked git action");
    };
    assert_eq!(tag, "git.runStackedAction");
    assert_eq!(payload["cwd"], CHECKOUT);
    assert_eq!(payload["threadId"], "t");
    assert_eq!(payload["action"], "commit_push_pr");
    events.send_blocking(Ok(Some(json!({"kind":"action_finished","result":{"toast":{"title":"Done"}}})))).unwrap();
    events.send_blocking(Ok(None)).unwrap();
    cx.run_until_parked();

    let actions = cx.debug_bounds("project-actions-header").unwrap();
    cx.simulate_click(actions.center(), Default::default());
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.inspector_section),
        Some(InspectorSection::ProjectActions)
    );
    let connection::Command::Call { tag, reply, .. } = requests.try_recv().unwrap() else {
        panic!("project actions must load settings");
    };
    assert_eq!(tag, "server.getSettings");
    reply.send_blocking(Ok(json!({}))).unwrap();
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("action-save").is_some(),
        "action editor is painted"
    );

    for expected in [false, true] {
        let toggle = cx.debug_bounds("toggle-inspector").unwrap();
        cx.simulate_click(toggle.center(), Default::default());
        cx.run_until_parked();
        assert_eq!(app.read_with(cx, |app, _| app.inspector_open), expected);
        assert_eq!(cx.debug_bounds("action-save").is_some(), expected);
    }
    let more = cx.debug_bounds("thread-menu").unwrap();
    cx.simulate_click(more.center(), Default::default());
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.thread_menu_open));
    assert!(
        cx.debug_bounds("archive").is_some(),
        "actual thread commands appear"
    );
}
