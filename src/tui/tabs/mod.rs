//! Tab identifiers and per-tab state.

pub mod config_schema;
pub mod config_tab;
pub mod operate_tab;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabId {
    Config,
    Operate,
    View,
}

impl TabId {
    pub const ALL: [TabId; 3] = [TabId::Config, TabId::Operate, TabId::View];

    pub fn label(self) -> &'static str {
        match self {
            TabId::Config => "1:Config",
            TabId::Operate => "2:Operate",
            TabId::View => "3:View",
        }
    }

    pub fn next(self) -> TabId {
        match self {
            TabId::Config => TabId::Operate,
            TabId::Operate => TabId::View,
            TabId::View => TabId::Config,
        }
    }

    pub fn prev(self) -> TabId {
        match self {
            TabId::Config => TabId::View,
            TabId::Operate => TabId::Config,
            TabId::View => TabId::Operate,
        }
    }
}
