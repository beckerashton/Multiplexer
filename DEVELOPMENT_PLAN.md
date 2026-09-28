# Multiplexer development coordination

The coordinating agent organizes development, resolves scope and interface
decisions, assigns work, and reviews evidence. It does not write implementation
code. FEATURE_SET.md is the product brief; changes must preserve its requirements.

## Model allocation

Assignments initially use the smaller hosted models exposed in this session.
These are practical task allocations, not claims about benchmark scores.

| Work package | Complexity | Assigned model | Dependency / acceptance gate |
| --- | --- | --- | --- |
| Interaction contract and acceptance matrix | Medium | Luna, high reasoning | Feature brief; complete unambiguous bindings and lifecycle rules |
| Architecture, dependency evaluation, shared interfaces | High | Terra, high reasoning | Runnable toolchain, mature terminal component, fixed ownership boundaries |
| Layout geometry, resize, spatial navigation and swaps | Medium–high | Terra, high reasoning | Architecture contract; deterministic geometry and invariant tests |
| Tabs, pane stacks, carry and non-destructive removal | High | Terra, high reasoning | Layout interface; stable session identity and no implicit process kills |
| Input decoding, configurable commands, literal input | Medium–high | Luna, high reasoning | Interaction and architecture contracts; byte-preserving routing tests |
| Broadcast target selection and fan-out policy | Medium | Luna, high reasoning | Session identifiers; no hidden sessions or cross-tab recipients |
| PTY lifecycle, rendering, event loop and integration | Very high | Terra, high reasoning | Model/input contracts; live PTY smoke tests and cleanup |
| Selection, clipboard and application mouse coexistence | High | Terra, high reasoning | Terminal integration; explicit selection modifier and copy behavior |
| User documentation and configuration examples | Low–medium | Luna, medium reasoning | Actual implemented commands; runnable examples |
| Independent acceptance review and regression checks | High | Terra, high reasoning | Integrated build; requirement-by-requirement evidence |

## Execution policy

1. Settle architecture and interaction contracts in parallel. Do not start
   incompatible implementations before their common interface is explicit.
2. Assign separate file ownership to layout/session, input/broadcast, and
   terminal/integration workers. Only the integration owner edits shared build
   files. Workers report required interface changes before editing others' files.
3. Use at most three workers concurrently. Keep tasks small enough to describe
   inputs, outputs, invariants, and verification criteria precisely.
4. Route failed tests and review findings back to their developer. The coordinator
   never patches implementation code. Escalate a bounded difficult task from Luna
   to Terra after a concrete failure, rather than silently moving it to the root.
5. Completion means implemented behavior plus verification evidence. Documentation,
   scaffolding, and passing unit tests alone do not establish a finished terminal
   multiplexer. Record unsupported behavior and untested application scenarios.

## Milestone gates

- M0: Architecture, interaction decisions, shared interfaces and acceptance cases.
- M1: Pure layout/session and input/broadcast modules with focused tests.
- M2: Integrated shell with live PTYs, rendering, tabs, stacks and pane movement.
- M3: Configurable bindings, confirmation flows, broadcast indicators, mouse copy.
- M4: Regression checks, live-session evidence, usage documentation and gap review.

## Dispatch record

- Architecture worker: Terra; architecture, toolchain and interface contract.
- Interaction worker: Luna; INTERACTION_SPEC.md and ACCEPTANCE.md.
- Domain worker: Terra; pure Rust layout/session modules and invariant tests.
- Interaction worker follow-up: Luna; raw-byte input/configuration and broadcast
  policy modules once shared types are frozen.
- Architecture worker follow-up: Terra; terminal/PTY/render/event-loop integration
  and dependency verification after publishing the scaffold.

## Current execution status

Both initial hosted workers failed before producing artifacts because the account
reported a usage limit. The reported retry time was 6:46 PM. Architecture and
interaction work are therefore blocked, not completed. No implementation
language or terminal dependency has been accepted yet.

The documented local-model evaluation supports Qwen3 4B for constrained tasks;
Qwen2.5-Coder 7B passed its coding repair but had weaker shell reliability.
Hermes 3 and GLM 4 failed the recorded coding repair, so they are not assigned
autonomous development work. This small evaluation does not establish that any
local profile can safely own full terminal integration.

