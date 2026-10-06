# Opinionated Terminal Multiplexer — Feature Set and Delegation Brief

## Purpose

Build a personal terminal multiplexer for shell work, Codex CLI sessions, and
Neovim. The design prioritizes fast keyboard-based pane management while
preserving normal application input, especially Vim and Neovim motions.

The product is not intended to be a general terminal "OS." It should remain a
small, opinionated multiplexer whose main abstraction is a tab containing a
tiling layout of live terminal panes.

## Product principles

- **Pass-through by default.** Unless an explicit multiplexer command mode or
  leader sequence or direct modified shortcut is active, ordinary keys go to the focused terminal.
  Bare `h`, `j`, `k`, and `l` must therefore remain available to Neovim and
  ordinary shells.
- **Processes are durable objects.** Moving, swapping, stacking, or changing
  focus must not restart a terminal process. This is especially important for
  long-running Codex and SSH sessions.
- **Layout and process lifecycle are separate.** Commands that alter the
  tiling tree must be distinguishable from commands that terminate a process.
- **Keyboard first; mouse is additive.** The mouse is needed for selecting and
  copying text, not as a substitute for pane management.
- **Visible state.** Active broadcast input, multiplexer mode, focused pane,
  and pane-stack position need clear but unobtrusive indicators.

## Core vocabulary and model

### Top-level tab

A tab is a top-level workspace. Each tab owns an independent tiling layout.
Selecting a tab does not stop the processes in another tab.

### Layout tree

Each tab has a resizable, binary tiling tree. Interior nodes encode a
horizontal or vertical split and a split ratio. Leaf nodes are pane slots.

### Pane slot and pane stack

A pane slot occupies one leaf of the layout tree. It can contain a **pane
stack**: one or more live terminal sessions sharing that same physical layout
position. Exactly one member is visible and receives normal input at a time.

This enables, for example, two Codex sessions from different working
directories to alternate in the same layout position without reconstructing
the layout. A small border badge should expose the active item and count (for
example, `2/4`).

### Terminal session

A terminal session is a live PTY-backed child process, its terminal state, and
its scrollback. It may be the active member of a pane stack or a non-visible
member. Moving a pane between tabs moves its complete pane stack and all live
terminal sessions.

## Required v1 feature set

### 1. Tiling, sizing, and focus

- Create multiple panes in a tab.
- Split horizontally or vertically, either within the focused pane or across the whole tab.
- Resize adjacent panes.
- Navigate to neighboring panes in the four spatial directions.
- Swap the focused pane slot with its neighboring pane in a chosen direction.
- Preserve terminal processes, scrollback, and pane stacks through resizing,
  focus changes, and swaps.

### 2. Top-level tabs

- Create, select, and close top-level tabs.
- Support at least tabs 1 through 9.
- `Alt-1` through `Alt-9` selects stable tabs 1 through 9, creating a shell
  in a missing tab. Empty tabs are removed when navigated away from.
- A top bar shows each tab number and pane count, with focused pane/total
  for the active tab. Panes have rounded borders.
- Tab selection must preserve every tab's independent layout and running
  processes.

### 3. Pane stacks

- Add terminal sessions to a focused pane slot, creating a stack when needed.
- Cycle or select the active terminal in the focused pane stack.
- Display the focused pane stack's active index and count.
- Retain the layout position when swapping the visible terminal in a stack.
- Carry a focused pane slot, including its complete stack, to a numbered tab.
  The destination tab must retain the same live processes.

### 4. Pane and process lifecycle

Provide distinct commands for the following operations:

- Create a new terminal by splitting horizontally.
- Create a new terminal by splitting vertically.
- Remove or delete a pane from the layout.
- Kill the focused terminal process.
- Kill a complete pane stack, if the final interaction design keeps this as a
  separate command.

The UI must ask for confirmation before a destructive process-kill operation,
or offer a clearly configured non-interactive policy. Layout removal must not
silently kill an unrelated long-running process.

### 5. Keyboard interaction

