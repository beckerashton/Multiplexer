use std::path::PathBuf;

macro_rules! id_type {
    ($name:ident) => {
        #[derive(
            Clone,
            Copy,
            Debug,
            Default,
            Eq,
            Hash,
            Ord,
            PartialEq,
            PartialOrd,
            serde::Serialize,
            serde::Deserialize,
        )]
        pub struct $name(pub u64);
    };
}

id_type!(TabId);
id_type!(SlotId);
id_type!(SessionId);

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Axis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CellRect {
    pub x: u16,
    pub y: u16,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionState {
    Starting,
    Live,
    Exited,
    KillRequested,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BroadcastScope {
    Focused,
    VisibleTab,
    Manual,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkspaceCommand {
    ToggleVisibility {
        element: VisibilityElement,
        global: bool,
    },
    ToggleFold,
    ToggleBorderless,
    ToggleBorderlessStackLines,
    SetJumpMark(u8),
    JumpToMark(u8),
    SplitFocused {
        axis: Axis,
        session: SessionSpec,
    },
    SplitTab {
        axis: Axis,
        session: SessionSpec,
    },
    AddToFocusedStack {
        session: SessionSpec,
    },
    Focus(Direction),
    /// Visit pane slots in layout order without changing stack selection.
    CyclePane {
        delta: i8,
    },
    Resize {
        direction: Direction,
        cells: i16,
    },
    Equalize(Axis),
    Swap(Direction),
    SwapMember(Direction),
    CarryMemberToTab {
        number: usize,
    },
    SelectStackMember {
        index: usize,
    },
    CycleStack {
        delta: i8,
    },
    CreateTab {
        session: SessionSpec,
    },
    SelectTab(TabId),
    NavigateTab {
        number: usize,
    },
    CarryToTab {
        number: usize,
    },
    RequestCloseTab(TabId),
    ConfirmCloseTab(TabId),
    CarryFocusedSlot {
        destination: TabId,
    },
    RemoveFocusedSlot,
    RequestKillFocusedSession,
    ConfirmKillFocusedSession,
    RequestKillFocusedStack,
    ConfirmKillFocusedStack,
    SetBroadcastScope(BroadcastScope),
    ToggleManualBroadcastTarget(SlotId),
    ResetBroadcast,
    SelectionMode,
    /// Runtime-only destructive action. The terminal host must obtain a
    /// second explicit confirmation before leaving raw/alternate-screen mode.
    RequestQuit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LifecycleEffect {
    Spawn {
        session: SessionId,
        spec: SessionSpec,
    },
    Terminate {
        session: SessionId,
    },
}

/// Decorations which can be toggled from the visibility menus.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VisibilityElement {
    TopBar,
    Borders,
    InsideBorders,
    OutsideBorders,
    Top,
    Left,
    Bottom,
    Right,
    StackLines,
    Folds,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PaneVisibility {
    pub top: bool,
    pub left: bool,
    pub bottom: bool,
    pub right: bool,
    pub stack_lines: bool,
}
impl Default for PaneVisibility {
    fn default() -> Self {
        Self {
            top: true,
            left: true,
            bottom: true,
            right: true,
            stack_lines: true,
        }
    }
}
impl PaneVisibility {
    pub fn visible(self, element: VisibilityElement) -> bool {
        match element {
            VisibilityElement::Borders => self.top && self.left && self.bottom && self.right,
            VisibilityElement::Top => self.top,
            VisibilityElement::Left => self.left,
            VisibilityElement::Bottom => self.bottom,
            VisibilityElement::Right => self.right,
            VisibilityElement::StackLines => self.stack_lines,
            _ => true,
        }
    }
    pub fn set(&mut self, element: VisibilityElement, visible: bool) {
        match element {
            VisibilityElement::Borders => {
                self.top = visible;
                self.left = visible;
                self.bottom = visible;
                self.right = visible;
            }
            VisibilityElement::Top => self.top = visible,
            VisibilityElement::Left => self.left = visible,
            VisibilityElement::Bottom => self.bottom = visible,
            VisibilityElement::Right => self.right = visible,
            VisibilityElement::StackLines => self.stack_lines = visible,
            _ => (),
        }
    }
}
