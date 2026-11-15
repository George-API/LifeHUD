<div align="center">

<img src="assets/icon.png" width="80" alt="">

# Life HUD

Your day at a glance, docked to the edge of your screen.<br>
A heads-up display for Windows, written in Rust.

[![Rust](https://img.shields.io/badge/Rust-000000?logo=rust&logoColor=white)](https://www.rust-lang.org)
[![Release](https://img.shields.io/github/v/release/George-API/LifeHUD?color=2ea44f)](https://github.com/George-API/LifeHUD/releases/latest)
[![CI](https://github.com/George-API/LifeHUD/actions/workflows/ci.yml/badge.svg)](https://github.com/George-API/LifeHUD/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

<a href="https://github.com/George-API/LifeHUD/releases/latest/download/lifehud.exe"><img src="https://img.shields.io/badge/Download_for_Windows-free-2ea44f?style=for-the-badge" height="44" alt="Download Life HUD for Windows"></a>

<sub>Windows 10 or 11 · 18 MB · no admin needed</sub>

<br>
<br>

<img src="docs/images/sidebar.png" width="340" alt="Life HUD as a sidebar: the time, the weather, Spotify playing in the music card, and today's tasks and notes">

<br>
<br>

</div>

Clock, weather, music, tasks and more, in a slim bar across the top of your screen or a column down
the side. It reserves its space like the taskbar does, so windows resize around it instead of hiding
under it.

- **Native and light.** One 18 MB exe. No Electron, no runtime, ~0.5% of one core with music playing.
- **Private.** No account, no telemetry, no ads. Everything stays on your machine.
- **Yours.** Pick any four widgets and one of 22 themes, most in light and dark.

## Install

**[Download `lifehud.exe`](https://github.com/George-API/LifeHUD/releases/latest/download/lifehud.exe)** and open it. That's the whole install.

> If Windows says *"Windows protected your PC"*, click **More info → Run anyway**. The app isn't
> code-signed yet, so Windows asks once.

Prefer a terminal? This does the same in PowerShell:

```powershell
$f = "$env:TEMP\lifehud.exe"; iwr https://github.com/George-API/LifeHUD/releases/latest/download/lifehud.exe -OutFile $f; & $f
```

Life HUD installs itself for your user (no admin), docks to the top of your screen and opens Settings
so you can pick your city, widgets and theme.

- **Update:** download and open it again.
- **Uninstall:** *Settings → Apps → Life HUD*, like any other app.
- **Portable:** put an empty file named `portable` next to the exe and it runs without installing.

Needs Windows 10 (2004) or later. Releases are built by [CI](.github/workflows/release.yml) from this
repo and ship with a SHA-256.

## Widgets

| Widget | What it shows |
| --- | --- |
| Weather | Hourly and daily from [Open-Meteo](https://open-meteo.com), cached for offline |
| Music | Spotify or any app with Windows media controls: seek, volume, right-click for playlists |
| Tasks & Notes | A to-do list that rolls unfinished tasks over, plus a scratchpad |
| YouTube & Netflix | New uploads and the Top 10, played in the widget, plus what's playing in your browser |
| Reddit | r/popular and your subreddits |
| News, Sports, Stocks | Headlines, your teams' scores, a watchlist |
| Search | Google, or ask ChatGPT, Claude or Gemini with your own API key |

Long-press the HUD to rearrange widgets, or use *Settings → Widgets*. Themes are under the paintbrush.

<img src="docs/images/settings-widgets.png" width="720" alt="Settings, Widgets page">

## Privacy

- Data lives in `%LOCALAPPDATA%\LifeHUD`. Each widget talks only to its own source (Open-Meteo, the
  feeds in [`news.rs`](src/services/news.rs), ESPN, YouTube, Reddit, Netflix, Yahoo Finance). Sports,
  YouTube, Reddit, Netflix and Stocks only connect while they're on screen.
- Music and "now watching" come from Windows media controls. Browser window titles are checked in
  memory to tell YouTube from Netflix, never stored or logged.
- AI messages go only to the provider you picked, only when you hit send. Conversations aren't saved;
  keys live in Windows Credential Manager.

## Tips

- The tray icon opens Settings; its menu has show/hide, layout, search and widget editing.
- In edit mode, drag cards to reorder, drag one to the tray to remove it, or drop a widget onto a card
  to swap. Esc finishes, Ctrl+Z undoes.
- `lifehud.exe --exit` quits and gives the screen space back.
- A window that won't move aside belongs to an app running as admin; Windows doesn't allow it.
- Uninstall from *Settings → Apps*. Your data stays in `%LOCALAPPDATA%\LifeHUD` until you delete it.

## Building

Rust 1.92+ with the MSVC toolchain.

```
cargo run --release
```

Built on [Slint](https://slint.dev), with Win32 and WinRT called directly. Debug builds can render any
widget offscreen with `cargo run -- --snapshot <dir> <widget>`.

## Contributing

Each widget is a data source in `src/services/`, a controller in `src/feeds/` and a Slint file in
`ui/widgets/`. [CONTRIBUTING.md](CONTRIBUTING.md) walks through adding one. Widgets, themes and fixes
are all welcome.

For bugs, attach `%LOCALAPPDATA%\LifeHUD\lifehud.log`. Report security issues via
[SECURITY.md](SECURITY.md).

## License

MIT. The UI uses Slint under its royalty-free desktop license, which requires the "Made with Slint"
badge on the About page; keep it in your own builds.

Not affiliated with Spotify, YouTube, Reddit, Netflix, ESPN, Yahoo, OpenAI, Anthropic or Google.
