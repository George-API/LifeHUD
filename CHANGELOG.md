# Changelog

## Unreleased

### Look

- **Signature themes now come in light and dark,** and follow the *Light or dark* setting (Auto follows
  Windows) the way Classic always has. A refreshed, more premium palette: Graphite (true black /
  porcelain), Titanium, Midnight, Champagne (formerly Starlight) and Rosé gain their missing half, and
  four new themes join them: **Peach**, **Olive**, **Amethyst** and **Lagoon**. Every variant keeps
  4.5:1 contrast for secondary text and accents.
- **Slate, a mid-tone tech grey** between the light and dark themes: brushed-aluminium glass, crisp
  white text and an ice-blue accent. It keeps one look in every mode, like the mood themes.

### Top HUD

- **Weather fills its card** like the feed widgets: the conditions spread across the card and the
  three-day forecast sits at the right edge. It now drops the forecast below about 470 px instead of
  clipping it. Music's *Open Spotify* button keeps its natural size instead of stretching.
- The snapshot tool takes `tw=<px>` to render the Top HUD at other monitor widths.

## 1.0.1 (2026-11-14)

### Look

- **A sleeker app icon,** sharp at every size from the 16 px tray icon up: a slimmer bar, frosted
  tiles and a hairline edge so it reads on light and dark backgrounds. `tools/make-icon.py` now
  draws it (at 1024 px, downsampled) in place of the PowerShell script.

### Performance and reliability

- **Weather and News only refresh while their card is on screen**, like the other widgets. Changing
  their settings still fetches at once, so Settings can show what it found.
- **Failing services back off.** A source that is down or rate-limiting (HTTP 429) is retried after a
  few minutes, then twice as long each time, up to 30 minutes, instead of every few minutes for as long
  as the outage lasts. Each Sports league backs off on its own. The first success resets it.
- **News asks before downloading.** Feeds that support it (ETag or Last-Modified) answer an unchanged
  refresh with a few hundred bytes instead of the whole feed.
- **No refetch on every start:** a forecast less than 15 minutes old is shown as is, and refreshed when
  it's due.

## 1.0.0 (2026-11-14)

### Widgets

- **One list of up to four widgets, shared by both layouts.** In the Top HUD the first two sit left of
  the clock and the rest right; any widget can go anywhere. The default is Weather, Tasks & Notes,
  Reddit and Music. Saved arrangements longer than four keep their first four.
- **Edit Widgets mode.** Long-press the clock or empty glass (or use the tray, or *Settings → Widgets*) to
  rearrange the live HUD: drag to reorder, drag in from the tray to add or onto a card to replace, drag
  onto the tray or use ⊖ to remove. Esc or a click on empty space finishes; Ctrl+Z undoes (up to 50
  steps).
- **Settings → Widgets.** An overview of every widget with an *On HUD* / *Not shown* badge, plus
  to-scale drag-and-drop previews of both layouts.
