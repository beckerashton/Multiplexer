use std::path::PathBuf;

use mux_core::{
    Axis, BroadcastScope, CellRect, Direction, DomainError, LifecycleEffect, SessionId,
    SessionSpec, SessionState, SlotId, TabId, Workspace, WorkspaceCommand,
};

fn shell(name: &str) -> SessionSpec {
    SessionSpec {
        program: PathBuf::from(name),
        args: Vec::new(),
        cwd: None,
    }
}

fn active_tab(view: &mux_core::WorkspaceView) -> &mux_core::TabView {
    view.tabs
        .iter()
        .find(|tab| tab.id == view.active_tab)
        .unwrap()
}

#[test]
fn equalize_changes_only_the_active_layout_without_lifecycle_effects() {
    let (mut workspace, _) = Workspace::new(shell("one"));
    let initial = workspace
        .execute(WorkspaceCommand::Equalize(Axis::Vertical))
        .unwrap();
    assert!(!initial.changed);
    assert!(initial.effects.is_empty());
    let source = workspace.view().active_tab;
    workspace
        .execute(WorkspaceCommand::CreateTab {
            session: shell("other tab"),
        })
        .unwrap();
    workspace
        .execute(WorkspaceCommand::SelectTab(source))
        .unwrap();
    for name in ["two", "three"] {
        workspace
            .execute(WorkspaceCommand::SplitFocused {
                axis: Axis::Vertical,
                session: shell(name),
            })
            .unwrap();
    }
    workspace
        .execute(WorkspaceCommand::AddToFocusedStack {
            session: shell("stacked"),
        })
        .unwrap();
    let mut before = workspace.view();
    let transition = workspace
        .execute(WorkspaceCommand::Equalize(Axis::Vertical))
        .unwrap();
    assert!(transition.changed);
    assert!(transition.effects.is_empty());
    let after = workspace.view();
    before
        .tabs
        .iter_mut()
        .find(|t| t.id == source)
        .unwrap()
        .layout = active_tab(&after).layout.clone();
    assert_eq!(before, after);
    assert!(
        !workspace
            .execute(WorkspaceCommand::Equalize(Axis::Vertical))
            .unwrap()
            .changed
    );
}

#[test]
fn carry_moves_the_full_stack_and_removes_a_sole_source_tab() {
    let (mut workspace, _) = Workspace::new(shell("default"));
    workspace
        .execute(WorkspaceCommand::AddToFocusedStack {
            session: shell("stacked"),
        })
        .unwrap();
    let source = workspace.view().active_tab;
    let source_slot = active_tab(&workspace.view()).focused_slot;
    let source_sessions = active_tab(&workspace.view()).slots[&source_slot]
        .stack
        .sessions
        .clone();
    workspace
        .execute(WorkspaceCommand::CreateTab {
            session: shell("destination"),
        })
        .unwrap();
    let destination = workspace.view().active_tab;
    workspace
        .execute(WorkspaceCommand::SelectTab(source))
        .unwrap();
    workspace
        .execute(WorkspaceCommand::CarryFocusedSlot { destination })
        .unwrap();
    let view = workspace.view();
    assert_eq!(view.active_tab, destination);
    assert_eq!(view.tabs.len(), 1);
    let destination_view = active_tab(&view);
    assert_eq!(destination_view.focused_slot, source_slot);
    assert_eq!(
        destination_view.slots[&source_slot].stack.sessions,
        source_sessions
    );
    assert!(
        source_sessions
            .iter()
            .all(|id| view.sessions.contains_key(id))
    );
}

#[test]
fn removal_merges_complete_stack_without_killing_or_reordering_destination() {
    let (mut workspace, _) = Workspace::new(shell("one"));
    workspace
        .execute(WorkspaceCommand::SplitFocused {
            axis: Axis::Vertical,
            session: shell("two"),
        })
        .unwrap();
    let first = SlotId(1);
    workspace
        .execute(WorkspaceCommand::Focus(Direction::Left))
        .unwrap();
    workspace
        .execute(WorkspaceCommand::AddToFocusedStack {
            session: shell("three"),
        })
        .unwrap();
    let source_sessions = active_tab(&workspace.view()).slots[&first]
        .stack
        .sessions
        .clone();
    let destination_active = active_tab(&workspace.view()).slots[&first].stack.active;
    workspace
        .execute(WorkspaceCommand::Focus(Direction::Right))
        .unwrap();
    let transition = workspace
        .execute(WorkspaceCommand::RemoveFocusedSlot)
        .unwrap();
    assert!(transition.effects.is_empty());
    let view = workspace.view();
    let tab = active_tab(&view);
    assert_eq!(tab.slots.len(), 1);
    let merged = &tab.slots[&first].stack;
    assert_eq!(
        &merged.sessions[..source_sessions.len()],
        &source_sessions[..]
    );
    assert_eq!(merged.active, destination_active);
    assert_eq!(
        workspace.execute(WorkspaceCommand::RemoveFocusedSlot),
        Err(DomainError::FinalSlotRemoval)
    );
}

