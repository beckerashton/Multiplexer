# Interaction specification

This is the Linux-first v1 contract for the terminal-hosted multiplexer. The
router receives normalized terminal key, paste, and mouse events together with
their original byte payload when one exists, and returns either a multiplexer
command or a pass-through event. Legacy application input and paste retain their original bytes. Recognized
host CSI-u/modifyOtherKeys control reports are normalized to legacy input for
hosted applications. Unknown reports remain opaque. Direct modified shortcuts
are consumed by the multiplexer; bare h/j/k/l remain application input.

## State and routing

The default leader is `Ctrl-b` (byte `0x02`). It is configurable, as are all
bindings in the table below. Pressing the leader displays a popup grouped by
command family. Leader mode stays active until a bound command is entered or
Escape is pressed; it has no timeout. Unbound keys are consumed and keep the
popup open. `leader leader` sends one literal leader byte to the application
and exits leader mode. Pasted content remains application input and does not
execute commands or dismiss leader mode.


`Esc` while the single leader is pending dismisses the popup and is consumed. `Esc` in a numbered carry prompt or another explicit
submode cancels that submode and is consumed. Outside a multiplexer command,
`Esc` is ordinary application input.

The router recognizes the default bindings from normalized terminal events,
not from a desktop key-event API. In a byte-only terminal path, uppercase
commands therefore require the terminal to send the corresponding uppercase
byte. Unknown CSI, kitty-keyboard, Alt, and paste sequences are opaque and
are forwarded with their original bytes. Applications receive no bytes while
a recognized multiplexer command is being entered.

### Default bindings

| Input | Action | Result and boundary |
| --- | --- | --- |
| `Alt-1` … `Alt-9` | Select tab 1 … 9 | Creates a missing numbered tab with one shell; clears broadcast state on a tab change. |
| `leader leader` | Literal leader | Writes one leader byte to the current normal target set. |
| `Alt-h/j/k/l` | Focus left/down/up/right | Up/down visits stack members before the spatial neighbor. Up/down at an outer edge wraps to the opposite edge in the same tab and column; a full-height stack cycles its own members. Left/right at an outer edge wraps to the previous/next populated tab. Empty tabs are skipped. Entry preserves the focused pane and displayed member. |
| `Ctrl-Alt-h/j/k/l` | Move displayed member | Up/down reorders within the stack; at a spatial boundary, swaps displayed sessions with the neighbor and follows the moved session. |
| `leader` then `Ctrl-Alt-h/j/k/l` | Swap whole stack | Swaps slots with the spatial neighbor, preserving stack order and active member. |
| `leader Ctrl-h/j/k/l` | Resize focused pane left/down/up/right | Moves the selected shared boundary by one cell (5% when cell geometry is unavailable); clamps to minimum sizes. |
| `leader \|` | Split vertically | Creates a left/right split at the focused slot, with the new terminal on the right. |
| `leader -` | Split horizontally | Creates a top/bottom split at the focused slot, with the new terminal below. |
| `leader a` | Add terminal to stack | Starts one session in the focused slot without changing the layout tree; the new session becomes active. |
| `Alt-[` / `Alt-]` | Previous/next pane | Traverses layout order, skipping hidden members. At either end, visits the previous/next populated tab; with only one populated tab, wraps locally. Retains destination focus and displayed member. |
| `leader 1` … `leader 9` | Select stack member 1 … 9 | Selects by one-based stack position; an out-of-range number is a no-op with status feedback. |
| `leader d` | Remove focused layout slot | Merges its complete stack into a deterministic neighboring slot; it never kills a session. Refuses when the tab has one slot. |
| `leader x` | Kill focused session | Always asks for explicit confirmation, then kills only the active stack member if confirmed. |
| `leader X` | Kill complete stack | Always asks for explicit confirmation, then kills every session in that slot if confirmed. |
| `leader t` | Create tab | Creates and selects a new tab with one terminal session. |
| `leader w` | Close tab | Sole tab is refused. Any live session requires confirmation; confirmation terminates sessions in that tab and closes it. |
| `Ctrl-Alt-1` … `Ctrl-Alt-9` | Carry displayed member to tab 1 … 9 | Moves one session into its own pane; creates a missing destination without an extra shell. |
| `leader` then `Ctrl-Alt-1` … `Ctrl-Alt-9` | Carry whole stack | Moves the complete stack, retaining its active member. |
| `leader b` | Toggle all-visible broadcast | Targets the visible session in every slot of the current tab, including the focused slot. |
| `leader B` | Toggle manual broadcast | Activates the current tab's explicitly selected target slots; refuses activation with an empty set. |
| `leader m` | Toggle focused slot in manual set | Adds/removes that slot's currently visible session from the manual set. The set is visibly counted. |
| `leader r` | Reset broadcast | Deactivates broadcast and clears all manual targets in the current tab. |
| `leader y` | Copy selection | Runs the configured local clipboard helper for the current selection; no selection is a no-op with status feedback. |
| `leader q` | Request quit | Starts an explicit confirmation before leaving the multiplexer and terminating its sessions. |

