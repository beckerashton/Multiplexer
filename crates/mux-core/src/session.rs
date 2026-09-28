use crate::{SessionId, SessionSpec, SessionState};

/// Metadata owned by the pure model. The PTY and emulator remain outside this
/// crate; their lifecycle adapter reports state changes by stable id.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Session {
    pub id: SessionId,
    pub spec: SessionSpec,
    pub state: SessionState,
}

impl Session {
    pub fn new(id: SessionId, spec: SessionSpec) -> Self {
        Self {
            id,
            spec,
            state: SessionState::Starting,
        }
    }
}

/// Ordered sessions occupying one physical pane. `active` is an index in
/// `sessions`, or `None` only for an intentionally empty slot.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SlotStack {
    sessions: Vec<SessionId>,
    active: Option<usize>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SlotStackView {
    pub sessions: Vec<SessionId>,
    pub active: Option<usize>,
}

impl SlotStack {
    pub fn with_session(session: SessionId) -> Self {
        Self {
            sessions: vec![session],
            active: Some(0),
        }
    }

    pub fn sessions(&self) -> &[SessionId] {
        &self.sessions
    }

    pub fn active_index(&self) -> Option<usize> {
        self.active
    }

    pub fn active_session(&self) -> Option<SessionId> {
        self.active
            .and_then(|index| self.sessions.get(index).copied())
    }

    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    pub fn add(&mut self, session: SessionId) {
        self.sessions.push(session);
        self.active = Some(self.sessions.len() - 1);
    }

    pub fn select(&mut self, index: usize) -> bool {
        if index >= self.sessions.len() {
            return false;
        }
        self.active = Some(index);
        true
    }

    pub fn cycle(&mut self, delta: i8) -> bool {
        let count = self.sessions.len();
        if count == 0 || delta == 0 {
            return false;
        }
        let current = self.active.unwrap_or(0) as isize;
        let next = (current + delta as isize).rem_euclid(count as isize) as usize;
        self.active = Some(next);
        true
    }

    pub fn swap_active_with(&mut self, index: usize) {
        if let Some(active) = self.active {
            self.sessions.swap(active, index);
            self.active = Some(index);
        }
    }

    pub fn replace_active(&mut self, session: SessionId) {
        if let Some(active) = self.active {
            self.sessions[active] = session;
        }
    }

    pub fn remove_active(&mut self) -> Option<SessionId> {
        let index = self.active?;
        let removed = self.sessions.remove(index);
        self.active = match self.sessions.len() {
            0 => None,
            count => Some(index.min(count - 1)),
        };
        Some(removed)
    }

    pub fn remove(&mut self, session: SessionId) -> bool {
        let Some(index) = self.sessions.iter().position(|id| *id == session) else {
            return false;
        };
        let active = self.active.unwrap_or(0);
        self.sessions.remove(index);
        self.active = if self.sessions.is_empty() {
            None
        } else {
            Some(if index < active {
                active - 1
            } else {
                active.min(self.sessions.len() - 1)
            })
        };
        true
    }

    /// Move all sessions from `other` without changing session ids. The
    /// destination's active member remains selected, as required when a pane
    /// is removed into its neighboring stack.
    pub fn append_stack(&mut self, mut other: SlotStack) {
        self.sessions.append(&mut other.sessions);
        if self.active.is_none() && !self.sessions.is_empty() {
            self.active = Some(0);
        }
    }

    pub fn view(&self) -> SlotStackView {
        SlotStackView {
            sessions: self.sessions.clone(),
            active: self.active,
        }
    }
}
