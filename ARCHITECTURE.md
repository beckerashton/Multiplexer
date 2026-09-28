# Multiplexer v1 architecture

## Decision

Multiplexer v1 is a Linux-first, terminal-hosted Rust application. It runs in
the user's existing terminal emulator and does not require a desktop window,
server, or existing multiplexer session.

The dependency boundary is deliberately narrow:

| Concern | Chosen component | Reason |
| --- | --- | --- |
| PTY and child lifecycle | `portable-pty` 0.9.0 | Published, cross-platform PTY API from the WezTerm project. It supplies spawning, IO, resizing, and child control without taking ownership of tabs or layouts. |
| VT parsing, screen state, scrollback | `vt100` 0.16.2 | Published parser designed for terminal-hosting applications such as screen/tmux. It exposes cells, alternate screen, cursor, application cursor/keypad, bracketed paste, and xterm mouse modes. |
| Host terminal lifecycle | `crossterm` 0.29.0 | Provides raw mode, alternate screen, mouse/focus/paste control sequences, and restoration. |
| Frame composition and incremental rendering | `ratatui` 0.28.1 with its Crossterm backend | Composes `vt100::Screen` cells and workspace chrome into a buffer, diffs successive frames, and emits only changed cells. |
| Linux clipboard | configured helper (`wl-copy`, `xclip`, then `xsel`) | Matches terminal-hosted Linux practice without adding a GUI or clipboard-library dependency. |
| Configuration | `serde` + `toml` | Declarative configurable bindings with no embedded scripting runtime. |

The app owns the layout tree, tab and stack state, command routing, broadcast
selection, terminal-cell composition, and lifecycle policy. It does **not**
implement a VT parser, a PTY, a font renderer, or a standalone window system.

`vt100` is selected over a pinned internal WezTerm/Alacritty emulator because
it has a published crates.io API and a smaller dependency surface. The M2 live
compatibility gate below is mandatory: it must successfully exercise Neovim,
an SSH session, alternate screens, application cursor keys, bracketed paste,
and mouse reporting. If that gate exposes an unsupported sequence that blocks
those workflows, replace only the `TerminalBackend` implementation with an
adapter around a pinned WezTerm `term` revision. No domain, routing, or layout
interface changes are allowed for that contingency.

### Options rejected for v1

| Option | Why it is not the default |
| --- | --- |
| GTK4/VTE window | VTE is mature, but it changes the requested product from an SSH-capable terminal multiplexer to a native desktop terminal. |
| tmux configuration/control-mode façade | tmux can retain hidden panes to simulate stacks, but a custom layer would either duplicate tmux's layout authority or encode the logical stack in parked tmux windows. Control mode still returns terminal bytes that a custom UI must parse/render. The state reconciliation and external-server dependency outweigh the implementation saved. |
| `wezterm-term` / `alacritty_terminal` direct dependency | Both have capable emulators but are internal APIs, requiring a git revision pin and an upgrade audit for every upstream change. |
| libvterm/libtsm plus C renderer | They solve parsing, but need vendoring or a system dependency and a separate raw input, color rendering, clipboard, and build setup. The current libvterm repository is archived. |

## Toolchain and dependencies

The repository pins Rust 1.85.0 because it is the first Rust release supporting
edition 2024. A project-local toolchain is installed in `.tools/`; it is ignored
by Git and makes no system package changes.

```bash
RUSTUP_HOME="$PWD/.tools/rustup" CARGO_HOME="$PWD/.tools/cargo" \
  "$PWD/.tools/cargo/bin/cargo" build --workspace --locked
RUSTUP_HOME="$PWD/.tools/rustup" CARGO_HOME="$PWD/.tools/cargo" \
  "$PWD/.tools/cargo/bin/cargo" test --workspace --locked
RUSTUP_HOME="$PWD/.tools/rustup" CARGO_HOME="$PWD/.tools/cargo" \
  "$PWD/.tools/cargo/bin/cargo" clippy --workspace --all-targets -- -D warnings
RUSTUP_HOME="$PWD/.tools/rustup" CARGO_HOME="$PWD/.tools/cargo" \
  "$PWD/.tools/cargo/bin/cargo" run --locked -p multiplexer
```

The first build resolves the pinned dependency versions and creates
`Cargo.lock`; all later CI and developer commands use `--locked`. The current
host had no Cargo on `PATH`; the project-local Rust 1.85.0 toolchain is now
available at `.tools/cargo/bin/cargo`.