The configuration must reject duplicate command bindings and must display the
effective binding table in the status/help view. `Alt-1` through `Alt-9` are
configurable defaults, but the implementation must retain a way to select all
nine tabs.

The spatial neighbor resolver first chooses the candidate sharing the greatest
boundary with the focused slot, then the candidate with the nearest center,
then layout traversal order. Focus, swap, and removal use this same ordering,
so ties are deterministic.

### Routing integration contract

The input boundary must process each raw read left to right and expose an
ordered action stream (`Vec<InputRoute>` or an equivalent drainable queue). A
single read may contain several leader commands, literal pass-through bytes,
and a paste payload. The stream preserves their order: recognized command
bytes are consumed, opaque bytes are forwarded exactly, and bytes after a
focus/tab command use the newly committed focus/tab state. When a command is
reached, the router may return that command with the remainder retained in its
incremental decoder; integration applies it, refreshes context, and drains the
remainder with the fresh context before accepting another raw chunk. In the
frozen Rust shape, `InputEvent::{Bytes, Paste, Timeout}` enters
`InputRouter::route(event, RouterContext, BindingConfig)` and returns
`InputRoute::{Forward(Vec<u8>), Command(WorkspaceCommand), Consume}`. Explicit
tab and slot targets resolve to stable IDs when the command is committed;
ordinary focus-relative commands execute against the current workspace focus.

The broadcast worker receives committed raw bytes plus the current tab ID and
origin session ID and returns the complete recipient session-ID set for that
action. It must not unconditionally add the origin: a manual set may
deliberately exclude the focused session. All-visible scope resolves every
visible slot in the supplied tab; manual scope resolves only its marked slot
IDs. A command action is never sent to the broadcast worker.

## Terminal byte ambiguities

Legacy Alt shortcuts use an Escape prefix. The 50 ms configurable timeout
separates standalone Escape from a shortcut or incomplete escape sequence.
Alt-h/j/k/l focuses panes and stack members; Escape followed by Ctrl-h/j/k/l
moves the displayed member. A preceding leader promotes member movement/carry
to whole-stack movement/carry. Legacy Alt-[ resolves after the Alt timeout;
complete CSI sequences still pass through. Alt-] jumps immediately.
Alt-1…9 selects numbered tabs unless `alt_numbers` is disabled. Other
sequences remain application input. Paste bypasses shortcut decoding.

The host requests Kitty's disambiguate-escape-codes keyboard flag and restores
the previous mode on exit. CSI-u and xterm modifyOtherKeys reports are accepted,
including Ctrl-Alt-digits. Modifier 7 represents Ctrl+Alt: Ctrl-Alt-1 is
`ESC [ 49 ; 7 u` or `ESC [ 27 ; 7 ; 49 ~`. Legacy terminals that encode a
modified digit identically to Alt-digit require a terminal key mapping or a
configured alternative shortcut. Recognized extended control reports such as
Ctrl-b and Escape are normalized before confirmation/leader handling.

## Layout and lifecycle semantics

A nonempty tab has one or more pane slots, each with one or more terminal
sessions and exactly one active member. An empty active tab remains visible
until navigation leaves it, at which point it is removed. Navigating to a
missing numbered tab creates one shell; tab numbers do not shift after deletion. Focus, resizing, swapping, stack
cycling, and tab selection preserve each session's PTY, process, terminal
state, scrollback, and stable session ID.

### Splits, stacks, and swaps

Splitting starts a new terminal session in the new slot. `leader a` starts a
new session in the existing slot instead. Stack order is stable except when a
session is explicitly killed or a slot is removed. A swap exchanges slot
objects, including their complete stacks and geometry ownership; it does not
restart or reorder the sessions within either stack.

### Carry

Carry moves the focused slot as one object, including every session in its
stack, from the current tab to the chosen tab. Carrying to the current tab is
a no-op that leaves focus unchanged. Carrying the sole slot of a tab to a
different tab succeeds: after the move, the now-empty source tab is removed.
All carried process identities remain alive. A missing destination is created
without spawning a shell; an empty destination receives the carried layout
slot as its sole pane. A nonempty destination splits its focused pane
vertically at 50/50, inserting the carried slot to the right and focusing it.
The source layout collapses the removed leaf. A same-tab carry is a no-op.

### Layout removal

