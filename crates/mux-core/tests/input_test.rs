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
            InputRoute::Command(WorkspaceCommand::SplitFocused {
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
        [InputRoute::Command(WorkspaceCommand::SplitFocused { .. })]
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
            InputRoute::Command(WorkspaceCommand::SplitFocused {
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
        router.route(InputEvent::Bytes(vec![0x02, b'm']), &context(), &config),
        vec![
            InputRoute::Consume,
            InputRoute::Command(WorkspaceCommand::ToggleManualBroadcastTarget(SlotId(7)))
        ]
    );
    assert_eq!(
        router.route(InputEvent::Bytes(vec![0x02, b'r']), &context(), &config),
        vec![
            InputRoute::Consume,
            InputRoute::Command(WorkspaceCommand::ResetBroadcast)
        ]
    );
    assert_eq!(
        router.route(InputEvent::Bytes(vec![0x02, b'y']), &context(), &config),
        vec![
            InputRoute::Consume,
            InputRoute::Command(WorkspaceCommand::CopySelection)
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
