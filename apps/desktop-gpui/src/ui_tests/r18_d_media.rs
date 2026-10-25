use super::*;
use gpui_kit::Modifiers;

#[gpui_kit::test]
fn r18_d_html_media_uses_owned_signing_denial_retry_and_never_loads_code_examples(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| {
        app.apply_thread_update("t", snapshot(vec![json!({"id":"a","role":"assistant","text":"<img src='charts/my plot.png' alt='chart' onerror='alert(1)'>\n\n```html\n<img src='literal.png'>\n```","streaming":false})], "idle"), cx).unwrap();
        cx.notify();
    }));
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("sign HTML image")
    };
    assert_eq!(tag, "assets.createUrl");
    let expected =
        json!({"resource":{"_tag":"media-file","threadId":"t","path":"charts/my plot.png"}});
    assert_eq!(payload, expected);
    reply.send_blocking(Err("asset denied".into())).unwrap();
    cx.run_until_parked();
    assert!(
        requests.try_recv().is_err(),
        "literal code must not request assets"
    );
    let bounds = cx
        .debug_bounds("workspace-image-retry")
        .expect("denied preview has retry");
    cx.simulate_click(bounds.center(), Modifiers::none());
    cx.run_until_parked();
    let connection::Command::Call {
        tag,
        payload,
        reply,
    } = requests.try_recv().unwrap()
    else {
        panic!("retry HTML image")
    };
    assert_eq!(tag, "assets.createUrl");
    assert_eq!(payload, expected);
    reply.send_blocking(Err("still denied".into())).unwrap();
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("workspace-image-open-charts/my plot.png")
            .is_some()
    );
    assert!(requests.try_recv().is_err());
}
