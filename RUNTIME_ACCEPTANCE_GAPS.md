# Runtime acceptance review — 2026-09-20

The executable now has bounded PTY evidence for ordinary shell input, a raw
standalone Escape, the leader-q confirmation path, Neovim editing, and the
live visible-broadcast/carry workflow. The checks below describe the tested
surface and the release work that remains.

| Acceptance area | Current evidence | Release status |
| --- | --- | --- |
| Ordinary input and standalone Escape | `runtime_smoke` sends an exact shell marker and a standalone `0x1b` to a raw child reader. | Verified for these paths |
| Neovim compatibility | `neovim_smoke` launches `/usr/bin/nvim -u NONE -i NONE --noplugin` in an isolated PTY, writes `hello-hjkl`, exits with Escape/`:wq`, waits for a shell-owned post-exit marker, and proves the shell remains responsive. | Verified |
| Clipboard and mouse | `clipboard_smoke` selects exact `CLIP` text with Shift-SGR drag, copies it through a fake `wl-copy`, and verifies a non-Shift SGR report is translated and forwarded to a child. | Verified with fake helper |
| Bracketed paste | The framer preserves both bracketed-paste markers and keeps the payload atomic; live child delivery is not yet covered by a PTY test. | Partially verified |
| Tiling | The renderer clips to cell width, applies attributes, handles cursor and wide-cell continuations, and the live session test exercises split/stack geometry. Full visual layout coverage remains pending. | Partially verified |
| Destructive actions | The runtime renders confirmation prompts with affected counts and replacement-shell text, and the model tests cancellation/confirmation. A live kill/close PTY flow remains pending. | Partially verified |
| Exit | `runtime_smoke` and `neovim_smoke` both confirm `leader-q` with `y` and observe multiplexer exit. A full all-children shutdown audit is still pending. | Partially verified |
| Scoped runtime errors | Spawn, write, and read failures are converted to status messages; injected live failure coverage remains pending. | Partially verified |
| Broadcast and carry | `live_sessions` verifies visible-stack and cross-tab exclusions, and carries a complete stack while asserting the original shell PIDs remain unchanged. Manual-target live coverage remains pending. | Partially verified |
| Terminal restoration | `terminal_restore` compares the controlling PTY termios before and after confirmed quit and checks alternate-screen enter/leave sequences. | Verified for confirmed quit |

Reproducible focused evidence:

```sh
RUSTUP_HOME="$PWD/.tools/rustup" CARGO_HOME="$PWD/.tools/cargo" \
  "$PWD/.tools/cargo/bin/cargo" test -p multiplexer --test runtime_smoke
RUSTUP_HOME="$PWD/.tools/rustup" CARGO_HOME="$PWD/.tools/cargo" \
  "$PWD/.tools/cargo/bin/cargo" test -p multiplexer --test neovim_smoke
RUSTUP_HOME="$PWD/.tools/rustup" CARGO_HOME="$PWD/.tools/cargo" \
  "$PWD/.tools/cargo/bin/cargo" test -p multiplexer --test live_sessions
RUSTUP_HOME="$PWD/.tools/rustup" CARGO_HOME="$PWD/.tools/cargo" \
  "$PWD/.tools/cargo/bin/cargo" test -p multiplexer --test clipboard_smoke
RUSTUP_HOME="$PWD/.tools/rustup" CARGO_HOME="$PWD/.tools/cargo" \
  "$PWD/.tools/cargo/bin/cargo" test -p multiplexer --test terminal_restore
```

These tests do not establish SSH or Codex compatibility, real host clipboard
provider behavior, cancelled-quit restoration, or full all-child shutdown.
The remaining release gate covers bracketed paste delivery and manual
broadcast recipients alongside the renderer and scoped-error work listed
above.
