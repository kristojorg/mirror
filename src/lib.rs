pub mod cli;
pub mod downloader;
pub mod file_manager;
pub mod html_parser;

// Re-export main types for convenience
pub use cli::MirrorCommand;
pub use downloader::{DownloadPriority, DownloadTask, WebsiteMirror};
pub use file_manager::FileManager;
pub use html_parser::{HtmlParser, ResourceLink, ResourceType};
