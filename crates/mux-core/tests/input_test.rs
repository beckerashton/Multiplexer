use std::path::PathBuf;

use mux_core::{
    Axis, BindingAction, BindingConfig, BroadcastScope, Direction, InputEvent, InputRoute,
    InputRouter, RouterContext, SessionSpec, SlotId, TabId, WorkspaceCommand,
};

fn context() -> RouterContext {
    RouterContext {
        current_tab: TabId(1),
        tab_by_number: [
            Some(TabId(1)),
            Some(TabId(2)),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ],
        focused_slot: SlotId(7),
        default_session: SessionSpec {
            program: PathBuf::from("/bin/sh"),
            args: Vec::new(),
            cwd: None,
        },
        broadcast_scope: BroadcastScope::Focused,
    }
}

#[test]
fn equalize_bindings_route_each_axis_and_leave_plain_input_alone() {
    let mut config = BindingConfig::default();
    for (key, axis) in [(b'=', Axis::Vertical), (b'+', Axis::Horizontal)] {
        let mut router = InputRouter::new();
        assert_eq!(
            router.route(InputEvent::Bytes(vec![key]), &context(), &config),
            vec![InputRoute::Forward(vec![key])]
        );
        assert_eq!(
            router.route(
                InputEvent::Bytes(vec![config.leader(), key]),
                &context(),
                &config
            ),
            vec![
                InputRoute::Consume,
                InputRoute::Command(WorkspaceCommand::Equalize(axis))
            ]
        );
        assert!(!router.leader_pending());
    }
    config
        .rebind(b'e', BindingAction::Equalize(Axis::Vertical))
        .unwrap();
    let mut router = InputRouter::new();
    assert_eq!(
        router.route(
            InputEvent::Bytes(vec![config.leader(), b'e']),
            &context(),
            &config
        ),
        vec![
            InputRoute::Consume,
            InputRoute::Command(WorkspaceCommand::Equalize(Axis::Vertical))
        ]
    );
    assert_eq!(config.binding(b'='), None);
}

#[test]
fn ordinary_bytes_are_forwarded_byte_for_byte() {
    let mut router = InputRouter::new();
    let config = BindingConfig::default();
    let routes = router.route(
        InputEvent::Bytes(vec![0, 0x1b, b'[', b'2', b'0', b'~']),
        &context(),
        &config,
    );
    assert_eq!(
        routes,
        vec![InputRoute::Forward(vec![0, 0x1b, b'[', b'2', b'0', b'~'])]
    );
}

#[test]
fn leader_commands_are_consumed_and_literal_leader_is_forwarded() {
    let mut router = InputRouter::new();
    let config = BindingConfig::default();
    assert_eq!(
        router.route(InputEvent::Bytes(vec![0x02, b'|']), &context(), &config),
        vec![
            InputRoute::Consume,
            InputRoute::Command(WorkspaceCommand::SplitTab {
                axis: Axis::Vertical,
                session: context().default_session
            })
        ]
    );
    assert_eq!(
        router.route(InputEvent::Bytes(vec![0x02, 0x02]), &context(), &config),
        vec![InputRoute::Consume, InputRoute::Forward(vec![0x02])]
    );
}

#[test]
fn split_keys_route_local_and_global_commands_on_both_axes() {
    let config = BindingConfig::default();
    for (key, axis, global) in [
        (b'-', Axis::Horizontal, false),
        (b'\\', Axis::Vertical, false),
        (b'_', Axis::Horizontal, true),
        (b'|', Axis::Vertical, true),
    ] {
        let mut router = InputRouter::new();
        let session = context().default_session;
        let command = if global {
            WorkspaceCommand::SplitTab { axis, session }
        } else {
            WorkspaceCommand::SplitFocused { axis, session }
        };
        assert_eq!(
            router.route(InputEvent::Bytes(vec![0x02, key]), &context(), &config),
            vec![InputRoute::Consume, InputRoute::Command(command)]
        );
    }
}