#[test]
fn destructive_kill_requires_confirmation_and_leaves_an_empty_tab() {
    let (mut workspace, _) = Workspace::new(shell("default"));
    let request = workspace
        .execute(WorkspaceCommand::RequestKillFocusedSession)
        .unwrap();
    assert!(request.effects.is_empty());
    let transition = workspace
        .execute(WorkspaceCommand::ConfirmKillFocusedSession)
        .unwrap();
    assert!(matches!(
        transition.effects.as_slice(),
        [LifecycleEffect::Terminate {
            session: SessionId(1)
        }]
    ));
    let view = workspace.view();
    assert!(active_tab(&view).slots.is_empty());
    let emptied = view.active_tab;
    workspace
        .execute(WorkspaceCommand::NavigateTab { number: 2 })
        .unwrap();
    assert!(workspace.view().tabs.iter().all(|tab| tab.id != emptied));
}

#[test]
fn same_tab_carry_is_noop_and_invalid_destination_is_rejected() {
    let (mut workspace, _) = Workspace::new(shell("default"));
    let tab = workspace.view().active_tab;
    assert!(
        !workspace
            .execute(WorkspaceCommand::CarryFocusedSlot { destination: tab })
            .unwrap()
            .changed
    );
    assert_eq!(
        workspace.execute(WorkspaceCommand::CarryFocusedSlot {
            destination: TabId(99)
        }),
        Err(DomainError::UnknownTab(TabId(99)))
    );
}

#[test]
fn local_and_global_splits_preserve_nested_layouts_and_sessions() {
    for axis in [Axis::Horizontal, Axis::Vertical] {
        for global in [false, true] {
            let (mut workspace, _) = Workspace::new(shell("one"));
            let bounds = CellRect {
                x: 4,
                y: 2,
                cols: 120,
                rows: 80,
            };
            workspace.set_bounds(bounds);
            for axis in [Axis::Vertical, Axis::Horizontal] {
                workspace
                    .execute(WorkspaceCommand::SplitFocused {
                        axis,
                        session: shell("existing"),
                    })
                    .unwrap();
            }
            workspace
                .execute(WorkspaceCommand::AddToFocusedStack {
                    session: shell("stacked"),
                })
                .unwrap();
            let before = workspace.view();
            let old_tab = active_tab(&before);
            let old_rects = old_tab.layout.geometry(bounds, before.minimum_pane_size);
            let target = if global {
                bounds
            } else {
                old_rects[&old_tab.focused_slot]
            };
            let (mut first, mut second) = (target, target);
            match axis {
                Axis::Horizontal => {
                    first.rows /= 2;
                    second.y += first.rows;
                    second.rows -= first.rows;
                }
                Axis::Vertical => {
                    first.cols /= 2;
                    second.x += first.cols;
                    second.cols -= first.cols;
                }
            }
            let session = shell("new");
            let command = if global {
                WorkspaceCommand::SplitTab {
                    axis,
                    session: session.clone(),
                }
            } else {
                WorkspaceCommand::SplitFocused {
                    axis,
                    session: session.clone(),
                }
            };
            let transition = workspace.execute(command).unwrap();
            let after = workspace.view();
            let tab = active_tab(&after);
            let rects = tab.layout.geometry(bounds, after.minimum_pane_size);
            assert_eq!(rects[&tab.focused_slot], second);
            let mut expected = if global {
                old_tab.layout.geometry(first, before.minimum_pane_size)
            } else {
                old_rects
            };
            if !global {
                expected.insert(old_tab.focused_slot, first);
            }
            for (slot, rect) in expected {
                assert_eq!(rects[&slot], rect);
                assert_eq!(tab.slots[&slot], old_tab.slots[&slot]);
            }
            assert_eq!(after.sessions.len(), before.sessions.len() + 1);
            for (id, state) in before.sessions {
                assert_eq!(after.sessions[&id], state);
            }
            assert_eq!(
                transition.effects,
                vec![LifecycleEffect::Spawn {
                    session: tab.slots[&tab.focused_slot].stack.sessions[0],
                    spec: session,
                }]
            );
            assert!(transition.changed);
        }
    }
}

