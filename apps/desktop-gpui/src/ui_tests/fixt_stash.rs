use super::*;

#[gpui_kit::test]
fn fixt_stash_shortcut_restores_the_single_saved_entry_when_composer_is_empty(
    cx: &mut TestAppContext,
) {
    let (app, cx, requests) = harness(cx);
    cx.simulate_input("Recover this 🌍");
    cx.simulate_keystrokes("cmd-s");
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, cx| app.composer.read(cx).value().is_empty()));
    assert_eq!(
        app.read_with(cx, |app, _| app.composer_stashes()[0].prompt.clone()),
        "Recover this 🌍"
    );
    assert!(composer_focused(&app, cx));
    cx.simulate_keystrokes("cmd-s");
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, cx| app.composer.read(cx).value().to_string()),
        "Recover this 🌍",
        "empty-composer stash shortcut must restore the sole saved entry"
    );
    assert!(app.read_with(cx, |app, _| app.composer_stashes().is_empty()));
    assert!(
        requests.try_recv().is_err(),
        "local stash must not dispatch remote mutations"
    );
}
