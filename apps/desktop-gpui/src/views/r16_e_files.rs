//! File-heading preparation happens once per preview receipt; only visible rows paint.
use crate::NativeApp;
use gpui_kit::component::{
    Selectable, Sizable,
    button::{Button, ButtonVariants},
};
use gpui_kit::{
    Context, Render, SharedString, WeakEntity, Window, div, prelude::*, rems, uniform_list,
};
use std::{cell::Cell, rc::Rc};
struct Heading {
    path: SharedString,
    label: SharedString,
}
pub(crate) struct FileList {
    app: WeakEntity<NativeApp>,
    rows: Rc<Vec<Heading>>,
    selected: usize,
    pub(crate) constructions: Rc<Cell<usize>>,
}
impl NativeApp {
    pub(crate) fn render_diff_file_list(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let app = cx.entity().downgrade();
        let entity = self
            .diffs
            .file_list
            .get_or_init(|| {
                cx.new(|_| FileList {
                    app,
                    rows: Rc::new(
                        self.diffs
                            .files
                            .iter()
                            .map(|file| Heading {
                                path: file.path.clone().into(),
                                label: file
                                    .stats
                                    .map_or_else(
                                        || file.path.clone(),
                                        |(added, removed)| {
                                            format!("{}  +{added} −{removed}", file.path)
                                        },
                                    )
                                    .into(),
                            })
                            .collect(),
                    ),
                    selected: self.diffs.selected_file,
                    constructions: Rc::new(Cell::new(0)),
                })
            })
            .clone();
        entity.update(cx, |list, cx| {
            if list.selected != self.diffs.selected_file {
                list.selected = self.diffs.selected_file;
                cx.notify();
            }
        });
        div()
            .id("diff-files")
            .h(rems(6.))
            .flex_shrink_0()
            .child(entity.cached(gpui_kit::StyleRefinement::default().w_full().h(rems(6.))))
    }
}
impl Render for FileList {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let rows = self.rows.clone();
        let app = self.app.clone();
        let selected = self.selected;
        let count = self.constructions.clone();
        uniform_list("diff-file-list", rows.len(), move |range, _, _| {
            count.set(count.get() + range.len());
            range
                .map(|index| {
                    let app = app.clone();
                    let row = &rows[index];
                    Button::new(("diff-file", index))
                        .debug_selector(move || format!("diff-file-{index}"))
                        .ghost()
                        .small()
                        .justify_start()
                        .label(row.label.clone())
                        .tooltip(row.path.clone())
                        .selected(selected == index)
                        .on_click(move |_, _, cx| {
                            let _ = app.update(cx, |app, cx| app.select_diff_file(index, cx));
                        })
                })
                .collect()
        })
        .h(rems(6.))
        .w_full()
    }
}
