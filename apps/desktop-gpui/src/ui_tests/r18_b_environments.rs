use super::*;
const OFFICE: &str = "https://office.example";
const LAB: &str = "https://lab.example";

fn connect(
    app: &Entity<NativeApp>,
    cx: &mut VisualTestContext,
    id: &str,
    key: Option<&str>,
) -> async_channel::Sender<connection::Update> {
    let (commands, requests) = async_channel::bounded(64);
    let (updates, receiver) = async_channel::bounded(64);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.disconnect(cx);
            app.start_owned_session(
                connection::Session {
                    commands,
                    updates: receiver,
                },
                cx,
            );
        })
    });
    cx.run_until_parked();
    let requests = Requests::from(requests);
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("identity");
    };
    assert_eq!(tag, "server.getConfig");
    assert_eq!(payload, json!({}));
    let config =
        json!({"environment":{"environmentId":id,"label":id},"providers":[],"settings":{}});
    reply.try_send(Ok(config.clone())).unwrap();
    cx.run_until_parked();
    updates.try_send(connection::Update::Shell(Box::new(serde_json::from_value(json!({
        "kind":"snapshot", "snapshot":{"snapshotSequence":0,
        "projects":[{"id":"p","title":"Same title","workspaceRoot":"/tmp/fixture","repositoryIdentity":key.map(|key| json!({"canonicalKey":key}))}],
        "threads":[{"id":"t","projectId":"p","title":format!("{id} thread"),"archivedAt":null}]}
    })).unwrap()))).unwrap();
    updates
        .try_send(connection::Update::Shell(Box::new(
            shell::ShellItem::Synchronized,
        )))
        .unwrap();
    cx.run_until_parked();
    let connection::Command::Subscribe { tag, .. } = requests.try_recv().unwrap() else {
        panic!("config stream");
    };
    assert_eq!(tag, "subscribeServerConfig");
    let connection::Command::Call { tag, reply, .. } = requests.try_recv().unwrap() else {
        panic!("config refresh");
    };
    assert_eq!(tag, "server.getConfig");
    reply.try_send(Ok(config)).unwrap();
    cx.run_until_parked();
    updates
}

#[gpui_kit::test]
fn r18_b_global_canonical_key_cannot_alias_unidentified_project(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    let lab = cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.environment_scope = OFFICE.into();
            for origin in [OFFICE, LAB] {
                app.server
                    .update(cx, |input, cx| input.set_value(origin, window, cx));
                app.save_environment(cx);
            }
            app.server
                .update(cx, |input, cx| input.set_value(OFFICE, window, cx));
            app.create_environment_page(LAB, window, cx)
        })
    });
    // The canonical key deliberately equals the old origin:id fallback string.
    let office_updates = connect(&app, cx, "office-id", None);
    let lab_updates = connect(&lab, cx, "lab-id", Some("https://office.example:p"));
    let toggle = cx.debug_bounds("all-environments").unwrap();
    cx.simulate_click(toggle.center(), Default::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("environment-project-group-0").is_some());
    assert!(cx.debug_bounds("environment-project-group-1").is_some());
    assert!(
        cx.debug_bounds("environment-thread-https://office.example-t")
            .is_some()
    );
    assert!(
        cx.debug_bounds("environment-thread-https://lab.example-t")
            .is_some()
    );
    // A disconnected enabled page retains its own last known navigation rows.
    lab_updates
        .try_send(connection::Update::Phase(connection::Phase::Reconnecting {
            attempt: 1,
            retry_in: std::time::Duration::from_secs(1),
            reason: "fixture disconnected".into(),
        }))
        .unwrap();
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("environment-thread-https://lab.example-t")
            .is_some()
    );
    assert!(!office_updates.is_closed());
}
