# Multiplexer

Multiplexer is a Linux-first terminal-hosted workspace for shell, Neovim,
SSH, and Codex sessions. It keeps terminal processes in stable PTY-backed
sessions while panes, stacks, and tabs are rearranged.

Build and test with the project-local Rust toolchain:

```sh
RUSTUP_HOME="$PWD/.tools/rustup" CARGO_HOME="$PWD/.tools/cargo" \
  "$PWD/.tools/cargo/bin/cargo" test --workspace
```

The executable accepts an optional configuration path before entering raw
terminal mode:

```sh
target/debug/multiplexer --help
target/debug/multiplexer --config "$HOME/.config/multiplexer/config.toml"
```

With no `--config` argument, built-in defaults are used. A configuration file
is TOML:

```toml
leader = "Ctrl-b"
alt_timeout_ms = 50
alt_numbers = true
alt_tabs = ["1", "2", "3", "4", "5", "6", "7", "8", "9"]

[bindings]
focus_left = "Alt-h"
focus_down = "Alt-j"
focus_up = "Alt-k"
focus_right = "Alt-l"
swap_left = "Ctrl-Alt-h"
swap_down = "Ctrl-Alt-j"
swap_up = "Ctrl-Alt-k"
swap_right = "Ctrl-Alt-l"
resize_left = "Ctrl-h"
resize_down = "Ctrl-j"
resize_up = "Ctrl-k"
resize_right = "Ctrl-l"
split_vertical = "|"
split_horizontal = "-"
stack_add = "a"
pane_previous = "Alt-["
pane_next = "Alt-]"
stack_1 = "1"
remove_slot = "d"
kill_session = "x"
kill_stack = "X"
create_tab = "t"
close_tab = "w"
carry_1 = "Ctrl-Alt-1"
carry_2 = "Ctrl-Alt-2"
carry_3 = "Ctrl-Alt-3"
carry_4 = "Ctrl-Alt-4"
carry_5 = "Ctrl-Alt-5"
carry_6 = "Ctrl-Alt-6"
carry_7 = "Ctrl-Alt-7"
carry_8 = "Ctrl-Alt-8"
carry_9 = "Ctrl-Alt-9"
broadcast_visible = "b"
broadcast_manual = "B"
broadcast_target = "m"
broadcast_reset = "r"
copy_selection = "y"
quit = "q"
```

Bindings may use `Alt-h`, `Ctrl-Alt-h`, or `Ctrl-Alt-1` for direct shortcuts.
Prefix a direct member move/carry binding with the leader to operate on its
whole stack. Explicit `swap_stack_left` (and other directions) and
`carry_stack_1` … `carry_stack_9` actions can also be rebound independently.
Keys without Alt are interpreted after the leader: one ASCII character,
`Ctrl-` plus one letter, a named key such as
`Esc`, `Tab`, `Enter`, or `Backspace`, or a byte such as `0x02`. Binding names
are validated, duplicate keys are rejected, and a binding cannot make the
leader unreachable. `alt_tabs` must contain nine distinct keys. The runtime
help view prints the effective leader, Alt-number settings, and every command
binding after configuration is loaded.

## Runtime smoke coverage

The current focused PTY checks include a raw standalone Escape and exact shell
input (`runtime_smoke`), real `/usr/bin/nvim` editing with `hjkl`, Escape,
`:wq`, and shell recovery (`neovim_smoke`), plus visible-stack broadcast,
cross-tab exclusion, and PID-preserving stack carry (`live_sessions`). Run
Shift-SGR copy through a fake `wl-copy`, ordinary SGR mouse forwarding, and
confirmed-quit termios restoration are covered by `clipboard_smoke` and
`terminal_restore`. Run them with the same project-local toolchain as the
workspace tests:

```sh
RUSTUP_HOME="$PWD/.tools/rustup" CARGO_HOME="$PWD/.tools/cargo" \
  "$PWD/.tools/cargo/bin/cargo" test -p multiplexer --test runtime_smoke \
    --test neovim_smoke --test live_sessions --test clipboard_smoke \
    --test terminal_restore
```

These checks do not cover SSH or Codex sessions, real host clipboard provider
behavior, bracketed-paste delivery, or manual broadcast targets; those remain
runtime acceptance work.

The default leader is `Ctrl-b`. Bare application input passes through. The
full interaction contract and acceptance matrix are in
[`INTERACTION_SPEC.md`](INTERACTION_SPEC.md) and
[`ACCEPTANCE.md`](ACCEPTANCE.md).

