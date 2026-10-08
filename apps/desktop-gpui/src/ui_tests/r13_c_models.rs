use super::composer_parity::provider_fixture;
use super::*;

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let bounds = cx.debug_bounds(selector).unwrap();
    cx.simulate_click(bounds.center(), gpui_kit::Modifiers::none());
    cx.run_until_parked();
}

#[gpui_kit::test]
fn r13_c_unavailable_model_setup_routes_to_settings_and_reports_refresh_failure(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            let mut provider = provider_fixture("codex", "Personal");
            provider.installed = false;
            app.provider_catalog = vec![provider];
            cx.notify();
        })
    });
    cx.run_until_parked();
    click(cx, "model-catalog");
    app.read_with(cx, |app, _| assert!(app.catalog_rows().is_empty()));
    click(cx, "catalog-setup-codex");
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("setup refresh");
    };
    assert_eq!(tag, "server.getConfig");
    assert_eq!(payload, json!({}));
    reply.try_send(Err("Catalog unavailable".into())).unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, cx| {
        assert!(app.settings_open);
        assert!(!app.catalog_open);
        assert_eq!(app.settings_query.read(cx).value().as_str(), "providers");
        assert!(!app.provider_catalog[0].selectable());
        assert!(app.settings_provider_busy.is_none());
    });
}

#[gpui_kit::test]
fn r13_c_model_favorite_hide_reverse_and_keyboard_send_use_exact_instance(cx: &mut TestAppContext) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.draft_store = None;
            app.provider_catalog = vec![
                provider_fixture("codex", "Personal"),
                provider_fixture("work", "Work"),
            ];
            cx.notify();
        })
    });
    cx.run_until_parked();
    click(cx, "model-catalog");
    cx.run_until_parked();
    click(cx, "catalog-favorite-work-0");
    click(cx, "catalog-favorites");
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(app.catalog_rows(), vec![("work".into(), "gpt-5.4".into())])
    });
    click(cx, "catalog-hide-work-0");
    cx.run_until_parked();
    app.read_with(cx, |app, _| assert!(app.catalog_rows().is_empty()));
    click(cx, "catalog-show-hidden");
    cx.run_until_parked();
    click(cx, "catalog-hide-work-0");
    click(cx, "catalog-show-hidden");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.composer_state
                .catalog_search
                .clone()
                .unwrap()
                .update(cx, |input, cx| input.focus(window, cx));
        })
    });
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    cx.simulate_input("Use work model");
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
    assert_eq!(payload["modelSelection"]["instanceId"], "work");
    assert_eq!(payload["modelSelection"]["model"], "gpt-5.4");
    reply.try_send(Err("Model unavailable now".into())).unwrap();
    cx.run_until_parked();
    app.read_with(cx, |app, cx| {
        assert_eq!(app.composer.read(cx).value().as_str(), "Use work model");
        assert!(app.model_preference("work", "gpt-5.4").unwrap().favorite);
        assert!(app.model_preference("codex", "gpt-5.4").is_none());
    });
    click(cx, "model-catalog");
    click(cx, "catalog-favorite-work-0");
    click(cx, "catalog-favorites");
    app.read_with(cx, |app, _| {
        assert!(app.catalog_rows().is_empty());
        assert!(app.model_preference("work", "gpt-5.4").is_none());
    });
}