#[test]
fn small_bounds_reject_split_and_carry_without_partially_changing_any_tab() {
    let (mut workspace, _) = Workspace::new(shell("default"));
    workspace.set_bounds(CellRect {
        x: 0,
        y: 0,
        cols: 8,
        rows: 3,
    });
    let before = workspace.view();
    assert_eq!(
        workspace.execute(WorkspaceCommand::SplitFocused {
            axis: Axis::Vertical,
            session: shell("rejected"),
        }),
        Err(DomainError::Layout(mux_core::LayoutError::MinimumSize))
    );
    assert_eq!(workspace.view(), before);
    for axis in [Axis::Horizontal, Axis::Vertical] {
        assert_eq!(
            workspace.execute(WorkspaceCommand::SplitTab {
                axis,
                session: shell("rejected"),
            }),
            Err(DomainError::Layout(mux_core::LayoutError::MinimumSize))
        );
        assert_eq!(workspace.view(), before);
    }
    workspace.set_bounds(CellRect {
        x: 0,
        y: 0,
        cols: 80,
        rows: 24,
    });
    workspace
        .execute(WorkspaceCommand::CreateTab {
            session: shell("destination"),
        })
        .unwrap();
    let destination = workspace.view().active_tab;
    workspace
        .execute(WorkspaceCommand::SelectTab(TabId(1)))
        .unwrap();
    workspace.set_bounds(CellRect {
        x: 0,
        y: 0,
        cols: 8,
        rows: 3,
    });
    let before_carry = workspace.view();
    assert_eq!(
        workspace.execute(WorkspaceCommand::CarryFocusedSlot { destination }),
        Err(DomainError::Layout(mux_core::LayoutError::MinimumSize))
    );
    assert_eq!(workspace.view(), before_carry);
}

#[test]
fn killing_last_member_of_nonsole_slot_removes_that_slot_not_the_other_tab_session() {
    let (mut workspace, _) = Workspace::new(shell("one"));
    workspace
        .execute(WorkspaceCommand::SplitFocused {
            axis: Axis::Vertical,
            session: shell("two"),
        })
        .unwrap();
    let killed = active_tab(&workspace.view()).focused_slot;
    workspace
        .execute(WorkspaceCommand::RequestKillFocusedSession)
        .unwrap();
    let transition = workspace
        .execute(WorkspaceCommand::ConfirmKillFocusedSession)
        .unwrap();
    assert!(matches!(
        transition.effects.as_slice(),
        [LifecycleEffect::Terminate { .. }]
    ));
    let view = workspace.view();
    let tab = active_tab(&view);
    assert_eq!(tab.slots.len(), 1);
    assert!(!tab.slots.contains_key(&killed));
    assert!(
        tab.slots
            .values()
            .all(|slot| !slot.stack.sessions.is_empty())
    );
}

#[test]
fn manual_broadcast_requires_eligible_members_and_exits_clear_the_last_target() {
    let (mut workspace, _) = Workspace::new(shell("one"));
    assert_eq!(
        workspace.execute(WorkspaceCommand::SetBroadcastScope(BroadcastScope::Manual)),
        Err(DomainError::NoManualBroadcastTargets)
    );
    let slot = active_tab(&workspace.view()).focused_slot;
    workspace
        .execute(WorkspaceCommand::ToggleManualBroadcastTarget(slot))
        .unwrap();
    workspace
        .execute(WorkspaceCommand::SetBroadcastScope(BroadcastScope::Manual))
        .unwrap();
    workspace
        .session_state_changed(SessionId(1), SessionState::Exited)
        .unwrap();
    let view = workspace.view();
    assert_eq!(view.broadcast_scope, BroadcastScope::Focused);
    assert!(active_tab(&view).manual_broadcast_targets.is_empty());
    assert_eq!(
        workspace.execute(WorkspaceCommand::ToggleManualBroadcastTarget(slot)),
        Err(DomainError::UnknownSlot(slot))
    );
}

#[test]
fn cancelling_after_a_natural_exit_never_revives_the_exited_session() {
    let (mut workspace, _) = Workspace::new(shell("one"));
    workspace
        .execute(WorkspaceCommand::RequestKillFocusedSession)
        .unwrap();
    workspace
        .session_state_changed(SessionId(1), SessionState::Exited)
        .unwrap();
    assert!(!workspace.clear_pending_confirmation());
    assert_eq!(
        workspace.view().sessions[&SessionId(1)],
        SessionState::Exited
    );
}

#[test]
fn numbered_tabs_are_created_on_demand_and_numbers_remain_stable() {
    let (mut workspace, _) = Workspace::new(shell("default"));
    let transition = workspace
        .execute(WorkspaceCommand::NavigateTab { number: 7 })
        .unwrap();
    assert!(matches!(
        transition.effects.as_slice(),
        [LifecycleEffect::Spawn { .. }]
    ));
    assert_eq!(
        workspace
            .view()
            .tabs
            .iter()
            .map(|tab| tab.number)
            .collect::<Vec<_>>(),
        vec![1, 7]
    );
    let id = workspace.view().active_tab;
    assert!(
        workspace
            .execute(WorkspaceCommand::NavigateTab { number: 7 })
            .unwrap()
            .effects
            .is_empty()
    );
    assert_eq!(workspace.view().active_tab, id);
    workspace
        .execute(WorkspaceCommand::RequestKillFocusedStack)
        .unwrap();
    workspace
        .execute(WorkspaceCommand::ConfirmKillFocusedStack)
        .unwrap();
    assert!(active_tab(&workspace.view()).slots.is_empty());
    workspace
        .execute(WorkspaceCommand::NavigateTab { number: 1 })
        .unwrap();
    assert_eq!(workspace.view().tabs.len(), 1);
    workspace
        .execute(WorkspaceCommand::NavigateTab { number: 7 })
        .unwrap();
    assert_ne!(workspace.view().active_tab, id);
    assert_eq!(active_tab(&workspace.view()).number, 7);
}

