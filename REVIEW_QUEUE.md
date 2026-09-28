# Coordinator review queue

This is a working checklist, not a release claim. The coordinator does not edit
implementation files; each finding goes to a developer for repair and evidence.

## Runtime owner: interaction worker (Luna), following Terra handoff

- [x] Use configured leader timeout in the running event loop.
- [x] Verify stable deadlines for raw Escape prefixes and router state; do not
  reset deadlines every redraw or require the input stream to go idle forever.
- [x] Preserve agreed bracketed-paste framing and never interpret pasted bytes
  as commands or confirmation answers.
- [x] Display destructive scope/count and replacement-shell consequences;
  cancellation must stop the pending operation even with more bytes in the read.
- [x] Report failed spawns as exited/unavailable sessions, successful starts as
  live, and keep failures local to the affected session.
- [x] Clip headers/status/content to pane bounds, retain VT colors/styles and
  wide-cell behavior, and honor focused child cursor visibility.
- [x] Avoid redundant resize signals, handle tiny windows, and keep hidden
  sessions draining.
- [x] Wire Shift-drag selection, highlight and clipboard copy; translate ordinary
  mouse reports for the focused child without broadcasting them.
- [x] Verify normal quit restores terminal modes.
- [ ] Audit every descendant-process shutdown case; normal quit and backend
  child lifecycle have tests, but this broader audit remains unverified.
- [x] Pass formatting, workspace tests and clippy without warnings after the
  final runtime changes; independently rerun by the coordinator.

## Independent live tests: interaction worker (Luna)

- [x] Check all-visible input reaches current visible sessions only.
- [x] Check hidden stack members and other tabs do not receive broadcast bytes.
- [x] Check full-stack carry and cycling preserve real shell PIDs.
- [x] Drain outer PTY output, wait on observable readiness, and clean up/reap
  children on every failure path.

Coordinator reran `cargo test -p multiplexer --test live_sessions --locked`:
passed. This verifies actual shell PID and delivery behavior, not just model IDs.

## Completion review

- [x] Update RUNTIME_ACCEPTANCE_GAPS.md to distinguish verified behavior from
  remaining limitations; old blocked findings are not automatically current.
- [x] Run a real isolated Neovim smoke test if available; do not label local
  stand-ins as verified external SSH or Codex sessions.
- [x] Ensure README matches the shipped commands and reports any unmet gates.

Independent owner evidence: `neovim_smoke` passes an actual Neovim
insert/Escape/save/exit plus shell-recovery workflow. `terminal_restore` passes
normal-quit termios/control-character and alternate-screen restoration. External
SSH and Codex workflows are not represented as verified by those tests.
