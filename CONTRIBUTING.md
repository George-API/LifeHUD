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

## Common additions

- **A theme:** add a palette to `PALETTES` in `src/theme.rs`. Mood themes can also use a sticker set
  from `ui/stickers.slint`.
- **A news source:** add a line to `CURATED` or `TOP` in `src/services/news.rs`. Check that it's a free,
  public RSS/Atom feed and that the parser tests still pass.
- **Weather icons:** edit and run `tools/gen_weather_icon.py`.