#[test]
fn carry_to_missing_tab_preserves_stack_without_spawning_a_shell() {
    let (mut workspace, _) = Workspace::new(shell("default"));
    workspace
        .execute(WorkspaceCommand::AddToFocusedStack {
            session: shell("second"),
        })
        .unwrap();
    let before = active_tab(&workspace.view()).slots.clone();
    let transition = workspace
        .execute(WorkspaceCommand::CarryToTab { number: 9 })
        .unwrap();
    assert!(transition.effects.is_empty());
    let view = workspace.view();
    assert_eq!(view.tabs.len(), 1);
    assert_eq!(active_tab(&view).number, 9);
    assert_eq!(active_tab(&view).slots, before);
}

#[test]
fn natural_exit_empties_tab_and_split_can_repopulate_it() {
    for axis in [Axis::Horizontal, Axis::Vertical] {
        for global in [false, true] {
            let (mut workspace, _) = Workspace::new(shell("default"));
            workspace
                .session_state_changed(SessionId(1), SessionState::Exited)
                .unwrap();
            assert!(active_tab(&workspace.view()).slots.is_empty());
            let session = shell("replacement");
            let command = if global {
                WorkspaceCommand::SplitTab { axis, session }
            } else {
                WorkspaceCommand::SplitFocused { axis, session }
            };
            let transition = workspace.execute(command).unwrap();
            assert_eq!(transition.effects.len(), 1);
            assert_eq!(active_tab(&workspace.view()).slots.len(), 1);
        }
    }
}

#[test]
fn an_unfocused_panes_natural_exit_does_not_steal_focus() {
    let (mut workspace, _) = Workspace::new(shell("first"));
    for _ in 0..2 {
        workspace
            .execute(WorkspaceCommand::SplitFocused {
                axis: Axis::Vertical,
                session: shell("other"),
            })
            .unwrap();
    }
    let focused = active_tab(&workspace.view()).focused_slot;
    workspace
        .session_state_changed(SessionId(1), SessionState::Exited)
        .unwrap();
    assert_eq!(active_tab(&workspace.view()).focused_slot, focused);
    assert_eq!(active_tab(&workspace.view()).slots.len(), 2);
}

#[test]
fn directional_focus_wraps_vertically_and_preserves_entry_selection() {
    let (mut workspace, _) = Workspace::new(shell("one"));
    let first = active_tab(&workspace.view()).focused_slot;
    for name in ["two", "three"] {
        workspace
            .execute(WorkspaceCommand::AddToFocusedStack {
                session: shell(name),
            })
            .unwrap();
    }
    workspace
        .execute(WorkspaceCommand::SelectStackMember { index: 1 })
        .unwrap();
    workspace
        .execute(WorkspaceCommand::SplitFocused {
            axis: Axis::Horizontal,
            session: shell("below"),
        })
        .unwrap();
    let below = active_tab(&workspace.view()).focused_slot;
    workspace
        .execute(WorkspaceCommand::Focus(Direction::Up))
        .unwrap();
    let view = workspace.view();
    assert_eq!(active_tab(&view).focused_slot, first);
    assert_eq!(active_tab(&view).slots[&first].stack.active, Some(1));
    workspace
        .execute(WorkspaceCommand::Focus(Direction::Up))
        .unwrap();
    assert_eq!(
        active_tab(&workspace.view()).slots[&first].stack.active,
        Some(0)
    );
    workspace
        .execute(WorkspaceCommand::Focus(Direction::Up))
        .unwrap();
    assert_eq!(active_tab(&workspace.view()).focused_slot, below);
    workspace
        .execute(WorkspaceCommand::Focus(Direction::Down))
        .unwrap();
    assert_eq!(active_tab(&workspace.view()).focused_slot, first);
    for _ in 0..2 {
        workspace
            .execute(WorkspaceCommand::Focus(Direction::Down))
            .unwrap();
    }
    assert_eq!(active_tab(&workspace.view()).focused_slot, first);
    workspace
        .execute(WorkspaceCommand::Focus(Direction::Down))
        .unwrap();
    assert_eq!(active_tab(&workspace.view()).focused_slot, below);
    workspace
        .execute(WorkspaceCommand::CyclePane { delta: -1 })
        .unwrap();
    assert_eq!(
        active_tab(&workspace.view()).slots[&first].stack.active,
        Some(2)
    );
    workspace
        .execute(WorkspaceCommand::SelectStackMember { index: 1 })
        .unwrap();
    workspace
        .execute(WorkspaceCommand::CyclePane { delta: 1 })
        .unwrap();
    assert_eq!(active_tab(&workspace.view()).focused_slot, below);
    workspace
        .execute(WorkspaceCommand::CyclePane { delta: 1 })
        .unwrap();
    assert_eq!(
        active_tab(&workspace.view()).slots[&first].stack.active,
        Some(1)
    );
}

