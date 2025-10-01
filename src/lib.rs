pub mod cli;
pub mod downloader;
pub mod file_manager;
pub mod html_rewriter;
pub mod parser;
pub mod persistent_state;
pub mod run_logger;
pub mod url_mapper;

// Re-export main types for convenience
pub use cli::MirrorCommand;
pub use downloader::{DownloadPriority, WebsiteMirror};
pub use file_manager::FileManager;
pub use html_rewriter::HtmlRewriter;
pub use parser::{CssParser, HtmlParser, ResourceLink, ResourceType};
pub use persistent_state::DownloadTask;
pub use url_mapper::UrlMapper;