#[test]
fn leader_persists_until_command_or_escape() {
    let mut router = InputRouter::new();
    let config = BindingConfig::default();
    router.route(InputEvent::Bytes(vec![0x02]), &context(), &config);
    for _ in 0..3 {
        assert!(
            router
                .route(InputEvent::Timeout, &context(), &config)
                .is_empty()
        );
        assert!(router.leader_pending());
    }
    assert_eq!(
        router.route(InputEvent::Bytes(vec![b'!']), &context(), &config),
        vec![InputRoute::Consume]
    );
    assert!(router.leader_pending());
    assert!(matches!(
        router
            .route(InputEvent::Bytes(vec![b'|']), &context(), &config)
            .as_slice(),
        [InputRoute::Command(WorkspaceCommand::SplitTab { .. })]
    ));
    assert!(!router.leader_pending());
    router.route(InputEvent::Bytes(vec![0x02, 0x1b]), &context(), &config);
    router.route(InputEvent::Timeout, &context(), &config);
    assert!(!router.leader_pending());
    assert_eq!(
        router.route(InputEvent::Bytes(vec![b'z']), &context(), &config),
        vec![InputRoute::Forward(vec![b'z'])]
    );
}

#[test]
fn alt_number_is_chunk_safe_and_can_be_disabled() {
    let mut router = InputRouter::new();
    let config = BindingConfig::default();
    assert!(
        router
            .route(InputEvent::Bytes(vec![0x1b]), &context(), &config)
            .is_empty()
    );
    assert_eq!(
        router.route(InputEvent::Bytes(vec![b'2']), &context(), &config),
        vec![InputRoute::Command(WorkspaceCommand::NavigateTab {
            number: 2
        })]
    );

    let mut disabled = BindingConfig::default();
    disabled.set_alt_numbers_enabled(false);
    let mut router = InputRouter::new();
    assert_eq!(
        router.route(InputEvent::Bytes(vec![0x1b, b'2']), &context(), &disabled),
        vec![InputRoute::Forward(vec![0x1b, b'2'])]
    );
}

#[test]
fn paste_is_atomic_and_does_not_trigger_leader() {
    let mut router = InputRouter::new();
    let config = BindingConfig::default();
    let _ = router.route(InputEvent::Bytes(vec![0x02]), &context(), &config);
    assert_eq!(
        router.route(
            InputEvent::Paste(vec![0x02, b'h', b'\n']),
            &context(),
            &config
        ),
        vec![InputRoute::Forward(vec![0x02, b'h', b'\n'])]
    );
}

#[test]
fn command_routes_stop_at_first_command_for_fresh_context() {
    let mut router = InputRouter::new();
    let config = BindingConfig::default();
    let first = router.route(
        InputEvent::Bytes(vec![0x02, b't', 0x1b, b'2']),
        &context(),
        &config,
    );
    assert_eq!(
        first,
        vec![
            InputRoute::Consume,
            InputRoute::Command(WorkspaceCommand::CreateTab {
                session: context().default_session
            })
        ]
    );
    assert!(router.has_pending_bytes());
    let second_context = RouterContext {
        current_tab: TabId(2),
        ..context()
    };
    assert_eq!(
        router.route(InputEvent::Bytes(Vec::new()), &second_context, &config),
        vec![InputRoute::Command(WorkspaceCommand::NavigateTab {
            number: 2
        })]
    );
}

#[test]
fn all_bindings_emit_expected_command_shapes() {
    let mut router = InputRouter::new();
    let config = BindingConfig::default();
    assert_eq!(
        router.route(InputEvent::Bytes(vec![0x02, b'|']), &context(), &config),
        vec![
            InputRoute::Consume,
            InputRoute::Command(WorkspaceCommand::SplitTab {
                axis: Axis::Vertical,
                session: context().default_session
            })
        ]
    );
    assert_eq!(
        router.route(
            InputEvent::Bytes(b"\x1b[50;7u".to_vec()),
            &context(),
            &config
        ),
        vec![InputRoute::Command(WorkspaceCommand::CarryMemberToTab {
            number: 2
        })]
    );
    assert_eq!(
        router.route(
            InputEvent::Bytes(vec![0x02, b'b', b'm']),
            &context(),
            &config
        ),
        vec![
            InputRoute::Consume,
            InputRoute::Consume,
            InputRoute::Command(WorkspaceCommand::ToggleManualBroadcastTarget(SlotId(7)))
        ]
    );
    assert_eq!(
        router.route(
            InputEvent::Bytes(vec![0x02, b'b', b'r']),
            &context(),
            &config
        ),
        vec![
            InputRoute::Consume,
            InputRoute::Consume,
            InputRoute::Command(WorkspaceCommand::ResetBroadcast)
        ]
    );
    assert_eq!(
        router.route(InputEvent::Bytes(vec![0x02, b'y']), &context(), &config),
        vec![
            InputRoute::Consume,
            InputRoute::Command(WorkspaceCommand::SelectionMode)
        ]
    );
    assert_eq!(
        router.route(InputEvent::Bytes(vec![0x02, b'q']), &context(), &config),
        vec![
            InputRoute::Consume,
            InputRoute::Command(WorkspaceCommand::RequestQuit)
        ]
    );
}

