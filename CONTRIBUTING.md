# Contributing

Thanks for helping make Life HUD better. A few principles keep it calm and small.

## Principles

- **Calm first.** The HUD is ambient. Prefer fewer elements, quieter colours and status that only appears
  when something is wrong.
- **Native and light.** Use Windows APIs before adding a crate. Measure memory and CPU before and after
  any change that touches rendering or polling, and include the numbers in your PR.
- **Both layouts are first-class.** A widget change should look deliberate in the Top HUD *and* the
  Sidebar.

## Workflow

1. `cargo fmt`, `cargo clippy --all-targets` (no warnings) and `cargo test` should all pass.
2. Keep unsafe code inside `src/platform`, with a short comment explaining why each block is sound.
3. Never block the UI thread. Do I/O on a service thread and post results back with
   `app::post` / `slint::invoke_from_event_loop`.
4. For UI changes, attach screenshots of both layouts in a light and a dark theme.
5. Give every icon-only button a `label` (`IconButton { label: "…" }`) so screen readers and UI
   Automation can find it.

## Rendering gotchas

Life HUD uses Slint's software renderer, which is far lighter than the GPU for a mostly static HUD. It
has a few rules worth knowing before you spend an hour on them:

- **`transform-scale` and `transform-rotation` are ignored.** Animating them costs frames and shows
  nothing. Use opacity, size, position or a shadow instead, or bake rotations into artwork (the
  sticker tiles do).
- **Anything that animates redraws its area on every frame.** Prefer animations that run once to ones
  that repeat on a timer, and don't ease values that change every second (a progress bar should step).
- **Invisible isn't free.** An element at opacity 0 is still laid out. Set `visible: false` to take it
  out of the frame (the HUD does this while it switches layout).
- **Popups are clipped by their window**, and the Top HUD is only ~100 px tall. For menus, use
  `context_menu::show` in `src/context_menu.rs`, which opens a small themed window at the cursor.

The layout switch logs one line per fold (`layout fold: N frames in M ms (F fps, worst gap G ms)`) in
`%LOCALAPPDATA%\LifeHUD\lifehud.log`, which is handy for checking a change didn't cost frames.

## Project layout

```
src/
  main.rs           Entry point: uninstall/install-on-first-run, single instance, then the app
  app.rs            Controller: state, UI wiring, docking, the layout fold and Edit Widgets
  platform/         All Win32/WinRT access (and all unsafe code): AppBar, acrylic, monitors,
                    window refit, tray, menu placement, credentials, install, browser titles
  services/         Background threads, one per data source, plus shared HTTPS (SChannel), image
                    decoding, per-app volume and the on-screen-only poller
  feeds/            UI-thread controllers for the feed widgets and their settings pages
  chat.rs           Search & AI panel
  context_menu.rs   Themed context menus
  volume.rs         Music volume slider glue
  settings.rs, storage.rs, tasks.rs, playlists.rs, theme.rs, locale.rs
  snapshot.rs       Debug-only offscreen renderer (see below)
ui/
  theme.slint       Design tokens (every colour is set from Rust)
  hud.slint         The docked window, both layouts and Edit Widgets
  widgets/          One file per widget
  feeds/            Shared pieces for the feed widgets and their settings pages
  edit.slint, drag.slint   Edit Widgets overlay and shared drag bookkeeping
  chat.slint, menu.slint   Search & AI panel and the themed menu window
  settings.slint    Settings window, built from settings-kit.slint
packaging/winget/   winget manifests (see docs/RELEASING.md)
tools/              Icon and sticker generators, installer script
```

To see a widget or Settings page without docking anything, debug builds can render offscreen:
`cargo run -- --snapshot <dir> <widget> [light]` or `cargo run -- --snapshot <dir> settings <page>...`.

## Common additions

- **A widget:** add its id and name to `WIDGETS` in `src/settings.rs`. Put its data source in
  `src/services/` (use `poller::spawn`, so it only fetches while it's on screen), its controller in
  `src/feeds/`, and its UI in `ui/widgets/`. Then give it a one-line blurb (`src/feeds/settings.rs`),
  an icon (`WidgetLook` in `ui/settings.slint`), and a settings page if it needs one. List
  any new host in the README's Privacy section.
- **A theme:** add a palette to `PALETTES` in `src/theme.rs`. Mood themes can also use a sticker set
  from `ui/stickers.slint`. To add or change a set, edit its motif SVGs in `ui/stickers/` and run
  `tools/gen_sticker_tiles.py` to rebuild the tiles.
- **A news source:** add a line to `CURATED` or `TOP` in `src/services/news.rs`. Check that it's a free,
  public RSS/Atom feed and that the parser tests still pass.
- **Weather icons:** edit and run `tools/gen_weather_icon.py`.

Releases, signing and winget are covered in [docs/RELEASING.md](docs/RELEASING.md).
