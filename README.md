<div align="center">

<img src="assets/icon.png" width="80" alt="">

# Life HUD

A heads-up display for Windows, docked to the edge of your screen.<br>
Written in Rust.

[![Rust](https://img.shields.io/badge/Rust-000000?logo=rust&logoColor=white)](https://www.rust-lang.org)
[![Release](https://img.shields.io/github/v/release/George-API/LifeHUD?color=2ea44f)](https://github.com/George-API/LifeHUD/releases/latest)
[![CI](https://github.com/George-API/LifeHUD/actions/workflows/ci.yml/badge.svg)](https://github.com/George-API/LifeHUD/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

<br>

<img src="docs/images/sidebar.png" width="340" alt="Life HUD as a sidebar: the time, the weather, Spotify playing in the music card, today's tasks and notes, and stocks">

<br>
<br>

</div>

Life HUD puts a clock and four widgets of your choice in a slim bar across the top of the screen, or a
column down the side. It reserves that space the way the taskbar does, so windows resize around it
instead of sliding under it. Fullscreen apps still cover it.

It's a single native exe written in Rust: no Electron, no runtime, nothing to install alongside it. It
uses about half a percent of one CPU core with music playing. No account, no telemetry.

## Install

Download `lifehud.exe` from the [latest release](https://github.com/George-API/LifeHUD/releases/latest)
and run it. That's the install: it copies itself to `%LOCALAPPDATA%\Programs\LifeHUD`, adds itself to
the Start menu and to *Settings → Apps*, and starts. No admin, no wizard. Running a newer download
later updates it the same way.

Want it portable instead? Put an empty file named `portable` next to the exe.

Windows 10 (2004) or later. The exe isn't signed yet, so SmartScreen will ask the first time
(*More info → Run anyway*). Releases are built by [CI](.github/workflows/release.yml) from this repo and
come with a SHA-256.

## What's in it

- Clock and date, in your regional format
- Weather from [Open-Meteo](https://open-meteo.com): hourly and daily, cached for when you're offline
- Tasks and a scratchpad. Unfinished tasks roll over to the next day.
- Music: Spotify, or anything that uses Windows media controls, with seek and the player's own volume.
  Right-click it for your saved playlists.
- Reddit: r/popular and your subreddits, with a quick picker
- YouTube and Netflix: new uploads from your channels, Netflix's Top 10, and whatever you're watching in
  your browser right now
- News, sports scores for your teams, and a stock watchlist
- A search box: Enter goes to Google, or send the question to ChatGPT, Claude or Gemini with your own key

Pick any four widgets; the bar and the sidebar always show the same ones. Long-press the HUD to drag
them around, or use *Settings → Widgets*. Twelve themes, a hover away under the paintbrush.

<img src="docs/images/settings-widgets.png" width="720" alt="Settings, Widgets page">

## Privacy

Everything stays in `%LOCALAPPDATA%\LifeHUD`. Each widget talks only to its own source: Open-Meteo,
the news feeds in [`news.rs`](src/services/news.rs), ESPN, YouTube, Reddit, Netflix and Yahoo Finance.
Sports, YouTube, Reddit, Netflix and Stocks only connect while they're on screen.

Music and "now watching" are read locally from Windows media controls. To tell a YouTube tab from a
Netflix one, Life HUD compares the titles of open browser windows in memory; they're never stored or
logged.

The AI tabs send messages only when you hit send, and only to the provider you picked. Conversations
aren't saved, and API keys are kept in Windows Credential Manager.

## Usage

The tray icon opens Settings, and its menu has show/hide, layout, *Search & Ask…*, *Edit Widgets…* and
quit. Launching the exe again also opens Settings. `lifehud.exe --exit` quits and gives the screen space
back.

In edit mode, drag cards to reorder them, drag one onto the tray to remove it, or drop a widget from the
tray onto a card to swap it in. Esc or a click on empty space finishes; Ctrl+Z undoes.

If a window won't move out of the way, it belongs to an app running as admin; Windows doesn't allow it.
Uninstall from *Settings → Apps* like anything else. Your tasks, notes and settings stay in
`%LOCALAPPDATA%\LifeHUD` until you delete that folder.

## Building

Rust 1.92+ with the MSVC toolchain.

```
cargo run --release
```

Rust and [Slint](https://slint.dev), talking to Win32 and WinRT directly. See
[CONTRIBUTING.md](CONTRIBUTING.md) for the layout of the code, and [SECURITY.md](SECURITY.md) for
reporting vulnerabilities.

## License

MIT. The UI uses Slint under its royalty-free desktop license, which needs the "Made with Slint" badge
on the About page; keep it if you ship your own builds.

Not affiliated with Spotify, YouTube, Reddit, Netflix, ESPN, Yahoo, OpenAI, Anthropic or Google.
