use crate::Axis;
use crate::config::{BindingAction, BindingConfig};
use std::collections::VecDeque;

use crate::{BroadcastScope, TabId, WorkspaceCommand};

/// Raw input delivered by the terminal reader. Paste payloads are atomic and
/// never pass through the leader or Alt-number decoder.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InputEvent {
    Bytes(Vec<u8>),
    Paste(Vec<u8>),
    Timeout,
}

/// Context used to resolve commands which name a tab or need a default shell.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RouterContext {
    pub current_tab: TabId,
    pub tab_by_number: [Option<TabId>; 9],
    pub focused_slot: crate::SlotId,
    pub default_session: crate::SessionSpec,
    pub broadcast_scope: BroadcastScope,
}

/// Ordered result of routing one input event. Each returned item is applied in
/// order; integration must refresh RouterContext after every Command before
/// routing another raw chunk.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InputRoute {
    Forward(Vec<u8>),
    Command(WorkspaceCommand),
    Consume,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct InputRouter {
    leader_pending: bool,
    alt_pending: bool,
    carry_pending: bool,
    resize_mode: bool,
    broadcast_pending: bool,
    csi: Vec<u8>,
    pending_bytes: VecDeque<u8>,
}

impl InputRouter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn resize_mode(&self) -> bool {
        self.resize_mode
    }

    pub fn broadcast_pending(&self) -> bool {
        self.broadcast_pending
    }

    pub fn modal(&self) -> bool {
        self.resize_mode || self.broadcast_pending
    }

    pub fn leader_pending(&self) -> bool {
        self.leader_pending
    }

    pub fn alt_pending(&self) -> bool {
        self.alt_pending || !self.csi.is_empty()
    }

    pub fn carry_pending(&self) -> bool {
        self.carry_pending
    }

    pub fn has_pending_bytes(&self) -> bool {
        !self.pending_bytes.is_empty()
    }

    pub fn route(
        &mut self,
        event: InputEvent,
        context: &RouterContext,
        config: &BindingConfig,
    ) -> Vec<InputRoute> {
        match event {
            InputEvent::Bytes(bytes) => {
                self.pending_bytes.extend(bytes);
                self.route_pending(context, config)
            }
            InputEvent::Paste(bytes) => self.route_paste(bytes),
            InputEvent::Timeout => self.route_timeout(context, config),
        }
    }

    fn route_pending(
        &mut self,
        context: &RouterContext,
        config: &BindingConfig,
    ) -> Vec<InputRoute> {
        let mut routes = Vec::new();
        let mut forward = Vec::new();
        while let Some(byte) = self.pending_bytes.pop_front() {
            let mut emitted = self.route_byte(byte, context, config);
            let command_seen = emitted
                .iter()
                .any(|route| matches!(route, InputRoute::Command(_)));
            for route in emitted.drain(..) {
                match route {
                    InputRoute::Forward(mut bytes) => forward.append(&mut bytes),
                    other => {
                        if !forward.is_empty() {
                            routes.push(InputRoute::Forward(std::mem::take(&mut forward)));
                        }
                        routes.push(other);
                    }
                }
            }
            if command_seen {
                break;
            }
        }
        if !forward.is_empty() {
            routes.push(InputRoute::Forward(forward));
        }
        routes
    }

    fn route_byte(
        &mut self,
        byte: u8,
        context: &RouterContext,
        config: &BindingConfig,
    ) -> Vec<InputRoute> {
        if !self.csi.is_empty() {
            self.csi.push(byte);
            if (0x40..=0x7e).contains(&byte) || self.csi.len() >= 64 {
                let bytes = std::mem::take(&mut self.csi);
                if let Some((key, modifiers)) = decode_modified_key(&bytes) {
                    return self
                        .modified_key(key, modifiers, context, config)
                        .unwrap_or_else(|| {
                            if self.leader_pending || self.modal() {
                                vec![InputRoute::Consume]
                            } else {
                                vec![InputRoute::Forward(bytes)]
                            }
                        });
                }
                return if self.leader_pending || self.modal() {
                    vec![InputRoute::Consume]
                } else {
                    vec![InputRoute::Forward(bytes)]
                };
            }
            return Vec::new();
        }
        if self.carry_pending {
            self.carry_pending = false;
            if (b'1'..=b'9').contains(&byte) {
                let index = (byte - b'1') as usize;
                if let Some(destination) = context.tab_by_number[index] {
                    return vec![InputRoute::Command(WorkspaceCommand::CarryFocusedSlot {
                        destination,
                    })];
                }
                return vec![InputRoute::Consume];
            }
            if byte == 0x1b {
                return vec![InputRoute::Consume];
            }
            return vec![InputRoute::Forward(vec![byte])];
        }

        if self.alt_pending {
            self.alt_pending = false;
            if matches!(byte, b'[' | b'O') {
                self.csi = vec![0x1b, byte];
                return Vec::new();
            }
            let (key, modifiers) = if (1..=26).contains(&byte) {
                (b'a' + byte - 1, 7)
            } else {
                (byte, 3)
            };
            if let Some(routes) = self.modified_key(key, modifiers, context, config) {
                return routes;
            }
            return if self.leader_pending || self.modal() {
                vec![InputRoute::Consume]
            } else {
                vec![InputRoute::Forward(vec![0x1b, byte])]
            };
        }

        if self.modal() {
            if byte == 0x1b {
                self.alt_pending = true;
                return Vec::new();
            }
            return self.modal_key(byte, context, config);
        }

        if self.leader_pending {
            if byte != config.leader() && byte != 0x1b && config.binding(byte).is_none() {
                return vec![InputRoute::Consume];
            }
            if byte == 0x1b {
                self.alt_pending = true;
                return Vec::new();
            }
            self.leader_pending = false;
            if byte == config.leader() {
                return vec![InputRoute::Forward(vec![config.leader()])];
            }
            return self.command_for(byte, context, config);
        }

        if byte == config.leader() {
            self.leader_pending = true;
            return vec![InputRoute::Consume];
        }
        if byte == 0x1b {
            self.alt_pending = true;
            return Vec::new();
        }
        vec![InputRoute::Forward(vec![byte])]
    }

    fn command_for(
        &mut self,
        byte: u8,
        context: &RouterContext,
        config: &BindingConfig,
    ) -> Vec<InputRoute> {
        let Some(action) = config.binding(byte) else {
            return vec![InputRoute::Forward(vec![byte])];
        };
        self.action_for(action, context)
    }

    fn modified_key(
        &mut self,
        key: u8,
        modifiers: u8,
        context: &RouterContext,
        config: &BindingConfig,
    ) -> Option<Vec<InputRoute>> {
        if self.modal() {
            if modifiers == 1 && key == 0x1b {
                self.resize_mode = false;
                self.broadcast_pending = false;
                return Some(vec![InputRoute::Consume]);
            }
            if modifiers == 1 || modifiers == 2 {
                let key = if modifiers == 2 {
                    key.to_ascii_uppercase()
                } else {
                    key
                };
                return Some(self.modal_key(key, context, config));
            }
            if self.resize_mode && modifiers == 3 {
                if let Some(action @ BindingAction::Focus(_)) =
                    config.modified_binding(modifiers, key)
                {
                    return Some(self.action_for(action, context));
                }
            }
            return Some(vec![InputRoute::Consume]);
        }
        if let Some(action) = config.modified_binding(modifiers, key) {
            let action = if self.leader_pending {
                match action {
                    BindingAction::SwapMember(direction) => BindingAction::Swap(direction),
                    BindingAction::CarryMemberToTab(number) => BindingAction::CarryToTab(number),
                    other => other,
                }
            } else {
                action
            };
            self.leader_pending = false;
            return Some(self.action_for(action, context));
        }
        if modifiers == 3 && config.alt_numbers_enabled() {
            if let Some(index) = config
                .alt_tab_keys()
                .iter()
                .position(|candidate| *candidate == key)
            {
                self.leader_pending = false;
                return Some(vec![InputRoute::Command(WorkspaceCommand::NavigateTab {
                    number: index + 1,
                })]);
            }
        }
        None
    }

    fn modal_key(
        &mut self,
        key: u8,
        context: &RouterContext,
        config: &BindingConfig,
    ) -> Vec<InputRoute> {
        if self.resize_mode {
            let direction = match key.to_ascii_lowercase() {
                b'h' => Some(crate::Direction::Left),
                b'j' => Some(crate::Direction::Down),
                b'k' => Some(crate::Direction::Up),
                b'l' => Some(crate::Direction::Right),
                _ => None,
            };
            if let Some(direction) = direction {
                return vec![InputRoute::Command(WorkspaceCommand::Resize {
                    direction,
                    cells: if key.is_ascii_uppercase() { 5 } else { 1 },
                })];
            }
        } else if let Some(action) = config.broadcast_bindings().get(&key) {
            self.broadcast_pending = false;
            return self.action_for(*action, context);
        }
        vec![InputRoute::Consume]
    }

    fn action_for(&mut self, action: BindingAction, context: &RouterContext) -> Vec<InputRoute> {
        let command = match action {
            BindingAction::ResizeMode => {
                self.resize_mode = true;
                return vec![InputRoute::Consume];
            }
            BindingAction::BroadcastMenu => {
                self.broadcast_pending = true;
                return vec![InputRoute::Consume];
            }
            BindingAction::Focus(direction) => WorkspaceCommand::Focus(direction),
            BindingAction::SwapMember(direction) => WorkspaceCommand::SwapMember(direction),
            BindingAction::CarryMemberToTab(number) => {
                WorkspaceCommand::CarryMemberToTab { number }
            }
            BindingAction::Swap(direction) => WorkspaceCommand::Swap(direction),
            BindingAction::Resize(direction) => WorkspaceCommand::Resize {
                direction,
                cells: 1,
            },
            BindingAction::Equalize(axis) => WorkspaceCommand::Equalize(axis),
            BindingAction::SplitVertical => WorkspaceCommand::SplitFocused {
                axis: Axis::Vertical,
                session: context.default_session.clone(),
            },
            BindingAction::SplitHorizontal => WorkspaceCommand::SplitFocused {
                axis: Axis::Horizontal,
                session: context.default_session.clone(),
            },
            BindingAction::SplitVerticalGlobal => WorkspaceCommand::SplitTab {
                axis: Axis::Vertical,
                session: context.default_session.clone(),
            },
            BindingAction::SplitHorizontalGlobal => WorkspaceCommand::SplitTab {
                axis: Axis::Horizontal,
                session: context.default_session.clone(),
            },
            BindingAction::AddToStack => WorkspaceCommand::AddToFocusedStack {
                session: context.default_session.clone(),
            },
            BindingAction::PreviousPane => WorkspaceCommand::CyclePane { delta: -1 },
            BindingAction::NextPane => WorkspaceCommand::CyclePane { delta: 1 },
            BindingAction::PreviousStack => WorkspaceCommand::CycleStack { delta: -1 },
            BindingAction::NextStack => WorkspaceCommand::CycleStack { delta: 1 },
            BindingAction::SelectStackMember(index) => {
                if index == 0 {
                    return vec![InputRoute::Consume];
                }
                WorkspaceCommand::SelectStackMember { index: index - 1 }
            }
            BindingAction::RemoveSlot => WorkspaceCommand::RemoveFocusedSlot,
            BindingAction::KillSession => WorkspaceCommand::RequestKillFocusedSession,
            BindingAction::KillStack => WorkspaceCommand::RequestKillFocusedStack,
            BindingAction::CreateTab => WorkspaceCommand::CreateTab {
                session: context.default_session.clone(),
            },
            BindingAction::CloseTab => WorkspaceCommand::RequestCloseTab(context.current_tab),
            BindingAction::CarryToTab(number) => WorkspaceCommand::CarryToTab { number },
            BindingAction::Carry => {
                self.carry_pending = true;
                return vec![InputRoute::Consume];
            }
            BindingAction::BroadcastVisible => {
                let scope = if context.broadcast_scope == BroadcastScope::VisibleTab {
                    BroadcastScope::Focused
                } else {
                    BroadcastScope::VisibleTab
                };
                WorkspaceCommand::SetBroadcastScope(scope)
            }
            BindingAction::BroadcastManual => {
                let scope = if context.broadcast_scope == BroadcastScope::Manual {
                    BroadcastScope::Focused
                } else {
                    BroadcastScope::Manual
                };
                WorkspaceCommand::SetBroadcastScope(scope)
            }
            BindingAction::ToggleManualTarget => {
                WorkspaceCommand::ToggleManualBroadcastTarget(context.focused_slot)
            }
            BindingAction::ResetBroadcast => WorkspaceCommand::ResetBroadcast,
            BindingAction::SelectionMode => WorkspaceCommand::SelectionMode,
            BindingAction::Quit => WorkspaceCommand::RequestQuit,
        };
        vec![InputRoute::Command(command)]
    }

    fn route_paste(&mut self, bytes: Vec<u8>) -> Vec<InputRoute> {
        if self.modal() {
            self.alt_pending = false;
            self.csi.clear();
            return vec![InputRoute::Consume];
        }
        self.carry_pending = false;
        let mut routes = Vec::new();
        if !self.csi.is_empty() {
            routes.push(InputRoute::Forward(std::mem::take(&mut self.csi)));
        }
        if self.alt_pending {
            self.alt_pending = false;
            routes.push(InputRoute::Forward(vec![0x1b]));
        }
        if !bytes.is_empty() {
            routes.push(InputRoute::Forward(bytes));
        }
        routes
    }

    fn route_timeout(
        &mut self,
        context: &RouterContext,
        config: &BindingConfig,
    ) -> Vec<InputRoute> {
        if !self.csi.is_empty() {
            let bytes = std::mem::take(&mut self.csi);
            // Legacy Alt-[ shares the CSI prefix. Wait for its timeout so
            // complete arrow/function-key reports remain application input.
            if bytes == b"\x1b[" {
                if let Some(routes) = self.modified_key(b'[', 3, context, config) {
                    return routes;
                }
            }
            return if self.leader_pending || self.modal() {
                vec![InputRoute::Consume]
            } else {
                vec![InputRoute::Forward(bytes)]
            };
        }
        if self.alt_pending {
            self.alt_pending = false;
            if self.leader_pending || self.modal() {
                self.leader_pending = false;
                self.resize_mode = false;
                self.broadcast_pending = false;
                return vec![InputRoute::Consume];
            }
            return vec![InputRoute::Forward(vec![0x1b])];
        }
        if self.carry_pending {
            self.carry_pending = false;
        }
        Vec::new()
    }
}

/// Decode CSI-u and xterm modifyOtherKeys press/repeat reports. Unknown or
/// malformed sequences stay byte-for-byte application input.
pub fn decode_modified_key(bytes: &[u8]) -> Option<(u8, u8)> {
    let body = std::str::from_utf8(bytes.strip_prefix(b"\x1b[")?).ok()?;
    let (key, modifiers) = if let Some(body) = body.strip_suffix('u') {
        let mut fields = body.split(';');
        let key = fields.next()?.split(':').next()?.parse::<u8>().ok()?;
        let mut modifier_fields = fields.next().unwrap_or("1").split(':');
        let modifiers = modifier_fields.next()?.parse::<u8>().ok()?;
        let event = modifier_fields.next().unwrap_or("1");
        if event != "1" && event != "2" {
            return None;
        }
        (key, modifiers)
    } else {
        let mut fields = body.strip_suffix('~')?.split(';');
        if fields.next()? != "27" {
            return None;
        }
        let modifiers = fields.next()?.parse::<u8>().ok()?;
        let key = fields.next()?.parse::<u8>().ok()?;
        if fields.next().is_some() {
            return None;
        }
        (key, modifiers)
    };
    Some((key, modifiers))
}