Fallback dispatch: Qwen3 4B implements and tests a bounded, standalone broadcast
targeting policy in the kit-created `multiplexer-broadcast-20260919` workspace.
This candidate does not commit the product to Python and is not an integrated
multiplexer. Only the existing qwen3-4b profile is used, through the standard
network-isolated Bubblewrap runner and profile Unix socket. No model profile,
network policy, or host filesystem exposure is changed.

Fallback outcome: rejected. The local worker produced a candidate with incorrect
selected-target ordering and a failing test. Its repair pass claimed edits and
passing tests without performing either. Coordinator verification ran six tests,
with one failure. No candidate code was imported into the project. Further work
on this component is assigned to the hosted input/broadcast worker. This is why
completion gates require tool evidence rather than a model's final report.

The hosted architecture and interaction assignments were retried after the
reported reset time and are now progressing. The accepted direction is a
terminal-hosted Rust application using portable-pty, vt100 and crossterm. A native
GTK/VTE window proposal was rejected because it changes the intended product
surface. Library compatibility and actual Neovim behavior remain validation
gates; the architecture worker owns dependency/toolchain verification.

Coordinator review decisions: the app stays terminal-hosted; carrying a tab's
sole slot to another existing tab moves its complete stack and removes the now
empty source tab; ordinary layout deletion still refuses the sole slot. A leader
timeout forwards the buffered literal leader. Killing the final session requires
a prompt that explicitly includes any replacement shell. Destructive tab close
must have an explicit confirmation boundary before lifecycle effects execute.

M0 scaffold validation: architecture worker reported workspace check, test,
format and clippy passes on project-local Rust 1.85 with Cargo.lock. These are
scaffold checks, not feature acceptance. Coordinator required ordered multiple
routes per raw input chunk, contextual command resolution, and full broadcast
recipient sets so manual mode may exclude the focused pane. M1 domain and
input/broadcast implementations are dispatched; M2/M3 integration is dispatched
to the architecture worker as a follow-up.

Interim coordinator verification: `cargo test --workspace --locked` passed two
real PTY backend tests (output/PID retention and newline input delivery). Pure
module tests and application integration were still being written at this
checkpoint; compiler warnings remained. This is not an M2 acceptance pass.

Additional review findings returned to owners include preserving the destination
active stack member during merge, spatial tie-breaking and T-junction resize,
resetting broadcast on every tab transition, cleaning up empty slots after
confirmed kills, not resurrecting exited sessions when cancelling a prompt,
bounded PTY output queues, draining output tails, and a visible confirmed quit
path. Findings must be resolved with implementation and regression evidence.

## Resumed development checkpoint

After another account usage-limit interruption, the user asked to continue and
the usage window was verified available. All three hosted workers were resumed
from existing files. A runtime `RouterContext` field mismatch was found by the
coordinator's workspace test and returned to the integration owner.

The coordinator independently verified twelve pure-domain tests passing: four
layout tests and eight workspace tests. They cover partition/minimum geometry,
T-junction resize, spatial choice, swap identity, full-stack sole-source carry,
atomic refusal, non-destructive merge, confirmed kills, and broadcast/exit
invariants. Input tests and full application acceptance are still pending at
this checkpoint.

## Final integration ownership update

The domain worker completed selection and clipboard extraction/helper modules
with dedicated tests. It now owns final runtime integration (main, raw input,
rendering, PTY/guard wiring, selection/copy wiring), taking over from the
architecture worker. The architecture worker now owns independent live-PTY
runtime smoke tests and the acceptance gap review. The input worker owns
configuration/help integration support in runtime_config.rs and README drafting,
as well as its existing input/broadcast modules.

This reassignment followed repeated incomplete runtime checkpoints. Review found
ordinary bytes being buffered unnecessarily, fragmented paste-end handling,
standalone Escape not being flushed, absent visible confirmation/quit paths, and
unfinished renderer/configuration wiring. Passing component tests does not
close these findings; independent application-level tests are required.

## September 20 verification checkpoint

Luna completed input/broadcast/configuration plus runtime_config and README.
Reported evidence: 23 mux-core tests, five runtime-config tests, and focused
clippy with warnings denied. Configuration validates simultaneous key swaps,
leader/command conflicts, nine distinct Alt tab keys, and configurable leader
and Alt timeouts. Confirmed quit now passes the controlling-PTY smoke test.

