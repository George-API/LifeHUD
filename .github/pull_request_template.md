## What does this change?

<!-- One or two sentences, and link the issue it fixes, e.g. "Fixes #12". -->

## Checklist

- [ ] `cargo fmt`, `cargo clippy --all-targets` (no warnings) and `cargo test` pass
- [ ] Unsafe code stays in `src/platform`, with a comment explaining why it's sound
- [ ] No I/O on the UI thread
- [ ] UI changes: screenshots of **both layouts** in a light and a dark theme
- [ ] Rendering or polling changes: memory and CPU numbers before and after
