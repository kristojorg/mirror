pub mod cli;
pub mod downloader;
pub mod file_manager;
pub mod html_parser;
pub mod html_rewriter;
pub mod url_mapper;

// Re-export main types for convenience
pub use cli::MirrorCommand;
pub use downloader::{DownloadPriority, DownloadTask, WebsiteMirror};
pub use file_manager::FileManager;
pub use html_parser::{HtmlParser, ResourceLink, ResourceType};
pub use html_rewriter::HtmlRewriter;
pub use url_mapper::UrlMapper;
