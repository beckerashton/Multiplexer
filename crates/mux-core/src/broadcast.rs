use std::collections::BTreeSet;

use crate::{BroadcastScope, SessionId, SessionState, WorkspaceView};

/// The complete recipient set for one forwarded byte payload. IDs are sorted,
/// deduplicated, and contain only sessions visible in the active tab.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BroadcastPlan {
    pub recipients: Vec<SessionId>,
}

impl BroadcastPlan {
    pub fn recipients(&self) -> &[SessionId] {
        &self.recipients
    }
}

/// Resolve a broadcast scope from one immutable workspace snapshot.
///
/// The focused session is included only for `Focused` and when it is selected
/// by `VisibleTab` or `Manual`; the integration layer must write once to every
/// returned ID and must not add the origin session independently.
pub fn plan_broadcast(view: &WorkspaceView, scope: BroadcastScope) -> BroadcastPlan {
    let Some(tab) = view.tabs.iter().find(|tab| tab.id == view.active_tab) else {
        return BroadcastPlan::default();
    };
    let mut recipients = BTreeSet::new();

    match scope {
        BroadcastScope::Focused => {
            if let Some(slot) = tab.slots.get(&tab.focused_slot) {
                add_visible(&mut recipients, slot, view);
            }
        }
        BroadcastScope::VisibleTab => {
            for slot in tab.slots.values() {
                add_visible(&mut recipients, slot, view);
            }
        }
        BroadcastScope::Manual => {
            for slot_id in &tab.manual_broadcast_targets {
                if let Some(slot) = tab.slots.get(slot_id) {
                    add_visible(&mut recipients, slot, view);
                }
            }
        }
    }

    BroadcastPlan {
        recipients: recipients.into_iter().collect(),
    }
}

fn add_visible(recipients: &mut BTreeSet<SessionId>, slot: &crate::SlotView, view: &WorkspaceView) {
    let Some(active) = slot.stack.active else {
        return;
    };
    let Some(session) = slot.stack.sessions.get(active).copied() else {
        return;
    };
    if matches!(
        view.sessions.get(&session),
        Some(SessionState::Starting | SessionState::Live)
    ) {
        recipients.insert(session);
    }
}
