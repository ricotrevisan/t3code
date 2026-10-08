use super::*;

#[test]
fn r14_e_wrapping_keeps_every_character_and_numbers_only_first_fragment() {
    let line = Line {
        text: "é🙂abcdef".into(),
        old: Some(7),
        new: Some(9),
        kind: ' ',
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
