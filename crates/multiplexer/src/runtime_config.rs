use std::{collections::BTreeMap, fs, path::Path};

use mux_core::{Axis, BindingAction, BindingConfig, Direction};
use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RuntimeConfigError {
    #[error("cannot read config {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error("invalid config TOML: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("invalid {field} key {value:?}: {reason}")]
    Key {
        field: String,
        value: String,
        reason: String,
    },
    #[error("invalid binding {name:?}: {reason}")]
    Binding { name: String, reason: String },
    #[error("alt_tabs must contain exactly nine keys")]
    AltTabCount,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    leader: Option<String>,
    leader_timeout_ms: Option<u16>,
    alt_timeout_ms: Option<u16>,
    alt_numbers: Option<bool>,
    alt_tabs: Option<Vec<String>>,
    #[serde(default)]
    bindings: BTreeMap<String, String>,
}

/// Runtime configuration loaded at startup. A missing path uses all defaults;
/// a supplied path must exist and parse completely.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RuntimeConfig {
    pub bindings: BindingConfig,
}

impl RuntimeConfig {
    pub fn load(path: Option<&Path>) -> Result<Self, RuntimeConfigError> {
        let Some(path) = path else {
            return Ok(Self::default());
        };
        let text = fs::read_to_string(path).map_err(|source| RuntimeConfigError::Io {
            path: path.display().to_string(),
            source,
        })?;
        Self::from_toml_str(&text)
    }

    pub fn from_toml_str(text: &str) -> Result<Self, RuntimeConfigError> {
        let file: FileConfig = toml::from_str(text)?;
        let mut bindings = BindingConfig::default();

        // Rebind command names before setting the leader. This lets a config
        // move the command which occupied a proposed leader key out of the
        // way while still rejecting an unreachable final configuration.
        let mut parsed_bindings = Vec::new();
        for (name, key_text) in &file.bindings {
            let action = parse_action(name).map_err(|reason| RuntimeConfigError::Binding {
                name: name.clone(),
                reason,
            })?;
            let key = parse_binding_key(key_text).map_err(|reason| RuntimeConfigError::Key {
                field: format!("bindings.{name}"),
                value: key_text.clone(),
                reason,
            })?;
            if action.is_broadcast() && (key.0 != 0 || key.1 == 0x1b) {
                return Err(RuntimeConfigError::Binding {
                    name: name.clone(),
                    reason: "broadcast submenu commands need an unmodified key other than Escape"
                        .into(),
                });
            }
            parsed_bindings.push((name.clone(), key, action));
        }
        // Remove every overridden action before assigning custom keys. This
        // makes simultaneous swaps (left=l and right=h) deterministic.
        for (_, _, action) in &parsed_bindings {
            bindings.remove_action(*action);
        }

        if let Some(leader) = file.leader.as_deref() {
            let key = parse_key(leader).map_err(|reason| RuntimeConfigError::Key {
                field: "leader".into(),
                value: leader.into(),
                reason,
            })?;
            bindings
                .set_leader(key)
                .map_err(|error| RuntimeConfigError::Binding {
                    name: "leader".into(),
                    reason: error.to_string(),
                })?;
        }
        if let Some(timeout) = file.alt_timeout_ms {
            bindings
                .set_alt_timeout_ms(timeout)
                .map_err(|error| RuntimeConfigError::Binding {
                    name: "alt_timeout_ms".into(),
                    reason: error.to_string(),
                })?;
        }
        if let Some(timeout) = file.leader_timeout_ms {
            bindings.set_leader_timeout_ms(timeout).map_err(|error| {
                RuntimeConfigError::Binding {
                    name: "leader_timeout_ms".into(),
                    reason: error.to_string(),
                }
            })?;
        }
        if let Some(enabled) = file.alt_numbers {
            bindings.set_alt_numbers_enabled(enabled);
        }
        if let Some(keys) = file.alt_tabs {
            if keys.len() != 9 {
                return Err(RuntimeConfigError::AltTabCount);
            }
            let mut parsed_keys = [0_u8; 9];
            for (index, key_text) in keys.iter().enumerate() {
                let key = parse_key(key_text).map_err(|reason| RuntimeConfigError::Key {
                    field: format!("alt_tabs[{index}]"),
                    value: key_text.clone(),
                    reason,
                })?;
                parsed_keys[index] = key;
            }
            bindings.set_alt_tab_keys(parsed_keys).map_err(|error| {
                RuntimeConfigError::Binding {
                    name: "alt_tabs".into(),
                    reason: error.to_string(),
                }
            })?;
        }
        for (name, key, action) in parsed_bindings {
            (if key.0 == 0 {
                bindings.bind(key.1, action)
            } else {
                bindings.bind_modified(key.0, key.1, action)
            })
            .map_err(|error| RuntimeConfigError::Binding {
                name,
                reason: error.to_string(),
            })?;
        }

        Ok(Self { bindings })
    }