- **New widgets:** Sports (live scores for your leagues and teams, plus headlines), YouTube (new uploads
  from your channels and a Watch Later list), Reddit, Netflix (the weekly global Top 10 with rank
  movement) and Stocks (watchlist and indices with sparklines, in each quote's currency). None needs an
  account.
- **Videos play inside the widget.** Click a YouTube video, or a Netflix title, and it plays in the
  widget itself: a frameless Microsoft Edge window kept exactly over the widget's player area, so your
  sign-in and the sites' DRM keep working. It follows the HUD, hides while the layout changes, and
  closes if the widget leaves the HUD. *Settings → YouTube* can switch to a pop-out window or the
  browser instead. YouTube's player is served from a page on 127.0.0.1 only, behind a per-launch token
  (see [SECURITY.md](SECURITY.md)).
- **Now watching.** A YouTube or Netflix video playing in your browser (Chrome, Edge, Firefox, Brave,
  Opera, Vivaldi) shows in that site's widget with its thumbnail, progress and play/pause. Clicking it
  brings the browser to the front. Nothing is fetched.
- **Music:** drag the timeline to seek where the player allows it (Spotify does), a volume slider for the
  player's own mixer session (never the system volume), and bigger controls. Right-click for your saved
  playlists, pinned first, plus Liked Songs and Your Library; add them in *Settings → Music* by pasting
  a share link.
- **Reddit:** r/popular by default. Click the subreddit name on *Yours* to pick one subreddit, all of
  yours, or a suggestion. It follows Reddit's rate-limit headers, so Popular loads reliably, and a
  Popular post without an image shows a Reddit tile in the Sidebar.
- **Search & AI.** The magnifying glass (or *Search & Ask…* in the tray) opens a panel: Enter searches
  Google in your browser, and the ChatGPT, Claude and Gemini tabs use your own API keys. Keys are kept
  in Windows Credential Manager and conversations only in memory.

### Look and feel

- **Smoother layout switch.** One continuous, jerk-free fold paced to the display's refresh rate
  (167 fps on a 164 Hz screen, no dropped frames, up from a ~64 fps timer).
- **Theme quick-swap.** A paintbrush on the HUD opens the theme menu on hover or click.
- **Themes.** Coast, a light sea-glass mood, replaces Nebula (a saved Nebula becomes Neon). Graphite is
  now true black with extra contrast, and Titanium a cool silver. Every theme meets 4.5:1 contrast for
  secondary text and accents. Mood patterns are quieter and more varied.
- **Settings redesigned** in the style of macOS System Settings: compact controls, a *Widgets* group in
  the sidebar, and *Reset layout*.
- **Context menus match the theme**, starting with the music card's right-click menu.
- **Lighter clock** numerals, and **sharper text** from whole-pixel font sizes.
- **Calmer weather icons**: softer sun, rain and lightning colours.
- **More breathing room** between cards (14px in the Top HUD, 16px in the Sidebar). The Top HUD's
  default height is 9% of the screen, and its News, Reddit and Sports strips show exactly three rows.

### Install and distribution

- **Installs itself.** Running the downloaded exe copies it to `%LOCALAPPDATA%\Programs\LifeHUD`, adds a
  Start menu entry and an uninstaller in *Settings → Apps*, and relaunches. No admin, no wizard. A newer
  download updates it the same way; an empty `portable` file next to the exe opts out.
- **winget-ready** (`packaging/winget`, `GeorgeAPI.LifeHUD`), with automatic update PRs after the first
  submission. Releases can be code-signed (Azure Artifact Signing) and carry a build provenance
  attestation (`gh attestation verify`). See `docs/RELEASING.md`.

### Accessibility

- Screen-reader labels and actions for every HUD button, menu item, tab and slider, and a keyboard or
  screen-reader way to do everything Edit Widgets does by dragging.

### Performance and reliability

- Feed widgets only use the network while they're on screen, on a cadence that suits their data, and
  fetch in parallel.
- The music progress bar no longer animates every second, and Mood patterns are one tiled image per
  card instead of 96 elements.
- Mood pattern tiles are drawn once and cached, instead of re-rendered from SVG on every repaint.
- Only the Reddit thumbnails on screen stay decoded in memory.
- Headline ages update in place instead of rebuilding the list every minute.
- Images are downloaded over HTTPS only, and only from each service's own hosts.

### Development

- Minimum Rust is 1.92 (what Slint 1.18 needs), checked in CI. CI uses current actions with a read-only
  token and `--locked` builds; Dependabot keeps actions and crates current.
- `cargo run -- --snapshot` renders widgets and Settings pages offscreen for review.

## 0.1.0 (2026-11-14)

First public release.

- Top HUD and Sidebar layouts, with an animated switch between them.
- Shell AppBar reservation, proportional refit of open windows, and fullscreen awareness.
- Widgets: lock-screen clock, weather, tasks + notes, two-tab news, Spotify / media controls.
- Twelve themes: six Signature colour themes and six Mood themes with patterned cards.
- macOS-style Settings window.
