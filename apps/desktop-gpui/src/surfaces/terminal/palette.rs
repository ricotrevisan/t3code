//! Terminal palette roles resolve through the active Zed/Kit theme. Explicit ANSI
//! truecolor/indexed colours are process output, not application styling literals.
use alacritty_terminal::vte::ansi::{Color, NamedColor};
use gpui_kit::component::Theme;
use gpui_kit::{Hsla, rgba};

pub(super) fn resolve(color: Color, theme: &Theme) -> Hsla {
    match color {
        Color::Spec(rgb) => {
            rgba(((rgb.r as u32) << 24) | ((rgb.g as u32) << 16) | ((rgb.b as u32) << 8) | 255)
                .into()
        }
        Color::Named(named) => match named {
            NamedColor::Foreground | NamedColor::BrightForeground => theme.foreground,
            NamedColor::Background => theme.background,
            NamedColor::Cursor => theme.caret,
            _ => ansi(named as usize, theme),
        },
        Color::Indexed(index) if index < 16 => ansi(index as usize, theme),
        Color::Indexed(index) => {
            let (r, g, b) = if index >= 232 {
                let v = 8 + (index as u32 - 232) * 10;
                (v, v, v)
            } else {
                let i = index as u32 - 16;
                let level = |n| if n == 0 { 0 } else { 55 + n * 40 };
                (level(i / 36), level(i / 6 % 6), level(i % 6))
            };
            rgba((r << 24) | (g << 16) | (b << 8) | 255).into()
        }
    }
}
fn ansi(index: usize, theme: &Theme) -> Hsla {
    match index % 16 {
        0 => theme.background,
        1 => theme.red,
        2 => theme.green,
        3 => theme.yellow,
        4 => theme.blue,
        5 => theme.magenta,
        6 => theme.cyan,
        7 => theme.foreground,
        8 => theme.muted_foreground,
        9 => theme.red_light,
        10 => theme.green_light,
        11 => theme.yellow_light,
        12 => theme.blue_light,
        13 => theme.magenta_light,
        14 => theme.cyan_light,
        _ => theme.foreground,
    }
}