The standalone-Escape smoke test was found to have its own defects: canonical
child input, file-existence polling before content arrival, and missing host-PTY
draining. The runtime owner is correcting the harness while preserving the
exact-byte deadline assertion. Final rendering/mouse integration and broader
live-session acceptance remain open; the coordinator estimated roughly 60%
verified v1 completion in response to the user's status question.

## Final stabilization handoff

After further usage-window interruptions, Luna is the final runtime owner for
main, rendering, mouse and selection integration, as well as the independent
clipboard/Neovim tests. Terra's domain worker has relinquished those files.
The architecture worker owns only the terminal-restoration test for this pass.
The coordinator still writes documentation only and returns all code findings
to developers.

Verified progress includes real Neovim edit/save/exit and shell recovery, stable
PIDs through complete-stack carry, visible-only broadcast exclusions, raw Escape,
and confirmed quit. Final stabilization must resolve live clipboard/mouse test
failures, selection highlights/new-drag anchors, and finish full lint/test gates.

## Final handoff — September 20

The delegated implementation is a runnable Linux v1 candidate. Terra completed
architecture, layout/session logic, PTY groundwork, selection helpers and
restoration tests; Luna completed input/configuration/broadcast, independent
application tests and final runtime stabilization. The coordinator authored no
implementation or test code.

After the final runtime changes, the coordinator independently verified:

- `cargo test --workspace --locked --quiet` — pass.
- `cargo fmt --all --check` — pass.
- `cargo clippy --workspace --all-targets --locked -- -D warnings` — pass.

Live tests cover ordinary input/Escape, confirmed quit, real Neovim editing and
shell recovery, PID-preserving complete-stack carry, visible-only broadcast,
Shift selection/copy through a fake helper, application mouse coordinate
translation, and normal-quit terminal restoration. Idle redraw flooding was
removed and spawn results now update session liveness.

This is a local-trial handoff, not a claim of exhaustive terminal compatibility.
Real clipboard-provider behavior, external SSH/Codex workflows, expanded manual
broadcast/paste coverage and a complete descendant-shutdown audit remain listed
in RUNTIME_ACCEPTANCE_GAPS.md. See README.md for launch/configuration and
REVIEW_QUEUE.md for the reviewed implementation findings.

### Ready implementation briefs after M0

**Layout/session worker — Terra.** Own the pure layout and session-model modules
and their tests. Implement binary splits with minimum dimensions, deterministic
neighbor selection, bounded resizing, slot swapping, independent tabs, full-stack
carry and non-destructive removal. Use stable identifiers; emit lifecycle intents
rather than spawning or killing processes. Gate: test partition geometry,
degenerate terminal sizes, empty source tabs, same-tab carry, full-stack identity,
last-slot removal and invalid operations. Depends on accepted shared interfaces.

**Input/broadcast worker — Luna.** Own input decoding, command bindings,
configuration parsing, routing policy and tests. Preserve ordinary bytes, define
incremental escape-sequence decoding, handle literal leader and Alt-number
ambiguity, route paste predictably, and exclude hidden/stale/cross-tab recipients.
Gate: byte-for-byte pass-through cases, chunked escape sequences, command
consumption, binding conflicts and target transitions. Depends on accepted
interaction spec and session identifiers. Split this into smaller tasks if a
single worker cannot keep decoding and command state independently testable.

**Terminal/integration worker — Terra.** Own the executable, dependency manifest,
PTY and emulator adapter, render/event loop, mouse selection and clipboard
adapter. Connect model and routing interfaces without moving process ownership
into layout code. Handle resize signals, child exit, continued draining of hidden
sessions, shell teardown and terminal restoration after errors. Gate: live PTY
tests preserving PID across carry/swap/stack selection; visible broadcast and
confirmation states; alternate-screen/mouse/paste checks; copy through the chosen
Linux clipboard integration. Treat clipboard and terminal integration as separate
bounded assignments even if the same worker handles them sequentially.

**Independent acceptance worker — Terra.** After integration, review the feature
brief against actual behavior and run the agreed test suite. Check terminal state
restoration and process cleanup as well as unit invariants. Use local stand-ins
for long-running sessions where external SSH credentials are unavailable; report
real Codex/Neovim/SSH scenarios as unverified unless actually exercised. Return
findings to implementation owners, then rerun only affected checks.

**Documentation worker — Luna.** Once commands are stable, write installation,
launch, configuration, key reference, lifecycle semantics, clipboard requirements
and known limitations from the implemented behavior. Do not advertise pending
features as complete.
