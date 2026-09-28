mod backend;
mod clipboard;
mod keyboard;
mod mouse;
mod pane_geometry;
mod raw_input;
mod render;
mod runtime_config;
mod selection;
mod terminal_guard;

use std::{
    io::{self, Read},
    path::PathBuf,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use backend::{TerminalBackend, TerminalEvent};
use clipboard::{ClipboardEnvironment, copy_with_helper};
use crossterm::terminal;
use mouse::{Frame as MouseFrame, SgrDecoder};
use mux_core::{
    BindingConfig, InputEvent, InputRoute, LifecycleEffect, PendingConfirmation, RouterContext,
    SessionSpec, SessionState, Workspace, WorkspaceCommand, plan_broadcast,
};
use raw_input::RawInput;
use runtime_config::RuntimeConfig;
use selection::Selection;
use selection::selected_text;
use std::cell::RefCell;

thread_local! { static CURRENT_SELECTION: RefCell<Option<Selection>> = const { RefCell::new(None) }; }
use terminal_guard::TerminalGuard;

const INPUT_TICK: Duration = Duration::from_millis(16);
const MOUSE_SEQUENCE_TIMEOUT: Duration = Duration::from_millis(50);
const CONFIRM_TIMEOUT: Duration = Duration::from_secs(10);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut config_path = None;
    let mut args = std::env::args().skip(1);
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--help" | "-h" => {
                println!(
                    "multiplexer [--config PATH]\n\n{}",
                    RuntimeConfig::default().effective_help()
                );
                return Ok(());
            }
            "--config" => {
                config_path = Some(PathBuf::from(
                    args.next().ok_or("--config requires a path")?,
                ))
            }
            _ => return Err(format!("unknown option: {argument}").into()),
        }
    }
    let config = RuntimeConfig::load(config_path.as_deref())?;
    let guard = TerminalGuard::enter()?;
    run(guard, config.bindings)
}

