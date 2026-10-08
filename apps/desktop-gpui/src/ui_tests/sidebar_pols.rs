use super::*;

fn populated(app: &Entity<NativeApp>, cx: &mut VisualTestContext) {
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.sidebar_state.priority = false;
            app.sidebar_width = 288.;
            app.shell.threads[0].branch = Some("feature/first".into());
            app.shell.threads[0].active_order_key = Some("a".into());
            for index in 0..40 {
                app.shell.threads.push(
                    serde_json::from_value(json!({
                        "id": format!("pol-{index}"), "projectId":"p",
                        "title":format!("A long thread title {index}"),
                        "branch":format!("feature/long-branch-{index}"),
                        "activeOrderKey":format!("b{index:02}")
                    }))
                    .unwrap(),
                );
            }
            app.shell.revision += 1;
            cx.notify();
        })
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn sidebar_pols_search_has_usable_width_and_conditional_clear(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    populated(&app, cx);
    assert!(
        cx.debug_bounds("clear-search").is_none(),
        "empty search must not spend width on Clear"
    );
    let field = cx.debug_bounds("sidebar-search-field").unwrap();
    assert!(field.size.width >= px(128.), "search width: {field:?}");
    cx.simulate_click(field.center(), Default::default());
    cx.simulate_keystrokes("a b");
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, cx| app.search.read(cx).value().to_string()),
        "ab"
    );
    let clear = cx.debug_bounds("clear-search").unwrap();
    assert!(
        clear.size.width <= px(28.),
        "clear must be a compact x: {clear:?}"
    );
    let field = cx.debug_bounds("sidebar-search-field").unwrap();
    assert!(
        field.size.width >= px(128.),
        "non-empty search width: {field:?}"
    );
    cx.simulate_click(clear.center(), Default::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("clear-search").is_none());
    assert!(app.read_with(cx, |app, cx| app.search.read(cx).value().is_empty()));
}

#[gpui_kit::test]
fn sidebar_pols_header_remains_usable_at_sidebar_width_limits(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    populated(&app, cx);
    for width in [208., 288., 360., 480.] {
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.sidebar_width = width;
                app.search
                    .update(cx, |input, cx| input.set_value("query", window, cx));
                cx.notify();
            })
        });
        cx.run_until_parked();
        let sidebar = cx.debug_bounds("sidebar").unwrap();
        let field = cx.debug_bounds("sidebar-search-field").unwrap();
        let scope = cx.debug_bounds("sidebar-scope").unwrap();
        let new = cx.debug_bounds("sidebar-new-thread").unwrap();
        let minimum = if width == 208. { px(64.) } else { px(128.) };
        assert!(
            field.size.width >= minimum,
            "usable input at {width}: {field:?}"
        );
        assert!(
            scope.right() <= sidebar.right(),
            "scope clipped at {width}: {scope:?}"
        );
        assert!(
            new.right() <= sidebar.right(),
            "new action clipped at {width}: {new:?}"
        );
    }
}

#[gpui_kit::test]
fn sidebar_pols_scope_is_compact_in_view_row_and_can_reset(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    populated(&app, cx);
    let scope = cx.debug_bounds("sidebar-scope").unwrap();
    let priority = cx.debug_bounds("priority-view").unwrap();
    let project = cx.debug_bounds("project-view").unwrap();
    assert_eq!(scope.top(), priority.top(), "scope belongs in the view row");
    assert!(
        scope.size.width <= px(28.),
        "compact scope control: {scope:?}"
    );
    let sidebar = cx.debug_bounds("sidebar").unwrap();
    assert!(scope.left() >= project.right() && scope.right() <= sidebar.right());
    cx.simulate_click(scope.center(), Default::default());
    cx.run_until_parked();
    let choice = cx.debug_bounds("scope-p").unwrap();
    cx.simulate_click(choice.center(), Default::default());
    cx.run_until_parked();
    assert_eq!(
        app.read_with(cx, |app, _| app.sidebar_state.project_scope.clone()),
        Some("p".into())
    );
    let reset = cx.debug_bounds("reset-project-scope").unwrap();
    cx.simulate_click(reset.center(), Default::default());
    cx.run_until_parked();
    assert!(app.read_with(cx, |app, _| app.sidebar_state.project_scope.is_none()));
}

#[gpui_kit::test]
fn sidebar_pols_priority_rows_contain_both_lines_without_overlap(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    populated(&app, cx);
    let priority = cx.debug_bounds("priority-view").unwrap();
    cx.simulate_click(priority.center(), Default::default());
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.simulate_next_frame(cx);
    });
    cx.run_until_parked();
    for (first, second, title, meta) in [
        (
            "thread-t",
            "thread-pol-0",
            "sidebar-title-t",
            "sidebar-meta-t",
        ),
        (
            "thread-pol-0",
            "thread-pol-1",
            "sidebar-title-pol-0",
            "sidebar-meta-pol-0",
        ),
        (
            "thread-pol-1",
            "thread-pol-2",
            "sidebar-title-pol-1",
            "sidebar-meta-pol-1",
        ),
    ] {
        let row = cx.debug_bounds(first).unwrap();
        let next = cx.debug_bounds(second).unwrap();
        let title = cx.debug_bounds(title).unwrap();
        let meta = cx.debug_bounds(meta).unwrap();
        assert!(
            title.top() >= row.top() && meta.bottom() <= row.bottom(),
            "two-line content escapes its row: {row:?}, {title:?}, {meta:?}"
        );
        assert!(
            title.left() >= row.left() && title.right() <= row.right(),
            "title escapes row width"
        );
        assert!(
            meta.left() >= row.left() && meta.right() <= row.right(),
            "metadata escapes row width"
        );
        assert!(title.bottom() <= meta.top(), "title and metadata overlap");
        assert!(
            row.bottom() <= next.top(),
            "adjacent rows overlap: {row:?}, {next:?}"
        );
        assert!(meta.bottom() <= next.top(), "metadata overlaps next row");
    }
}

#[gpui_kit::test]
fn sidebar_pols_switch_to_priority_keeps_first_shelf_visible(cx: &mut TestAppContext) {
    let (app, cx, _) = harness(cx);
    populated(&app, cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.sidebar_state
                .scroll
                .set_offset(point(px(0.), px(-400.)));
            cx.notify();
        })
    });
    cx.run_until_parked();
    let priority = cx.debug_bounds("priority-view").unwrap();
    cx.simulate_click(priority.center(), Default::default());
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.simulate_next_frame(cx);
    });
    cx.run_until_parked();
    let viewport = cx.debug_bounds("sidebar-list").unwrap();
    let header = cx.debug_bounds("priority-pinned").unwrap();
    assert!(
        header.top() >= viewport.top() && header.bottom() <= viewport.bottom(),
        "first shelf hidden after switch: {header:?} vs {viewport:?}"
    );
    let row = cx.debug_bounds("thread-t").unwrap();
    assert!(row.top() >= viewport.top() && row.bottom() <= viewport.bottom());
}