#[test]
fn member_swaps_follow_focus_and_leave_hidden_neighbors_in_place() {
    let (mut workspace, _) = Workspace::new(shell("a"));
    workspace
        .execute(WorkspaceCommand::AddToFocusedStack {
            session: shell("b"),
        })
        .unwrap();
    let left = active_tab(&workspace.view()).focused_slot;
    let before = active_tab(&workspace.view()).slots[&left].stack.clone();
    workspace
        .execute(WorkspaceCommand::SwapMember(Direction::Up))
        .unwrap();
    let reordered = active_tab(&workspace.view()).slots[&left].stack.clone();
    assert_eq!(
        reordered.sessions,
        vec![before.sessions[1], before.sessions[0]]
    );
    assert_eq!(reordered.active, Some(0));
    workspace
        .execute(WorkspaceCommand::SplitFocused {
            axis: Axis::Vertical,
            session: shell("c"),
        })
        .unwrap();
    workspace
        .execute(WorkspaceCommand::AddToFocusedStack {
            session: shell("d"),
        })
        .unwrap();
    let right = active_tab(&workspace.view()).focused_slot;
    let right_stack = active_tab(&workspace.view()).slots[&right].stack.clone();
    workspace
        .execute(WorkspaceCommand::Focus(Direction::Left))
        .unwrap();
    workspace
        .execute(WorkspaceCommand::SwapMember(Direction::Right))
        .unwrap();
    let view = workspace.view();
    let tab = active_tab(&view);
    assert_eq!(tab.focused_slot, right);
    assert_eq!(
        tab.slots[&left].stack.sessions,
        vec![right_stack.sessions[1], before.sessions[0]]
    );
    assert_eq!(
        tab.slots[&right].stack.sessions,
        vec![right_stack.sessions[0], before.sessions[1]]
    );
    assert_eq!(tab.slots[&right].stack.active, Some(1));
    let stacks = tab.slots.clone();
    workspace
        .execute(WorkspaceCommand::Swap(Direction::Left))
        .unwrap();
    assert_eq!(active_tab(&workspace.view()).slots, stacks);
    assert_eq!(active_tab(&workspace.view()).focused_slot, right);
}

#[test]
fn carrying_one_member_preserves_other_members_and_tab_selection() {
    let (mut workspace, _) = Workspace::new(shell("a"));
    workspace
        .execute(WorkspaceCommand::AddToFocusedStack {
            session: shell("b"),
        })
        .unwrap();
    workspace
        .execute(WorkspaceCommand::AddToFocusedStack {
            session: shell("c"),
        })
        .unwrap();
    workspace
        .execute(WorkspaceCommand::SelectStackMember { index: 1 })
        .unwrap();
    let source_tab = workspace.view().active_tab;
    let source = active_tab(&workspace.view()).focused_slot;
    let members = active_tab(&workspace.view()).slots[&source]
        .stack
        .sessions
        .clone();
    let result = workspace
        .execute(WorkspaceCommand::CarryMemberToTab { number: 2 })
        .unwrap();
    assert!(result.effects.is_empty());
    let view = workspace.view();
    assert_eq!(view.tabs.len(), 2);
    let destination = active_tab(&view);
    assert_eq!(
        destination.slots[&destination.focused_slot].stack.sessions,
        vec![members[1]]
    );
    workspace
        .execute(WorkspaceCommand::SelectTab(source_tab))
        .unwrap();
    let view = workspace.view();
    assert_eq!(
        active_tab(&view).slots[&source].stack.sessions,
        vec![members[0], members[2]]
    );
    assert_eq!(active_tab(&view).slots[&source].stack.active, Some(1));
    assert!(
        !workspace
            .execute(WorkspaceCommand::CarryMemberToTab { number: 1 })
            .unwrap()
            .changed
    );
    workspace.set_bounds(CellRect {
        x: 0,
        y: 0,
        cols: 8,
        rows: 3,
    });
    let before = workspace.view();
    assert!(
        workspace
            .execute(WorkspaceCommand::CarryMemberToTab { number: 2 })
            .is_err()
    );
    assert_eq!(workspace.view(), before);
}