    /// Human-readable effective bindings for a help/status view.
    pub fn effective_help(&self) -> String {
        let config = &self.bindings;
        let mut lines = vec![
            format!("leader: {}", format_key(config.leader())),
            "leader mode: persistent (Esc cancels)".into(),
            format!("alt_numbers: {}", config.alt_numbers_enabled()),
            format!("alt_timeout_ms: {}", config.alt_timeout_ms()),
        ];
        lines.extend(self.binding_help());
        lines.join("\n")
    }
    /// Keep similar functions and keybindings grouped as commands are added or changed.
    /// Compact only matching effective bindings; custom keys remain explicit.
    pub fn popup_help(&self) -> Vec<(String, bool)> {
        use BindingAction::*;
        let config = &self.bindings;
        let mut bindings = Vec::new();
        for ((mods, key), action) in config.modified_bindings() {
            let key = format!(
                "{} {}",
                if *mods == 7 { "<ctrl><alt>" } else { "<alt>" },
                format_key(*key)
            );
            bindings.push((*action, key.clone()));
            let whole = match action {
                SwapMember(d) => Some(Swap(*d)),
                CarryMemberToTab(n) => Some(CarryToTab(*n)),
                _ => None,
            };
            if let Some(action) = whole {
                bindings.push((action, format!("<leader> {key}")));
            }
        }
        for (key, action) in config.bindings() {
            let key = match *key {
                1..=26 => format!("<ctrl> {}", (b'a' + key - 1) as char),
                _ => format_key(*key),
            };
            bindings.push((*action, format!("<leader> {key}")));
        }

        let mut groups: BTreeMap<&str, Vec<String>> = BTreeMap::new();
        let mut add = |group, actions: Vec<(BindingAction, String)>, keys: &str, label: &str| {
            let entries: Vec<_> = actions
                .iter()
                .flat_map(|(action, label)| {
                    bindings
                        .iter()
                        .filter(move |(bound, _)| bound == action)
                        .map(move |(_, key)| (key.clone(), label.clone()))
                })
                .collect();
            if entries.is_empty() {
                return;
            }
            let expected: Vec<_> = keys.chars().collect();
            let prefix = entries
                .first()
                .and_then(|(key, _)| expected.first().and_then(|first| key.strip_suffix(*first)));
            let compact = prefix.filter(|prefix| {
                entries.len() == actions.len()
                    && entries.len() == expected.len()
                    && entries
                        .iter()
                        .zip(&expected)
                        .all(|((key, _), suffix)| key == &format!("{prefix}{suffix}"))
            });
            let rows = groups.entry(group).or_default();
            if let Some(prefix) = compact {
                let keys = match keys {
                    "123456789" => "1-9".to_owned(),
                    "hjkl" => "hjkl".to_owned(),
                    _ => expected
                        .iter()
                        .map(char::to_string)
                        .collect::<Vec<_>>()
                        .join("/"),
                };
                rows.push(format!("{prefix}{keys} ~ {label}"));
            } else {
                rows.extend(
                    entries
                        .into_iter()
                        .map(|(key, label)| format!("{key} ~ {label}")),
                );
            }
        };
        let singles = |items: &[(BindingAction, &str)]| {
            items
                .iter()
                .map(|(action, label)| (*action, (*label).to_owned()))
                .collect()
        };
        let directions = |action: fn(Direction) -> BindingAction, label: &str| {
            [
                Direction::Left,
                Direction::Down,
                Direction::Up,
                Direction::Right,
            ]
            .into_iter()
            .map(|direction| (action(direction), format!("{label} {direction:?}")))
            .collect()
        };
        let numbers = |action: fn(usize) -> BindingAction, label: &str| {
            (1..=9)
                .map(|n| (action(n), format!("{label} {n}")))
                .collect()
        };

        add(
            "Manage Panes",
            singles(&[
                (SplitHorizontal, "Split Horizontal (Local)"),
                (SplitVertical, "Split Vertical (Local)"),
            ]),
            "-\\",
            "Split Horizontal / Vertical (Local)",
        );
        add(
            "Manage Panes",
            singles(&[
                (SplitHorizontalGlobal, "Split Horizontal (Global)"),
                (SplitVerticalGlobal, "Split Vertical (Global)"),
            ]),
            "_|",
            "Split Horizontal / Vertical (Global)",
        );
        add(
            "Manage Panes",
            singles(&[(RemoveSlot, "Remove Slot")]),
            "d",
            "Remove Slot",
        );
        add(
            "Manage Panes",
            directions(Focus, "Change Focus"),
            "hjkl",
            "Change Focus",
        );
        add(
            "Manage Panes",
            singles(&[(PreviousPane, "Previous Pane"), (NextPane, "Next Pane")]),
            "[]",
            "Previous / Next Pane",
        );
        add(
            "Manage Panes",
            directions(SwapMember, "Move Displayed Pane"),
            "hjkl",
            "Move Displayed Pane",
        );
        add(
            "Manage Panes",
            directions(Swap, "Swap Stacks"),
            "hjkl",
            "Swap Stacks",
        );
        add(
            "Manage Panes",
            singles(&[(ResizeMode, "Resize Mode")]),
            "r",
            "Resize Mode",
        );
        add(
            "Manage Panes",
            singles(&[
                (Equalize(Axis::Vertical), "Equalize Widths"),
                (Equalize(Axis::Horizontal), "Equalize Heights"),
            ]),
            "=+",
            "Equalize Widths / Heights",
        );
        add(
            "Manage Panes",
            directions(Resize, "Resize Pane"),
            "hjkl",
            "Resize Pane",
        );
        add(
            "Manage Panes",
            singles(&[(KillSession, "Kill Session"), (KillStack, "Kill Stack")]),
            "xX",
            "Kill Session / Stack",
        );

        add(
            "Manage Stacks",
            singles(&[(AddToStack, "Add Terminal to Stack")]),
            "a",
            "Add Terminal to Stack",
        );
        add(
            "Manage Stacks",
            numbers(SelectStackMember, "Select Stack Member"),
            "123456789",
            "Select Stack Member",
        );
        add(
            "Manage Stacks",
            singles(&[
                (PreviousStack, "Previous Stack Member"),
                (NextStack, "Next Stack Member"),
            ]),
            "",
            "",
        );

        add(
            "Manage Tabs",
            singles(&[(CreateTab, "Create Tab"), (CloseTab, "Close Tab")]),
            "tw",
            "Create / Close Tab",
        );
        add(
            "Manage Tabs",
            numbers(CarryMemberToTab, "Carry Pane to Tab"),
            "123456789",
            "Carry Pane to Tab",
        );
        add(
            "Manage Tabs",
            numbers(CarryToTab, "Carry Stack to Tab"),
            "123456789",
            "Carry Stack to Tab",
        );
        add(
            "Manage Tabs",
            singles(&[(Carry, "Choose Tab for Stack")]),
            "",
            "",
        );

        add(
            "Broadcast",
            singles(&[(BroadcastMenu, "Broadcast Menu")]),
            "b",
            "Broadcast Menu",
        );
        for (action, key, label) in [
            (BroadcastVisible, "b", "Toggle Visible Broadcast"),
            (BroadcastManual, "B", "Toggle Manual Broadcast"),
            (ToggleManualTarget, "m", "Toggle Manual Target"),
            (ResetBroadcast, "r", "Reset Broadcast"),
        ] {
            add("Broadcast", singles(&[(action, label)]), key, label);
        }
        add(
            "Other",
            singles(&[(SelectionMode, "Selection Mode")]),
            "y",
            "Selection Mode",
        );
        add("Other", singles(&[(Quit, "Quit")]), "q", "Quit");

        if config.alt_numbers_enabled() {
            let rows = groups.entry("Manage Tabs").or_default();
            if config.alt_tab_keys() == b"123456789" {
                rows.insert(0, "<alt> 1-9 ~ Switch / Create Tab".into());
            } else {
                let tabs = config.alt_tab_keys().iter().enumerate().map(|(i, key)| {
                    format!("<alt> {} ~ Switch / Create Tab {}", format_key(*key), i + 1)
                });
                rows.splice(0..0, tabs);
            }
        }
        let mut lines = Vec::new();
        for group in [
            "Manage Panes",
            "Manage Stacks",
            "Manage Tabs",
            "Broadcast",
            "Other",
        ] {
            if let Some(rows) = groups.remove(group) {
                lines.push((group.to_owned(), true));
                lines.extend(rows.into_iter().map(|row| (row, false)));
            }
        }
        lines
    }

