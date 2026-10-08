//! Server PTY screen model. No local subprocesses or clipboard escape handling.
use alacritty_terminal::{
    Term,
    event::{Event, EventListener},
    grid::{Dimensions, Scroll},
    index::{Column, Line, Point, Side},
    selection::{Selection, SelectionType},
    term::{Config, Osc52, TermMode},
    vte::ansi::{Color, Processor},
};
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub struct Replies(Arc<Mutex<Vec<String>>>);
impl EventListener for Replies {
    fn send_event(&self, event: Event) {
        if let Event::PtyWrite(text) = event {
            self.0.lock().unwrap().push(text);
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    pub cols: usize,
    pub rows: usize,
}
impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}
impl Size {
    pub fn new(cols: usize, rows: usize) -> Self {
        Self {
            cols: cols.clamp(2, 500),
            rows: rows.clamp(1, 200),
        }
    }
}
pub struct Screen {
    pub term: Term<Replies>,
    parser: Processor,
    replies: Replies,
    pub size: Size,
}
#[derive(Clone)]
pub struct Cell {
    pub col: usize,
    pub row: usize,
    pub text: String,
    pub fg: Color,
    pub bg: Color,
    pub flags: alacritty_terminal::term::cell::Flags,
    pub selected: bool,
    pub cursor: bool,
}
impl Screen {
    pub fn new(size: Size) -> Self {
        let replies = Replies::default();
        let term = Term::new(
            Config {
                scrolling_history: 10_000,
                osc52: Osc52::Disabled,
                ..Config::default()
            },
            &size,
            replies.clone(),
        );
        Self {
            term,
            parser: Processor::new(),
            replies,
            size,
        }
    }
    pub fn feed(&mut self, text: &str) {
        self.parser.advance(&mut self.term, text.as_bytes());
    }
    pub fn reset(&mut self, text: &str) {
        *self = Self::new(self.size);
        self.feed(text);
    }
    pub fn resize(&mut self, size: Size) {
        self.term.resize(size);
        self.size = size;
    }
    pub fn take_replies(&mut self) -> Vec<String> {
        std::mem::take(&mut *self.replies.0.lock().unwrap())
    }
    pub fn scroll(&mut self, lines: i32) {
        self.term.scroll_display(Scroll::Delta(lines));
    }
    pub fn select(&mut self, row: usize, col: usize, start: bool) {
        let point = Point::new(
            Line(row as i32 - self.term.grid().display_offset() as i32),
            Column(col.min(self.size.cols - 1)),
        );
        if start {
            self.term.selection = Some(Selection::new(SelectionType::Simple, point, Side::Left));
        } else if let Some(selection) = &mut self.term.selection {
            selection.update(point, Side::Right);
        }
    }
    pub fn selection(&self) -> Option<String> {
        self.term.selection_to_string()
    }
    /// One-based buffer coordinates (including scrollback), not viewport rows.
    pub fn selection_context(&self) -> Option<(usize, usize, String)> {
        let text = self.selection()?.replace("\r\n", "\n");
        let text = text.trim_matches('\n').to_owned();
        if text.is_empty() {
            return None;
        }
        let range = self.term.selection.as_ref()?.to_range(&self.term)?;
        let history = self.term.grid().history_size() as i32;
        let start = (range.start.line.0 + history + 1).max(1) as usize;
        let end = (range.end.line.0 + history + 1).max(start as i32) as usize;
        Some((start, end, text))
    }
    /// Literal buffer search. Navigate with a stable buffer coordinate rather than a
    /// viewport coordinate, so scrolling to an old match does not change the next result.
    pub fn search(
        &mut self,
        query: &str,
        after: Option<(i32, usize)>,
        reverse: bool,
    ) -> Option<(i32, usize)> {
        if query.is_empty() {
            return None;
        }
        let mut matches = Vec::new();
        let grid = self.term.grid();
        for row in -(grid.history_size() as i32)..self.size.rows as i32 {
            let mut text = String::new();
            let mut columns = Vec::new();
            for col in 0..self.size.cols {
                let cell = &grid[Line(row)][Column(col)];
                if cell
                    .flags
                    .contains(alacritty_terminal::term::cell::Flags::WIDE_CHAR_SPACER)
                {
                    continue;
                }
                columns.extend(std::iter::repeat_n(col, cell.c.len_utf8()));
                text.push(cell.c);
                if let Some(extra) = cell.zerowidth() {
                    for ch in extra {
                        columns.extend(std::iter::repeat_n(col, ch.len_utf8()));
                        text.push(*ch);
                    }
                }
            }
            for (byte, _) in text.match_indices(query) {
                matches.push(((row, columns[byte]), columns[byte + query.len() - 1]));
            }
        }
        let result = if reverse {
            matches
                .iter()
                .rev()
                .find(|(point, _)| after.is_none_or(|after| *point < after))
                .or(matches.last())
        } else {
            matches
                .iter()
                .find(|(point, _)| after.is_none_or(|after| *point > after))
                .or(matches.first())
        }
        .copied()?;
        let ((row, start), end) = result;
        let mut selection = Selection::new(
            SelectionType::Simple,
            Point::new(Line(row), Column(start)),
            Side::Left,
        );
        selection.update(Point::new(Line(row), Column(end)), Side::Right);
        self.term.selection = Some(selection);
        let offset = self.term.grid().display_offset() as i32;
        let desired = if row < -offset {
            -row
        } else if row >= self.size.rows as i32 - offset {
            (self.size.rows as i32 - 1 - row).max(0)
        } else {
            offset
        };
        self.scroll(desired - offset);
        Some((row, start))
    }
    pub fn clear_selection(&mut self) {
        self.term.selection = None;
    }
    pub fn paste(&self, text: &str) -> String {
        if self.term.mode().contains(TermMode::BRACKETED_PASTE) {
            format!("\x1b[200~{}\x1b[201~", text.replace('\x1b', ""))
        } else {
            text.replace("\r\n", "\n").replace('\n', "\r")
        }
    }
    pub fn cells(&self) -> Vec<Cell> {
        let content = self.term.renderable_content();
        let cursor = content.cursor.point;
        let visible_cursor =
            content.mode.contains(TermMode::SHOW_CURSOR) && content.display_offset == 0;
        content
            .display_iter
            .map(|indexed| {
                let cell = &indexed.cell;
                let mut text = cell.c.to_string();
                if let Some(extra) = cell.zerowidth() {
                    text.extend(extra);
                }
                Cell {
                    col: indexed.point.column.0,
                    row: (indexed.point.line.0 + content.display_offset as i32) as usize,
                    text,
                    fg: cell.fg,
                    bg: cell.bg,
                    flags: cell.flags,
                    selected: content
                        .selection
                        .as_ref()
                        .is_some_and(|range| range.contains(indexed.point)),
                    cursor: visible_cursor && indexed.point == cursor,
                }
            })
            .collect()
    }
    pub fn key(&self, key: &str, text: Option<&str>, control: bool, alt: bool) -> Option<String> {
        let value = if control && key.len() == 1 && key.as_bytes()[0].is_ascii_alphabetic() {
            char::from(key.as_bytes()[0].to_ascii_lowercase() - b'a' + 1).to_string()
        } else {
            let application = self.term.mode().contains(TermMode::APP_CURSOR);
            match key {
                "enter" => "\r".into(),
                "backspace" => "\x7f".into(),
                "tab" => "\t".into(),
                "escape" => "\x1b".into(),
                "up" => if application { "\x1bOA" } else { "\x1b[A" }.into(),
                "down" => if application { "\x1bOB" } else { "\x1b[B" }.into(),
                "right" => if application { "\x1bOC" } else { "\x1b[C" }.into(),
                "left" => if application { "\x1bOD" } else { "\x1b[D" }.into(),
                "home" => "\x1b[H".into(),
                "end" => "\x1b[F".into(),
                "delete" => "\x1b[3~".into(),
                "pageup" => "\x1b[5~".into(),
                "pagedown" => "\x1b[6~".into(),
                _ => text?.to_owned(),
            }
        };
        Some(if alt { format!("\x1b{value}") } else { value })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::vte::ansi::NamedColor;
    #[test]
    fn terminal_search_wraps_both_directions_and_selects_unicode_scrollback() {
        let mut screen = Screen::new(Size::new(20, 2));
        screen.feed("first 猫\r\nsecond\r\nlast 猫");
        let first = screen.search("猫", None, false).unwrap();
        assert!(first.0 < 0);
        assert_eq!(screen.selection().as_deref(), Some("猫"));
        assert_eq!(screen.selection_context().unwrap(), (1, 1, "猫".into()));
        let last = screen.search("猫", Some(first), false).unwrap();
        assert!(last > first);
        assert_eq!(screen.search("猫", Some(last), false), Some(first));
        assert_eq!(screen.search("猫", Some(first), true), Some(last));
        assert!(screen.search("missing", None, false).is_none());
    }
    #[test]
    fn terminal_colours_cursor_clear_and_chunked_parser() {
        let mut screen = Screen::new(Size::new(10, 3));
        screen.feed("\x1b[3");
        screen.feed("1mred\x1b[0m\x1b[2;4HX");
        assert_eq!(
            screen.term.grid()[Line(0)][Column(0)].fg,
            Color::Named(NamedColor::Red)
        );
        assert_eq!(screen.term.grid()[Line(1)][Column(3)].c, 'X');
        assert_eq!(
            screen.term.grid().cursor.point,
            Point::new(Line(1), Column(4))
        );
        screen.feed("\x1b[2J");
        assert!(screen.cells().iter().all(|cell| cell.text == " "));
    }
    #[test]
    fn terminal_alternate_screen_restores_primary_and_bounds_scrollback() {
        let mut screen = Screen::new(Size::new(10, 3));
        screen.feed("primary\x1b[?1049halt");
        assert!(screen.term.mode().contains(TermMode::ALT_SCREEN));
        screen.feed("\x1b[?1049l");
        assert_eq!(screen.term.grid()[Line(0)][Column(0)].c, 'p');
        screen.feed(&"line\r\n".repeat(10_100));
        assert_eq!(screen.term.grid().history_size(), 10_000);
    }
    #[test]
    fn terminal_selection_paste_application_cursor_and_device_reply() {
        let mut screen = Screen::new(Size::new(10, 3));
        screen.feed("hello");
        screen.select(0, 0, true);
        screen.select(0, 4, false);
        assert_eq!(screen.selection().as_deref(), Some("hello"));
        screen.feed("\x1b[?2004h\x1b[?1h\x1b[6n");
        assert_eq!(screen.paste("a\nb"), "\x1b[200~a\nb\x1b[201~");
        assert_eq!(
            screen.key("up", None, false, false).as_deref(),
            Some("\x1bOA")
        );
        assert_eq!(screen.take_replies(), vec!["\x1b[1;6R"]);
        assert_eq!(screen.key("c", None, true, false).as_deref(), Some("\x03"));
    }
}