No system library is required by the chosen core. Clipboard capability depends
on a configured helper (`wl-copy` on Wayland; `xclip`, then `xsel` on X11);
copy failure must leave the selection intact and show an in-app status message.

## Package and file ownership

Only the owner may edit a listed file. A worker proposing a public-type change
must first send the exact change to the integration owner and wait for approval.
Existing product files, including `FEATURE_SET.md`, are not implementation
targets.

| Owner | Files | Responsibility |
| --- | --- | --- |
| Architecture/integration | `ARCHITECTURE.md`, root `Cargo.toml`, `rust-toolchain.toml`, `.gitignore`, `crates/mux-core/src/types.rs`, `crates/multiplexer/**` | Dependency pins, executable, raw-byte host-input reader, host terminal guard, PTY/emulator adapter, renderer, clipboard-helper adapter, event loop, and integration tests. |
| Layout/session | `crates/mux-core/src/layout.rs`, `workspace.rs`, `session.rs`, `crates/mux-core/tests/layout_test.rs`, `workspace_test.rs`; add their `lib.rs` exports only | Pure geometry and domain state. No `crossterm`, `portable-pty`, `vt100`, filesystem, child process, or clipboard imports. |
| Input/broadcast | `crates/mux-core/src/input.rs`, `broadcast.rs`, `config.rs`, `crates/mux-core/tests/input_test.rs`, `broadcast_test.rs`; add their `lib.rs` exports only | Configurable command recognition and target planning. No PTY, renderer, or process control. |
| Interaction specification | `INTERACTION_SPEC.md`, `ACCEPTANCE.md` | Binding choices, confirmation prompts, visible-state requirements, and executable acceptance cases. |

`crates/mux-core/src/lib.rs` is a public export list. Each module owner may add
only its own module declaration/re-export. The integration owner resolves
conflicts after both modules land.

## Domain model and frozen interfaces

`crates/mux-core/src/types.rs` contains the stable newtypes (`TabId`, `SlotId`,
`SessionId`) and shared enums (`Axis`, `Direction`, `CellRect`, `SessionSpec`,
`SessionState`, `BroadcastScope`, `WorkspaceCommand`, `LifecycleEffect`). IDs
are opaque and never reused in a running process.

The layout/session worker implements these public interfaces:

```rust
pub struct LayoutTree;                 // leaves are SlotId only
pub struct Workspace;                  // owns tabs, layouts, stacks, focus, IDs
pub struct WorkspaceView;              // immutable integration/routing snapshot
pub struct Transition {
    pub changed: bool,
    pub effects: Vec<LifecycleEffect>,
}
pub enum DomainError { /* invalid target, unsatisfiable geometry, policy refusal */ }

impl LayoutTree {
    pub fn split(&mut self, slot: SlotId, axis: Axis, ratio: u16)
        -> Result<SlotId, DomainError>;
    pub fn remove(&mut self, slot: SlotId) -> Result<(), DomainError>;
    pub fn geometry(&self, bounds: CellRect, minimum: (u16, u16))
        -> Result<std::collections::BTreeMap<SlotId, CellRect>, DomainError>;
    pub fn neighbor(&self, slot: SlotId, direction: Direction, bounds: CellRect)
        -> Option<SlotId>;
    pub fn resize(&mut self, slot: SlotId, direction: Direction, cells: i16,
                  bounds: CellRect, minimum: (u16, u16)) -> Result<(), DomainError>;
    pub fn swap_slots(&mut self, first: SlotId, second: SlotId) -> Result<(), DomainError>;
}

impl Workspace {
    pub fn execute(&mut self, command: WorkspaceCommand)
        -> Result<Transition, DomainError>;
    pub fn view(&self) -> WorkspaceView;
}
```

`Workspace` is the sole owner of tab selection, each tab's `LayoutTree`, the
focused slot, and ordered `Vec<SessionId>` stacks with one visible active
member. `LayoutTree` never sees a session ID. A layout operation returns a
domain error rather than killing a process. `Workspace::execute` returns
`LifecycleEffect::Spawn` or `LifecycleEffect::Terminate`; the integration layer
performs that effect after the domain transition is accepted. A failed spawn is
reported back as `SessionState::Exited`; it must not roll the new slot into a
different identity.

The input/broadcast worker implements these public interfaces:

