mod backend;
mod clipboard;
mod keyboard;
mod layout_persistence;
mod mouse;
mod pane_geometry;
mod raw_input;
mod render;
mod runtime_config;
mod selection;
mod selection_mode;
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
use layout_persistence::LayoutStore;
use mouse::{Frame as MouseFrame, SgrDecoder};
use mux_core::{
    BindingConfig, InputEvent, InputRoute, LifecycleEffect, PendingConfirmation, RouterContext,
    SessionSpec, SessionState, Workspace, WorkspaceCommand, plan_broadcast,
};
use raw_input::RawInput;
use runtime_config::RuntimeConfig;
use selection_mode::{Outcome as SelectionOutcome, SelectionMode};

use terminal_guard::TerminalGuard;

const INPUT_TICK: Duration = Duration::from_millis(16);
const MOUSE_SEQUENCE_TIMEOUT: Duration = Duration::from_millis(50);
const CONFIRM_TIMEOUT: Duration = Duration::from_secs(10);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut config_path = None;
    let mut fresh_layout = false;
    let mut args = std::env::args().skip(1);
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--help" | "-h" => {
                println!(
                    "multiplexer [--config PATH] [--fresh]\n\n{}",
                    RuntimeConfig::default().effective_help()
                );
                return Ok(());
            }
            "--config" => {
                config_path = Some(PathBuf::from(
                    args.next().ok_or("--config requires a path")?,
                ))
            }
            "--fresh" => fresh_layout = true,
            _ => return Err(format!("unknown option: {argument}").into()),
        }
    }
    let config = RuntimeConfig::load(config_path.as_deref())?;
    let guard = TerminalGuard::enter()?;
    run(guard, config.bindings, fresh_layout)
}

