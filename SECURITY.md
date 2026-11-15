# Security policy

## Supported versions

Only the latest release receives fixes.

## Reporting a vulnerability

Please **don't open a public issue** for security problems. Instead, report them privately through
[GitHub's private vulnerability reporting](https://github.com/George-API/LifeHUD/security/advisories/new).

Include what you found, how to reproduce it, and which version you tested. You'll get an
acknowledgement within a week. Once a fix is released, you'll be credited in the changelog (unless you'd
rather not be).

## Scope

Life HUD has no accounts and no server. These are the areas most worth a look:

- **Untrusted network data.** Feeds and APIs it parses: RSS/Atom news and Reddit, YouTube pages and
  feeds, ESPN, Netflix's Top 10, Yahoo Finance and Open-Meteo (`src/services/`). Responses are size-capped;
  images are fetched over HTTPS from each service's own hosts and decoded by Windows
  (`src/services/images.rs`).
- **Anything handed to the Windows shell.** Only `http(s)://` links, the repo's own GitHub pages, and
  validated `spotify:` URIs from the saved playlist list should ever be opened (`src/app.rs`,
  `src/feeds/mod.rs`, `src/platform/mod.rs`).
- **The installer.** First run copies the exe to `%LOCALAPPDATA%\Programs\LifeHUD`, writes a per-user
  Settings → Apps entry and Start menu shortcut, and relaunches through the shell; `--uninstall` reverses
  it (`src/platform/install.rs`). No admin rights are used.
- **Local data.** Files in `%LOCALAPPDATA%\LifeHUD`, and browser window titles, which are read only to
  tell a YouTube tab from a Netflix one and must never be stored or logged (`src/platform/browsers.rs`).
- **`unsafe` Win32/WinRT code**, all of it in `src/platform/`.
- **The local embed server** that hosts YouTube's official player for in-widget video
  (`src/services/embed.rs`). It must bind `127.0.0.1` only, on a random port; require the per-launch
  128-bit token in every path; reject any request whose `Host` isn't exactly `127.0.0.1:<port>`
  (which defeats DNS rebinding from web pages); serve only its one fixed page; accept only an
  11-character `[A-Za-z0-9_-]` video id; and cap request size and read time. Ways around any of
  these are in scope. So is the Edge window it docks over the widget (`src/platform/docked.rs`).
- **The optional AI chat** (`src/services/ai.rs`, `src/chat.rs`, `src/platform/credentials.rs`). API
  keys must stay in Windows Credential Manager and never reach `settings.json`, logs or error messages,
  and model output must only ever be displayed as text, never opened or run.