## Incremental rendering checks

Ratatui composes pane cells and workspace chrome and emits frame differences.
Ignored mouse motion produces no output; unchanged frames skip cursor output
as well. Updates are buffered with synchronized-output markers and cursor
restoration. Full clears are reserved for initialization and terminal resizing.

The fullscreen PTY regression runs at 240×67 and 320×90 cells, checks idle and
mouse silence, limits a typed character to fewer than 256 output bytes, and
verifies resize and cursor behavior:

```sh
RUSTUP_HOME="$PWD/.tools/rustup" CARGO_HOME="$PWD/.tools/cargo" \
  "$PWD/.tools/cargo/bin/cargo" test -p multiplexer --test rendering_smoke -- --nocapture
```

Renderer unit tests additionally cover colored/styled cells, selection,
wide and combining characters, stale-cell erasure, and cursor-only updates.

## Pane navigation and tabs

- **Alt-h/j/k/l:** focus left/down/up/right. Up/down visits stack members first,
  then spatial neighbors. At top/bottom edges, wraps to the opposite edge in
  the same tab and column; a full-height stack cycles its own members.
  Left/right at an outer edge wraps to the previous/next populated tab, skipping
  empty tabs and retaining the destination's focused pane and stack member.
- **Alt-[ / Alt-]:** previous/next pane in layout order, skipping hidden members.
  At the ends, visits the previous/next populated tab; with only one populated
  tab, wraps within that tab. Entering a stack preserves its displayed member.
- **Ctrl-Alt-h/j/k/l:** move the displayed member. Up/down reorders it inside
  its stack; crossing to a neighboring pane swaps the two displayed members.
- **Ctrl-Alt-1…9:** carry only the displayed member to its own pane in that tab,
  creating an absent destination without an extra shell.
- **Ctrl-b then Ctrl-Alt-h/j/k/l or 1…9:** swap or carry the whole stack,
  preserving its displayed member. A customized leader replaces Ctrl-b.
- **Alt-1…9:** select the numbered tab; a missing tab starts one shell.
- **Ctrl-b:** show the effective keybinding popup. Press a leader binding to
  act, or Escape to dismiss. Related commands are grouped in two columns using
  the compact style in [popup_example.md](popup_example.md). Keep similar
  functions and keybindings grouped as commands are added or changed. Leader
  mode stays open without a timeout. Unbound keys leave it open; press the leader
  twice to send a literal leader byte.

The UI uses Dracula colors: a dark background, purple focus accents, muted
blue-gray inactive borders, and pink broadcast/red confirmation indicators.
Terminal default text and background follow the theme; explicit application
ANSI and RGB colors are preserved.

Each pane has a rounded border. The top row shows stable tab numbers followed
by one square per pane in layout order, for example `1 ▫▫▫▫ │ 2 ▪▫`.
The filled square marks the focused pane in the active tab; `∅` marks an empty
tab. Stacks show numbered collapsed edges above and below the expanded member
according to stack order. On short panes, edges are limited to preserve content.
The expanded pane border also shows its stack position.
Tab numbers are stable when other tabs disappear. When the last session exits
or is killed, the active tab stays empty until you leave it, then is removed.
Use leader `|`, `-`, or `a` to start a shell in an empty tab.

Ctrl-Alt-number requires a terminal that can distinguish modified digits. The
host requests Kitty keyboard disambiguation and accepts both
[CSI-u](https://sw.kovidgoyal.net/kitty/keyboard-protocol/) and
[xterm modifyOtherKeys](https://invisible-island.net/xterm/modified-keys.html).
Legacy Alt-[ waits for the configured Alt timeout because it shares the arrow-key
prefix; CSI-u reports are unambiguous.
A legacy terminal that sends Ctrl-Alt-1 identically to Alt-1 cannot distinguish
the commands; configure it to send `ESC [ 49 ; 7 u` for Ctrl-Alt-1 (50 for 2,
and so on), or rebind `carry_1`…`carry_9` in the config. Ordinary legacy input
and paste retain their bytes; recognized extended control keys are normalized
for hosted applications. Keyboard reporting mode is restored on exit.

`todo_smoke` verifies these interactions together in a 240×67 PTY.

Legacy `leader_timeout_ms` configuration is still accepted for compatibility, but
it does not expire leader mode. The Alt/Escape sequence timeout remains active.

Arrow keys follow each recipient application’s cursor-key mode, including when
broadcasting. Lock-state flags are removed from extended cursor reports.