fn run(
    _guard: TerminalGuard,
    config: BindingConfig,
    fresh_layout: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let default_session = SessionSpec {
        program: PathBuf::from(std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())),
        args: Vec::new(),
        cwd: std::env::current_dir().ok(),
    };
    let mut startup_status = String::new();
    let state_path = layout_persistence::default_path();
    if let Err(error) = &state_path {
        startup_status = format!("layout persistence unavailable: {error}");
    }
    let saved_layout = if fresh_layout {
        None
    } else {
        match &state_path {
            Ok(path) => match layout_persistence::load(path) {
                Ok(saved) => saved,
                Err(error) => {
                    startup_status =
                        format!("could not load saved layout: {error}; starting fresh");
                    None
                }
            },
            Err(_) => None,
        }
    };
    let (mut workspace, initial) = match saved_layout {
        Some(saved) => match Workspace::restore_layout(saved, default_session.clone()) {
            Ok(restored) => restored,
            Err(error) => {
                append_status(
                    &mut startup_status,
                    format!("saved layout ignored: {error}"),
                );
                Workspace::new(default_session.clone())
            }
        },
        None => Workspace::new(default_session.clone()),
    };
    let mut layout_store = state_path
        .ok()
        .map(|path| LayoutStore::new(path, workspace.layout_snapshot()));
    sync_bounds(&mut workspace)?;
    let mut backend = TerminalBackend::new(10_000);
    let mut status = startup_status;
    apply_effects(
        &mut backend,
        initial.effects,
        terminal::size()?,
        &mut workspace,
        &mut status,
    );
    if let Some(store) = &mut layout_store {
        if let Err(error) = store.save_initial(&workspace.layout_snapshot()) {
            append_status(&mut status, format!("could not save layout: {error}"));
        }
    }
    let mut router = mux_core::InputRouter::new();
    let mut raw = RawInput::default();
    let mut mouse = SgrDecoder::default();
    let mut keyboard = keyboard::KeyboardDecoder::default();
    let mut mouse_deadline = None;
    let mut selection_mode: Option<SelectionMode> = None;
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
                    if selection_mode
                        .as_ref()
                        .is_some_and(|mode| mode.session == session)
                    {
                        selection_mode = None;
                    }
                    dirty = true;
                    let _ = workspace.session_state_changed(session, SessionState::Exited);
                    status = format!("session {} exited ({code})", session.0);
                    save_changed_layout(&mut layout_store, &workspace, &mut status);
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
        if workspace.view().bounds != previous_bounds {
            if selection_mode.take().is_some() {
                status = "selection cancelled after terminal resize".into();
            }
            dirty = true;
        }
        if dirty {
            renderer.draw(
                &workspace,
                &mut backend,
                &router,
                &config,
                &status,
                quit_deadline.is_some(),
                selection_mode.as_ref(),
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
        let mut event_handler = EventHandler {
            workspace: &mut workspace,
            backend: &mut backend,
            router: &mut router,
            selection_mode: &mut selection_mode,
            config: &config,
            default_session: &default_session,
            layout_store: &mut layout_store,
            status: &mut status,
            pending_deadline: &mut pending_deadline,
            quit_deadline: &mut quit_deadline,
        };
        match input_rx.recv_timeout(INPUT_TICK) {
            Ok(chunk) => {
                for event in raw.push(&chunk) {
                    if let InputEvent::Bytes(bytes) = event {
                        for frame in mouse.push(&bytes) {
                            match frame {
                                MouseFrame::Bytes(bytes) => {
                                    for bytes in keyboard.push(&bytes) {
                                        dirty = true;
                                        if event_handler.handle(InputEvent::Bytes(bytes))? {
                                            return Ok(());
                                        }
                                    }
                                }
                                MouseFrame::Mouse(event) => {
                                    if event_handler.selection_mode.is_some() {
                                        continue;
                                    }
                                    let previous_status = event_handler.status.clone();
                                    if event_handler
                                        .workspace
                                        .view()
                                        .pending_confirmation
                                        .is_none()
                                        && event_handler.quit_deadline.is_none()
                                    {
                                        if scroll_mouse(
                                            event_handler.workspace,
                                            event_handler.backend,
                                            &event,
                                        ) {
                                            dirty = true;
                                        } else {
                                            forward_mouse(
                                                event_handler.workspace,
                                                event_handler.backend,
                                                &event,
                                                event_handler.status,
                                            );
                                        }
                                    }
                                    dirty |= *event_handler.status != previous_status;
                                }
                            }
                        }
                        continue;
                    }
                    dirty = true;
                    // A paste is an input boundary: preserve any preceding
                    // partial key before routing the opaque paste payload.
                    if event_handler.handle_bytes(keyboard.flush())? {
                        return Ok(());
                    }
                    if matches!(event, InputEvent::Paste(_))
                        && (event_handler
                            .workspace
                            .view()
                            .pending_confirmation
                            .is_some()
                            || event_handler.quit_deadline.is_some())
                    {
                        *event_handler.status =
                            "paste ignored while confirmation is pending".into();
                        continue;
                    }
                    if event_handler.handle(event)? {
                        return Ok(());
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if mouse_deadline.is_some_and(|deadline| now >= deadline) {
                    dirty = true;
                    for frame in mouse.flush() {
                        if let MouseFrame::Bytes(bytes) = frame {
                            if event_handler.handle(InputEvent::Bytes(bytes))? {
                                return Ok(());
                            }
                        }
                    }
                    if event_handler.handle_bytes(keyboard.flush())? {
                        return Ok(());
                    }
                    mouse_deadline = None;
                }
                if router_deadline.is_some_and(|deadline| now >= deadline) {
                    dirty = true;
                    for event in raw.flush_marker_prefix() {
                        if event_handler.handle(event)? {
                            return Ok(());
                        }
                    }
                    let _ = event_handler.handle(InputEvent::Timeout)?;
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

struct EventHandler<'a> {
    workspace: &'a mut Workspace,
    backend: &'a mut TerminalBackend,
    router: &'a mut mux_core::InputRouter,
    selection_mode: &'a mut Option<SelectionMode>,
    config: &'a BindingConfig,
    default_session: &'a SessionSpec,
    layout_store: &'a mut Option<LayoutStore>,
    status: &'a mut String,
    pending_deadline: &'a mut Option<Instant>,
    quit_deadline: &'a mut Option<Instant>,
}

impl EventHandler<'_> {
    fn handle(&mut self, event: InputEvent) -> Result<bool, Box<dyn std::error::Error>> {
        if self.selection_mode.is_none() {
            // InputRouter stops after a command, so route one byte at a time
            // to refresh workspace context for following bytes (Ctrl-b,q,y).
            if let InputEvent::Bytes(bytes) = &event {
                if bytes.len() > 1 {
                    for byte in bytes {
                        if self.handle_one(InputEvent::Bytes(vec![*byte]))? {
                            return Ok(true);
                        }
                    }
                    return Ok(false);
                }
            }
        }
        self.handle_one(event)
    }

    fn handle_one(&mut self, event: InputEvent) -> Result<bool, Box<dyn std::error::Error>> {
        let workspace = &mut *self.workspace;
        let backend = &mut *self.backend;
        let router = &mut *self.router;
        let selection_mode = &mut *self.selection_mode;
        let config = self.config;
        let default_session = self.default_session;
        let layout_store = &mut *self.layout_store;
        let status = &mut *self.status;
        let pending_deadline = &mut *self.pending_deadline;
        let quit_deadline = &mut *self.quit_deadline;
        if let Some(mode) = selection_mode.as_mut() {
            if let InputEvent::Bytes(bytes) = &event {
                match mode.input(bytes) {
                    SelectionOutcome::Continue => {}
                    SelectionOutcome::Cancel => {
                        *selection_mode = None;
                        *status = "selection cancelled".into();
                    }
                    SelectionOutcome::Yank(text) => {
                        match copy_with_helper(&text, &ClipboardEnvironment::default()) {
                            Ok(()) => {
                                *selection_mode = None;
                                *status = "selection copied".into();
                            }
                            Err(error) => {
                                *status = format!("copy failed: {error}; y retries, Esc cancels")
                            }
                        }
                    }
                }
            }
            return Ok(false);
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
                            apply_command(command, workspace, backend, status, layout_store);
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
                        plan_broadcast(&workspace.view(), workspace.view().broadcast_scope)
                            .recipients
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
                        backend.set_scrollback(session, 0);
                        if let Err(error) = backend.write(session, &input) {
                            *status = format!("session {} write failed: {error}", session.0);
                        }
                    }
                }
                InputRoute::Command(WorkspaceCommand::RequestQuit) => {
                    *quit_deadline = Some(Instant::now() + CONFIRM_TIMEOUT);
                    *status = "quit multiplexer? y/n".into();
                }
                InputRoute::Command(WorkspaceCommand::SelectionMode) => {
                    let view = workspace.view();
                    let tab = view
                        .tabs
                        .iter()
                        .find(|tab| tab.id == view.active_tab)
                        .expect("active tab");
                    let session = tab.slots.get(&tab.focused_slot).and_then(|slot| {
                        slot.stack
                            .active
                            .and_then(|index| slot.stack.sessions.get(index))
                            .copied()
                    });
                    if let Some((session, screen)) =
                        session.and_then(|id| backend.screen(id).map(|screen| (id, screen)))
                    {
                        *selection_mode = Some(SelectionMode::new(session, screen));
                        *status = String::new();
                    } else {
                        *status = "no pane buffer to select".into();
                    }
                }
                InputRoute::Command(command) => {
                    apply_command(command, workspace, backend, status, layout_store);
                    if workspace.view().pending_confirmation.is_some() {
                        *pending_deadline = Some(Instant::now() + CONFIRM_TIMEOUT);
                    }
                }
                InputRoute::Consume => {}
            }
        }
        Ok(false)
    }

    fn handle_bytes(
        &mut self,
        events: impl IntoIterator<Item = Vec<u8>>,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        for bytes in events {
            if self.handle(InputEvent::Bytes(bytes))? {
                return Ok(true);
            }
        }
        Ok(false)
    }
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
    layout_store: &mut Option<LayoutStore>,
) {
    let result = if let WorkspaceCommand::Focus(direction) = command {
        workspace.focus_at(direction, focused_cursor(workspace, backend))
    } else {
        workspace.execute(command)
    };
    match result {
        Ok(transition) => apply_effects(
            backend,
            transition.effects,
            terminal::size().unwrap_or((80, 24)),
            workspace,
            status,
        ),
        Err(error) => *status = error.to_string(),
    }
    if let Err(error) = sync_bounds(workspace) {
        append_status(status, error.to_string());
    }
    save_changed_layout(layout_store, workspace, status);
}

fn focused_cursor(workspace: &Workspace, backend: &TerminalBackend) -> Option<(u16, u16)> {
    let view = workspace.view();
    let tab = view.tabs.iter().find(|tab| tab.id == view.active_tab)?;
    let slot = tab.slots.get(&tab.focused_slot)?;
    let session = *slot.stack.sessions.get(slot.stack.active?)?;
    let (row, col) = backend.screen(session)?.cursor_position();
    let rect = *tab
        .layout
        .geometry(view.bounds, view.minimum_pane_size)
        .get(&slot.id)?;
    let inner = pane_geometry::pane_content(rect, &slot.stack, tab.borderless);
    if inner.cols == 0 || inner.rows == 0 {
        return None;
    }
    Some((
        inner.x.saturating_add(col.min(inner.cols - 1)),
        inner.y.saturating_add(row.min(inner.rows - 1)),
    ))
}

fn save_changed_layout(
    layout_store: &mut Option<LayoutStore>,
    workspace: &Workspace,
    status: &mut String,
) {
    let Some(store) = layout_store else {
        return;
    };
    if let Err(error) = store.save_if_changed(&workspace.layout_snapshot()) {
        append_status(status, format!("could not save layout: {error}"));
    }
}

fn append_status(status: &mut String, message: String) {
    if !status.is_empty() {
        status.push_str("; ");
    }
    status.push_str(&message);
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
    workspace.set_bounds(pane_geometry::workspace_bounds(cols, rows, workspace.borderless()));
    Ok(())
}

/// Wheel scrolling belongs to the pane under the pointer. Applications that
/// request mouse input retain their wheel events; Shift overrides this on the
/// normal screen. Alternate-screen applications own their own history.
fn scroll_mouse(
    workspace: &Workspace,
    backend: &mut TerminalBackend,
    event: &mouse::MouseEvent,
) -> bool {
    if !event.press || event.motion || event.code & 64 == 0 || event.button > 1 {
        return false;
    }
    let view = workspace.view();
    let tab = view
        .tabs
        .iter()
        .find(|tab| tab.id == view.active_tab)
        .expect("active tab");
    for (slot_id, rect) in tab.layout.geometry(view.bounds, view.minimum_pane_size) {
        let Some(slot) = tab.slots.get(&slot_id) else {
            continue;
        };
        let inner = pane_geometry::pane_content(rect, &slot.stack, tab.borderless);
        if !pane_geometry::contains(inner, event.x, event.y) {
            continue;
        }
        let Some(session) = slot
            .stack
            .active
            .and_then(|index| slot.stack.sessions.get(index))
            .copied()
        else {
            continue;
        };
        let Some(screen) = backend.screen(session) else {
            continue;
        };
        if screen.alternate_screen()
            || (!event.shift && screen.mouse_protocol_mode() != vt100::MouseProtocolMode::None)
        {
            return false;
        }
        let offset = if event.button == 0 {
            screen.scrollback().saturating_add(3)
        } else {
            screen.scrollback().saturating_sub(3)
        };
        backend.set_scrollback(session, offset);
        return true;
    }
    false
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
    let rect = pane_geometry::pane_content(rect, &slot.stack, tab.borderless);
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
