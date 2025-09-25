pub mod cli;
pub mod downloader;
pub mod file_manager;
pub mod html_parser;
pub mod html_rewriter;
pub mod mirror_state;
pub mod persistent_state;
pub mod run_logger;
pub mod url_mapper;

// Re-export main types for convenience
pub use cli::MirrorCommand;
pub use downloader::{DownloadPriority, WebsiteMirror};
pub use persistent_state::DownloadTask;
pub use file_manager::FileManager;
pub use html_parser::{HtmlParser, ResourceLink, ResourceType};
pub use html_rewriter::HtmlRewriter;
pub use mirror_state::MirrorState;
pub use url_mapper::UrlMapper;