```rust
pub struct InputRouter;
pub struct BindingConfig;
pub struct RouterContext {
    pub tab_by_number: [Option<TabId>; 9],
    pub focused_slot: SlotId,
    pub default_session: SessionSpec,
}
pub enum InputEvent { Bytes(Vec<u8>), Paste(Vec<u8>), Timeout }
pub enum InputRoute {
    Forward(Vec<u8>),                  // byte-for-byte payload for PTY delivery
    Command(WorkspaceCommand),         // resolved using RouterContext
    Consume,
}
pub struct BroadcastPlan {
    pub recipients: Vec<SessionId>,    // complete, deduplicated and sorted
}

impl InputRouter {
    pub fn route(&mut self, event: InputEvent, context: &RouterContext,
                 config: &BindingConfig) -> Vec<InputRoute>;
}

pub fn plan_broadcast(view: &WorkspaceView, scope: BroadcastScope) -> BroadcastPlan;
```

The router receives the raw stdin byte stream, with its own bounded decoder for
bracketed paste, SGR mouse reports, and the configured Escape/Alt delay. One
input chunk can contain ordinary bytes and several commands, so `route` returns
an ordered vector; every byte and command appears exactly once in that vector.
The integration loop applies each command before asking the router to process
the next decoded unit, rebuilding `RouterContext` from the resulting view each
time. It must never resolve a later numbered tab/slot target against a stale
pre-chunk view.
It forwards normal bytes byte-for-byte: it may never decode and re-encode a
pass-through key. `RouterContext` resolves numbered tabs and the focused slot
against one immutable workspace snapshot and supplies the default `SessionSpec`
for split, stack-add, and new-tab commands. This keeps the router pure while
giving it enough context to return executable `WorkspaceCommand`s. The
`TerminalBackend` writes each forwarded byte slice directly to the recipients
chosen below and only encodes local mouse events that it intercepts for the
child using the focused screen's advertised mouse protocol. `Alt-1` through
`Alt-9` use the configurable Escape-prefix delay. Alt-h/j/k/l focuses panes,
Ctrl-Alt-h/j/k/l swaps them, and Ctrl-Alt-1…9 carries stacks. A keyboard
decoder handles CSI-u and modifyOtherKeys reports before confirmation handling.
Recognized extended control keys are normalized for the legacy guest parser.
Host keyboard disambiguation is pushed on entry and popped during cleanup.

`plan_broadcast` uses one `WorkspaceView` snapshot and returns the complete
recipient set, including the focused session only when the current broadcast
scope selects it. It returns currently visible sessions from the current tab,
excludes hidden stack members and stale IDs, and cannot cross a tab. The
integration layer writes each forwarded byte slice exactly once to every live
recipient in this set; it does not perform an unconditional focused-session
write. This allows manual broadcast to exclude the focused pane and prevents
duplicate delivery.

The terminal/integration owner implements private adapters:

```rust
trait TerminalBackend {
    fn spawn(&mut self, id: SessionId, spec: &SessionSpec, size: (u16, u16))
        -> Result<(), TerminalError>;
    fn resize(&mut self, id: SessionId, size: (u16, u16)) -> Result<(), TerminalError>;
    fn write(&mut self, id: SessionId, bytes: &[u8]) -> Result<(), TerminalError>;
    fn screen(&self, id: SessionId) -> Option<&vt100::Screen>;
    fn drain_events(&mut self) -> Vec<TerminalEvent>;
    fn terminate(&mut self, id: SessionId) -> Result<(), TerminalError>;
}
```

`TerminalBackend` owns one reader task and parser per session. It continuously
drains every live PTY, including hidden stack members and tabs not currently
drawn, so child processes never block on an unread output pipe. Rendering reads
but never mutates `Workspace`. A size/layout change resizes only sessions that
are visible in a rendered slot; hidden sessions keep their last PTY dimensions
until selected, then are resized before their first frame.

## Rendering, input, and mouse policy

The host enters raw mode, alternate screen, bracketed-paste capture, focus
capture, and mouse capture through a `TerminalGuard`. A dedicated raw-stdin
reader sends byte chunks to the `InputRouter`; it does not use crossterm's
semantic key-event reader, since that would re-encode ordinary application
input. `TerminalGuard`'s `Drop` implementation must always restore all modes
and cursor visibility. A panic hook performs the same restoration before
reporting the panic.

