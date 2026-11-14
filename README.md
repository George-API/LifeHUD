# Life HUD

**Your desktop is where you work. Your Life HUD is where you stay connected to your day.**

A native Windows 11 desktop HUD that reserves a slim edge of the screen for weather, the time, today's
tasks, quick notes and Spotify. Maximized apps fill the rest of the desktop instead of hiding underneath it.

## Run it

Requirements: Windows 10 2004+ (Windows 11 recommended) and the .NET 10 SDK.

```powershell
dotnet run --project src/LifeHUD -c Release
# or produce a standalone build
dotnet publish src/LifeHUD -c Release -r win-x64 --self-contained false -o publish
```

* **Tray icon:** left-click opens Settings (or brings back a hidden HUD). Right-click gives Show/Hide,
  layout switching and Quit.
* `LifeHUD.exe --exit` asks a running instance to quit cleanly. Launching it a second time opens Settings.
* Data is stored locally in `%LOCALAPPDATA%\LifeHUD` (`settings.json`, `tasks.json`, `notes.md`, a weather cache and a log).

## The widgets

| Widget | Source | Notes |
|---|---|---|
| Date & Time | System clock | Ticks on the second boundary. 12/24h and optional seconds. Has a greeting in the sidebar. |
| Weather | [Open-Meteo](https://open-meteo.com) (no API key) | Uses Windows location, or a city you set. Refreshes every 15 min, and on reconnect and resume. Falls back to cached data with an "Offline · 10:05" label. |
| Daily Tasks | Local JSON | Enter to add. The check animation plays, then completed items settle below open ones. Items completed on earlier days clear at midnight; unfinished ones carry over. |
| Quick Notes | Local Markdown file | Auto-saves 600 ms after you stop typing, with atomic writes. Also flushed on exit and sign-out. |
| Spotify | Windows media session (SMTC) | Reads track, artwork, timeline and controls straight from the Spotify desktop app, with no sign-in or Web API. Falls back to any other player. A glow is tinted from the album art. |

## The two layouts

Both layouts are first-class. Each widget has two hand-built presentations; neither layout is a rotated
copy of the other.

**Top HUD** (default 10% of screen height). The clock sits at the exact centre of the screen. The two
halves are equal-width strips: weather and tasks on the left, notes and music on the right.
- ✅ Very quick to glance at. Your eyes are already near the top of the screen.
- ✅ Leaves the full screen width for work. On wide monitors this costs much less than a sidebar.
- ⚠️ Low vertical space. Tasks show about 2 rows and notes about 3 lines, so this layout is for capture and
  glancing, not long sessions. Weather shows a 3-day mini forecast and drops it on narrow screens.

**Life Sidebar** (default 14% of screen width). An editorial column: greeting and clock, full weather
(hourly strip plus a 6-day forecast with temperature range bars), music with a timeline, a full task list
and a real writing area for notes.
- ✅ Much richer. Tasks and notes become usable workspaces, and the forecast is useful for planning.
- ✅ Reads naturally top to bottom, like a phone widget stack.
- ⚠️ Takes width, which matters on 16:9 laptops. It suits ultrawide and 1440p+ screens best. On short
  screens it drops the hourly strip automatically.

**Recommendation:** use the Top HUD on laptops and single 16:9 displays. Use the Sidebar on ultrawide
monitors, or put it on a secondary monitor (Settings → Display).

## Architecture

```
src/LifeHUD
├── Interop/        AppBar (SHAppBarMessage), DWM/acrylic effects, monitor + DPI enumeration
├── Services/       settings (debounced, atomic JSON), theme, weather, media (SMTC), startup, tray
├── ViewModels/     one per widget + HUD + settings (plain MVVM, no framework)
├── Widgets/        WidgetView base + one view per widget, WidgetRegistry declares placement
├── Controls/       vector WeatherIcon, ProgressRing, RangeBar
└── Views/          HudWindow (the appbar host), SettingsWindow (WPF Fluent theme + Mica)
```

* **Workspace reservation.** The HUD registers as a shell AppBar (`ABM_NEW` / `ABM_QUERYPOS` / `ABM_SETPOS`),
  so Windows shrinks the work area around it. It reclaims its slot on `ABN_POSCHANGED`, re-registers when
  Explorer restarts (`TaskbarCreated`), and pins itself against Win+Arrow and DPI auto-resizes. Changing
  layout releases the old edge before claiming the new one. A crash handler releases the space so you are
  never left with a dead strip.
* **Fullscreen and gaming.** On `ABN_FULLSCREENAPP` the HUD drops out of topmost so games, videos and
  presentations get the whole screen. Fullscreen apps on *other* monitors are ignored.
* **DPI and multi-monitor.** The app is per-monitor v2 DPI aware. Sizes are a percentage of the chosen
  monitor, clamped to DIP minimums scaled by that monitor's DPI. Display changes are debounced and
  re-docked.
* **Glass.** Acrylic blur via `SetWindowCompositionAttribute`, which stays live while the window is inactive
  (DWM system backdrops go flat when inactive, and an always-on HUD is almost never active). It falls back
  to the DWM transient backdrop, then to a solid surface. The HUD is hidden from Alt+Tab and has square,
  borderless edges.
* **Theme.** Every colour is a `DynamicResource` written by `ThemeService`. It follows Windows light/dark
  mode and the accent colour live, or uses a fixed light/dark override.
* **Adding a widget.** Subclass `WidgetView`, give it a horizontal and a vertical presentation, and add one
  line to `WidgetRegistry` that says where it goes in each layout.

## Known limitations / next steps

* Spotify control relies on Windows media sessions, so the Spotify desktop app (not the web player alone)
  gives the best experience. Seeking and like/shuffle would need the Spotify Web API.
* The tray menu uses the standard WinForms menu and doesn't follow dark mode.
* No installer yet. `--startup` registration uses the HKCU Run key pointing at the current exe location.