    pub fn broadcast_help(&self) -> Vec<(String, bool)> {
        let mut lines = vec![("Broadcast".into(), true)];
        for (key, action) in self.bindings.broadcast_bindings() {
            let label = match action {
                BindingAction::BroadcastVisible => "Toggle Visible Broadcast",
                BindingAction::BroadcastManual => "Toggle Manual Broadcast",
                BindingAction::ToggleManualTarget => "Toggle Manual Target",
                BindingAction::ResetBroadcast => "Reset Broadcast",
                _ => continue,
            };
            lines.push((format!("{} ~ {label}", format_key(*key)), false));
        }
        lines
    }

    pub fn resize_help(&self) -> Vec<(String, bool)> {
        let mut lines = vec![
            ("Resize Mode".into(), true),
            ("h/j/k/l ~ Left / Down / Up / Right (1 cell)".into(), false),
            ("H/J/K/L ~ Left / Down / Up / Right (5 cells)".into(), false),
            ("Change Focus".into(), true),
        ];
        for ((mods, key), action) in self.bindings.modified_bindings() {
            if *mods == 3 {
                if let BindingAction::Focus(direction) = action {
                    lines.push((
                        format!("Alt-{} ~ Focus {direction:?}", format_key(*key)),
                        false,
                    ));
                }
            }
        }
        lines
    }

