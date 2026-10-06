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
target/debug/multiplexer --fresh
```

The current tab, pane split trees and ratios, focused panes, and stack positions
are saved in `$XDG_STATE_HOME/multiplexer/layout.toml` (or
`$HOME/.local/state/multiplexer/layout.toml` when `XDG_STATE_HOME` is unset).
The next launch restores that layout with new default shells. It does not
restore running programs, shell state, working directories, or terminal
contents. Use `--fresh` to start with one pane and replace the saved layout.

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
resize_mode = "r"
broadcast_menu = "b"
split_vertical = "\\"
split_horizontal = "-"
split_vertical_global = "|"
split_horizontal_global = "_"
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
selection_mode = "y"
quit = "q"
```

Bindings may use `Alt-h`, `Ctrl-Alt-h`, or `Ctrl-Alt-1` for direct shortcuts.
Prefix a direct member move/carry binding with the leader to operate on its
whole stack. Explicit `swap_stack_left` (and other directions) and
`carry_stack_1` … `carry_stack_9` actions can also be rebound independently.
Broadcast action keys are interpreted inside the broadcast submenu and can share keys with the main menu. Other keys without Alt are interpreted after the leader: one ASCII character,
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
Vim-style selection and clipboard retry through fake helpers, ordinary SGR mouse forwarding, and
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
- **Ctrl-b then `-` / `\`:** split the focused pane horizontally / vertically.
- **Ctrl-b then `_` / `|`:** split the whole tab horizontally / vertically. The existing layout stays together above / left of the new focused pane.
- **Ctrl-b then r:** enter resize mode. `hjkl` pushes the focused pane or stack’s left/bottom/top/right edge outward by one cell; `HJKL` uses five-cell steps. Alt-hjkl changes focus while staying in resize mode. Escape exits. Terminal edges and neighboring minimum sizes limit expansion.
- **Ctrl-b then b:** open the broadcast submenu. `b` toggles visible-pane broadcast, `B` toggles manual broadcast, `m` toggles the focused pane’s manual target, and `r` resets broadcast. A command closes the submenu; Escape cancels. Unbound keys leave it open.
- **Ctrl-b:** show the effective keybinding popup. Press a leader binding to
  act, or Escape to dismiss. Related commands are grouped in two columns using
  the compact style in [popup_example.md](popup_example.md). Keep similar
  functions and keybindings grouped as commands are added or changed. Leader
  mode stays open without a timeout. Unbound keys leave it open; press the leader
  twice to send a literal leader byte.

The UI uses Dracula colors: purple focus accents, muted
blue-gray inactive borders, and pink broadcast/red confirmation indicators.
Terminal default text follows the theme, while the background uses the host
terminal's default. Explicit application ANSI and RGB colors are preserved.

Each pane has a rounded border. The top row shows stable tab numbers followed
by one square per pane in layout order, for example `1 ▫▫▫▫ │ 2 ▪▫`.
The filled square marks the focused pane in the active tab; `∅` marks an empty
tab. Stacks show numbered collapsed edges above and below the expanded member
according to stack order. On short panes, edges are limited to preserve content.
The expanded pane border also shows its stack position.
Tab numbers are stable when other tabs disappear. When the last session exits
or is killed, the active tab stays empty until you leave it, then is removed.
Use any split binding or leader `a` to start a shell in an empty tab.

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

## Pane scrollback

Scroll the mouse wheel over a shell pane to browse its last 10,000 lines,
three lines per wheel event. Scroll down to return to live output; typing or
pasting also returns the receiving pane to live output. The cursor is hidden
while viewing history. Leader `y` enters keyboard selection mode to copy history.

Applications that enable mouse reporting retain their own wheel scrolling.
Shift-wheel overrides this on the normal terminal screen. Alternate-screen
applications (such as Neovim) continue to manage their own scrolling.

Command families should use submenus as bindings expand. Keep the main popup, submenu help, and configuration documentation aligned.

## Keyboard selection

Leader `y` opens a frozen snapshot of the focused pane and its retained
scrollback, starting at the application cursor. The live process keeps running.

- `hjkl`: left/down/up/right; `w/b/e`: next word, previous word, word end.
- `0/^/$`: line start, first nonblank, line end; `gg/G`: buffer start/end.
- Numeric prefixes repeat motions (for example, `10k`); `3gg` or `3G` goes to row 3.
- Ctrl-u/d: half-page up/down; Ctrl-b/f: page up/down.
- `v`: character selection; `V`: whole-line selection. Repeat to clear the
  selection, or switch between them while retaining the anchor.
- `y`: copy the selection to the system clipboard and exit. With no selection,
  copy the current line. Escape cancels and restores live output.

The status bar shows the mode and buffer position. Other keys, mouse reports,
and pasted text are consumed while selecting; they never reach the application
or broadcast targets. A terminal resize cancels selection.

Copy tries `wl-copy` on Wayland, then `xclip` and `xsel` as fallbacks; X11
uses `xclip` then `xsel`. On failure the selection stays open, the status bar
shows the error, and `y` retries. The old `copy_selection` configuration name
is accepted as an alias for `selection_mode`.
