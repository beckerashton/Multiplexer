use std::collections::BTreeMap;

use crate::{Axis, Direction};

/// A command token recognized after the configured leader.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BindingAction {
    Focus(Direction),
    PreviousPane,
    NextPane,
    Swap(Direction),
    SwapMember(Direction),
    CarryMemberToTab(usize),
    Resize(Direction),
    ResizeMode,
    Equalize(Axis),
    BroadcastMenu,
    SplitVertical,
    SplitHorizontal,
    SplitVerticalGlobal,
    SplitHorizontalGlobal,
    AddToStack,
    PreviousStack,
    NextStack,
    SelectStackMember(usize),
    RemoveSlot,
    KillSession,
    KillStack,
    CreateTab,
    CloseTab,
    Carry,
    CarryToTab(usize),
    BroadcastVisible,
    BroadcastManual,
    ToggleManualTarget,
    ResetBroadcast,
    ToggleBorderless,
    SetJumpMark,
    JumpToMark,
    SelectionMode,
    Quit,
}

impl BindingAction {
    pub fn is_broadcast(self) -> bool {
        matches!(
            self,
            Self::BroadcastVisible
                | Self::BroadcastManual
                | Self::ToggleManualTarget
                | Self::ResetBroadcast
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BindingError {
    EmptySequence,
    SequenceTooLong,
    DuplicateKey { key: u8, existing: BindingAction },
    DuplicateAltTabKey { key: u8 },
    InvalidLeader,
    LeaderConflictsWithBinding,
    BindingConflictsWithLeader,
    InvalidStackIndex,
    InvalidAltTimeout,
    InvalidLeaderTimeout,
}

impl std::fmt::Display for BindingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptySequence => f.write_str("a binding cannot be empty"),
            Self::SequenceTooLong => f.write_str("v1 bindings are one byte after the leader"),
            Self::DuplicateKey { key, existing } => {
                write!(f, "byte 0x{key:02x} is already bound to {existing:?}")
            }
            Self::DuplicateAltTabKey { key } => {
                write!(f, "Alt-tab byte 0x{key:02x} is already in use")
            }
            Self::InvalidLeader => f.write_str("the leader must be a non-zero byte"),
            Self::LeaderConflictsWithBinding => {
                f.write_str("the leader is already a command binding")
            }
            Self::BindingConflictsWithLeader => {
                f.write_str("a command binding cannot equal the leader")
            }
            Self::InvalidStackIndex => f.write_str("stack member indices are one-based"),
            Self::InvalidAltTimeout => {
                f.write_str("Alt-number timeout must be between 1 and 1000 ms")
            }
            Self::InvalidLeaderTimeout => {
                f.write_str("leader timeout must be between 250 and 3000 ms")
            }
        }
    }
}

impl std::error::Error for BindingError {}

/// Runtime binding configuration. The defaults are deliberately byte based so
/// ordinary terminal input can be forwarded without key re-encoding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BindingConfig {
    leader: u8,
    leader_timeout_ms: u16,
    alt_timeout_ms: u16,
    alt_numbers_enabled: bool,
    alt_tab_keys: [u8; 9],
    bindings: BTreeMap<u8, BindingAction>,
    modified: BTreeMap<(u8, u8), BindingAction>,
    broadcast: BTreeMap<u8, BindingAction>,
}