    pub fn binding_help(&self) -> Vec<String> {
        let config = &self.bindings;
        let mut lines = Vec::new();
        if config.alt_numbers_enabled() {
            for (index, key) in config.alt_tab_keys().iter().enumerate() {
                lines.push(format!("Alt-{}: tab {}", format_key(*key), index + 1));
            }
        }
        for ((modifiers, key), action) in config.modified_bindings() {
            lines.push(format!(
                "{}{}: {}",
                if *modifiers == 7 { "Ctrl-Alt-" } else { "Alt-" },
                format_key(*key),
                action_name(*action)
            ));
        }
        for ((mods, key), action) in config.modified_bindings() {
            let whole = match action {
                BindingAction::SwapMember(d) => Some(BindingAction::Swap(*d)),
                BindingAction::CarryMemberToTab(n) => Some(BindingAction::CarryToTab(*n)),
                _ => None,
            };
            if let Some(action) = whole {
                lines.push(format!(
                    "leader {}{}: {}",
                    if *mods == 7 { "Ctrl-Alt-" } else { "Alt-" },
                    format_key(*key),
                    action_name(action)
                ));
            }
        }
        for (key, action) in config.bindings() {
            lines.push(format!(
                "leader {}: {}",
                format_key(*key),
                action_name(*action)
            ));
        }
        for (menu_key, action) in config.bindings() {
            if *action == BindingAction::BroadcastMenu {
                for (key, action) in config.broadcast_bindings() {
                    lines.push(format!(
                        "leader {} {}: {}",
                        format_key(*menu_key),
                        format_key(*key),
                        action_name(*action)
                    ));
                }
            }
        }
        lines.push(
            "resize mode: hjkl = 1 cell; HJKL = 5 cells; Alt-hjkl = focus; Esc = exit".into(),
        );
        lines.push("selection mode: hjkl w/b/e 0/^/$ gg/G; counts; Ctrl-u/d/b/f pages; v/V select; y yank; Esc cancel".into());
        lines
    }
}

