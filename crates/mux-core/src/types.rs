use std::path::PathBuf;

macro_rules! id_type {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(pub u64);
    };
}

id_type!(TabId);
id_type!(SlotId);
id_type!(SessionId);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
    SplitFocused {
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