#[test]
fn duplicate_binding_and_invalid_leader_are_rejected() {
    let mut config = BindingConfig::default();
    assert!(
        config
            .bind_modified(3, b'h', BindingAction::Focus(Direction::Up))
            .is_err()
    );
    assert!(config.set_leader(0).is_err());
    assert!(config.set_leader(0x1b).is_err());
    assert!(config.set_alt_timeout_ms(0).is_err());
    assert!(config.set_alt_timeout_ms(1001).is_err());
    assert!(config.set_alt_tab_key(1, b'1').is_err());
}

#[test]
fn modified_navigation_is_chunk_safe_and_old_leader_bindings_are_removed() {
    let config = BindingConfig::default();
    for (bytes, expected) in [
        (b"\x1bh".to_vec(), WorkspaceCommand::Focus(Direction::Left)),
        (vec![27, 8], WorkspaceCommand::SwapMember(Direction::Left)),
        (
            b"\x1b[108;3u".to_vec(),
            WorkspaceCommand::Focus(Direction::Right),
        ),
        (
            b"\x1b[106;7u".to_vec(),
            WorkspaceCommand::SwapMember(Direction::Down),
        ),
        (
            b"\x1b[27;7;57~".to_vec(),
            WorkspaceCommand::CarryMemberToTab { number: 9 },
        ),
        (
            b"\x1b9".to_vec(),
            WorkspaceCommand::NavigateTab { number: 9 },
        ),
    ] {
        for split in 0..=bytes.len() {
            let mut router = InputRouter::new();
            let mut routes = router.route(
                InputEvent::Bytes(bytes[..split].to_vec()),
                &context(),
                &config,
            );
            routes.extend(router.route(
                InputEvent::Bytes(bytes[split..].to_vec()),
                &context(),
                &config,
            ));
            assert_eq!(
                routes,
                vec![InputRoute::Command(expected.clone())],
                "split {split} of {bytes:?}"
            );
        }
    }
    for key in b"hjklHJKLc" {
        assert_eq!(config.binding(*key), None);
    }
    let mut router = InputRouter::new();
    assert!(
        router
            .route(
                InputEvent::Bytes(b"\x1b[49;7".to_vec()),
                &context(),
                &config
            )
            .is_empty()
    );
    assert_eq!(
        router.route(InputEvent::Timeout, &context(), &config),
        vec![InputRoute::Forward(b"\x1b[49;7".to_vec())]
    );
}

#[test]
fn arrows_remain_atomic_and_do_not_dismiss_leader() {
    let config = BindingConfig::default();
    for key in [b"\x1b[A".as_slice(), b"\x1bOB", b"\x1b[1;5D"] {
        let mut router = InputRouter::new();
        for byte in &key[..key.len() - 1] {
            assert!(
                router
                    .route(InputEvent::Bytes(vec![*byte]), &context(), &config)
                    .is_empty()
            );
        }
        assert_eq!(
            router.route(
                InputEvent::Bytes(vec![*key.last().unwrap()]),
                &context(),
                &config
            ),
            vec![InputRoute::Forward(key.to_vec())]
        );
        router.route(InputEvent::Bytes(vec![2]), &context(), &config);
        assert_eq!(
            router.route(InputEvent::Bytes(key.to_vec()), &context(), &config),
            vec![InputRoute::Consume]
        );
        assert!(router.leader_pending());
    }
}