#[test]
fn directional_focus_uses_the_cursor_at_t_junctions_and_vertical_wraps() {
    for direction in [
        Direction::Left,
        Direction::Right,
        Direction::Up,
        Direction::Down,
    ] {
        let (mut workspace, _) = Workspace::new(shell("one"));
        let (axis, perpendicular, reverse) = match direction {
            Direction::Left => (Axis::Vertical, Axis::Horizontal, Direction::Right),
            Direction::Right => (Axis::Vertical, Axis::Horizontal, Direction::Left),
            Direction::Up => (Axis::Horizontal, Axis::Vertical, Direction::Down),
            Direction::Down => (Axis::Horizontal, Axis::Vertical, Direction::Up),
        };
        let forward = matches!(direction, Direction::Right | Direction::Down);
        let original = active_tab(&workspace.view()).focused_slot;
        if forward {
            workspace
                .execute(WorkspaceCommand::SplitFocused {
                    axis,
                    session: shell("two"),
                })
                .unwrap();
        }
        let first = active_tab(&workspace.view()).focused_slot;
        workspace
            .execute(WorkspaceCommand::SplitFocused {
                axis: perpendicular,
                session: shell("three"),
            })
            .unwrap();
        let second = active_tab(&workspace.view()).focused_slot;
        if forward {
            workspace.execute(WorkspaceCommand::Focus(reverse)).unwrap();
            assert_eq!(active_tab(&workspace.view()).focused_slot, original);
        } else {
            workspace
                .execute(WorkspaceCommand::SplitTab {
                    axis,
                    session: shell("source"),
                })
                .unwrap();
        }
        let view = workspace.view();
        let tab = active_tab(&view);
        let rects = tab.layout.geometry(view.bounds, view.minimum_pane_size);
        let source = rects[&tab.focused_slot];
        for expected in [first, second] {
            let target = rects[&expected];
            let cursor = if axis == Axis::Vertical {
                (source.x + source.cols / 2, target.y + target.rows / 2)
            } else {
                (target.x + target.cols / 2, source.y + source.rows / 2)
            };
            let mut positioned = workspace.clone();
            let transition = positioned.focus_at(direction, Some(cursor)).unwrap();
            assert_eq!(active_tab(&positioned.view()).focused_slot, expected);
            assert!(transition.effects.is_empty());
            if axis == Axis::Horizontal {
                let mut wrapped = workspace.clone();
                wrapped.focus_at(reverse, Some(cursor)).unwrap();
                assert_eq!(active_tab(&wrapped.view()).focused_slot, expected);
            }
        }
    }
}

#[test]
fn horizontal_tab_entry_tracks_cursor_row_even_from_a_full_height_pane() {
    let (mut workspace, _) = Workspace::new(shell("source"));
    let source = workspace.view().active_tab;
    workspace
        .execute(WorkspaceCommand::NavigateTab { number: 2 })
        .unwrap();
    let destination = workspace.view().active_tab;
    let top = active_tab(&workspace.view()).focused_slot;
    workspace
        .execute(WorkspaceCommand::SplitFocused {
            axis: Axis::Horizontal,
            session: shell("bottom"),
        })
        .unwrap();
    let bottom = active_tab(&workspace.view()).focused_slot;
    let view = workspace.view();
    let rects = active_tab(&view)
        .layout
        .geometry(view.bounds, view.minimum_pane_size);
    workspace
        .execute(WorkspaceCommand::SelectTab(source))
        .unwrap();
    for direction in [Direction::Left, Direction::Right] {
        for expected in [top, bottom] {
            let mut positioned = workspace.clone();
            let rect = rects[&expected];
            positioned
                .focus_at(direction, Some((1, rect.y + rect.rows / 2)))
                .unwrap();
            let view = positioned.view();
            assert_eq!(view.active_tab, destination);
            assert_eq!(active_tab(&view).focused_slot, expected);
        }
    }
}

