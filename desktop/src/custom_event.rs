//! Custom event type for desktop ruffle

use crate::player::PlayerOptions;

/// User-defined events.
pub enum RuffleEvent {
    /// Indicates that one or more tasks are ready to poll on our executor.
    TaskPoll,

    /// Indicates that an asynchronous SWF metadata load has been completed.
    OnMetadata(ruffle_core::swf::HeaderExt),

    /// The user requested to pick and then open a file.
    BrowseAndOpen(Box<PlayerOptions>),

    /// The user requested to open a URL.
    OpenURL(url::Url, Box<PlayerOptions>),

    /// The user requested to close the current SWF.
    CloseFile,

    /// The user requested to exit Ruffle.
    ExitRequested,

    /// The user selected an item in the right-click context menu.
    ContextMenuItemClicked(usize),
}