#[test]
fn pane_jumps_handle_legacy_brackets_and_extended_reports() {
    let config = BindingConfig::default();
    for (bytes, delta) in [
        (b"\x1b]".as_slice(), 1),
        (b"\x1b[93;3u", 1),
        (b"\x1b[91;3u", -1),
    ] {
        for split in 0..=bytes.len() {
            let mut router = InputRouter::new();
            let mut routes = router.route(
                InputEvent::Bytes(bytes[..split].to_vec()),
                &context(),
                &config,
            );
            routes.extend(router.route(
                InputEvent::Bytes(bytes[split..].to_vec()),
                &context(),
                &config,
            ));
            assert_eq!(
                routes,
                vec![InputRoute::Command(WorkspaceCommand::CyclePane { delta })]
            );
        }
    }
    let mut router = InputRouter::new();
    assert!(
        router
            .route(InputEvent::Bytes(b"\x1b[".to_vec()), &context(), &config)
            .is_empty()
    );
    assert_eq!(
        router.route(InputEvent::Timeout, &context(), &config),
        vec![InputRoute::Command(WorkspaceCommand::CyclePane {
            delta: -1
        })]
    );
    assert!(config.binding(b'[').is_none());
    assert!(config.binding(b']').is_none());
}

#[test]
fn leader_promotes_member_movement_to_whole_stack_movement() {
    let config = BindingConfig::default();
    for (bytes, expected) in [
        (
            b"\x1b[104;7u".as_slice(),
            WorkspaceCommand::Swap(Direction::Left),
        ),
        (b"\x1b[50;7u", WorkspaceCommand::CarryToTab { number: 2 }),
    ] {
        let mut router = InputRouter::new();
        router.route(InputEvent::Bytes(vec![2]), &context(), &config);
        assert_eq!(
            router.route(InputEvent::Bytes(bytes.to_vec()), &context(), &config),
            vec![InputRoute::Command(expected)]
        );
        assert!(!router.leader_pending());
    }
}

#[test]
fn resize_mode_persists_for_small_large_and_alt_focus_steps_until_escape() {
    let config = BindingConfig::default();
    let mut router = InputRouter::new();
    router.route(InputEvent::Bytes(b"\x02r".to_vec()), &context(), &config);
    assert!(router.resize_mode());
    assert!(!router.leader_pending());
    for (key, direction) in [
        (b'h', Direction::Left),
        (b'j', Direction::Down),
        (b'k', Direction::Up),
        (b'l', Direction::Right),
    ] {
        for (bytes, command) in [
            (
                vec![key],
                WorkspaceCommand::Resize {
                    direction,
                    cells: 1,
                },
            ),
            (
                vec![key.to_ascii_uppercase()],
                WorkspaceCommand::Resize {
                    direction,
                    cells: 5,
                },
            ),
            (
                format!("\x1b[{};2u", key).into_bytes(),
                WorkspaceCommand::Resize {
                    direction,
                    cells: 5,
                },
            ),
            (vec![27, key], WorkspaceCommand::Focus(direction)),
            (
                format!("\x1b[{};3u", key).into_bytes(),
                WorkspaceCommand::Focus(direction),
            ),
        ] {
            assert_eq!(
                router.route(InputEvent::Bytes(bytes), &context(), &config),
                vec![InputRoute::Command(command)]
            );
            assert!(router.resize_mode());
        }
    }
    router.route(InputEvent::Timeout, &context(), &config);
    for event in [
        InputEvent::Bytes(b"q\x02!\x1b[A".to_vec()),
        InputEvent::Paste(b"hjkl\x1b".to_vec()),
    ] {
        let routes = router.route(event, &context(), &config);
        assert!(routes.iter().all(|route| *route == InputRoute::Consume));
        assert!(router.resize_mode());
    }
    router.route(InputEvent::Bytes(vec![27]), &context(), &config);
    assert_eq!(
        router.route(InputEvent::Timeout, &context(), &config),
        vec![InputRoute::Consume]
    );
    assert!(!router.resize_mode());
    assert_eq!(
        router.route(InputEvent::Bytes(b"hjkl".to_vec()), &context(), &config),
        vec![InputRoute::Forward(b"hjkl".to_vec())]
    );
}

