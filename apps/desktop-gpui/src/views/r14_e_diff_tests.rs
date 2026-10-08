use super::*;

#[test]
fn r16_e_syntax_uses_separate_before_after_buffers_and_safe_wrapped_utf8_ranges() {
    let patch = "@@ -1,3 +1,3 @@\n-fn before() {\n+fn after() {\n     let café = \"🙂\";\n }\n";
    let view = PatchView::with_path(
        patch,
        Options {
            split: true,
            columns: Some(8),
        },
        "src/main.rs",
    );
    assert!(view.syntax[0].text().to_string().contains("fn before()"));
    assert!(!view.syntax[0].text().to_string().contains("fn after()"));
    assert!(view.syntax[1].text().to_string().contains("fn after()"));
    let theme = gpui_kit::component::highlighter::HighlightTheme::default_dark();
    let mut colored = 0;
    for (a, b) in view.displayed.iter() {
        for line in [a, b].into_iter().flatten() {
            if let Some((side, start, prefix)) = line.syntax {
                let end = start + line.text.len() - prefix;
                for (range, style) in view.syntax[side].styles(&(start..end), &*theme) {
                    let local = range.start - start + prefix..range.end - start + prefix;
                    assert!(line.text.is_char_boundary(local.start));
                    assert!(line.text.is_char_boundary(local.end));
                    assert!(local.end <= line.text.len());
                    colored += usize::from(style.color.is_some());
                }
            }
        }
    }
    assert!(colored > 0, "actual syntax styles, not just diff colors");
    // Identical context text can be inside a removed multiline comment only on
    // the before side. It must not borrow the after buffer's parser state.
    let view = PatchView::with_path(
        "@@ -1,3 +1,3 @@\n-/*\n+fn after() {\n same\n-*/\n+}\n",
        Options {
            split: true,
            columns: None,
        },
        "a.rs",
    );
    let (old, new) = &view.displayed[2];
    assert_eq!(old.as_ref().unwrap().syntax.unwrap().0, 0);
    assert_eq!(new.as_ref().unwrap().syntax.unwrap().0, 1);
    let colors = |line: &Line| {
        let (side, start, prefix) = line.syntax.unwrap();
        view.syntax[side]
            .styles(&(start..start + line.text.len() - prefix), &*theme)
            .into_iter()
            .map(|(_, style)| style.color)
            .collect::<Vec<_>>()
    };
    assert_ne!(colors(old.as_ref().unwrap()), colors(new.as_ref().unwrap()));
}

#[test]
fn r14_e_wrapping_keeps_every_character_and_numbers_only_first_fragment() {
    let line = Line {
        text: "é🙂abcdef".into(),
        old: Some(7),
        new: Some(9),
        kind: ' ',
        syntax: None,
        old_syntax: None,
    };
    let rows = fragments(&line, Some(3));
    assert_eq!(
        rows.iter().map(|row| row.text.as_ref()).collect::<String>(),
        line.text.as_ref()
    );
    assert_eq!((rows[0].old, rows[0].new), (Some(7), Some(9)));
    assert!(
        rows[1..]
            .iter()
            .all(|row| row.old.is_none() && row.new.is_none())
    );
    assert_eq!(fragments(&line, None).len(), 1);
}
#[test]
fn r14_e_cached_split_reflow_preserves_unmatched_lines_and_reverse_layout() {
    let patch = "@@ -7,3 +9,2 @@\n-old1\n-old2\n+new\n same\n";
    let mut view = PatchView::new(
        patch,
        Options {
            split: true,
            columns: None,
        },
    );
    assert_eq!(view.displayed.len(), 4);
    assert_eq!(view.displayed[1].0.as_ref().unwrap().old, Some(7));
    assert_eq!(view.displayed[1].1.as_ref().unwrap().new, Some(9));
    assert_eq!(view.displayed[2].0.as_ref().unwrap().old, Some(8));
    assert!(view.displayed[2].1.is_none());
    view.options = Options {
        split: false,
        columns: Some(2),
    };
    view.rebuild();
    assert!(view.displayed.len() > view.lines.len());
    view.options = Options {
        split: false,
        columns: None,
    };
    view.rebuild();
    assert_eq!(
        view.displayed
            .iter()
            .map(|(line, _)| line.as_ref().unwrap().text.as_ref())
            .collect::<Vec<_>>(),
        patch.lines().collect::<Vec<_>>()
    );
}
