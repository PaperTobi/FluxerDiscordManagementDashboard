//! The interactive parts of pages (hydrated in the browser).

mod community;
mod copy;
mod person;
mod picker;
mod preview;
mod recorder;
mod say;
mod sidebar;
mod system;
mod wall;
mod widgets;

pub use community::GuildLive;
pub use copy::CopyText;
pub use person::PersonLive;
pub use picker::MemberPicker;
pub use preview::PreviewPlayer;
pub use recorder::ClipRecorder;
pub use say::SayGate;
pub use sidebar::SidebarLive;
pub use system::SystemLive;
pub use wall::WallLive;
pub use widgets::{Avatar, Sparkline};