fn run(_guard: TerminalGuard, config: BindingConfig) -> Result<(), Box<dyn std::error::Error>> {
    let default_session = SessionSpec {
        program: PathBuf::from(std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())),
        args: Vec::new(),
        cwd: std::env::current_dir().ok(),
    };
    let (mut workspace, initial) = Workspace::new(default_session.clone());
    sync_bounds(&mut workspace)?;
    let mut backend = TerminalBackend::new(10_000);
    let mut status = String::new();
    apply_effects(
        &mut backend,
        initial.effects,
        terminal::size()?,
        &mut workspace,
        &mut status,
    );
    let mut router = mux_core::InputRouter::new();
    let mut raw = RawInput::default();
    let mut mouse = SgrDecoder::default();
    let mut keyboard = keyboard::KeyboardDecoder::default();
    let mut mouse_deadline = None;
    let mut selection: Option<Selection> = None;
    let mut router_deadline = None;
    let mut pending_deadline = None;
    let mut quit_deadline = None;
    let (input_tx, input_rx) = mpsc::sync_channel(64);
    thread::spawn(move || {
        let mut stdin = io::stdin().lock();
        let mut bytes = [0_u8; 4096];
        while let Ok(count) = stdin.read(&mut bytes) {
            if count == 0 || input_tx.send(bytes[..count].to_vec()).is_err() {
                break;
            }
        }
    });
    let mut renderer = render::Renderer::new()?;
    let mut dirty = true;
    loop {
        let events = backend.drain_events(256);
        let visible = if events.is_empty() {
            Vec::new()
        } else {
            let view = workspace.view();
            view.tabs
                .iter()
                .find(|tab| tab.id == view.active_tab)
                .into_iter()
                .flat_map(|tab| tab.slots.values())
                .filter_map(|slot| {
                    slot.stack
                        .active
                        .and_then(|index| slot.stack.sessions.get(index))
                        .copied()
                })
                .collect::<Vec<_>>()
        };
        for event in events {
            match event {
                TerminalEvent::Exited { session, code } => {
                    dirty = true;
                    let _ = workspace.session_state_changed(session, SessionState::Exited);
                    status = format!("session {} exited ({code})", session.0);
                }
                TerminalEvent::ReadError { session, message } => {
                    dirty = true;
                    status = format!("session {} read error: {message}", session.0)
                }
                TerminalEvent::Output { session, .. } => dirty |= visible.contains(&session),
            }
        }
        let previous_bounds = workspace.view().bounds;
        sync_bounds(&mut workspace)?;
        dirty |= workspace.view().bounds != previous_bounds;
        if dirty {
            renderer.draw(
                &workspace,
                &mut backend,
                &router,
                &config,
                &status,
                quit_deadline.is_some(),
                selection.as_ref(),
            )?;
            dirty = false;
        }
        let now = Instant::now();
        if pending_deadline.is_some_and(|deadline| now >= deadline) {
            workspace.clear_pending_confirmation();
            pending_deadline = None;
            status = "confirmation timed out; nothing was killed".into();
            dirty = true;
        }
        if quit_deadline.is_some_and(|deadline| now >= deadline) {
            quit_deadline = None;
            status = "quit confirmation timed out".into();
            dirty = true;
        }
        match input_rx.recv_timeout(INPUT_TICK) {
            Ok(chunk) => {
                for event in raw.push(&chunk) {
                    if let InputEvent::Bytes(bytes) = event {
                        for frame in mouse.push(&bytes) {
                            match frame {
                                MouseFrame::Bytes(bytes) => {
                                    for bytes in keyboard.push(&bytes) {
                                        dirty = true;
                                        if handle_event(
                                            InputEvent::Bytes(bytes),
                                            &mut workspace,
                                            &mut backend,
                                            &mut router,
                                            &config,
                                            &default_session,
                                            &mut status,
                                            &mut pending_deadline,
                                            &mut quit_deadline,
                                        )? {
                                            return Ok(());
                                        }
                                    }
                                }
                                MouseFrame::Mouse(event) => {
                                    let previous_selection = selection;
                                    let previous_status = status.clone();
                                    if event.shift && event.button == 0 {
                                        update_selection(
                                            &workspace,
                                            &backend,
                                            &event,
                                            &mut selection,
                                        )
                                    } else if workspace.view().pending_confirmation.is_none()
                                        && quit_deadline.is_none()
                                    {
                                        forward_mouse(&workspace, &backend, &event, &mut status);
                                    }
                                    dirty |= selection != previous_selection
                                        || status != previous_status;
                                }
                            }
                        }
                        continue;
                    }
                    dirty = true;
                    // A paste is an input boundary: preserve any preceding
                    // partial key before routing the opaque paste payload.
                    for bytes in keyboard.flush() {
                        if handle_event(
                            InputEvent::Bytes(bytes),
                            &mut workspace,
                            &mut backend,
                            &mut router,
                            &config,
                            &default_session,
                            &mut status,
                            &mut pending_deadline,
                            &mut quit_deadline,
                        )? {
                            return Ok(());
                        }
                    }
                    if matches!(event, InputEvent::Paste(_))
                        && (workspace.view().pending_confirmation.is_some()
                            || quit_deadline.is_some())
                    {
                        status = "paste ignored while confirmation is pending".into();
                        continue;
                    }
                    if handle_event(
                        event,
                        &mut workspace,
                        &mut backend,
                        &mut router,
                        &config,
                        &default_session,
                        &mut status,
                        &mut pending_deadline,
                        &mut quit_deadline,
                    )? {
                        return Ok(());
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if mouse_deadline.is_some_and(|deadline| now >= deadline) {
                    dirty = true;
                    for frame in mouse.flush() {
                        if let MouseFrame::Bytes(bytes) = frame {
                            if handle_event(
                                InputEvent::Bytes(bytes),
                                &mut workspace,
                                &mut backend,
                                &mut router,
                                &config,
                                &default_session,
                                &mut status,
                                &mut pending_deadline,
                                &mut quit_deadline,
                            )? {
                                return Ok(());
                            }
                        }
                    }
                    for bytes in keyboard.flush() {
                        if handle_event(
                            InputEvent::Bytes(bytes),
                            &mut workspace,
                            &mut backend,
                            &mut router,
                            &config,
                            &default_session,
                            &mut status,
                            &mut pending_deadline,
                            &mut quit_deadline,
                        )? {
                            return Ok(());
                        }
                    }
                    mouse_deadline = None;
                }
                if router_deadline.is_some_and(|deadline| now >= deadline) {
                    dirty = true;
                    for event in raw.flush_marker_prefix() {
                        if handle_event(
                            event,
                            &mut workspace,
                            &mut backend,
                            &mut router,
                            &config,
                            &default_session,
                            &mut status,
                            &mut pending_deadline,
                            &mut quit_deadline,
                        )? {
                            return Ok(());
                        }
                    }
                    let _ = handle_event(
                        InputEvent::Timeout,
                        &mut workspace,
                        &mut backend,
                        &mut router,
                        &config,
                        &default_session,
                        &mut status,
                        &mut pending_deadline,
                        &mut quit_deadline,
                    )?;
                    router_deadline = None;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
        }
        if raw.marker_prefix_pending() {
            router_deadline.get_or_insert_with(|| {
                Instant::now() + Duration::from_millis(config.alt_timeout_ms().into())
            });
        }
        if !raw.marker_prefix_pending() {
            update_router_deadline(&router, &config, &mut router_deadline);
        }
        if mouse.has_pending() || keyboard.has_pending() {
            mouse_deadline.get_or_insert_with(|| Instant::now() + MOUSE_SEQUENCE_TIMEOUT);
        } else {
            mouse_deadline = None;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_event(
    event: InputEvent,
    workspace: &mut Workspace,
    backend: &mut TerminalBackend,
    router: &mut mux_core::InputRouter,
    config: &BindingConfig,
    default_session: &SessionSpec,
    status: &mut String,
    pending_deadline: &mut Option<Instant>,
    quit_deadline: &mut Option<Instant>,
) -> Result<bool, Box<dyn std::error::Error>> {
    // InputRouter intentionally stops after a command so callers can refresh
    // workspace context. Feeding a host chunk one byte at a time gives the
    // confirmation layer first chance at bytes following that command (for
    // example Ctrl-b,q,y in one terminal read).
    if let InputEvent::Bytes(bytes) = &event {
        if bytes.len() > 1 {
            for byte in bytes {
                if handle_event(
                    InputEvent::Bytes(vec![*byte]),
                    workspace,
                    backend,
                    router,
                    config,
                    default_session,
                    status,
                    pending_deadline,
                    quit_deadline,
                )? {
                    return Ok(true);
                }
            }
            return Ok(false);
        }
    }
    if quit_deadline.is_some() {
        if let InputEvent::Bytes(bytes) = event {
            for byte in bytes {
                match byte {
                    b'y' => return Ok(true),
                    b'n' | 0x1b => {
                        *quit_deadline = None;
                        *status = "quit cancelled".into();
                    }
                    _ => *status = "confirm quit with y, cancel with n or Esc".into(),
                }
            }
        }
        return Ok(false);
    }
    if let Some(pending) = workspace.view().pending_confirmation {
        if let InputEvent::Bytes(bytes) = event {
            for byte in bytes {
                match byte {
                    b'n' | 0x1b => {
                        workspace.clear_pending_confirmation();
                        *pending_deadline = None;
                        *status = "confirmation cancelled".into();
                    }
                    b'y' => {
                        let command = match pending {
                            PendingConfirmation::CloseTab(tab) => {
                                WorkspaceCommand::ConfirmCloseTab(tab)
                            }
                            PendingConfirmation::KillSession { .. } => {
                                WorkspaceCommand::ConfirmKillFocusedSession
                            }
                            PendingConfirmation::KillStack { .. } => {
                                WorkspaceCommand::ConfirmKillFocusedStack
                            }
                        };
                        apply_command(command, workspace, backend, status);
                        *pending_deadline = None;
                    }
                    _ => *status = "confirm with y, cancel with n or Esc".into(),
                }
            }
        }
        return Ok(false);
    }
    let context = router_context(workspace, default_session);
    let pasted = matches!(&event, InputEvent::Paste(_));
    for route in router.route(event, &context, config) {
        match route {
            InputRoute::Forward(bytes) => {
                for session in
                    plan_broadcast(&workspace.view(), workspace.view().broadcast_scope).recipients
                {
                    let input = if pasted {
                        bytes.clone()
                    } else {
                        keyboard::application_cursor_key(
                            &bytes,
                            backend
                                .screen(session)
                                .is_some_and(|screen| screen.application_cursor()),
                        )
                    };
                    if let Err(error) = backend.write(session, &input) {
                        *status = format!("session {} write failed: {error}", session.0);
                    }
                }
            }
            InputRoute::Command(WorkspaceCommand::RequestQuit) => {
                *quit_deadline = Some(Instant::now() + CONFIRM_TIMEOUT);
                *status = "quit multiplexer? y/n".into();
            }
            InputRoute::Command(WorkspaceCommand::CopySelection) => {
                let text = CURRENT_SELECTION.with(|stored| {
                    stored.borrow().as_ref().and_then(|selection| {
                        backend
                            .screen(selection.session)
                            .map(|screen| selected_text(screen, selection))
                    })
                });
                match text.filter(|text| !text.is_empty()) {
                    Some(text) => match copy_with_helper(&text, &ClipboardEnvironment::default()) {
                        Ok(()) => *status = "selection copied".into(),
                        Err(error) => *status = format!("copy failed: {error}"),
                    },
                    None => *status = "no Shift selection to copy".into(),
                }
            }
            InputRoute::Command(command) => {
                apply_command(command, workspace, backend, status);
                if workspace.view().pending_confirmation.is_some() {
                    *pending_deadline = Some(Instant::now() + CONFIRM_TIMEOUT);
                }
            }
            InputRoute::Consume => {}
        }
    }
    Ok(false)
}

fn update_router_deadline(
    router: &mux_core::InputRouter,
    config: &BindingConfig,
    deadline: &mut Option<Instant>,
) {
    if router.carry_pending() || router.alt_pending() || router.has_pending_bytes() {
        deadline.get_or_insert_with(|| {
            Instant::now()
                + if router.alt_pending() {
                    Duration::from_millis(config.alt_timeout_ms().into())
                } else {
                    Duration::from_millis(config.leader_timeout_ms().into())
                }
        });
    } else {
        *deadline = None;
    }
}
fn router_context(workspace: &Workspace, default_session: &SessionSpec) -> RouterContext {
    let view = workspace.view();
    let tab = view
        .tabs
        .iter()
        .find(|tab| tab.id == view.active_tab)
        .expect("active tab");
    let mut tab_by_number = [None; 9];
    for tab in &view.tabs {
        if (1..=9).contains(&tab.number) {
            tab_by_number[tab.number - 1] = Some(tab.id);
        }
    }
    RouterContext {
        current_tab: view.active_tab,
        tab_by_number,
        focused_slot: tab.focused_slot,
        default_session: default_session.clone(),
        broadcast_scope: view.broadcast_scope,
    }
}
fn apply_command(
    command: WorkspaceCommand,
    workspace: &mut Workspace,
    backend: &mut TerminalBackend,
    status: &mut String,
) {
    match workspace.execute(command) {
        Ok(transition) => apply_effects(
            backend,
            transition.effects,
            terminal::size().unwrap_or((80, 24)),
            workspace,
            status,
        ),
        Err(error) => *status = error.to_string(),
    }
}
fn apply_effects(
    backend: &mut TerminalBackend,
    effects: Vec<LifecycleEffect>,
    size: (u16, u16),
    workspace: &mut Workspace,
    status: &mut String,
) {
    for effect in effects {
        match effect {
            LifecycleEffect::Spawn { session, spec } => {
                let state = match backend.spawn(session, &spec, size) {
                    Ok(()) => SessionState::Live,
                    Err(error) => {
                        *status = format!("could not start session {}: {error}", session.0);
                        SessionState::Exited
                    }
                };
                let _ = workspace.session_state_changed(session, state);
            }
            LifecycleEffect::Terminate { session } => {
                if let Err(error) = backend.terminate(session) {
                    *status = format!("could not terminate session {}: {error}", session.0);
                }
            }
        }
    }
}
fn sync_bounds(workspace: &mut Workspace) -> io::Result<()> {
    let (cols, rows) = terminal::size()?;
    workspace.set_bounds(pane_geometry::workspace_bounds(cols, rows));
    Ok(())
}

fn update_selection(
    workspace: &Workspace,
    backend: &TerminalBackend,
    event: &mouse::MouseEvent,
    selection: &mut Option<Selection>,
) {
    if !event.shift || event.button != 0 || event.code & 64 != 0 {
        return;
    }
    let view = workspace.view();
    let tab = view
        .tabs
        .iter()
        .find(|tab| tab.id == view.active_tab)
        .expect("active tab");
    let bounds = view.bounds;
    let Some(rect) = tab
        .layout
        .geometry(bounds, view.minimum_pane_size)
        .get(&tab.focused_slot)
        .copied()
    else {
        return;
    };
    let Some(slot) = tab.slots.get(&tab.focused_slot) else {
        return;
    };
    let rect = pane_geometry::content(pane_geometry::stack_frame(rect, &slot.stack));
    if !pane_geometry::contains(rect, event.x, event.y) {
        return;
    }
    let Some(session) = slot
        .stack
        .active
        .and_then(|index| slot.stack.sessions.get(index))
        .copied()
    else {
        return;
    };
    if backend.screen(session).is_none() {
        return;
    }
    let point = (event.x - rect.x, event.y - rect.y);
    if event.press && !event.motion {
        *selection = Some(Selection {
            session,
            start: point,
            end: point,
        });
    } else if let Some(current) = selection
        .as_mut()
        .filter(|current| current.session == session)
    {
        current.end = point;
    } else {
        return;
    }
    CURRENT_SELECTION.with(|stored| *stored.borrow_mut() = *selection);
}

fn forward_mouse(
    workspace: &Workspace,
    backend: &TerminalBackend,
    event: &mouse::MouseEvent,
    status: &mut String,
) {
    let view = workspace.view();
    let tab = view
        .tabs
        .iter()
        .find(|tab| tab.id == view.active_tab)
        .expect("active tab");
    let Some(rect) = tab
        .layout
        .geometry(view.bounds, view.minimum_pane_size)
        .get(&tab.focused_slot)
        .copied()
    else {
        return;
    };
    let Some(slot) = tab.slots.get(&tab.focused_slot) else {
        return;
    };
    let rect = pane_geometry::content(pane_geometry::stack_frame(rect, &slot.stack));
    if !pane_geometry::contains(rect, event.x, event.y) {
        return;
    }
    let Some(session) = slot
        .stack
        .active
        .and_then(|index| slot.stack.sessions.get(index))
        .copied()
    else {
        return;
    };
    let Some(screen) = backend.screen(session) else {
        return;
    };
    if screen.mouse_protocol_mode() == vt100::MouseProtocolMode::None
        || screen.mouse_protocol_encoding() != vt100::MouseProtocolEncoding::Sgr
    {
        return;
    }
    let suffix = if event.press { 'M' } else { 'm' };
    let local = format!(
        "\x1b[<{};{};{}{}",
        event.code,
        event.x - rect.x + 1,
        event.y - rect.y + 1,
        suffix
    );
    if let Err(error) = backend.write(session, local.as_bytes()) {
        *status = format!("mouse write failed: {error}");
    }
}
