use std::path::PathBuf;

use mux_core::{
    BroadcastScope, SessionSpec, SessionState, SlotId, Workspace, WorkspaceCommand, plan_broadcast,
};

fn shell() -> SessionSpec {
    SessionSpec {
        program: PathBuf::from("/bin/sh"),
        args: Vec::new(),
        cwd: None,
    }
}

#[test]
fn visible_scope_excludes_hidden_dead_and_other_tab_sessions() {
    let (mut workspace, _) = Workspace::new(shell());
    let first = workspace.view().tabs[0].slots[&SlotId(1)].stack.sessions[0];
    workspace
        .execute(WorkspaceCommand::AddToFocusedStack { session: shell() })
        .unwrap();
    let second = workspace.view().tabs[0].slots[&SlotId(1)].stack.sessions[1];
    workspace
        .execute(WorkspaceCommand::CreateTab { session: shell() })
        .unwrap();
    let other_tab = workspace.view().active_tab;
    let other = workspace
        .view()
        .tabs
        .iter()
        .find(|tab| tab.id == other_tab)
        .unwrap()
        .slots
        .values()
        .next()
        .unwrap()
        .stack
        .sessions[0];
    workspace
        .execute(WorkspaceCommand::SelectTab(mux_core::TabId(1)))
        .unwrap();
    workspace
        .session_state_changed(first, SessionState::Exited)
        .unwrap();

    let plan = plan_broadcast(&workspace.view(), BroadcastScope::VisibleTab);
    assert_eq!(plan.recipients, vec![second]);
    assert!(!plan.recipients.contains(&other));
}

#[test]
fn manual_scope_can_exclude_focus_and_deduplicates() {
    let (mut workspace, _) = Workspace::new(shell());
    workspace
        .execute(WorkspaceCommand::SplitFocused {
            axis: mux_core::Axis::Vertical,
            session: shell(),
        })
        .unwrap();
    let view = workspace.view();
    let tab = &view.tabs[0];
    let focused = tab.focused_slot;
    let other = *tab.slots.keys().find(|id| **id != focused).unwrap();
    workspace
        .execute(WorkspaceCommand::ToggleManualBroadcastTarget(other))
        .unwrap();
    workspace
        .execute(WorkspaceCommand::SetBroadcastScope(BroadcastScope::Manual))
        .unwrap();
    let plan = plan_broadcast(&workspace.view(), BroadcastScope::Manual);
    let expected = workspace.view().tabs[0].slots[&other].stack.sessions[0];
    assert_eq!(plan.recipients, vec![expected]);
    assert!(
        !plan
            .recipients
            .contains(&workspace.view().tabs[0].slots[&focused].stack.sessions[0])
    );
}

#[test]
fn focused_scope_is_exactly_one_visible_session() {
    let (workspace, _) = Workspace::new(shell());
    let view = workspace.view();
    let expected = view.tabs[0].slots[&view.tabs[0].focused_slot]
        .stack
        .sessions[0];
    assert_eq!(
        plan_broadcast(&view, BroadcastScope::Focused).recipients,
        vec![expected]
    );
}