impl Default for BindingConfig {
    fn default() -> Self {
        let mut config = Self {
            leader: 0x02,
            leader_timeout_ms: 1000,
            alt_timeout_ms: 50,
            alt_numbers_enabled: true,
            alt_tab_keys: *b"123456789",
            bindings: BTreeMap::new(),
            modified: BTreeMap::new(),
            broadcast: BTreeMap::new(),
        };

        let defaults = [
            (b'r', BindingAction::ResizeMode),
            (b'=', BindingAction::Equalize(Axis::Vertical)),
            (b'+', BindingAction::Equalize(Axis::Horizontal)),
            (b'b', BindingAction::BroadcastMenu),
            (b'\\', BindingAction::SplitVertical),
            (b'-', BindingAction::SplitHorizontal),
            (b'|', BindingAction::SplitVerticalGlobal),
            (b'_', BindingAction::SplitHorizontalGlobal),
            (b'a', BindingAction::AddToStack),
            (b'1', BindingAction::SelectStackMember(1)),
            (b'2', BindingAction::SelectStackMember(2)),
            (b'3', BindingAction::SelectStackMember(3)),
            (b'4', BindingAction::SelectStackMember(4)),
            (b'5', BindingAction::SelectStackMember(5)),
            (b'6', BindingAction::SelectStackMember(6)),
            (b'7', BindingAction::SelectStackMember(7)),
            (b'8', BindingAction::SelectStackMember(8)),
            (b'9', BindingAction::SelectStackMember(9)),
            (b'd', BindingAction::RemoveSlot),
            (b'x', BindingAction::KillSession),
            (b'X', BindingAction::KillStack),
            (b't', BindingAction::CreateTab),
            (b'w', BindingAction::CloseTab),
            (b'b', BindingAction::BroadcastVisible),
            (b'B', BindingAction::BroadcastManual),
            (b'm', BindingAction::ToggleManualTarget),
            (b'r', BindingAction::ResetBroadcast),
            (b'z', BindingAction::ToggleBorderless),
            (b'y', BindingAction::SelectionMode),
            (b'q', BindingAction::Quit),
        ];
        for (key, action) in defaults {
            if action.is_broadcast() {
                config.broadcast.insert(key, action);
            } else {
                config.bindings.insert(key, action);
            }
        }
        for (key, direction) in [
            (b'h', Direction::Left),
            (b'j', Direction::Down),
            (b'k', Direction::Up),
            (b'l', Direction::Right),
        ] {
            config
                .modified
                .insert((3, key), BindingAction::Focus(direction));
            config
                .modified
                .insert((7, key), BindingAction::SwapMember(direction));
        }
        config
            .modified
            .insert((3, b'['), BindingAction::PreviousPane);
        config.modified.insert((3, b']'), BindingAction::NextPane);
        for number in 1..=9 {
            config.modified.insert(
                (7, b'0' + number as u8),
                BindingAction::CarryMemberToTab(number),
            );
        }
        config.modified.insert((7, b'g'), BindingAction::SetJumpMark);
        config.modified.insert((3, b'g'), BindingAction::JumpToMark);
        config
    }
}

impl BindingConfig {
    pub fn modified_bindings(&self) -> &BTreeMap<(u8, u8), BindingAction> {
        &self.modified
    }

    pub fn modified_binding(&self, modifiers: u8, key: u8) -> Option<BindingAction> {
        self.modified.get(&(modifiers, key)).copied()
    }

    pub fn bind_modified(
        &mut self,
        modifiers: u8,
        key: u8,
        action: BindingAction,
    ) -> Result<(), BindingError> {
        if let Some(existing) = self.modified.get(&(modifiers, key)) {
            return Err(BindingError::DuplicateKey {
                key,
                existing: *existing,
            });
        }
        if modifiers == 3 && self.alt_numbers_enabled && self.alt_tab_keys.contains(&key) {
            return Err(BindingError::DuplicateAltTabKey { key });
        }
        self.modified.insert((modifiers, key), action);
        Ok(())
    }

    pub fn leader(&self) -> u8 {
        self.leader
    }

    pub fn set_leader(&mut self, leader: u8) -> Result<(), BindingError> {
        if leader == 0 || leader == 0x1b {
            return Err(BindingError::InvalidLeader);
        }
        if self.bindings.contains_key(&leader) {
            return Err(BindingError::LeaderConflictsWithBinding);
        }
        self.leader = leader;
        Ok(())
    }

    pub fn alt_timeout_ms(&self) -> u16 {
        self.alt_timeout_ms
    }

    pub fn leader_timeout_ms(&self) -> u16 {
        self.leader_timeout_ms
    }

    pub fn set_leader_timeout_ms(&mut self, timeout_ms: u16) -> Result<(), BindingError> {
        if !(250..=3000).contains(&timeout_ms) {
            return Err(BindingError::InvalidLeaderTimeout);
        }
        self.leader_timeout_ms = timeout_ms;
        Ok(())
    }

    pub fn alt_numbers_enabled(&self) -> bool {
        self.alt_numbers_enabled
    }