Removal is a layout operation, never a process operation. Before changing the
tree, the router selects the deterministic spatial neighbor described above.
It appends every session from the removed slot to the destination slot in
their existing stack order and leaves the destination's active member active.
It then removes the leaf and collapses empty split ancestors. All source and
destination session IDs remain alive. Removal of the tab's sole slot is
refused and leaves focus, geometry, and processes unchanged.

### Kills and tab close

`leader x`, `leader X`, `leader w`, and `leader q` use a visible confirmation prompt that
names the affected session count and command/working-directory label when
available. `y` confirms and `n` or `Esc` cancels; neither cancellation nor a
prompt timeout kills anything. There is no non-interactive kill policy in v1.
An individual kill removes only that session; if it was the last member of a
stack, the pane is removed. Killing or exiting the last session leaves the
active tab empty; no replacement shell is spawned. Leaving that tab removes
it. Leader `|`, `-`, or `a` starts a shell within an empty tab. A stack kill
follows the same rule. Closing a
tab with live sessions confirms once for the complete tab and then terminates
all of its sessions. Closing the sole top-level tab is always refused.

## Tabs and broadcast scope

Tabs are independent layout roots. Selecting or creating a tab leaves all
other tabs' sessions running and preserves their focus, stack indices, and
geometry. Any tab change—`Alt-number`, tab creation, carry completion to a
different tab, or tab close—turns broadcast off and clears that tab's manual
target set before focus changes are exposed.

Normal input targets only the focused visible session. In all-visible mode,
each pane slot in the current tab contributes its visible stack member exactly
once. Hidden stack members, every session in another tab, and sessions in
removed or closed slots are excluded.

Manual targets are explicit pane-slot selections made with `leader m`. The
selection is displayed on each target slot and with a count in the status bar.
The set is scoped to the current tab, is cleared on any tab change, and is
resolved to the slot's currently visible session at send time. Thus cycling a
targeted stack deliberately changes which visible session receives subsequent
input; hidden members never receive broadcast by default. The focused slot is
not implicitly added, so a manual set can intentionally exclude the origin
session. Removing a targeted slot removes it from the set. `leader r` is the
visible reset path.

Broadcast mode has a persistent, high-contrast status-bar banner such as
`BROADCAST: ALL VISIBLE (3)` or `BROADCAST: MANUAL (2)`. A target marker is
also drawn on manually selected slots. Multiplexer commands, confirmation
responses, and leader sequences are routed only to the multiplexer; they are
never broadcast. If the last active target exits, broadcast turns off and the
status reports why.

Paste is routed as one raw byte payload to every current target. The router
does not interpret leader bytes inside the payload, add line endings, or
rewrite bracketed-paste markers. Each PTY's terminal state determines how its
application handles that byte sequence. A paste while a single leader is
pending cancels the pending leader and sends the complete payload normally.

## Mouse selection and clipboard

Pane management is keyboard-only in v1. Ordinary mouse reports, including
wheel and application drag events, pass to the focused terminal application.
When the terminal supplies SGR mouse reports with Shift held, Shift plus a
left-button drag enters multiplexer text selection and suppresses those mouse
reports from the application for the drag. Releasing the drag leaves the
selection visible; `leader y` copies it. Terminals that intercept Shift-drag
for their own selection continue to provide their native selection behavior.

On Wayland, copy uses a configured local helper, defaulting to `wl-copy`; on
X11 it tries `xclip` and then `xsel`. The helper receives UTF-8 selected text
on standard input. Missing helpers produce an explicit status error while
leaving the selection intact. OSC52 clipboard transport is deferred for a
later version and is not silently attempted. Paste remains the host
terminal's normal paste event and follows the raw-paste routing rule above.

## Indicators and platform limits

Panes use rounded full borders, with the focused border highlighted. The top
row lists stable tab numbers and one square per pane, using `1 ▫▫▫▫ │ 2 ▪▫`
when tab 2 pane 1 of 2 is focused. Empty tabs show `∅`. Pane indicators follow
layout traversal order. Stacks show numbered collapsed edges above the expanded
member for earlier sessions and below it for later sessions. Edges are limited
on short panes to preserve at least one content row when space permits. The
bottom row shows status and broadcast scope. A leader popup displays the
effective bindings in columns and hides the application cursor until dismissed.

The UI always exposes the focused pane, stack position (`2/4` style), pending
leader/carry/confirmation state, and active broadcast scope. A pending
leader or confirmation prompt is visibly distinct from pass-through mode.

The v1 support target is Linux terminals that provide UTF-8, SGR mouse
reports, and a normal PTY. Wayland/X11 clipboard helpers are optional runtime
dependencies. The specification does not assume a desktop key-event API,
OSC52 support, a particular terminal emulator, or a second input client.