#[test]
fn horizontal_tab_entry_chooses_the_aligned_edge_pane_instead_of_remembered_focus() {
    for direction in [Direction::Left, Direction::Right] {
        for bottom in [false, true] {
            let (mut workspace, _) = Workspace::new(shell("source top"));
            let source = workspace.view().active_tab;
            workspace
                .execute(WorkspaceCommand::SplitFocused {
                    axis: Axis::Horizontal,
                    session: shell("source bottom"),
                })
                .unwrap();
            if !bottom {
                workspace
                    .execute(WorkspaceCommand::Focus(Direction::Up))
                    .unwrap();
            }

            workspace
                .execute(WorkspaceCommand::NavigateTab { number: 2 })
                .unwrap();
            let destination = workspace.view().active_tab;
            let left_top = active_tab(&workspace.view()).focused_slot;
            workspace
                .execute(WorkspaceCommand::SplitFocused {
                    axis: Axis::Vertical,
                    session: shell("right top"),
                })
                .unwrap();
            let right_top = active_tab(&workspace.view()).focused_slot;
            workspace
                .execute(WorkspaceCommand::SplitFocused {
                    axis: Axis::Horizontal,
                    session: shell("right bottom"),
                })
                .unwrap();
            let right_bottom = active_tab(&workspace.view()).focused_slot;
            workspace
                .execute(WorkspaceCommand::Focus(Direction::Left))
                .unwrap();
            workspace
                .execute(WorkspaceCommand::SplitFocused {
                    axis: Axis::Horizontal,
                    session: shell("left bottom"),
                })
                .unwrap();
            let left_bottom = active_tab(&workspace.view()).focused_slot;
            let slots = [left_top, left_bottom, right_top, right_bottom];
            let expected_index =
                if direction == Direction::Left { 2 } else { 0 } + usize::from(bottom);
            let expected = slots[expected_index];
            workspace
                .execute(WorkspaceCommand::CyclePane {
                    delta: expected_index as i8 - 1,
                })
                .unwrap();
            workspace
                .execute(WorkspaceCommand::AddToFocusedStack {
                    session: shell("hidden"),
                })
                .unwrap();
            workspace
                .execute(WorkspaceCommand::SelectStackMember { index: 0 })
                .unwrap();
            let stack = active_tab(&workspace.view()).slots[&expected].stack.clone();
            // Remember a pane on the wrong edge and at the wrong height.
            let remembered_index = 3 - expected_index;
            workspace
                .execute(WorkspaceCommand::CyclePane {
                    delta: remembered_index as i8 - expected_index as i8,
                })
                .unwrap();
            workspace
                .execute(WorkspaceCommand::SelectTab(source))
                .unwrap();
            workspace
                .execute(WorkspaceCommand::NavigateTab { number: 2 })
                .unwrap();
            assert_eq!(
                active_tab(&workspace.view()).focused_slot,
                slots[remembered_index]
            );
            workspace
                .execute(WorkspaceCommand::SelectTab(source))
                .unwrap();
            workspace
                .execute(WorkspaceCommand::SetBroadcastScope(
                    BroadcastScope::VisibleTab,
                ))
                .unwrap();
            let sessions = workspace.view().sessions;

            let transition = workspace
                .execute(WorkspaceCommand::Focus(direction))
                .unwrap();
            let view = workspace.view();
            assert!(transition.changed);
            assert!(transition.effects.is_empty());
            assert_eq!(view.active_tab, destination);
            assert_eq!(active_tab(&view).focused_slot, expected);
            assert_eq!(active_tab(&view).slots[&expected].stack, stack);
            assert_eq!(view.broadcast_scope, BroadcastScope::Focused);
            assert_eq!(view.sessions, sessions);
        }
    }
}

#[test]
fn directional_edges_and_pane_jumps_wrap_populated_tabs_and_restore_selection() {
    let (mut workspace, _) = Workspace::new(shell("one"));
    let one = workspace.view().active_tab;
    workspace
        .execute(WorkspaceCommand::NavigateTab { number: 2 })
        .unwrap();
    let empty = workspace.view().active_tab;
    let view = workspace.view();
    let slot = active_tab(&view).focused_slot;
    let exiting = active_tab(&view).slots[&slot].stack.sessions[0];
    workspace
        .execute(WorkspaceCommand::NavigateTab { number: 3 })
        .unwrap();
    let three = workspace.view().active_tab;
    workspace
        .execute(WorkspaceCommand::AddToFocusedStack {
            session: shell("hidden"),
        })
        .unwrap();
    workspace
        .execute(WorkspaceCommand::SelectStackMember { index: 0 })
        .unwrap();
    let selected = active_tab(&workspace.view()).focused_slot;
    workspace
        .session_state_changed(exiting, SessionState::Exited)
        .unwrap();
    assert!(
        workspace
            .view()
            .tabs
            .iter()
            .find(|t| t.id == empty)
            .unwrap()
            .slots
            .is_empty()
    );
    workspace.execute(WorkspaceCommand::SelectTab(one)).unwrap();
    workspace
        .execute(WorkspaceCommand::SetBroadcastScope(
            BroadcastScope::VisibleTab,
        ))
        .unwrap();
    workspace
        .execute(WorkspaceCommand::Focus(Direction::Right))
        .unwrap();
    let view = workspace.view();
    assert_eq!(view.active_tab, three);
    assert_eq!(view.broadcast_scope, BroadcastScope::Focused);
    assert_eq!(active_tab(&view).focused_slot, selected);
    assert_eq!(active_tab(&view).slots[&selected].stack.active, Some(0));
    workspace
        .execute(WorkspaceCommand::Focus(Direction::Right))
        .unwrap();
    assert_eq!(workspace.view().active_tab, one);
    workspace
        .execute(WorkspaceCommand::Focus(Direction::Left))
        .unwrap();
    assert_eq!(workspace.view().active_tab, three);
    workspace
        .execute(WorkspaceCommand::CyclePane { delta: 1 })
        .unwrap();
    assert_eq!(workspace.view().active_tab, one);
    workspace
        .execute(WorkspaceCommand::CyclePane { delta: -1 })
        .unwrap();
    assert_eq!(workspace.view().active_tab, three);
    // Vertical movement stays in tab 3 and cycles its sole stack.
    for (direction, member) in [
        (Direction::Up, 1),
        (Direction::Down, 0),
        (Direction::Down, 1),
        (Direction::Down, 0),
    ] {
        workspace
            .execute(WorkspaceCommand::Focus(direction))
            .unwrap();
        assert_eq!(workspace.view().active_tab, three);
        assert_eq!(
            active_tab(&workspace.view()).slots[&selected].stack.active,
            Some(member)
        );
    }
}

