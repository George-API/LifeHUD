# Roadmap

Where Life HUD is heading. This is a direction, not a commitment. Ideas and votes are welcome in
[issues](https://github.com/George-API/LifeHUD/issues/new/choose).

## Shipped in 1.0.0

Five new widgets (Sports, YouTube, Reddit, Netflix, Stocks), one widget list shared by both layouts,
Edit Widgets mode, "now watching" for browser video, seek and per-app volume for music, the Search & AI
panel, theme quick-swap, install on first run, and a smoother layout switch. Details are in the
[changelog](../CHANGELOG.md).

A few things shipped differently from the original plans, on purpose:

- **YouTube** plays videos in a pop-out player rather than an embedded web view, which keeps the HUD
  light.
- **Reddit's** JSON API now needs OAuth, so the widget reads its public RSS feeds, which carry no scores.
- **Stocks** use Yahoo Finance's public chart data, so no API key is needed.

## In progress

- **Video inside the YouTube and Netflix widgets**, by docking a frameless Edge app window over the
  widget, so your sign-in and the sites' DRM keep working.

## Next

- **Signed releases and a winget listing.** The release workflow and manifests are ready
  ([RELEASING.md](RELEASING.md)); this needs a code-signing certificate and the first public release.
- **Email.** Your inbox at a glance: unread count and the latest messages, opening in your mail app or
  browser. Gmail (Google OAuth) and Outlook / Microsoft 365 (Microsoft Graph), read-only, with sign-in
  tokens kept in Windows Credential Manager. It would be the first widget that needs an account, so it
  needs an explicit sign-in flow and a clear privacy note.
- **Automatic updates**, building on the installer, which already updates in place when you run a newer
  download.

## Later

- More than one HUD across multiple displays.
- Your whole Spotify library in the music menu, not only saved playlists. This needs Spotify sign-in
  (OAuth with PKCE), and since Spotify now limits new developer apps to a few allowlisted users, each
  person would register their own free Spotify app and paste in its client ID.
- Schedules for followed teams beyond the coming week.