    pub fn set_alt_numbers_enabled(&mut self, enabled: bool) {
        self.alt_numbers_enabled = enabled;
    }

    pub fn alt_tab_key(&self, index: usize) -> Option<u8> {
        self.alt_tab_keys.get(index).copied()
    }

    pub fn alt_tab_keys(&self) -> &[u8; 9] {
        &self.alt_tab_keys
    }

    pub fn set_alt_tab_key(&mut self, index: usize, key: u8) -> Result<(), BindingError> {
        if index >= self.alt_tab_keys.len() || key == 0 || key == 0x1b {
            return Err(BindingError::InvalidLeader);
        }
        if self
            .alt_tab_keys
            .iter()
            .enumerate()
            .any(|(other, existing)| other != index && *existing == key)
        {
            return Err(BindingError::DuplicateAltTabKey { key });
        }
        self.alt_tab_keys[index] = key;
        Ok(())
    }

    pub fn set_alt_timeout_ms(&mut self, timeout_ms: u16) -> Result<(), BindingError> {
        if !(1..=1000).contains(&timeout_ms) {
            return Err(BindingError::InvalidAltTimeout);
        }
        self.alt_timeout_ms = timeout_ms;
        Ok(())
    }

    pub fn binding(&self, key: u8) -> Option<BindingAction> {
        self.bindings.get(&key).copied()
    }

    pub fn bindings(&self) -> &BTreeMap<u8, BindingAction> {
        &self.bindings
    }

    pub fn broadcast_bindings(&self) -> &BTreeMap<u8, BindingAction> {
        &self.broadcast
    }

    fn action_bindings_mut(&mut self, action: BindingAction) -> &mut BTreeMap<u8, BindingAction> {
        if action.is_broadcast() {
            &mut self.broadcast
        } else {
            &mut self.bindings
        }
    }

    pub fn bind(&mut self, key: u8, action: BindingAction) -> Result<(), BindingError> {
        self.validate_binding(key, action)?;
        let bindings = self.action_bindings_mut(action);
        if let Some(existing) = bindings.get(&key).copied() {
            return Err(BindingError::DuplicateKey { key, existing });
        }
        bindings.insert(key, action);
        Ok(())
    }

    pub fn replace_binding(&mut self, key: u8, action: BindingAction) {
        self.action_bindings_mut(action).insert(key, action);
    }

    /// Rebind an existing action, removing its previous key first.
    pub fn rebind(&mut self, key: u8, action: BindingAction) -> Result<(), BindingError> {
        self.validate_binding(key, action)?;
        let bindings = self.action_bindings_mut(action);
        if let Some(existing) = bindings.get(&key).copied() {
            if existing != action {
                return Err(BindingError::DuplicateKey { key, existing });
            }
        }
        bindings.retain(|_, existing| *existing != action);
        bindings.insert(key, action);
        Ok(())
    }

    pub fn remove_action(&mut self, action: BindingAction) {
        self.broadcast.retain(|_, existing| *existing != action);
        self.bindings.retain(|_, existing| *existing != action);
        self.modified.retain(|_, existing| *existing != action);
    }

    pub fn set_alt_tab_keys(&mut self, keys: [u8; 9]) -> Result<(), BindingError> {
        for (index, key) in keys.iter().enumerate() {
            if self.modified.contains_key(&(3, *key)) {
                return Err(BindingError::DuplicateAltTabKey { key: *key });
            }
            if *key == 0 || *key == 0x1b || keys[..index].contains(key) {
                return if keys[..index].contains(key) {
                    Err(BindingError::DuplicateAltTabKey { key: *key })
                } else {
                    Err(BindingError::InvalidLeader)
                };
            }
        }
        self.alt_tab_keys = keys;
        Ok(())
    }

    pub fn unbind(&mut self, key: u8) -> Option<BindingAction> {
        self.bindings.remove(&key)
    }

    fn validate_binding(&self, key: u8, action: BindingAction) -> Result<(), BindingError> {
        if key == self.leader && !action.is_broadcast() {
            return Err(BindingError::BindingConflictsWithLeader);
        }
        if matches!(action, BindingAction::SelectStackMember(0)) {
            return Err(BindingError::InvalidStackIndex);
        }
        Ok(())
    }
}
