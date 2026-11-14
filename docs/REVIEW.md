# Pre-launch review

Review of v0.1.0 before open-sourcing. All numbers were measured on Windows 11 (build 26200), 2560×1440,
with Spotify playing.

## Performance

| | C#/WPF prototype | Rust + Slint (GPU renderer) | **Rust + Slint (software renderer, shipped)** |
|---|---|---|---|
| Cold start to first window | ~15 s | 0.9 s | **0.3 s** |
| Private memory | 100 MB | 124 MB | **9 MB** |
| Working set | 185 MB | 123 MB | **40 MB** |
| Idle CPU (whole machine) | 0.05% | 0.32% | **0.09%** |
| Distribution | runtime + DLLs | 17 MB exe | **12 MB single exe** |

Decisions that came out of these measurements:

- **Software renderer.** GPU drivers commit large buffers per OpenGL context. For a HUD that redraws a
  few small regions per second, CPU rasterization with dirty regions is far lighter and looks identical.
- **Polling cadence.** Media polls once a second while playing and every 2.5 s when idle; control
  commands wake the poller immediately. Weather refreshes every 15 minutes (2 minutes after a failure),
  news every 20 minutes.
- **OS stacks over crates.** HTTPS uses SChannel through `native-tls`, so system proxies and certificates
  apply. Album art is decoded by WinRT `BitmapDecoder`, and dates are formatted by the Win32 locale APIs.
  This keeps the dependency tree and binary small.
- **Release profile.** `lto`, `codegen-units = 1`, `strip`, `panic = "abort"`. The panic hook still
  logs and releases the reserved screen space before exiting.

## Memory safety

- **Unsafe code** lives only in `src/platform/*`. Each block wraps a single Windows API call with its
  invariants documented next to it.
- **AppBar subclass.** The window procedure receives a raw pointer from `Rc::into_raw`. It stays valid
  until `Drop` removes the subclass and reclaims it. Work that would re-enter the UI is deferred out of
  the window procedure with `slint::Timer::single_shot(0)`.
- **EnumWindows / EnumDisplayMonitors callbacks** receive a `&mut` that lives on the caller's stack for
  the duration of the synchronous call.
- **Undocumented exports** (acrylic via `SetWindowCompositionAttribute`, dark menus via uxtheme ordinals
  135/136) are resolved with `GetProcAddress`. Every lookup tolerates the export being missing.
- **Threads.** COM is initialised per thread: STA on the UI thread, MTA on the workers. Workers never
  touch UI objects; they send plain data through `slint::invoke_from_event_loop`.

## Correctness and QA

Unit tests (`cargo test`, 31 passing) cover:

- weather parsing and condition mapping;
- RSS 2.0, RSS 1.0/RDF and Atom parsing, including entities, CDATA and dates;
- daily task rollover, reordering and model sync;
- settings defaults, repair, a BOM-tolerant loader and sidebar ordering;
- window-reflow geometry, theme palettes and media colour extraction.

Bugs found and fixed during this review:

| Issue | Fix |
|---|---|
| The task row's hover area sat on top of the checkbox and could swallow clicks | The whole row is now the touch target; checked end to end with real input and the saved file |
| quick-xml text trimming deleted the spaces around entities ("Rock & roll" became "Rock&roll") | Per-fragment trimming removed; whitespace is normalised once over the whole title |
| A `settings.json` saved with a UTF-8 BOM (Notepad, PowerShell) reset all settings | The loader strips a BOM; covered by a test |
| WinRT `HttpClient` failed TLS against some hosts (`0x80072F8F`) | Switched to `ureq` with native-tls (SChannel) |
| Weather card collapsed when it had no data | Explicit widths for each state |
| Two-line headlines overlapped | Rows have a fixed, uniform height |

Checked by hand:

- dock and undock, and switching layout (including the animation);
- fullscreen apps;
- hiding the HUD and restoring it from the tray;
- `--exit`, and launching a second instance;
- quitting releases the reserved space;
- offline start-up from cache;
- light, dark and every theme.

`cargo clippy --all-targets` reports no warnings.

## Security

- **Links.** Only `http(s)://` article links are passed to the shell. Feed content is treated as text and
  never as markup.
- **No secrets** are stored or required. All data stays in `%LOCALAPPDATA%\LifeHUD`.
- **Bounded input.** Response bodies are capped at 4 MB, with a 15 s timeout. A feed that fails to parse
  is logged and skipped.

## Known limitations

- Spotify control is limited to what Windows media controls expose: play/pause, next and previous. There
  is no seek, like or shuffle.
- Moving windows that belong to elevated (administrator) apps is blocked by Windows.
- Only one display hosts the HUD at a time.
- There is no installer yet. `tools/install.ps1` is a per-user install, and the binary is unsigned.
- Feed URLs are public endpoints controlled by their publishers and may change.