For each frame, the renderer reserves a top tab row and a bottom status row, obtains layout cell
rectangles, draws rounded full borders and stack badges, then composites each visible
session's `vt100::Screen` into its rectangle. The focused slot has a distinct
border. Pane content is inset one cell on all four sides; the renderer and
mouse hit testing use the same geometry helper. The leader opens an effective
bindings popup. The top bar uses stable tab numbers and focused-pane/count.
The status row always shows command/leader state, active broadcast
scope, active tab, and pending destructive confirmation. Ratatui owns buffer
comparison and terminal-cell output. Identical composed frames (including the
cursor) skip the draw entirely. Ignored mouse motion does not schedule a draw;
hidden-session output is parsed without invalidating the visible frame.

The renderer uses an explicitly sized Ratatui viewport so composition and
output share one terminal-size sample. A size change resets that viewport and
clears the display once; ordinary frames never clear the display. Complete
updates, including the final cursor, are buffered and written inside DEC 2026
synchronized-update markers. The cursor is hidden before painting and positioned
before being shown, providing a fallback on terminals that ignore those markers.
TerminalGuard ends synchronized mode during cleanup as well as restoring the
other host modes. Guest-side synchronized-update parsing is not added by this
renderer refactor; it remains a capability of the selected terminal parser.

Ratatui 0.28.1 preserves the Rust 1.85 toolchain and vt100 0.16.2. Ratatui 0.29
pins unicode-width 0.2.0, which conflicts with vt100's >=0.2.1 requirement within
that compatible version range. Ratatui 0.28 uses unicode-width 0.1 alongside
vt100's 0.2; pane geometry remains based on vt100 cells, with CJK/combining and
wide-to-narrow replacement covered by rendering tests. Upgrading Ratatui should
include a toolchain and Unicode compatibility review.

Mouse behavior is explicit. Holding `Shift` activates local selection mode;
drag updates a per-session cell-range selection and suppresses forwarding to
the child. Releasing leaves the selection visible; the configured clipboard
helper receives it only after the copy command. Without Shift, when a screen
requests mouse reporting, the terminal adapter forwards the original raw mouse
sequence after converting host cells to slot-local cells; otherwise a plain
click focuses that pane.

## Lifecycle and failure policy

- Splitting creates a new slot and returns a `Spawn` effect. Moving, swapping,
  carrying, focusing, stack cycling, and resizing return no lifecycle effect.
- Removing a slot is non-destructive: its full stack is merged into a
  deterministic adjacent slot. Removing the only slot is refused. This is the
  initial policy subject to the interaction specification.
- Killing a session/stack requires a confirmation command. On confirmation,
  integration asks the PTY child to terminate, continues draining until EOF,
  then reports exit to the domain model. A layout mutation never invokes it.
- Carrying a slot removes its entire stack from the source tab and inserts it as
  a deterministic adjacent split to the destination tab's focused slot. The
  destination is an empty tab when it has no slot. Carrying the source's sole
  slot removes that newly empty source tab after a successful transfer. Session
  IDs and backend objects are retained.
- Child exit removes only that session from its stack. An empty pane disappears;
  an empty active tab remains until navigation leaves it. Exited session/parser
  records remain separate from visible layout state. No replacement shell is
  spawned automatically after a last-pane exit or confirmed kill.
- Numbered navigation creates a missing tab with one shell. Numbered carry
  creates a missing destination container without an extra shell. Stable tab
  numbers are distinct from stable internal IDs. An empty tab internally retains
  one empty layout placeholder; it is omitted from visible pane counts.

## Milestones and acceptance gates

| Gate | Required evidence |
| --- | --- |
| M0 — contract | This document, interaction spec, frozen types, and a resolved/locked workspace build. |
| M1 — pure modules | Unit tests cover geometry partition/minimums, spatial neighbors, resize/swap, independent tabs, stack order, full-stack carry, non-destructive removal, byte-preserving route decisions, and broadcast exclusions. |
| M2 — live terminal | Launch a shell; retain PID and scrollback across swap/carry/stack cycle; prove hidden session output is drained; verify resize, alternate screen, application cursor keys, bracketed paste, Neovim, and a local SSH test target. |
| M3 — interaction safety | Config changes bindings; leader literal input works; destructive confirmation is enforced; broadcast status is visible and recipient set is correct; Shift-selection copies while an application mouse event remains forwardable. |
| M4 — release check | `cargo fmt --check`, clippy with warnings denied, complete tests, terminal restoration after normal exit and panic, and a requirement-by-requirement acceptance record. |

M2 is a hard compatibility gate for the selected emulator. A failure is a
reported architectural decision point, not a reason to weaken the Neovim/SSH
requirements or silently claim compatibility.