fn parse_action(name: &str) -> Result<BindingAction, String> {
    let normalized = name.trim().to_ascii_lowercase().replace('-', "_");
    let direction = |suffix: &str| match suffix {
        "left" => Some(Direction::Left),
        "right" => Some(Direction::Right),
        "up" => Some(Direction::Up),
        "down" => Some(Direction::Down),
        _ => None,
    };
    for (prefix, action) in [
        (
            "carry_stack_",
            BindingAction::CarryToTab as fn(usize) -> BindingAction,
        ),
        ("carry_", BindingAction::CarryMemberToTab),
    ] {
        let Some(suffix) = normalized.strip_prefix(prefix) else {
            continue;
        };
        return suffix
            .parse::<usize>()
            .ok()
            .filter(|number| (1..=9).contains(number))
            .map(action)
            .ok_or_else(|| "carry destination must be 1-9".into());
    }
    if normalized == "resize_mode" {
        return Ok(BindingAction::ResizeMode);
    }
    for (prefix, action, error) in [
        (
            "swap_stack_",
            BindingAction::Swap as fn(Direction) -> BindingAction,
            "unknown swap direction",
        ),
        ("focus_", BindingAction::Focus, "unknown focus direction"),
        ("swap_", BindingAction::SwapMember, "unknown swap direction"),
        ("resize_", BindingAction::Resize, "unknown resize direction"),
    ] {
        if let Some(suffix) = normalized.strip_prefix(prefix) {
            return direction(suffix).map(action).ok_or_else(|| error.into());
        }
    }
    if normalized == "pane_previous" {
        return Ok(BindingAction::PreviousPane);
    }
    if normalized == "pane_next" {
        return Ok(BindingAction::NextPane);
    }
    if let Some(suffix) = normalized.strip_prefix("stack_") {
        return match suffix {
            "add" | "new" => Ok(BindingAction::AddToStack),
            "prev" | "previous" => Ok(BindingAction::PreviousStack),
            "next" => Ok(BindingAction::NextStack),
            value => value
                .parse::<usize>()
                .map(BindingAction::SelectStackMember)
                .map_err(|_| "stack action must be add, prev, next, or 1-9".into()),
        };
    }
    match normalized.as_str() {
        "equalize_widths" => Ok(BindingAction::Equalize(Axis::Vertical)),
        "equalize_heights" => Ok(BindingAction::Equalize(Axis::Horizontal)),
        "split_vertical" => Ok(BindingAction::SplitVertical),
        "split_horizontal" => Ok(BindingAction::SplitHorizontal),
        "split_vertical_global" => Ok(BindingAction::SplitVerticalGlobal),
        "split_horizontal_global" => Ok(BindingAction::SplitHorizontalGlobal),
        "remove" | "remove_slot" => Ok(BindingAction::RemoveSlot),
        "kill" | "kill_session" => Ok(BindingAction::KillSession),
        "kill_stack" => Ok(BindingAction::KillStack),
        "new_tab" | "create_tab" => Ok(BindingAction::CreateTab),
        "close_tab" => Ok(BindingAction::CloseTab),
        "carry" => Ok(BindingAction::Carry),
        "broadcast_menu" => Ok(BindingAction::BroadcastMenu),
        "broadcast_visible" => Ok(BindingAction::BroadcastVisible),
        "broadcast_manual" => Ok(BindingAction::BroadcastManual),
        "broadcast_target" | "toggle_manual_target" => Ok(BindingAction::ToggleManualTarget),
        "broadcast_reset" | "reset_broadcast" => Ok(BindingAction::ResetBroadcast),
        "selection_mode" | "copy" | "copy_selection" => Ok(BindingAction::SelectionMode),
        "quit" | "request_quit" => Ok(BindingAction::Quit),
        _ => Err("unknown command name".into()),
    }
}

fn parse_binding_key(value: &str) -> Result<(u8, u8), String> {
    let value = value.trim();
    let lower = value.to_ascii_lowercase();
    for (prefix, modifiers) in [("ctrl-alt-", 7), ("alt-ctrl-", 7), ("alt-", 3)] {
        if lower.starts_with(prefix) {
            let key = &value[prefix.len()..];
            if key.len() != 1 || !key.is_ascii() {
                return Err("modified bindings need one ASCII key".into());
            }
            return Ok((modifiers, key.as_bytes()[0]));
        }
    }
    parse_key(value).map(|key| (0, key))
}

fn parse_key(value: &str) -> Result<u8, String> {
    let value = value.trim();
    if value.len() == 1 {
        return Ok(value.as_bytes()[0]);
    }
    let lower = value.to_ascii_lowercase();
    if let Some(hex) = lower.strip_prefix("0x") {
        return u8::from_str_radix(hex, 16).map_err(|_| "expected a byte such as 0x02".into());
    }
    if let Some(ctrl) = lower
        .strip_prefix("ctrl-")
        .or_else(|| lower.strip_prefix("c-"))
    {
        let bytes = ctrl.as_bytes();
        if bytes.len() == 1 && bytes[0].is_ascii_lowercase() {
            return Ok(bytes[0] & 0x1f);
        }
        return Err("Ctrl- must be followed by one ASCII letter".into());
    }
    match lower.as_str() {
        "escape" | "esc" => Ok(0x1b),
        "space" => Ok(b' '),
        "tab" => Ok(b'\t'),
        "enter" | "return" => Ok(b'\r'),
        "backspace" => Ok(0x08),
        _ => Err("expected one character, Ctrl-letter, named key, or 0xNN".into()),
    }
}