#[test]
fn broadcast_submenu_waits_for_a_command_or_escape_without_forwarding() {
    let config = BindingConfig::default();
    for (key, command) in [
        (
            b'b',
            WorkspaceCommand::SetBroadcastScope(BroadcastScope::VisibleTab),
        ),
        (
            b'B',
            WorkspaceCommand::SetBroadcastScope(BroadcastScope::Manual),
        ),
        (
            b'm',
            WorkspaceCommand::ToggleManualBroadcastTarget(SlotId(7)),
        ),
        (b'r', WorkspaceCommand::ResetBroadcast),
    ] {
        let mut router = InputRouter::new();
        router.route(InputEvent::Bytes(b"\x02b".to_vec()), &context(), &config);
        assert!(router.broadcast_pending());
        assert!(!router.leader_pending());
        router.route(InputEvent::Timeout, &context(), &config);
        for event in [
            InputEvent::Bytes(b"!\x1bh".to_vec()),
            InputEvent::Paste(b"b".to_vec()),
        ] {
            assert!(
                router
                    .route(event, &context(), &config)
                    .iter()
                    .all(|route| *route == InputRoute::Consume)
            );
            assert!(router.broadcast_pending());
        }
        assert_eq!(
            router.route(InputEvent::Bytes(vec![key]), &context(), &config),
            vec![InputRoute::Command(command)]
        );
        assert!(!router.broadcast_pending());
    }
    let mut router = InputRouter::new();
    router.route(
        InputEvent::Bytes(b"\x02b\x1b[27u".to_vec()),
        &context(),
        &config,
    );
    assert!(!router.broadcast_pending());
    assert_eq!(
        router.route(InputEvent::Bytes(b"r".to_vec()), &context(), &config),
        vec![InputRoute::Forward(b"r".to_vec())]
    );
}

#[test]
fn modal_entry_and_broadcast_commands_support_separate_binding_scopes() {
    let mut config = BindingConfig::default();
    config.rebind(b'f', BindingAction::ResizeMode).unwrap();
    config.rebind(b'c', BindingAction::BroadcastMenu).unwrap();
    config.rebind(b'f', BindingAction::ResetBroadcast).unwrap();
    let mut router = InputRouter::new();
    router.route(InputEvent::Bytes(b"\x02f".to_vec()), &context(), &config);
    assert!(router.resize_mode());
    router.route(InputEvent::Bytes(b"\x1b[27u".to_vec()), &context(), &config);
    assert!(!router.resize_mode());
    let routes = router.route(InputEvent::Bytes(b"\x02cf".to_vec()), &context(), &config);
    assert_eq!(
        routes.last(),
        Some(&InputRoute::Command(WorkspaceCommand::ResetBroadcast))
    );
    assert!(!router.broadcast_pending());
}

#[test]
fn borderless_and_jump_prefixes_consume_keys_and_cancel_safely() {
    let config = BindingConfig::default();
    let mut router = InputRouter::new();
    assert_eq!(router.route(InputEvent::Bytes(b"\x02z".to_vec()), &context(), &config),
        vec![InputRoute::Consume, InputRoute::Command(WorkspaceCommand::ToggleBorderless)]);
    for (prefix, set) in [(b"\x1b\x07".as_slice(), true), (b"\x1bg", false), (b"\x1b[103;7u", true), (b"\x1b[103;3u", false)] {
        // Fragmented terminal reads must behave like a single key report.
        for byte in prefix { router.route(InputEvent::Bytes(vec![*byte]), &context(), &config); }
        assert_eq!(router.jump_pending(), Some(set));
        router.route(InputEvent::Timeout, &context(), &config);
        assert_eq!(router.jump_pending(), Some(set));
        assert_eq!(router.route(InputEvent::Bytes(b"a".to_vec()), &context(), &config),
            vec![InputRoute::Command(if set { WorkspaceCommand::SetJumpMark(b'a') } else { WorkspaceCommand::JumpToMark(b'a') })]);
        assert_eq!(router.jump_pending(), None);
    }
    for cancel in [b"\x1b".as_slice(), b"\x1b[A", b"\x1bh", b"\x03"] {
        router.route(InputEvent::Bytes(b"\x1bg".to_vec()), &context(), &config);
        let routes = router.route(InputEvent::Bytes(cancel.to_vec()), &context(), &config);
        assert!(!routes.iter().any(|r| matches!(r, InputRoute::Command(_) | InputRoute::Forward(_))));
        router.route(InputEvent::Timeout, &context(), &config);
        assert_eq!(router.jump_pending(), None);
    }
    router.route(InputEvent::Bytes(b"\x1b\x07".to_vec()), &context(), &config);
    assert_eq!(router.route(InputEvent::Paste(b"abc".to_vec()), &context(), &config), vec![InputRoute::Consume]);
    assert_eq!(router.jump_pending(), None);
}
