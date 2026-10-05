# desktop-gpui

Experimental native desktop client for this fork, written in Rust on
[GPUI](https://gpui.rs), the UI framework from Zed. It is a separate surface from
`apps/desktop` (Electron): it talks to the server over the same WebSocket
contracts and does not embed a browser. See `README.md` for build, test, and
parity commands.

## Build from GPUI Kit

This fork uses [GPUI Kit](https://gpui-kit.com) for its UI. Depend on `gpui-kit`
alone and import through it: `use gpui_kit::*;` for the framework and
`gpui_kit::component::*` for controls. Do not add `gpui`, `gpui-component`, or
`gpui-base` as direct dependencies.

Reach for a published component before writing a `div`: pick a `variant` or
`size`, and add a component only when its behavior is genuinely new. Read the
design guides at <https://gpui-kit.com/docs/design-guides> before laying out a
surface; they are normative here.

## Zed aesthetic

The visual target is Zed: restrained, precise, information-dense. These rules
are the enforcement points for that target.

- Resolve colors from the active theme (`cx.theme()`). No raw hex or `rgb()` at a
  call site. The app installs Zed's `One Dark` in `src/theme.rs`; add or remap a
  semantic role there instead of adding a literal to `render`.
- Use `Button` variants for hierarchy: at most one `primary` action per surface,
  `ghost` for secondary and navigation, `danger` for destructive. Mark the active
  item with `Selectable::selected(...)`, not an ad-hoc background.
- Keep radii on the theme scale and spacing on the 2/4/8/12/16/24 scale. Prefer
  type and spacing to color for hierarchy.

## Keep the UI testable

`src/ui_tests.rs` drives the real view through `debug_selector` ids and
`debug_bounds`. When you move or restyle a control that a test addresses, keep
its selector id and keep it reachable in the test window; a control that the test
cannot paint is a regression, not a flake.