#[test]
fn resizing_a_stack_moves_its_edge_without_changing_members() {
    let (mut workspace, _) = Workspace::new(shell("one"));
    workspace.set_bounds(CellRect {
        x: 0,
        y: 0,
        cols: 137,
        rows: 41,
    });
    workspace
        .execute(WorkspaceCommand::SplitFocused {
            axis: Axis::Vertical,
            session: shell("two"),
        })
        .unwrap();
    workspace
        .execute(WorkspaceCommand::AddToFocusedStack {
            session: shell("three"),
        })
        .unwrap();
    let before = workspace.view();
    let tab = active_tab(&before);
    let slot = tab.focused_slot;
    let original_stack = tab.slots[&slot].stack.clone();
    let rect = tab.layout.geometry(before.bounds, before.minimum_pane_size)[&slot];
    for step in 1..=5 {
        workspace
            .execute(WorkspaceCommand::Resize {
                direction: Direction::Left,
                cells: 1,
            })
            .unwrap();
        let after = workspace.view();
        let tab = active_tab(&after);
        let resized = tab.layout.geometry(after.bounds, after.minimum_pane_size)[&slot];
        assert_eq!(resized.x, rect.x - step);
        assert_eq!(resized.cols, rect.cols + step);
        assert_eq!(tab.slots[&slot].stack, original_stack);
        assert_eq!(after.sessions, before.sessions);
    }
}

#[test]
fn large_resize_steps_use_remaining_space_at_minimum_sizes() {
    let (mut workspace, _) = Workspace::new(shell("one"));
    workspace.set_bounds(CellRect {
        x: 0,
        y: 0,
        cols: 22,
        rows: 20,
    });
    workspace
        .execute(WorkspaceCommand::SplitFocused {
            axis: Axis::Vertical,
            session: shell("two"),
        })
        .unwrap();
    workspace
        .execute(WorkspaceCommand::Resize {
            direction: Direction::Left,
            cells: 5,
        })
        .unwrap();
    let view = workspace.view();
    let tab = active_tab(&view);
    let rect = tab.layout.geometry(view.bounds, view.minimum_pane_size)[&tab.focused_slot];
    assert_eq!(rect.x, view.minimum_pane_size.cols);
    assert_eq!(rect.cols, 22 - view.minimum_pane_size.cols);
}

#[test]
fn borderless_is_per_tab_and_marks_follow_members_across_tabs() {
    let (mut workspace, _) = Workspace::new(shell("one"));
    workspace.execute(WorkspaceCommand::ToggleBorderless).unwrap();
    assert!(workspace.borderless());
    let (restored, _) = Workspace::restore_layout(workspace.layout_snapshot(), shell("restored")).unwrap();
    assert!(restored.borderless());
    workspace.execute(WorkspaceCommand::SetJumpMark(b'a')).unwrap();
    workspace.execute(WorkspaceCommand::AddToFocusedStack { session: shell("two") }).unwrap();
    workspace.execute(WorkspaceCommand::NavigateTab { number: 2 }).unwrap();
    assert!(!workspace.borderless());
    workspace.execute(WorkspaceCommand::JumpToMark(b'a')).unwrap();
    assert!(workspace.borderless());
    assert_eq!(active_tab(&workspace.view()).number, 1);
    let view = workspace.view();
    let tab = active_tab(&view);
    assert_eq!(tab.slots[&tab.focused_slot].stack.active, Some(0));
    workspace.execute(WorkspaceCommand::CarryMemberToTab { number: 2 }).unwrap();
    workspace.execute(WorkspaceCommand::NavigateTab { number: 1 }).unwrap();
    workspace.execute(WorkspaceCommand::JumpToMark(b'a')).unwrap();
    assert_eq!(active_tab(&workspace.view()).number, 2);
    assert!(!workspace.borderless());
    workspace.execute(WorkspaceCommand::SetJumpMark(b'a')).unwrap();
    workspace.execute(WorkspaceCommand::RequestKillFocusedSession).unwrap();
    workspace.execute(WorkspaceCommand::ConfirmKillFocusedSession).unwrap();
    assert!(!workspace.execute(WorkspaceCommand::JumpToMark(b'a')).unwrap().changed);
    assert!(!workspace.execute(WorkspaceCommand::JumpToMark(b'?')).unwrap().changed);
    workspace.execute(WorkspaceCommand::NavigateTab { number: 1 }).unwrap();
    workspace.execute(WorkspaceCommand::ToggleBorderless).unwrap();
    assert!(!workspace.borderless());
}
