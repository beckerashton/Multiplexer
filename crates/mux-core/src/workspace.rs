use std::collections::{BTreeMap, BTreeSet};

use crate::{
    Axis, BroadcastScope, CellRect, Direction, LayoutError, LayoutRemoval, LayoutTree,
    LifecycleEffect, Session, SessionId, SessionSpec, SessionState, SlotId, SlotStack,
    SlotStackView, TabId, WorkspaceCommand,
};

pub const DEFAULT_BOUNDS: CellRect = CellRect {
    x: 0,
    y: 0,
    cols: 80,
    rows: 24,
};
pub const MIN_PANE_SIZE: CellRect = CellRect {
    x: 0,
    y: 0,
    cols: 8,
    rows: 3,
};
pub const DEFAULT_SPLIT_RATIO: u16 = 500;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Transition {
    pub effects: Vec<LifecycleEffect>,
    pub changed: bool,
}

impl Transition {
    fn unchanged() -> Self {
        Self {
            effects: Vec::new(),
            changed: false,
        }
    }
    fn changed() -> Self {
        Self {
            effects: Vec::new(),
            changed: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PendingConfirmation {
    CloseTab(TabId),
    KillSession {
        tab: TabId,
        slot: SlotId,
        session: SessionId,
    },
    KillStack {
        tab: TabId,
        slot: SlotId,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DomainError {
    UnknownTab(TabId),
    InvalidTabNumber(usize),
    UnknownSlot(SlotId),
    UnknownSession(SessionId),
    InvalidStackIndex(usize),
    NoManualBroadcastTargets,
    FinalSlotRemoval,
    FinalTabClose,
    PendingConfirmationMismatch,
    Layout(LayoutError),
}

impl std::fmt::Display for DomainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidTabNumber(number) => write!(f, "invalid tab number {number}; use 1-9"),
            Self::UnknownTab(tab) => write!(f, "unknown tab {}", tab.0),
            Self::UnknownSlot(slot) => write!(f, "unknown slot {}", slot.0),
            Self::UnknownSession(session) => write!(f, "unknown session {}", session.0),
            Self::InvalidStackIndex(index) => write!(f, "invalid stack member index {index}"),
            Self::NoManualBroadcastTargets => {
                f.write_str("manual broadcast has no eligible targets")
            }
            Self::FinalSlotRemoval => f.write_str("the final pane slot cannot be removed"),
            Self::FinalTabClose => f.write_str("the final tab cannot be closed"),
            Self::PendingConfirmationMismatch => {
                f.write_str("no matching destructive action is pending confirmation")
            }
            Self::Layout(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for DomainError {}
impl From<LayoutError> for DomainError {
    fn from(value: LayoutError) -> Self {
        Self::Layout(value)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Tab {
    id: TabId,
    number: usize,
    layout: LayoutTree,
    focused_slot: SlotId,
    stacks: BTreeMap<SlotId, SlotStack>,
    manual_broadcast_targets: BTreeSet<SlotId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SlotView {
    pub id: SlotId,
    pub stack: SlotStackView,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TabView {
    pub id: TabId,
    pub number: usize,
    pub focused_slot: SlotId,
    pub layout: LayoutTree,
    pub slots: BTreeMap<SlotId, SlotView>,
    pub manual_broadcast_targets: BTreeSet<SlotId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceView {
    pub active_tab: TabId,
    pub tabs: Vec<TabView>,
    pub bounds: CellRect,
    pub minimum_pane_size: CellRect,
    pub broadcast_scope: BroadcastScope,
    pub pending_confirmation: Option<PendingConfirmation>,
    pub sessions: BTreeMap<SessionId, SessionState>,
}

/// A pure owner of tiling, tabs, session identity, and stack order. It never
/// starts or terminates a process itself; callers apply returned effects.
#[derive(Clone, Debug)]
pub struct Workspace {
    default_session: SessionSpec,
    next_tab: u64,
    next_slot: u64,
    next_session: u64,
    tabs: Vec<Tab>,
    active_tab: TabId,
    sessions: BTreeMap<SessionId, Session>,
    bounds: CellRect,
    minimum_pane_size: CellRect,
    broadcast_scope: BroadcastScope,
    pending_confirmation: Option<PendingConfirmation>,
}

impl Workspace {
    /// Creates the initial tab and returns the spawn request for its shell.
    pub fn new(default_session: SessionSpec) -> (Self, Transition) {
        let tab_id = TabId(1);
        let slot_id = SlotId(1);
        let session_id = SessionId(1);
        let session = Session::new(session_id, default_session.clone());
        let tab = Tab {
            id: tab_id,
            number: 1,
            layout: LayoutTree::new(slot_id),
            focused_slot: slot_id,
            stacks: BTreeMap::from([(slot_id, SlotStack::with_session(session_id))]),
            manual_broadcast_targets: BTreeSet::new(),
        };
        (
            Self {
                default_session: default_session.clone(),
                next_tab: 2,
                next_slot: 2,
                next_session: 2,
                tabs: vec![tab],
                active_tab: tab_id,
                sessions: BTreeMap::from([(session_id, session)]),
                bounds: DEFAULT_BOUNDS,
                minimum_pane_size: MIN_PANE_SIZE,
                broadcast_scope: BroadcastScope::Focused,
                pending_confirmation: None,
            },
            Transition {
                effects: vec![LifecycleEffect::Spawn {
                    session: session_id,
                    spec: default_session,
                }],
                changed: true,
            },
        )
    }

    pub fn set_bounds(&mut self, bounds: CellRect) {
        self.bounds = bounds;
    }

    pub fn set_minimum_pane_size(&mut self, minimum: CellRect) {
        self.minimum_pane_size = CellRect {
            cols: minimum.cols.max(1),
            rows: minimum.rows.max(1),
            ..minimum
        };
    }

    pub fn session_state_changed(
        &mut self,
        session: SessionId,
        state: SessionState,
    ) -> Result<(), DomainError> {
        let changed_session = session;
        self.sessions
            .get_mut(&changed_session)
            .ok_or(DomainError::UnknownSession(changed_session))?
            .state = state;
        if self
            .pending_confirmation
            .is_some_and(|pending| match pending {
                PendingConfirmation::KillSession {
                    session: pending, ..
                } => pending == changed_session,
                PendingConfirmation::KillStack { tab, slot } => self
                    .tab(tab)
                    .and_then(|tab| tab.stacks.get(&slot))
                    .is_some_and(|stack| stack.sessions().contains(&changed_session)),
                PendingConfirmation::CloseTab(_) => false,
            })
        {
            self.pending_confirmation = None;
        }
        if state == SessionState::Exited {
            let mut emptied = Vec::new();
            for tab in &mut self.tabs {
                for (slot, stack) in &mut tab.stacks {
                    if stack.remove(session) && stack.is_empty() {
                        emptied.push((tab.id, *slot));
                    }
                }
            }
            for (tab, slot) in emptied {
                self.reconcile_empty_slot(tab, slot, &mut Vec::new())?;
            }
        }
        self.prune_manual_broadcast_targets();
        Ok(())
    }

    pub fn clear_pending_confirmation(&mut self) -> bool {
        self.pending_confirmation.take().is_some()
    }

    pub fn execute(&mut self, command: WorkspaceCommand) -> Result<Transition, DomainError> {
        match command {
            WorkspaceCommand::SplitFocused { axis, session } => self.split_focused(axis, session),
            WorkspaceCommand::AddToFocusedStack { session } => self.add_to_focused_stack(session),
            WorkspaceCommand::Focus(direction) => self.focus(direction),
            WorkspaceCommand::CyclePane { delta } => self.cycle_pane(delta),
            WorkspaceCommand::Resize { direction, cells } => self.resize(direction, cells),
            WorkspaceCommand::Swap(direction) => self.swap(direction),
            WorkspaceCommand::SwapMember(direction) => self.swap_member(direction),
            WorkspaceCommand::SelectStackMember { index } => self.select_stack_member(index),
            WorkspaceCommand::CycleStack { delta } => {
                let focused = self.active_tab_ref().focused_slot;
                Ok(
                    if self
                        .active_tab_mut()
                        .stacks
                        .get_mut(&focused)
                        .expect("focused stack exists")
                        .cycle(delta)
                    {
                        Transition::changed()
                    } else {
                        Transition::unchanged()
                    },
                )
            }
            WorkspaceCommand::CreateTab { session } => self.create_tab(session),
            WorkspaceCommand::SelectTab(tab) => self.select_tab(tab),
            WorkspaceCommand::NavigateTab { number } => self.navigate_tab(number),
            WorkspaceCommand::CarryToTab { number } => self.carry_to_number(number, false),
            WorkspaceCommand::CarryMemberToTab { number } => self.carry_to_number(number, true),
            WorkspaceCommand::RequestCloseTab(tab) => self.request_close_tab(tab),
            WorkspaceCommand::ConfirmCloseTab(tab) => self.confirm_close_tab(tab),
            WorkspaceCommand::CarryFocusedSlot { destination } => {
                self.carry_focused_slot(destination)
            }
            WorkspaceCommand::RemoveFocusedSlot => self.remove_focused_slot(),
            WorkspaceCommand::RequestKillFocusedSession => self.request_kill_focused_session(),
            WorkspaceCommand::ConfirmKillFocusedSession => self.confirm_kill_focused_session(),
            WorkspaceCommand::RequestKillFocusedStack => self.request_kill_focused_stack(),
            WorkspaceCommand::ConfirmKillFocusedStack => self.confirm_kill_focused_stack(),
            WorkspaceCommand::SetBroadcastScope(scope) => {
                if scope == BroadcastScope::Manual
                    && self.active_tab_ref().manual_broadcast_targets.is_empty()
                {
                    return Err(DomainError::NoManualBroadcastTargets);
                }
                if self.broadcast_scope == scope {
                    Ok(Transition::unchanged())
                } else {
                    self.broadcast_scope = scope;
                    Ok(Transition::changed())
                }
            }
            WorkspaceCommand::ToggleManualBroadcastTarget(slot) => {
                self.toggle_manual_broadcast_target(slot)
            }
            WorkspaceCommand::ResetBroadcast => {
                let changed = self.reset_broadcast_state();
                Ok(if changed {
                    Transition::changed()
                } else {
                    Transition::unchanged()
                })
            }
            // Clipboard selection is terminal/UI state. The command is a
            // deliberate no-op in the pure workspace model.
            WorkspaceCommand::SelectionMode => Ok(Transition::unchanged()),
            // Exit confirmation belongs to the terminal host because only it
            // owns raw mode, alternate screen restoration, and PTY teardown.
            WorkspaceCommand::RequestQuit => Ok(Transition::unchanged()),
        }
    }

    pub fn view(&self) -> WorkspaceView {
        WorkspaceView {
            active_tab: self.active_tab,
            tabs: self
                .tabs
                .iter()
                .map(|tab| TabView {
                    id: tab.id,
                    number: tab.number,
                    focused_slot: tab.focused_slot,
                    layout: tab.layout.clone(),
                    slots: tab
                        .stacks
                        .iter()
                        .filter(|(_, stack)| !stack.is_empty())
                        .map(|(id, stack)| {
                            (
                                *id,
                                SlotView {
                                    id: *id,
                                    stack: stack.view(),
                                },
                            )
                        })
                        .collect(),
                    manual_broadcast_targets: tab.manual_broadcast_targets.clone(),
                })
                .collect(),
            bounds: self.bounds,
            minimum_pane_size: self.minimum_pane_size,
            broadcast_scope: self.broadcast_scope,
            pending_confirmation: self.pending_confirmation,
            sessions: self
                .sessions
                .iter()
                .map(|(id, session)| (*id, session.state))
                .collect(),
        }
    }

    fn split_focused(&mut self, axis: Axis, spec: SessionSpec) -> Result<Transition, DomainError> {
        if self
            .active_tab_ref()
            .stacks
            .values()
            .all(SlotStack::is_empty)
        {
            return self.add_to_focused_stack(spec);
        }
        let candidate = SlotId(self.next_slot);
        let focused = self.active_tab_ref().focused_slot;
        let mut proposed = self.active_tab_ref().layout.clone();
        proposed.split(focused, axis, DEFAULT_SPLIT_RATIO, candidate)?;
        if !self.layout_fits(&proposed) {
            return Err(DomainError::Layout(LayoutError::MinimumSize));
        }
        let new_slot = self.allocate_slot();
        let new_session = self.allocate_session(spec.clone());
        let tab = self.active_tab_mut();
        tab.layout
            .split(tab.focused_slot, axis, DEFAULT_SPLIT_RATIO, new_slot)?;
        tab.stacks
            .insert(new_slot, SlotStack::with_session(new_session));
        tab.focused_slot = new_slot;
        Ok(Transition {
            effects: vec![LifecycleEffect::Spawn {
                session: new_session,
                spec,
            }],
            changed: true,
        })
    }

    fn add_to_focused_stack(&mut self, spec: SessionSpec) -> Result<Transition, DomainError> {
        let session = self.allocate_session(spec.clone());
        let focused = self.active_tab_ref().focused_slot;
        self.active_tab_mut()
            .stacks
            .get_mut(&focused)
            .expect("focused stack exists")
            .add(session);
        Ok(Transition {
            effects: vec![LifecycleEffect::Spawn { session, spec }],
            changed: true,
        })
    }

    fn focus(&mut self, direction: Direction) -> Result<Transition, DomainError> {
        let tab = self.active_tab_ref();
        let stack = &tab.stacks[&tab.focused_slot];
        let member = stack.active_index().and_then(|active| match direction {
            Direction::Up => active.checked_sub(1),
            Direction::Down => (active + 1 < stack.sessions().len()).then_some(active + 1),
            Direction::Left | Direction::Right => None,
        });
        if let Some(index) = member {
            return self.select_stack_member(index);
        }
        let next = tab.layout.neighbor(
            tab.focused_slot,
            direction,
            self.bounds,
            self.minimum_pane_size,
        );
        match next {
            Some(next) if next != tab.focused_slot => {
                self.active_tab_mut().focused_slot = next;
                Ok(Transition::changed())
            }
            _ => match direction {
                Direction::Left => self.focus_populated_tab(-1),
                Direction::Right => self.focus_populated_tab(1),
                Direction::Up | Direction::Down => self.wrap_vertical_focus(direction),
            },
        }
    }

    fn wrap_vertical_focus(&mut self, direction: Direction) -> Result<Transition, DomainError> {
        let tab = self.active_tab_ref();
        let focused = tab.focused_slot;
        let Some(next) = tab.layout.vertical_wrap_neighbor(
            focused,
            direction,
            self.bounds,
            self.minimum_pane_size,
        ) else {
            return Ok(Transition::unchanged());
        };
        if next != focused {
            // Enter another stack at its currently displayed member.
            self.active_tab_mut().focused_slot = next;
            return Ok(Transition::changed());
        }
        let stack = &tab.stacks[&focused];
        if stack.sessions().len() > 1 {
            let index = if direction == Direction::Up {
                stack.sessions().len() - 1
            } else {
                0
            };
            return self.select_stack_member(index);
        }
        Ok(Transition::unchanged())
    }

    fn cycle_pane(&mut self, delta: i8) -> Result<Transition, DomainError> {
        let tab = self.active_tab_ref();
        let slots = tab.layout.slots();
        let Some(index) = slots.iter().position(|id| *id == tab.focused_slot) else {
            return Ok(Transition::unchanged());
        };
        let offset = index as isize + delta as isize;
        if offset < 0 || offset >= slots.len() as isize {
            let transition = self.focus_populated_tab(delta.signum())?;
            if transition.changed {
                return Ok(transition);
            }
        }
        let next = offset.rem_euclid(slots.len() as isize) as usize;
        if next == index {
            return Ok(Transition::unchanged());
        }
        // Enter the destination exactly as displayed, retaining its active member.
        self.active_tab_mut().focused_slot = slots[next];
        Ok(Transition::changed())
    }

    fn focus_populated_tab(&mut self, delta: i8) -> Result<Transition, DomainError> {
        let index = self.tab_index(self.active_tab).expect("active tab exists");
        let count = self.tabs.len();
        for step in 1..count {
            let candidate = (index as isize + step as isize * delta.signum() as isize)
                .rem_euclid(count as isize) as usize;
            let tab = &self.tabs[candidate];
            if tab.stacks.values().any(|stack| !stack.is_empty()) {
                // Reuse tab switching so selection is retained and broadcast
                // scope is reset exactly as for numbered-tab navigation.
                return self.select_tab(tab.id);
            }
        }
        Ok(Transition::unchanged())
    }

    fn resize(&mut self, direction: Direction, cells: i16) -> Result<Transition, DomainError> {
        let bounds = self.bounds;
        let minimum = self.minimum_pane_size;
        let focused = self.active_tab_ref().focused_slot;
        // Large mode steps should use the remaining space at a minimum-size
        // boundary instead of rejecting a step that can still partially move.
        let mut step = cells;
        loop {
            match self
                .active_tab_mut()
                .layout
                .resize(focused, direction, step, bounds, minimum)
            {
                Err(LayoutError::MinimumSize) if step.unsigned_abs() > 1 => {
                    step -= step.signum();
                }
                result => {
                    result?;
                    break;
                }
            }
        }
        Ok(if cells == 0 {
            Transition::unchanged()
        } else {
            Transition::changed()
        })
    }

    fn swap(&mut self, direction: Direction) -> Result<Transition, DomainError> {
        let tab = self.active_tab_ref();
        let Some(neighbor) = tab.layout.neighbor(
            tab.focused_slot,
            direction,
            self.bounds,
            self.minimum_pane_size,
        ) else {
            return Ok(Transition::unchanged());
        };
        let focused = tab.focused_slot;
        self.active_tab_mut().layout.swap_slots(focused, neighbor)?;
        Ok(Transition::changed())
    }

    fn swap_member(&mut self, direction: Direction) -> Result<Transition, DomainError> {
        let tab = self.active_tab_ref();
        let focused = tab.focused_slot;
        let stack = &tab.stacks[&focused];
        let Some(active) = stack.active_index() else {
            return Ok(Transition::unchanged());
        };
        let within = match direction {
            Direction::Up => active.checked_sub(1),
            Direction::Down => (active + 1 < stack.sessions().len()).then_some(active + 1),
            _ => None,
        };
        if let Some(index) = within {
            self.active_tab_mut()
                .stacks
                .get_mut(&focused)
                .unwrap()
                .swap_active_with(index);
            return Ok(Transition::changed());
        }
        let Some(neighbor) =
            tab.layout
                .neighbor(focused, direction, self.bounds, self.minimum_pane_size)
        else {
            return Ok(Transition::unchanged());
        };
        let Some(other) = tab.stacks[&neighbor].active_session() else {
            return Ok(Transition::unchanged());
        };
        let current = stack.active_session().unwrap();
        let tab = self.active_tab_mut();
        tab.stacks.get_mut(&focused).unwrap().replace_active(other);
        tab.stacks
            .get_mut(&neighbor)
            .unwrap()
            .replace_active(current);
        tab.focused_slot = neighbor;
        Ok(Transition::changed())
    }

    fn select_stack_member(&mut self, index: usize) -> Result<Transition, DomainError> {
        let focused = self.active_tab_ref().focused_slot;
        if !self
            .active_tab_mut()
            .stacks
            .get_mut(&focused)
            .expect("focused stack exists")
            .select(index)
        {
            return Err(DomainError::InvalidStackIndex(index));
        }
        Ok(Transition::changed())
    }

    fn create_tab(&mut self, spec: SessionSpec) -> Result<Transition, DomainError> {
        let number = (1..)
            .find(|number| !self.tabs.iter().any(|tab| tab.number == *number))
            .unwrap();
        let previous = self.active_tab;
        let tab_id = self.create_empty_tab(number);
        self.active_tab = tab_id;
        self.prune_empty_tab(previous);
        self.reset_broadcast_state();
        self.add_to_focused_stack(spec)
    }

    fn create_empty_tab(&mut self, number: usize) -> TabId {
        let id = self.allocate_tab();
        let slot = self.allocate_slot();
        self.tabs.push(Tab {
            id,
            number,
            layout: LayoutTree::new(slot),
            focused_slot: slot,
            stacks: BTreeMap::from([(slot, SlotStack::default())]),
            manual_broadcast_targets: BTreeSet::new(),
        });
        self.tabs.sort_by_key(|tab| tab.number);
        id
    }

    fn navigate_tab(&mut self, number: usize) -> Result<Transition, DomainError> {
        if !(1..=9).contains(&number) {
            return Err(DomainError::InvalidTabNumber(number));
        }
        if let Some(tab) = self.tabs.iter().find(|tab| tab.number == number) {
            return self.select_tab(tab.id);
        }
        let previous = self.active_tab;
        let id = self.create_empty_tab(number);
        self.active_tab = id;
        self.prune_empty_tab(previous);
        self.reset_broadcast_state();
        self.add_to_focused_stack(self.default_session.clone())
    }

    fn carry_to_number(
        &mut self,
        number: usize,
        member_only: bool,
    ) -> Result<Transition, DomainError> {
        if !(1..=9).contains(&number) {
            return Err(DomainError::InvalidTabNumber(number));
        }
        if self
            .active_tab_ref()
            .stacks
            .values()
            .all(SlotStack::is_empty)
        {
            return Ok(Transition::unchanged());
        }
        if let Some(tab) = self.tabs.iter().find(|tab| tab.number == number) {
            return self.carry_slot(tab.id, member_only);
        }
        // A carry creates only its destination container, never a spare shell.
        let id = self.create_empty_tab(number);
        match self.carry_slot(id, member_only) {
            Ok(transition) => Ok(transition),
            Err(error) => {
                self.tabs.retain(|tab| tab.id != id);
                Err(error)
            }
        }
    }

    fn prune_empty_tab(&mut self, id: TabId) {
        if id != self.active_tab && self.tabs.len() > 1 {
            self.tabs
                .retain(|tab| tab.id != id || !tab.stacks.values().all(SlotStack::is_empty));
        }
    }

    fn select_tab(&mut self, tab: TabId) -> Result<Transition, DomainError> {
        self.tab(tab).ok_or(DomainError::UnknownTab(tab))?;
        if tab == self.active_tab {
            return Ok(Transition::unchanged());
        }
        let previous = self.active_tab;
        self.active_tab = tab;
        self.prune_empty_tab(previous);
        self.reset_broadcast_state();
        Ok(Transition::changed())
    }

    fn request_close_tab(&mut self, tab: TabId) -> Result<Transition, DomainError> {
        self.tab(tab).ok_or(DomainError::UnknownTab(tab))?;
        if self.tabs.len() == 1 {
            return Err(DomainError::FinalTabClose);
        }
        self.pending_confirmation = Some(PendingConfirmation::CloseTab(tab));
        Ok(Transition::changed())
    }

    fn confirm_close_tab(&mut self, tab: TabId) -> Result<Transition, DomainError> {
        if self.pending_confirmation != Some(PendingConfirmation::CloseTab(tab)) {
            return Err(DomainError::PendingConfirmationMismatch);
        }
        let index = self.tab_index(tab).ok_or(DomainError::UnknownTab(tab))?;
        let tab = self.tabs.remove(index);
        self.pending_confirmation = None;
        if self.active_tab == tab.id {
            self.active_tab = self.tabs[index.min(self.tabs.len() - 1)].id;
        }
        let effects = tab
            .stacks
            .into_values()
            .flat_map(|stack| stack.sessions().to_vec())
            .map(|session| {
                if let Some(record) = self.sessions.get_mut(&session) {
                    record.state = SessionState::KillRequested;
                }
                LifecycleEffect::Terminate { session }
            })
            .collect();
        self.reset_broadcast_state();
        Ok(Transition {
            effects,
            changed: true,
        })
    }

    fn carry_focused_slot(&mut self, destination: TabId) -> Result<Transition, DomainError> {
        self.carry_slot(destination, false)
    }

    fn carry_slot(
        &mut self,
        destination: TabId,
        member_only: bool,
    ) -> Result<Transition, DomainError> {
        let source_id = self.active_tab;
        if self
            .active_tab_ref()
            .stacks
            .values()
            .all(SlotStack::is_empty)
        {
            return Ok(Transition::unchanged());
        }
        if source_id == destination {
            return Ok(Transition::unchanged());
        }
        let destination_index = self
            .tab_index(destination)
            .ok_or(DomainError::UnknownTab(destination))?;
        let source_index = self.tab_index(source_id).expect("active tab exists");
        let (source_slot, source_stack) = {
            let source = &self.tabs[source_index];
            (
                source.focused_slot,
                source
                    .stacks
                    .get(&source.focused_slot)
                    .expect("focused stack exists")
                    .clone(),
            )
        };
        let separate_member = member_only && source_stack.sessions().len() > 1;
        let carried_session = source_stack.active_session();
        let destination_slot = if separate_member {
            SlotId(self.next_slot)
        } else {
            source_slot
        };
        let source_stack = if separate_member {
            SlotStack::with_session(carried_session.expect("nonempty stack"))
        } else {
            source_stack
        };
        let destination_empty = self.tabs[destination_index]
            .stacks
            .values()
            .all(SlotStack::is_empty);
        let mut proposed_destination = if destination_empty {
            LayoutTree::new(destination_slot)
        } else {
            self.tabs[destination_index].layout.clone()
        };
        if !destination_empty {
            proposed_destination.insert_beside(
                self.tabs[destination_index].focused_slot,
                Axis::Vertical,
                DEFAULT_SPLIT_RATIO,
                destination_slot,
            )?;
        }
        if !self.layout_fits(&proposed_destination) {
            return Err(DomainError::Layout(LayoutError::MinimumSize));
        }
        self.tabs[destination_index].layout = proposed_destination;
        if destination_empty {
            self.tabs[destination_index].stacks.clear();
        }
        self.tabs[destination_index]
            .stacks
            .insert(destination_slot, source_stack);
        self.tabs[destination_index].focused_slot = destination_slot;

        if separate_member {
            self.next_slot += 1;
            self.tabs[source_index]
                .stacks
                .get_mut(&source_slot)
                .expect("source stack")
                .remove_active();
        } else if self.tabs[source_index].layout.slots().len() == 1 {
            self.tabs.remove(source_index);
        } else {
            let source = &mut self.tabs[source_index];
            source.layout.remove(source_slot)?;
            source.stacks.remove(&source_slot);
            source.manual_broadcast_targets.remove(&source_slot);
            let slots = source.layout.slots();
            source.focused_slot = *slots.first().expect("non-final removal leaves a slot");
        }
        self.active_tab = destination;
        self.reset_broadcast_state();
        Ok(Transition::changed())
    }

    fn remove_focused_slot(&mut self) -> Result<Transition, DomainError> {
        let focused = self.active_tab_ref().focused_slot;
        if self.active_tab_ref().layout.slots().len() == 1 {
            return Err(DomainError::FinalSlotRemoval);
        }
        let transfer_target = [
            Direction::Right,
            Direction::Left,
            Direction::Down,
            Direction::Up,
        ]
        .into_iter()
        .find_map(|direction| {
            self.active_tab_ref().layout.neighbor(
                focused,
                direction,
                self.bounds,
                self.minimum_pane_size,
            )
        });
        let removal = self.active_tab_mut().layout.remove(focused)?;
        let LayoutRemoval::Removed { neighbor } = removal else {
            return Err(DomainError::FinalSlotRemoval);
        };
        let neighbor = transfer_target.unwrap_or(neighbor);
        let tab = self.active_tab_mut();
        let stack = tab
            .stacks
            .remove(&focused)
            .expect("layout and stacks stay in sync");
        tab.stacks
            .get_mut(&neighbor)
            .expect("removal neighbor exists")
            .append_stack(stack);
        tab.manual_broadcast_targets.remove(&focused);
        tab.focused_slot = neighbor;
        Ok(Transition::changed())
    }

    fn request_kill_focused_session(&mut self) -> Result<Transition, DomainError> {
        let tab = self.active_tab_ref();
        let slot = tab.focused_slot;
        let session = tab
            .stacks
            .get(&slot)
            .and_then(SlotStack::active_session)
            .ok_or(DomainError::UnknownSlot(slot))?;
        self.sessions
            .get_mut(&session)
            .expect("stack references registered session")
            .state = SessionState::KillRequested;
        self.pending_confirmation = Some(PendingConfirmation::KillSession {
            tab: self.active_tab,
            slot,
            session,
        });
        Ok(Transition::changed())
    }

    fn confirm_kill_focused_session(&mut self) -> Result<Transition, DomainError> {
        let Some(PendingConfirmation::KillSession { tab, slot, session }) =
            self.pending_confirmation
        else {
            return Err(DomainError::PendingConfirmationMismatch);
        };
        let emptied_slot = {
            let stack = self
                .tab_mut(tab)
                .ok_or(DomainError::UnknownTab(tab))?
                .stacks
                .get_mut(&slot)
                .ok_or(DomainError::UnknownSlot(slot))?;
            if stack.active_session() != Some(session) {
                return Err(DomainError::PendingConfirmationMismatch);
            }
            stack.remove_active();
            stack.is_empty()
        };
        self.pending_confirmation = None;
        self.sessions
            .get_mut(&session)
            .expect("registered session")
            .state = SessionState::KillRequested;
        let mut effects = vec![LifecycleEffect::Terminate { session }];
        if emptied_slot {
            self.reconcile_empty_slot(tab, slot, &mut effects)?;
        }
        Ok(Transition {
            effects,
            changed: true,
        })
    }

    fn request_kill_focused_stack(&mut self) -> Result<Transition, DomainError> {
        let tab = self.active_tab_ref();
        let slot = tab.focused_slot;
        self.pending_confirmation = Some(PendingConfirmation::KillStack {
            tab: self.active_tab,
            slot,
        });
        Ok(Transition::changed())
    }

    fn confirm_kill_focused_stack(&mut self) -> Result<Transition, DomainError> {
        let Some(PendingConfirmation::KillStack { tab, slot }) = self.pending_confirmation else {
            return Err(DomainError::PendingConfirmationMismatch);
        };
        let stack = self
            .tab_mut(tab)
            .ok_or(DomainError::UnknownTab(tab))?
            .stacks
            .get_mut(&slot)
            .ok_or(DomainError::UnknownSlot(slot))?;
        let sessions = std::mem::take(stack).sessions().to_vec();
        self.pending_confirmation = None;
        for session in &sessions {
            self.sessions
                .get_mut(session)
                .expect("registered session")
                .state = SessionState::KillRequested;
        }
        let mut effects = sessions
            .into_iter()
            .map(|session| LifecycleEffect::Terminate { session })
            .collect::<Vec<_>>();
        self.reconcile_empty_slot(tab, slot, &mut effects)?;
        Ok(Transition {
            effects,
            changed: true,
        })
    }

    fn toggle_manual_broadcast_target(&mut self, slot: SlotId) -> Result<Transition, DomainError> {
        let is_eligible = self.slot_has_input_eligible_session(self.active_tab_ref(), slot);
        let tab = self.active_tab_mut();
        if !tab.stacks.contains_key(&slot) || !is_eligible {
            return Err(DomainError::UnknownSlot(slot));
        }
        if !tab.manual_broadcast_targets.insert(slot) {
            tab.manual_broadcast_targets.remove(&slot);
        }
        Ok(Transition::changed())
    }

    fn layout_fits(&self, layout: &LayoutTree) -> bool {
        layout
            .geometry(self.bounds, self.minimum_pane_size)
            .values()
            .all(|rect| {
                rect.cols >= self.minimum_pane_size.cols && rect.rows >= self.minimum_pane_size.rows
            })
    }
    fn reset_broadcast_state(&mut self) -> bool {
        let changed = self.broadcast_scope != BroadcastScope::Focused
            || self
                .tabs
                .iter()
                .any(|tab| !tab.manual_broadcast_targets.is_empty());
        self.broadcast_scope = BroadcastScope::Focused;
        for tab in &mut self.tabs {
            tab.manual_broadcast_targets.clear();
        }
        changed
    }
    fn slot_has_input_eligible_session(&self, tab: &Tab, slot: SlotId) -> bool {
        tab.stacks
            .get(&slot)
            .and_then(SlotStack::active_session)
            .and_then(|session| self.sessions.get(&session))
            .is_some_and(|session| {
                !matches!(
                    session.state,
                    SessionState::Exited | SessionState::KillRequested
                )
            })
    }
    fn prune_manual_broadcast_targets(&mut self) {
        let states = &self.sessions;
        for tab in &mut self.tabs {
            tab.manual_broadcast_targets.retain(|slot| {
                tab.stacks
                    .get(slot)
                    .and_then(SlotStack::active_session)
                    .and_then(|id| states.get(&id))
                    .is_some_and(|session| {
                        !matches!(
                            session.state,
                            SessionState::Exited | SessionState::KillRequested
                        )
                    })
            });
        }
        if self.broadcast_scope == BroadcastScope::Manual
            && self.active_tab_ref().manual_broadcast_targets.is_empty()
        {
            self.broadcast_scope = BroadcastScope::Focused;
        }
    }
    fn reconcile_empty_slot(
        &mut self,
        tab_id: TabId,
        slot: SlotId,
        effects: &mut Vec<LifecycleEffect>,
    ) -> Result<(), DomainError> {
        let tab = self.tab(tab_id).ok_or(DomainError::UnknownTab(tab_id))?;
        if tab.layout.slots().len() == 1 {
            return Ok(());
        }
        let target = [
            Direction::Right,
            Direction::Left,
            Direction::Down,
            Direction::Up,
        ]
        .into_iter()
        .find_map(|direction| {
            tab.layout
                .neighbor(slot, direction, self.bounds, self.minimum_pane_size)
        });
        let tab = self.tab_mut(tab_id).expect("tab exists");
        let removal = tab.layout.remove(slot)?;
        let LayoutRemoval::Removed { neighbor } = removal else {
            return Ok(());
        };
        tab.stacks.remove(&slot);
        tab.manual_broadcast_targets.remove(&slot);
        if tab.focused_slot == slot {
            tab.focused_slot = target.unwrap_or(neighbor);
        }
        let _ = effects;
        Ok(())
    }
    fn allocate_tab(&mut self) -> TabId {
        let id = TabId(self.next_tab);
        self.next_tab += 1;
        id
    }
    fn allocate_slot(&mut self) -> SlotId {
        let id = SlotId(self.next_slot);
        self.next_slot += 1;
        id
    }
    fn allocate_session(&mut self, spec: SessionSpec) -> SessionId {
        let id = SessionId(self.next_session);
        self.next_session += 1;
        self.sessions.insert(id, Session::new(id, spec));
        id
    }
    fn tab_index(&self, id: TabId) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.id == id)
    }
    fn tab(&self, id: TabId) -> Option<&Tab> {
        self.tab_index(id).map(|index| &self.tabs[index])
    }
    fn tab_mut(&mut self, id: TabId) -> Option<&mut Tab> {
        let index = self.tab_index(id)?;
        Some(&mut self.tabs[index])
    }
    fn active_tab_ref(&self) -> &Tab {
        self.tab(self.active_tab).expect("active tab exists")
    }
    fn active_tab_mut(&mut self) -> &mut Tab {
        let id = self.active_tab;
        self.tab_mut(id).expect("active tab exists")
    }
}
