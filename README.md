# Life HUD

A calm, native desktop HUD for Windows 11. Weather, time, tasks, notes, news and music live in a slim
strip across the top of your screen, or in a sidebar, and your other windows make room for it.

Built in Rust with [Slint](https://slint.dev). Single 12 MB executable, about 9 MB of private memory, starts in
under half a second.

## Features

- **Two layouts.** A Top HUD with the clock centred on the screen, or a Sidebar. Switching between them
  plays a short animation where the bar folds from one edge to the other.
- **Reserves real screen space.** It registers as a shell AppBar, so maximized windows fill the rest of
  the desktop. Optionally, open windows are refitted proportionally too: a window snapped to the left half
  stays the left half of what's left.
- **Steps aside for fullscreen.** Games, video and presentations get the whole screen.
- **Clock.** Lock-screen style. Follows your regional date format and 12/24-hour setting.
- **Weather.** From [Open-Meteo](https://open-meteo.com), with no account needed. Uses Windows location or a city you
  choose. Shows an hourly strip and a multi-day forecast with temperature range bars. Works offline from cache.
- **Tasks and notes.** One card shows a daily checklist next to a scratchpad. Unfinished tasks carry over;
  tasks finished on an earlier day are cleared at midnight. Everything saves automatically.
- **News.** Two tabs. *Curated* leans towards technology and science (Bloomberg Technology, The Verge,
  Ars Technica, Nature, NASA, …). *Top* has general headlines (Google News for your region, BBC, CBC,
  NYT, …). Headlines come from free public feeds and open in your browser.
- **Music.** Spotify (or any player that publishes to Windows media controls) with artwork, a timeline and
  controls. No sign-in needed.
- **Themes.**
  - *Signature* themes are pure colour: Classic (follows Windows), Graphite, Midnight, Starlight, Titanium
    and Rosé.
  - *Mood* themes add a subtle embossed pattern of motifs to the cards: Hearth, Atelier, Metropolis,
    Evergreen, Neon and Nebula.

## Install

### Build from source

Requirements: Windows 10 2004 or later (Windows 11 recommended), [Rust](https://rustup.rs) (stable, MSVC
toolchain) and the Visual Studio C++ Build Tools.

```powershell
git clone <this repo>
cd LifeHUD
powershell -ExecutionPolicy Bypass -File tools\install.ps1
```

`install.ps1` builds a release binary, copies it to `%LOCALAPPDATA%\Programs\LifeHUD`, and adds Desktop
and Start menu shortcuts. To develop, run `cargo run` instead.

### Use

- **Tray icon.** Left-click opens Settings, or brings back a hidden HUD. Right-click to show or hide the
  HUD, switch layout, or quit.
- **Settings.** Choose layout, size, display, theme, weather location and units, news sources, and the
  sidebar card order. You can also turn on *Open at login*.
- **Command line.** `lifehud.exe --exit` closes a running copy cleanly and gives the screen space back.
  Launching the app a second time opens Settings.

## Privacy

Everything you write stays on your PC, in `%LOCALAPPDATA%\LifeHUD`. The app has no accounts, no telemetry
and no analytics. It only makes these network requests:

| What | Where |
|---|---|
| Weather and city search | `api.open-meteo.com`, `geocoding-api.open-meteo.com` |
| Headlines | The feed URLs listed in `src/services/news.rs` |
| Music | Nothing. Read locally from Windows media controls. |
| Location | Windows Location Services, only if no city is set. Coordinates go only to Open-Meteo. |

## Architecture

```
ui/                 Slint markup
  theme.slint       Design tokens (every colour is set from Rust)
  hud.slint         The docked window and the two layouts
  widgets/          Clock, weather, today (tasks + notes), news, music, chrome
  settings.slint    macOS-style settings window (Cupertino widget style)
src/
  app.rs            Controller: state, wiring, docking and layout animation
  platform/         All Win32/WinRT access: AppBar, acrylic, monitors, window reflow, tray, single instance
  services/         Background threads: weather, news (RSS/Atom), media (SMTC), HTTPS (SChannel)
  theme.rs          Palettes; every token is derived from a handful of colours
  tasks.rs          Daily task list (unit-tested)
  settings.rs, storage.rs, locale.rs
```

Design notes:

- **Unsafe code is confined to `src/platform`.** Each block is a thin, commented wrapper around a Windows
  API.
- **The UI thread never blocks on I/O.** Services run on their own threads and post results back with
  `slint::invoke_from_event_loop`.
- **Rendering uses Slint's software renderer.** For a mostly static HUD it uses about 90% less memory than
  GPU rendering, at the same visual quality. See [docs/REVIEW.md](docs/REVIEW.md).
- **Adding a theme** is one entry in `src/theme.rs`.
- **Adding a news source** is one line in `src/services/news.rs`.

## Development

```powershell
cargo test                      # unit tests: parsing, tasks, settings, layout maths
cargo clippy --all-targets      # should be warning-free
cargo fmt
python tools\gen_weather_icon.py   # regenerates ui/weather-icon.slint
```

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

MIT. See [LICENSE](LICENSE).

The UI is built with [Slint](https://slint.dev), used under its Royalty-free Desktop license, which requires
attribution. The About page in Settings shows the "Made with Slint" badge. Please keep it if you
redistribute builds. News headlines and links belong to their publishers.