- `Alt-h/j/k/l` navigates pane focus; bare letters remain application input.
- `Ctrl-Alt-h/j/k/l` swaps pane slots in those directions.
- The leader opens a popup showing the effective command bindings.
- `Alt-1` through `Alt-9` switches top-level tabs directly.
- `Ctrl-Alt-1…9` carries the current pane and stack to that numbered tab.
- Provide commands for horizontal split, vertical split, pane deletion, and
  process kill.
- Keep all bindings configurable, including the leader key, because terminal
  emulators vary in how they encode `Alt` combinations.

### 6. Mouse text selection and copy

- Permit selecting visible terminal text with the mouse.
- Copy selected text to the system clipboard.
- Selection behavior should coexist predictably with terminal applications
  that use mouse input; selection should have an explicit modifier or a
  selection mode if direct interception would conflict.
- Do not make mouse-based pane management a v1 requirement.

### 7. Multi-input / broadcast typing

- Send the user's input to more than one terminal session at once.
- Initial target scopes:
  - focused pane only (normal behavior),
  - all visible panes in the current tab,
  - a manually selected set of panes.
- Broadcast targets may span pane stacks only through their currently visible
  session; hidden stack members are excluded by default.
- Broadcast mode must display a persistent, highly visible indicator.
- Broadcast input is scoped to the current top-level tab by default.

## Explicit non-goals for v1

- Full terminal emulator implementation from scratch.
- Daemon persistence, detach/reattach, shared multi-client sessions, web UI,
  SSH server, graphics-protocol passthrough, plugins, scripting, or a command
  palette.
- Floating windows and mouse-driven window management.
- Automatic semantic knowledge of Codex, SSH, or Neovim; they are ordinary
  terminal applications in v1.

## Interaction rules requiring a final decision

These are intentional open decisions, not implementation gaps:

1. **Leader/mode key.** Choose a terminal-reliable entry key that does not
   impair Neovim use.
2. **Directional-swap mapping (resolved).** Use `Ctrl-Alt-h/j/k/l`.
3. **Carry destination insertion.** When a pane is carried to a non-empty
   destination tab, define whether it splits beside the focused pane, is added
   to a stack, or uses another deterministic insertion rule. Recommended
   default: split beside the destination tab's focused pane.
4. **Layout deletion semantics.** Decide whether deleting a pane must first
   move its sessions to a neighboring pane stack, detach them, or terminate
   them after confirmation. No implicit termination is recommended.
5. **Clipboard integration.** Define supported OS/terminal environments and
   the system clipboard mechanism.
6. **Broadcast target selection UI.** Decide how panes enter or leave the
   manual target set and whether targets persist across tab changes.

## Implementation boundaries

The custom code should own layout-tree manipulation, focus resolution,
pane-stack management, command routing, tab management, and broadcast target
selection. It should reuse mature components for PTY lifecycle and terminal
emulation rather than rebuilding ANSI/VT compatibility.

TUIOS is a useful interaction reference, but is not the intended implementation
base: its scope is far broader than this feature set. Do not begin by forking
and deleting features unless a separate technical evaluation demonstrates a
clear maintenance advantage.

## Suggested delegated work packages

1. **Architecture spike:** compare a thin implementation built on existing
   PTY/terminal-emulation libraries with a configured existing multiplexer;
   recommend language, libraries, platform support, and a test strategy.
2. **Layout engine:** implement and test the binary split tree, geometry
   calculation, focus-neighbor lookup, resize, and directional swapping.
3. **Session model:** implement tabs, pane stacks, terminal-session ownership,
   carry-to-tab behavior, and non-destructive lifecycle transitions.
4. **Input router:** implement pass-through default behavior, leader/mode
   routing, binding configuration, and Alt-number tab switching.
5. **Terminal integration:** connect PTYs and terminal rendering, preserving
   live state through pane moves and stack changes.
6. **Selection and clipboard:** design and implement mouse selection/copy
   without breaking applications that need mouse events.
7. **Broadcast input:** implement target scopes, selection state, fan-out,
   and conspicuous safety indicators.
8. **End-to-end tests:** test Neovim pass-through, long-running Codex and SSH
   sessions, pane swaps, carries, stack cycling, and broadcast boundaries.
