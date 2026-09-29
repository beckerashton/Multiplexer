# Acceptance matrix

The checks below define the Linux-first v1 acceptance boundary. Automated
tests should run the multiplexer against a PTY harness. Each harness session
has a stable multiplexer session ID, a child PID/start time, and a marker that
can be read after every operation. A passing identity check means the session
ID and child PID/start time are unchanged and the marker/scrollback remains
available; creating a replacement process is a failure even if the screen
looks identical.

| ID | Setup and action | Expected result |
| --- | --- | --- |
| PASS-01 | Start one tab and type ordinary `h`, `j`, `k`, `l`, `Esc`, and a shell command. | Every byte reaches the focused PTY unchanged; no pane command runs. |
| PASS-02 | Send default `Ctrl-b`, then `h/j/k/l` in a four-pane layout. | Focus moves left/down/up/right according to the spatial resolver; no command byte reaches the PTY. |
| PASS-03 | Send `Ctrl-b` twice, then a sentinel command that reads `Ctrl-b`. | Exactly one leader byte reaches the focused PTY; no layout change occurs. |
| PASS-04 | Open leader mode, wait beyond one second, then enter an unbound key, a command, and Escape in separate attempts. | Waiting and unbound keys keep the grouped popup open; a command executes and closes it; Escape cancels without forwarding the leader. |
| PASS-05 | Send `Ctrl-Alt-h/j/k/l` for neighboring slots containing distinct session markers. | Slot geometry/focus is swapped in the selected direction; every session ID, PID/start time, marker, and scrollback is unchanged. |
| PASS-06 | Resize each side of a split repeatedly with leader `r`, then `hjkl` / `HJKL`; use Alt-hjkl to change focus and Escape to exit. | The intended shared boundary moves one cell (five with Shift) per repeat, clamps at minimum sizes, and never changes any session identity. |
| PASS-07 | Split with `leader |` and `leader -`; record both child identities. | New sessions appear in right and below positions respectively; the original session remains the same process. |
| PASS-08 | Add two sessions with `leader a`, cycle with `[` and `]`, and select stack positions `1` through `9`. | Only the visible member changes; stack order and all session IDs/PIDs remain stable; out-of-range selection is a no-op. |
| PASS-09 | Swap a slot containing a three-session stack with a slot containing one session, then cycle both stacks. | Complete stack objects exchange positions; active indices, member order, session IDs, PIDs, and scrollback survive. |
| PASS-10 | In tab 1, carry a multi-session slot to non-empty tab 2 with `Ctrl-Alt-2`. | Tab 2's focused slot is split left/right at 50/50; existing content is left, the complete carried stack is right and focused; all identities/PIDs are unchanged. |
| PASS-11 | Carry the sole slot of a tab to an existing different tab, then carry a slot to its own tab. | The complete stack moves, the now-empty source tab is removed automatically, and every process identity survives; same-tab carry is a no-op. |
| PASS-12 | Remove a slot whose stack has three sessions beside a destination slot with two sessions. | All three source sessions append to the destination stack in order; destination active member remains active; the tree collapses deterministically and all five identities survive. |
| PASS-13 | Attempt to remove the only slot in a tab. | Removal is refused; geometry, focus, stack, scrollback, and process identities are unchanged. |
| PASS-14 | Kill the active member with `leader x`; cancel once and confirm once, including the sole-slot case. | Cancellation leaves it alive; the one confirmation terminates exactly that process and, when it is the sole slot, leaves the active tab empty until navigation removes it; no other session is affected. |
| PASS-15 | Kill a complete stack with `leader X`, cancel/timeout, then confirm. | Cancel and timeout kill nothing; confirmation terminates every member, applies the defined final-slot rule, and does not kill another slot. |
| PASS-16 | Close a tab with live sessions; cancel, then confirm. Try closing the sole top-level tab. | Cancel leaves all sessions alive; confirmation terminates only that tab's sessions and closes it; the sole tab close is always refused. |
| PASS-17 | Select tabs with `Alt-1` through `Alt-9`, including an ESC-prefixed digit sequence, while each tab has a distinct marker. | The matching tab is selected and its layout/focus/processes are preserved; the documented 50 ms ESC/Alt rule is deterministic. |
| PASS-18 | Send standalone `Esc`, fast `Esc` plus a non-digit, and `Esc` plus a delayed digit to an application. | Standalone and unbound non-digit sequences reach the PTY as application input; delayed digits do not become tab selection; disabled Alt-number parsing passes Alt-digits through while directional shortcuts remain active. |
| PASS-19 | Activate all-visible broadcast in a tab with three slots, each with a two-session stack, and one unrelated tab. Send text and paste bytes containing `Ctrl-b`. | Exactly the three visible sessions in the current tab receive identical raw bytes; hidden members, every other tab, and the multiplexer command parser receive none. |
| PASS-20 | Select two manual targets with `leader b m` while deliberately excluding the focused slot, activate `leader b B`, then switch a targeted stack member and send text. | Only explicitly marked slots receive input; the focused origin is excluded; after cycling, the currently visible member of that targeted slot receives it; hidden members do not. |
| PASS-21 | Change tabs while all-visible or manual broadcast is active, then type. | Broadcast is off, manual targets are cleared, and only the newly focused session receives input. The status banner and markers disappear. |
| PASS-22 | Remove a manually targeted slot, kill its active session, and exit another target process. | Removed/dead targets are removed from the set; if no target remains broadcast turns off with a visible reason; no input leaks to another tab. |
| PASS-23 | With broadcast active, issue focus, split, carry, kill confirmation responses, and tab commands. | Multiplexer actions and confirmations are local; none is written to target PTYs. |
| PASS-24 | Inject a raw paste payload containing newlines, `Esc`, leader bytes, and bracketed-paste markers. | The exact payload is sent once to every current target, with no added newline, leader interpretation, or marker rewriting. |
| PASS-25 | Enter leader y, navigate across scrollback with Vim-style motions and counts, select with v/V, and yank with y. Cancel a second selection with Escape while output continues. | The selected text and cursor are highlighted in a frozen buffer; copy reaches the clipboard; all mode input stays local; live output resumes after yank/cancel. |
| PASS-26 | Run copy under Wayland with `wl-copy`, under X11 with `xclip` or `xsel`, and with no helper installed. | Selected UTF-8 text reaches the available helper; fallback order works; missing helper reports an error and keeps the selection. OSC52 is never required. |
| PASS-27 | Change the configured leader and several bindings, restart, and inspect the help/status view. | New bindings are effective, duplicate bindings are rejected, the effective table is visible, and ordinary pass-through remains the default. |
| PASS-28 | Run all layout operations while a long-lived sentinel, SSH-like process, and Codex-like process continue producing output. | No focus, resize, swap, stack, carry, or removal operation restarts or disconnects a process; output and scrollback remain attributable to the same session IDs. |
| PASS-29 | Feed one raw input chunk containing a recognized focus/tab command, ordinary bytes, a second recognized command, and a paste payload. | The router emits an ordered action stream; each command is consumed, each ordinary/paste byte sequence is preserved exactly, and every forwarded segment uses the focus/tab state committed by the preceding action. |
| PASS-30 | Issue `leader q`, cancel it, then issue it again and confirm. | Cancellation leaves every session and the terminal host alive; confirmation is explicit and exits only after the quit request is accepted. |

## Review gates

The release candidate must pass every row that applies to the configured
terminal. PASS-17 and PASS-18 must be run both with the default ESC/Alt parser
and with it disabled. PASS-25 must cover scrollback, wide and combining characters, soft-wrapped
rows, clipboard failure/retry, and suppressed paste and mouse input. Clipboard helper tests
must not treat OSC52 as a passing substitute.

Any failure that shows a changed session ID, child PID/start time, or missing
scrollback after a layout operation is a release blocker. Any kill without a
visible confirmation, any broadcast byte delivered to a hidden stack member
or another tab, and any tab change that leaves broadcast active is also a
release blocker.

## Todo interaction regression (2026-09-27)

`todo_smoke` runs a fullscreen 240×67 PTY and verifies rounded borders, the
leader popup and dismissal, legacy Alt focus, CSI-u Ctrl-Alt swapping/carry,
xterm modified-key carry, tab creation on navigation, stable numbered tabs,
natural-exit empty tabs and deletion on navigation, and extended-leader quit.
`rendering_smoke` retains the 240×67 and 320×90 incremental-output checks.