pub fn format_key(key: u8) -> String {
    match key {
        0x09 => "Tab".into(),
        0x0d => "Enter".into(),
        0x08 => "Backspace".into(),
        0x01..=0x1a => format!("Ctrl-{}", (b'a' + key - 1) as char),
        0x1b => "Esc".into(),
        b' ' => "Space".into(),
        byte if byte.is_ascii_graphic() => (byte as char).to_string(),
        byte => format!("0x{byte:02x}"),
    }
}

fn action_name(action: BindingAction) -> String {
    match action {
        BindingAction::Focus(direction) => format!("focus_{direction:?}").to_ascii_lowercase(),
        BindingAction::SwapMember(direction) => format!("swap_{direction:?}").to_ascii_lowercase(),
        BindingAction::CarryMemberToTab(number) => format!("carry_{number}"),
        BindingAction::Swap(direction) => format!("swap_stack_{direction:?}").to_ascii_lowercase(),
        BindingAction::ResizeMode => "resize_mode".into(),
        BindingAction::Equalize(Axis::Vertical) => "equalize_widths".into(),
        BindingAction::Equalize(Axis::Horizontal) => "equalize_heights".into(),
        BindingAction::BroadcastMenu => "broadcast_menu".into(),
        BindingAction::Resize(direction) => format!("resize_{direction:?}").to_ascii_lowercase(),
        BindingAction::SplitVertical => "split_vertical".into(),
        BindingAction::SplitHorizontal => "split_horizontal".into(),
        BindingAction::SplitVerticalGlobal => "split_vertical_global".into(),
        BindingAction::SplitHorizontalGlobal => "split_horizontal_global".into(),
        BindingAction::AddToStack => "stack_add".into(),
        BindingAction::PreviousPane => "pane_previous".into(),
        BindingAction::NextPane => "pane_next".into(),
        BindingAction::PreviousStack => "stack_previous".into(),
        BindingAction::NextStack => "stack_next".into(),
        BindingAction::SelectStackMember(index) => format!("stack_{index}"),
        BindingAction::RemoveSlot => "remove_slot".into(),
        BindingAction::KillSession => "kill_session".into(),
        BindingAction::KillStack => "kill_stack".into(),
        BindingAction::CreateTab => "create_tab".into(),
        BindingAction::CloseTab => "close_tab".into(),
        BindingAction::Carry => "carry".into(),
        BindingAction::CarryToTab(number) => format!("carry_stack_{number}"),
        BindingAction::BroadcastVisible => "broadcast_visible".into(),
        BindingAction::BroadcastManual => "broadcast_manual".into(),
        BindingAction::ToggleManualTarget => "broadcast_target".into(),
        BindingAction::ResetBroadcast => "broadcast_reset".into(),
        BindingAction::SelectionMode => "selection_mode".into(),
        BindingAction::Quit => "quit".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modal_bindings_have_separate_scopes_and_help_uses_custom_keys() {
        assert!(RuntimeConfig::from_toml_str("[bindings]\nresize_mode = \"z\"").is_ok());
        let config = RuntimeConfig::from_toml_str(
            "[bindings]\nresize_mode = \"z\"\nbroadcast_menu = \"c\"\nbroadcast_reset = \"z\"",
        )
        .unwrap();
        assert_eq!(
            config.bindings.binding(b'z'),
            Some(BindingAction::ResizeMode)
        );
        assert_eq!(
            config.bindings.broadcast_bindings().get(&b'z'),
            Some(&BindingAction::ResetBroadcast)
        );
        assert!(
            config
                .effective_help()
                .contains("leader c z: broadcast_reset")
        );
        assert!(
            config
                .popup_help()
                .iter()
                .any(|(line, _)| line == "<leader> c ~ Broadcast Menu")
        );
        assert!(
            config
                .broadcast_help()
                .iter()
                .any(|(line, _)| line == "z ~ Reset Broadcast")
        );
        assert!(
            !config
                .popup_help()
                .iter()
                .any(|(line, _)| line.contains("Reset Broadcast"))
        );
        assert!(RuntimeConfig::from_toml_str("[bindings]\nbroadcast_reset = \"b\"").is_err());
        assert!(RuntimeConfig::from_toml_str("[bindings]\nbroadcast_reset = \"Esc\"").is_err());
    }

    #[test]
    fn resize_help_lists_resize_keys_and_configured_focus_keys() {
        let config = RuntimeConfig::from_toml_str("[bindings]\nfocus_left = \"Alt-z\"").unwrap();
        let help = config.resize_help();
        assert!(help.iter().any(|(line, _)| line.contains("h/j/k/l")));
        assert!(help.iter().any(|(line, _)| line == "Alt-z ~ Focus Left"));
    }

    #[test]
    fn popup_groups_commands_and_preserves_custom_numbered_keys() {
        let mut config = RuntimeConfig::default();
        let help = config.popup_help();
        let expected: Vec<_> = include_str!("../../../popup_example.md")
            .lines()
            .take_while(|line| !line.starts_with("Maintenance:"))
            .filter(|line| !line.is_empty())
            .collect();
        assert_eq!(
            help.iter()
                .map(|(line, _)| line.as_str())
                .collect::<Vec<_>>(),
            expected
        );
        config
            .bindings
            .rebind(b'!', BindingAction::SelectStackMember(1))
            .unwrap();
        let help = config.popup_help();
        assert!(
            !help
                .iter()
                .any(|(line, _)| line == "<leader> 1-9 ~ Select Stack Member")
        );
        assert!(
            help.iter()
                .any(|(line, _)| line == "<leader> ! ~ Select Stack Member 1")
        );
        assert!(
            help.iter()
                .any(|(line, _)| line == "<leader> 9 ~ Select Stack Member 9")
        );
    }

    #[test]
    fn popup_preserves_custom_directions_and_tab_settings() {
        let config = RuntimeConfig::from_toml_str(
            "alt_numbers = false\n[bindings]\nfocus_left = \"z\"\nswap_left = \"Ctrl-Alt-u\"",
        )
        .unwrap();
        let help = config.popup_help();
        for expected in [
            "<leader> z ~ Change Focus Left",
            "<alt> j ~ Change Focus Down",
            "<ctrl><alt> u ~ Move Displayed Pane Left",
            "<leader> <ctrl><alt> u ~ Swap Stacks Left",
        ] {
            assert!(help.iter().any(|(line, _)| line == expected), "{expected}");
        }
        assert!(
            !help
                .iter()
                .any(|(line, _)| line.contains("Switch / Create Tab"))
        );
        let config = RuntimeConfig::from_toml_str(
            "alt_tabs = [\"9\", \"8\", \"7\", \"6\", \"5\", \"4\", \"3\", \"2\", \"1\"]",
        )
        .unwrap();
        assert!(
            config
                .popup_help()
                .iter()
                .any(|(line, _)| line == "<alt> 9 ~ Switch / Create Tab 1")
        );
    }

    #[test]
    fn local_and_global_split_bindings_can_be_customized() {
        let config = RuntimeConfig::from_toml_str(
            r#"
[bindings]
split_horizontal = "="
split_vertical = "/"
split_horizontal_global = "+"
split_vertical_global = "?"
equalize_widths = "e"
equalize_heights = "E"
"#,
        )
        .unwrap();
        for (key, action) in [
            (b'=', BindingAction::SplitHorizontal),
            (b'/', BindingAction::SplitVertical),
            (b'+', BindingAction::SplitHorizontalGlobal),
            (b'?', BindingAction::SplitVerticalGlobal),
        ] {
            assert_eq!(config.bindings.binding(key), Some(action));
        }
        for key in b"-\\_|" {
            assert_eq!(config.bindings.binding(*key), None);
        }
        let help = config.popup_help();
        for expected in [
            "<leader> = ~ Split Horizontal (Local)",
            "<leader> / ~ Split Vertical (Local)",
            "<leader> + ~ Split Horizontal (Global)",
            "<leader> ? ~ Split Vertical (Global)",
        ] {
            assert!(help.iter().any(|(line, _)| line == expected), "{expected}");
        }
    }

    #[test]
    fn equalize_bindings_and_help_use_effective_keys() {
        let config = RuntimeConfig::from_toml_str(
            "[bindings]\nequalize_widths = \"e\"\nequalize_heights = \"E\"",
        )
        .unwrap();
        for (key, axis, name, label) in [
            (b'e', Axis::Vertical, "equalize_widths", "Equalize Widths"),
            (
                b'E',
                Axis::Horizontal,
                "equalize_heights",
                "Equalize Heights",
            ),
        ] {
            assert_eq!(
                config.bindings.binding(key),
                Some(BindingAction::Equalize(axis))
            );
            assert!(
                config
                    .effective_help()
                    .contains(&format!("leader {}: {name}", key as char))
            );
            assert!(
                config
                    .popup_help()
                    .iter()
                    .any(|(line, _)| line == &format!("<leader> {} ~ {label}", key as char))
            );
        }
        assert_eq!(config.bindings.binding(b'='), None);
        assert_eq!(config.bindings.binding(b'+'), None);
        assert!(RuntimeConfig::from_toml_str("[bindings]\nequalize_widths = \"r\"").is_err());
    }

    #[test]
    fn defaults_and_effective_help_are_stable() {
        let config = RuntimeConfig::default();
        assert_eq!(config.bindings.leader(), 0x02);
        assert!(config.effective_help().contains("leader: Ctrl-b"));
        assert!(config.effective_help().contains("Alt-h: focus_left"));
        assert!(RuntimeConfig::load(None).is_ok());
        assert!(RuntimeConfig::load(Some(Path::new("/path/that/does/not/exist"))).is_err());
    }

    #[test]
    fn toml_rebinds_actions_and_alt_tabs() {
        let config = RuntimeConfig::from_toml_str(
            r#"
leader = "Ctrl-a"
leader_timeout_ms = 1200
alt_timeout_ms = 75
alt_numbers = false
alt_tabs = ["q", "w", "e", "r", "t", "y", "u", "i", "o"]

[bindings]
focus_left = "n"
stack_add = "a"
"#,
        )
        .unwrap();
        assert_eq!(config.bindings.leader(), 0x01);
        assert_eq!(config.bindings.leader_timeout_ms(), 1200);
        assert_eq!(config.bindings.alt_timeout_ms(), 75);
        assert!(!config.bindings.alt_numbers_enabled());
        assert_eq!(config.bindings.alt_tab_key(0), Some(b'q'));
        assert_eq!(
            config.bindings.binding(b'n'),
            Some(BindingAction::Focus(Direction::Left))
        );
        assert_eq!(config.bindings.binding(b'h'), None);
    }

    #[test]
    fn invalid_or_conflicting_config_is_rejected() {
        assert!(RuntimeConfig::from_toml_str("alt_tabs = [\"1\"]").is_err());
        assert!(RuntimeConfig::from_toml_str("leader = \"a\"").is_err());
        assert!(RuntimeConfig::from_toml_str("[bindings]\nfocus_left = \"Ctrl-b\"").is_err());
        assert!(RuntimeConfig::from_toml_str("[bindings]\nstack_0 = \"z\"").is_err());
        assert!(RuntimeConfig::from_toml_str("leader_timeout_ms = 100").is_err());
    }

    #[test]
    fn simultaneous_binding_and_alt_tab_swaps_are_supported() {
        let config = RuntimeConfig::from_toml_str(
            r#"
alt_tabs = ["2", "1", "3", "4", "5", "6", "7", "8", "9"]

[bindings]
focus_left = "l"
focus_right = "h"
"#,
        )
        .unwrap();
        assert_eq!(
            config.bindings.binding(b'l'),
            Some(BindingAction::Focus(Direction::Left))
        );
        assert_eq!(
            config.bindings.binding(b'h'),
            Some(BindingAction::Focus(Direction::Right))
        );
        assert_eq!(config.bindings.alt_tab_key(0), Some(b'2'));
        assert_eq!(config.bindings.alt_tab_key(1), Some(b'1'));
    }

    #[test]
    fn modified_bindings_can_be_rebound_and_conflicts_are_rejected() {
        let config = RuntimeConfig::from_toml_str(
            r#"
[bindings]
focus_left = "Alt-l"
focus_right = "Alt-h"
carry_1 = "z"
"#,
        )
        .unwrap();
        assert_eq!(
            config.bindings.modified_binding(3, b'l'),
            Some(BindingAction::Focus(Direction::Left))
        );
        assert_eq!(
            config.bindings.binding(b'z'),
            Some(BindingAction::CarryMemberToTab(1))
        );
        assert_eq!(config.bindings.modified_binding(7, b'1'), None);
        assert!(
            config
                .binding_help()
                .iter()
                .any(|line| line == "leader z: carry_1")
        );
        assert!(RuntimeConfig::from_toml_str("[bindings]\nfocus_left = \"Alt-l\"").is_err());
        assert!(RuntimeConfig::from_toml_str("[bindings]\nfocus_left = \"Alt-1\"").is_err());
        assert!(
            RuntimeConfig::from_toml_str("alt_numbers = false\n[bindings]\nfocus_left = \"Alt-1\"")
                .is_ok()
        );
    }

    #[test]
    fn key_parser_accepts_byte_and_control_forms() {
        assert_eq!(parse_key("Ctrl-b"), Ok(0x02));
        assert_eq!(parse_key("0x1b"), Ok(0x1b));
        assert_eq!(parse_key("|"), Ok(b'|'));
        assert!(parse_key("Ctrl-12").is_err());
    }
}
