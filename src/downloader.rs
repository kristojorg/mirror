use anyhow::Result;
use regex::Regex;
use reqwest::{Client, ClientBuilder, StatusCode};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::file_manager::FileManager;
use crate::parser::{CssParser, HtmlParser, ResourceType};
use crate::html_rewriter::HtmlRewriter;
use crate::persistent_state::{DownloadTask, PersistentState};
use crate::run_logger::RunLogger;
use crate::url_mapper::UrlMapper;

/// Result of processing a URL
#[derive(Debug, Clone)]
pub enum ProcessResult {
    Downloaded,
    SkippedAlreadyExists, // File already exists on disk (resumption)
    SkippedFiltered,      // Skipped due to resource type filter
    AlreadyVisited,       // Already processed in this session
    Error(String),
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DownloadPriority {
    Critical = 0, // CSS and JavaScript files
    High = 1,     // HTML pages
    Normal = 2,   // Images and other resources
}

impl PartialOrd for DownloadPriority {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for DownloadPriority {
    fn cmp(&self, other: &Self) -> Ordering {
        // Lower numbers = higher priority
        match (self, other) {
            (DownloadPriority::Critical, DownloadPriority::Critical) => Ordering::Equal,
            (DownloadPriority::Critical, _) => Ordering::Less,
            (DownloadPriority::High, DownloadPriority::Critical) => Ordering::Greater,
            (DownloadPriority::High, DownloadPriority::High) => Ordering::Equal,
            (DownloadPriority::High, _) => Ordering::Less,
            (DownloadPriority::Normal, DownloadPriority::Normal) => Ordering::Equal,
            (DownloadPriority::Normal, _) => Ordering::Greater,
        }
    }
}

// DownloadTask is now imported from persistent_state module

#[derive(Clone)]
pub struct WebsiteMirror {
    pub base_url: String,
    pub output_dir: PathBuf,
    pub max_depth: usize,
    pub ignore_robots: bool,
    pub download_external: bool,
    pub only_resources: Option<Vec<String>>,
    pub no_proxy: bool,
    pub ignore_patterns: Option<Vec<Regex>>,
    client: Client,
    file_manager: FileManager,
    state: Arc<PersistentState>, // Unified persistent state management
    run_logger: Arc<RunLogger>,  // Runtime logger for tracking this run
}

impl std::fmt::Debug for WebsiteMirror {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebsiteMirror")
            .field("base_url", &self.base_url)
            .field("output_dir", &self.output_dir)
            .field("max_depth", &self.max_depth)
            .field("ignore_robots", &self.ignore_robots)
            .field("download_external", &self.download_external)
            .field("only_resources", &self.only_resources)
            .field("no_proxy", &self.no_proxy)
            .field(
                "ignore_patterns",
                &self
                    .ignore_patterns
                    .as_ref()
                    .map(|p| format!("{} patterns", p.len())),
            )
            .finish()
    }
}

impl WebsiteMirror {
    /// Calculate relative path from source file to target file
    fn calculate_relative_path(from_path: &str, to_path: &str) -> String {
        use std::path::Path;

        let from = Path::new(from_path);
        let to = Path::new(to_path);

        // Get the directory of the source file
        let from_dir = from.parent().unwrap_or(Path::new(""));

        // Calculate relative path
        match pathdiff::diff_paths(to, from_dir) {
            Some(relative) => relative.to_string_lossy().to_string(),
            None => to_path.to_string(), // Fallback to absolute path
        }
    }

    /// Check if a URL should be ignored based on ignore patterns
    pub fn should_ignore_url(&self, url: &str) -> bool {
        if let Some(ref patterns) = self.ignore_patterns {
            for pattern in patterns {
                if pattern.is_match(url) {
                    log::debug!("Ignoring URL (matches pattern): {}", url);
                    return true;
                }
            }
        }
        false
    }

    /// Check if a resource type should be processed based on the only_resources filter
    pub fn should_process_resource_type(&self, resource_type: &ResourceType) -> bool {
        if let Some(ref only_resources) = self.only_resources {
            let type_str = match resource_type {
                ResourceType::Image => "images",
                ResourceType::CSS => "css",
                ResourceType::JavaScript => "js",
                ResourceType::Link => "html",
                ResourceType::Other => "other",
            };
            only_resources.iter().any(|r| r.to_lowercase() == type_str)
        } else {
            // If no filter specified, process all resource types
            true
        }
    }

    pub fn new(
        base_url: &str,
        output_dir: &Path,
        max_depth: usize,
        ignore_robots: bool,
        download_external: bool,
        only_resources: Option<Vec<String>>,
        no_proxy: bool,
        ignore_patterns: Option<Vec<String>>,
    ) -> Result<Self> {
        let client = Self::build_http_client(no_proxy)?;
        let file_manager = FileManager::new(output_dir)?;

        // Compile ignore patterns into regex
        let compiled_patterns = if let Some(patterns) = ignore_patterns {
            let mut compiled = Vec::new();
            for pattern in patterns {
                match Regex::new(&pattern) {
                    Ok(regex) => compiled.push(regex),
                    Err(e) => {
                        log::warn!("Invalid regex pattern '{}': {}", pattern, e);
                        return Err(anyhow::anyhow!(
                            "Invalid regex pattern '{}': {}",
                            pattern,
                            e
                        ));
                    }
                }
            }
            Some(compiled)
        } else {
            None
        };

        // Create the run logger first so PersistentState can log during initialization
        let run_logger = RunLogger::init(output_dir)?;
        let run_logger = Arc::new(run_logger);

        // Create persistent state (automatically loads existing state or creates new)
        let state = Arc::new(PersistentState::new(output_dir)?);

        // Set PersistentState for RunLogger
        run_logger.set_persistent_state(Arc::clone(&state));

        Ok(Self {
            base_url: base_url.to_string(),
            output_dir: output_dir.to_path_buf(),
            max_depth,
            ignore_robots,
            download_external,
            only_resources,
            no_proxy,
            ignore_patterns: compiled_patterns,
            client,
            file_manager,
            state,
            run_logger,
        })
    }

    fn build_http_client(no_proxy: bool) -> Result<Client> {
        // Build a simple HTTP client with default SSL handling
        let mut builder = ClientBuilder::new()
            .use_rustls_tls()
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
            .timeout(std::time::Duration::from_secs(480));

        if !no_proxy {
            let proxy = reqwest::Proxy::all(
                "https://user-spxihizegc:wk0c88X0N~nRibgUxm@gate.decodo.com:7000",
            )?;
            builder = builder.proxy(proxy);
        }

        let client = builder.build()?;

        Ok(client)
    }

    /// Get statistics from PersistentState
    pub fn get_statistics(&self) -> crate::persistent_state::Statistics {
        self.state.get_statistics()
    }

    /// Get a reference to the PersistentState
    pub fn get_state(&self) -> &Arc<PersistentState> {
        &self.state
    }

    /// Get a reference to the RunLogger for summary writing
    pub fn get_run_logger(&self) -> Arc<RunLogger> {
        self.run_logger.clone()
    }

    pub async fn mirror_website(&mut self) -> Result<()> {
        let command_args: Vec<String> = std::env::args().collect();
        if command_args.is_empty() {
            log::info!("Startup command: <unavailable>");
        } else {
            log::info!("Startup command: {}", command_args.join(" "));
        }
        log::info!(
            "Proxy: {}",
            if self.no_proxy {
                "disabled (--no-proxy)"
            } else {
                "enabled"
            }
        );
        log::info!("Starting mirror: {}", self.base_url);
        log::info!("Output: {:?}", self.output_dir);
        log::info!("Max depth: {}", self.max_depth);

        // Add the base URL to the download queue with high priority (HTML page)
        // Only add HTML pages if we're not filtering to specific resource types
        // and if the base URL doesn't match any ignore patterns
        if self.only_resources.is_none() || self.should_process_resource_type(&ResourceType::Link) {
            // Check if base URL should be ignored
            if !self.should_ignore_url(&self.base_url) {
                self.state.enqueue(DownloadTask {
                    url: self.base_url.clone(),
                    depth: 0,
                    priority: DownloadPriority::High,
                    resource_type: None,
                    source_url: None, // Base URL came from command line
                });
            } else {
                log::info!("Base URL matches ignore pattern - skipping");
            }
        } else {
            log::info!("Resource filter active - skipping HTML page crawling");
        }

        // Process the download queue
        loop {
            // Use PersistentState dequeue which automatically moves URL to processing
            let download_task = self.state.dequeue();

            if let Some(task) = download_task {
                let url = task.url.clone();
                let depth = task.depth;
                let priority = task.priority.clone();
                let resource_type = task.resource_type.clone();
                let source_url = task.source_url.clone();
                // Check depth limit (0 means unlimited)
                if self.max_depth > 0 && depth > self.max_depth {
                    continue;
                }

                if self.should_ignore_url(&url) {
                    log::debug!("Ignored URL: {}", url);
                    self.state.mark_ignored(
                        &url,
                        resource_type.as_ref(),
                        "Matched ignore pattern",
                        source_url.clone(),
                    );
                    continue;
                }

                let client = self.client.clone();
                let file_manager = self.file_manager.clone();
                let state = self.state.clone();
                let base_url = self.base_url.clone();
                let run_logger = self.run_logger.clone();
                let ignore_patterns = self.ignore_patterns.clone();

                // Process the download directly instead of spawning a task
                match Self::download_and_process_url(
                    &client,
                    &file_manager,
                    &url,
                    depth,
                    &state,
                    &base_url,
                    priority,
                    resource_type,
                    &run_logger,
                    &ignore_patterns,
                    source_url,
                )
                .await
                {
                    Ok(ProcessResult::Downloaded) => {
                        // Success logged in download_resource
                    }
                    Ok(ProcessResult::SkippedAlreadyExists) => {
                        // Already logged
                    }
                    Ok(ProcessResult::SkippedFiltered) => {
                        // Already logged
                    }
                    Ok(ProcessResult::AlreadyVisited) => {
                        // This is normal, no log needed
                    }
                    Ok(ProcessResult::Error(msg)) => {
                        log::error!("Error downloading {}: {}", url, msg);
                    }
                    Err(e) => {
                        log::error!("Unexpected error: {} - {}", url, e);
                    }
                }
            } else {
                // Check if all downloads are complete
                let queue_size = self.state.queue_size();

                if queue_size == 0 {
                    // Wait a bit for any ongoing downloads to complete
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

                    let final_queue_size = self.state.queue_size();
                    if final_queue_size == 0 {
                        break;
                    }
                }

                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            }
        }

        Ok(())
    }

    async fn download_and_process_html(
        client: &Client,
        file_manager: &FileManager,
        url: &str,
        depth: usize,
        state: &Arc<PersistentState>,
        base_url: &str,
        run_logger: &Arc<RunLogger>,
        ignore_patterns: &Option<Vec<Regex>>,
        source_url: Option<String>,
    ) -> Result<ProcessResult> {
        log::debug!("📄 Processing HTML page: {}", url);

        // Download the HTML page
        log::debug!("📥 Downloading HTML page: {}", url);
        let client_for_download = client.clone();
        let response = match client_for_download.get(url).send().await {
            Ok(resp) => resp,
            Err(e) => {
                log::error!("Failed to fetch HTML: {} - {}", url, e);
                // Mark errored in persistent state
                state.mark_errored(
                    url.to_string(),
                    format!("Request failed: {}", e),
                    ResourceType::Link,
                    source_url.clone(),
                );
                run_logger.track_error();
                return Ok(ProcessResult::Error(format!("Request failed: {}", e)));
            }
        };

        if response.status() != StatusCode::OK {
            log::warn!("HTTP {}: {}", response.status(), url);
            // Mark errored in persistent state
            state.mark_errored(
                url.to_string(),
                format!("HTTP {} for {}", response.status(), url),
                ResourceType::Link,
                source_url.clone(),
            );
            run_logger.track_error();
            return Ok(ProcessResult::Error(format!(
                "HTTP {} for {}",
                response.status(),
                url
            )));
        }

        let content = match response.bytes().await {
            Ok(bytes) => bytes,
            Err(e) => {
                log::error!("Failed to read response: {} - {}", url, e);
                // Mark errored in persistent state
                state.mark_errored(
                    url.to_string(),
                    format!("Failed to read response body: {}", e),
                    ResourceType::Link,
                    source_url.clone(),
                );
                run_logger.track_error();
                return Ok(ProcessResult::Error(format!(
                    "Failed to read response body: {}",
                    e
                )));
            }
        };

        let html_content = String::from_utf8_lossy(&content);

        // Create a new HTML parser with the current page's URL and HTML content
        let page_html_parser = HtmlParser::new(url, &html_content)?;
        let html_rewriter = HtmlRewriter::new();

        // Extract resources from HTML
        let resources = page_html_parser.extract_resources()?;

        // Get base tag HTML strings for removal (after parsing, before rewriting)
        let base_tag_htmls = page_html_parser.get_base_tag_htmls();

        // Phase 1: Build url_mappings upfront (original_url -> absolute_url)
        // This maps ALL URLs to their absolute forms for HTML normalization
        let mut url_mappings = HashMap::new();
        for resource in &resources {
            url_mappings.insert(resource.original_url.clone(), resource.resolved.clone());
        }

        // Phase 1: Queue HTML links for crawling (don't download other resources)
        for resource in &resources {
            // Only process HTML links for crawling
            if resource.resource_type == ResourceType::Link {
                // Only queue links from same domain
                if !resource.resolved.starts_with(base_url) {
                    continue;
                }

                // Skip if URL matches ignore pattern
                if let Some(ref patterns) = ignore_patterns {
                    let mut should_skip = false;
                    for pattern in patterns {
                        if pattern.is_match(&resource.resolved) {
                            log::debug!("Ignoring URL (matches pattern): {}", resource.resolved);
                            should_skip = true;
                            break;
                        }
                    }
                    if should_skip {
                        state.mark_ignored(
                            &resource.resolved,
                            Some(&resource.resource_type),
                            "Matched ignore pattern",
                            Some(url.to_string()),
                        );
                        continue;
                    }
                }

                // Queue for crawling
                state.enqueue(DownloadTask {
                    url: resource.resolved.clone(),
                    depth: depth + 1,
                    priority: DownloadPriority::High,
                    resource_type: Some(resource.resource_type.clone()),
                    source_url: Some(url.to_string()),
                });
            }
        }

        // Phase 1: Normalize HTML
        // 1. Replace URLs with absolute forms
        let mut html_content_updated = html_rewriter.rewrite_urls(&html_content, &url_mappings);
        // 2. Remove base tags
        html_content_updated = html_rewriter.remove_base_tags(&html_content_updated, &base_tag_htmls);

        // Save the updated HTML
        let url_mapper = UrlMapper::new()?;
        let local_html_path = url_mapper.url_to_local_path(url, &ResourceType::Link)?;
        let _saved_path =
            file_manager.save_file(&local_html_path, html_content_updated.as_bytes())?;

        // Track successful HTML download
        let size_bytes = html_content_updated.len() as u64;

        // Track downloaded in run logger
        run_logger.track_downloaded(size_bytes);

        // Mark completed in persistent state
        state.mark_completed(
            url.to_string(),
            local_html_path.to_string_lossy().to_string(),
            ResourceType::Link,
            size_bytes,
            source_url,
        );

        log::info!("Downloaded HTML: {}", url);

        Ok(ProcessResult::Downloaded)
    }

    async fn download_and_process_css(
        client: &Client,
        file_manager: &FileManager,
        url: &str,
        state: &Arc<PersistentState>,
        run_logger: &Arc<RunLogger>,
        source_url: Option<String>,
    ) -> Result<ProcessResult> {
        // Check if CSS already downloaded using persistent state
        if state.is_visited(url) {
            // Track as skipped in run logger (only if from previous run)
            if state.should_track_as_skipped(url) {
                run_logger.track_skipped();
            }
            return Ok(ProcessResult::SkippedAlreadyExists);
        }

        // Download the URL
        let response = match client.get(url).send().await {
            Ok(resp) => resp,
            Err(e) => {
                log::error!("Failed to fetch CSS: {} - {}", url, e);
                // Mark errored in persistent state
                state.mark_errored(
                    url.to_string(),
                    format!("Request failed: {}", e),
                    ResourceType::CSS,
                    source_url.clone(),
                );
                run_logger.track_error();
                return Ok(ProcessResult::Error(format!("Request failed: {}", e)));
            }
        };

        if response.status() != StatusCode::OK {
            log::warn!("HTTP {}: {}", response.status(), url);
            // Mark errored in persistent state
            state.mark_errored(
                url.to_string(),
                format!("HTTP {} for {}", response.status(), url),
                ResourceType::CSS,
                source_url.clone(),
            );
            run_logger.track_error();
            return Ok(ProcessResult::Error(format!(
                "HTTP {} for {}",
                response.status(),
                url
            )));
        }

        let content = match response.bytes().await {
            Ok(bytes) => bytes,
            Err(e) => {
                log::error!("Failed to read response: {} - {}", url, e);
                // Mark errored in persistent state
                state.mark_errored(
                    url.to_string(),
                    format!("Failed to read response body: {}", e),
                    ResourceType::CSS,
                    source_url.clone(),
                );
                run_logger.track_error();
                return Ok(ProcessResult::Error(format!(
                    "Failed to read response body: {}",
                    e
                )));
            }
        };

        // Process CSS files to extract background images
        let css_content = String::from_utf8_lossy(&content);
        let css_parser = CssParser::new(url)?;

        // Extract background images from CSS
        let background_resources = css_parser.extract_background_images(&css_content);

        // Don't log background images - too noisy

        // Download background images with normal priority (after CSS/JS)
        for resource in &background_resources {
            let url_mapper = UrlMapper::new()?;
            if let Err(_e) = Self::download_resource(
                client,
                file_manager,
                &url_mapper,
                &resource.resolved,
                &ResourceType::Image,
                state,
                run_logger,
                Some(url.to_string()), // Found in current CSS file
            )
            .await
            {
                // Error already logged in download_resource
            }
        }

        // Save the CSS file
        let url_mapper = UrlMapper::new()?;
        let local_path = url_mapper.url_to_local_path(url, &ResourceType::CSS)?;
        let _saved_path = file_manager.save_file(&local_path, &content)?;

        // Track successful CSS download
        let size_bytes = content.len() as u64;

        // Track downloaded in run logger
        run_logger.track_downloaded(size_bytes);

        // Mark completed in persistent state
        state.mark_completed(
            url.to_string(),
            local_path.to_string_lossy().to_string(),
            ResourceType::CSS,
            size_bytes,
            source_url,
        );

        log::info!("Downloaded CSS: {}", url);

        Ok(ProcessResult::Downloaded)
    }

    async fn download_and_process_url(
        client: &Client,
        file_manager: &FileManager,
        url: &str,
        depth: usize,
        state: &Arc<PersistentState>,
        base_url: &str,
        priority: DownloadPriority,
        resource_type: Option<ResourceType>,
        run_logger: &Arc<RunLogger>,
        ignore_patterns: &Option<Vec<Regex>>,
        source_url: Option<String>,
    ) -> Result<ProcessResult> {
        // PersistentState already handles deduplication via dequeue
        // The URL is already moved to processing when dequeued
        // So we don't need a separate visited check here

        let priority_str = match priority {
            DownloadPriority::Critical => "🔥 CRITICAL",
            DownloadPriority::High => "⚡ HIGH",
            DownloadPriority::Normal => "📥 NORMAL",
        };
        log::debug!("{} Processing: {} (depth: {})", priority_str, url, depth);

        // Route to appropriate handler based on resource type
        let result = match resource_type {
            Some(ResourceType::Link) | None => {
                // HTML page
                Self::download_and_process_html(
                    client,
                    file_manager,
                    url,
                    depth,
                    state,
                    base_url,
                    run_logger,
                    ignore_patterns,
                    source_url.clone(),
                )
                .await
            }
            Some(ResourceType::CSS) => {
                // CSS file
                Self::download_and_process_css(
                    client,
                    file_manager,
                    url,
                    state,
                    run_logger,
                    source_url.clone(),
                )
                .await
            }
            Some(resource_type) => {
                // Other resources (JS, images, etc.) - just download without processing
                log::debug!("📦 Processing resource: {}", url);
                let url_mapper = UrlMapper::new()?;
                Self::download_resource(
                    client,
                    file_manager,
                    &url_mapper,
                    url,
                    &resource_type,
                    state,
                    run_logger,
                    source_url, // Pass through from task
                )
                .await
            }
        };

        result
    }

    async fn download_resource(
        client: &Client,
        file_manager: &FileManager,
        url_mapper: &UrlMapper,
        url: &str,
        resource_type: &ResourceType,
        state: &Arc<PersistentState>,
        run_logger: &Arc<RunLogger>,
        source_url: Option<String>,
    ) -> Result<ProcessResult> {
        // Check if resource already downloaded using persistent state
        if state.is_visited(url) {
            // Track as skipped in run logger (only if from previous run)
            if state.should_track_as_skipped(url) {
                run_logger.track_skipped();
            }
            return Ok(ProcessResult::SkippedAlreadyExists);
        }

        // Get local path for saving
        let local_path = url_mapper.url_to_local_path(url, resource_type)?;

        // Determine resource type string for better logging
        let resource_type_str = match resource_type {
            ResourceType::CSS => "CSS",
            ResourceType::JavaScript => "JS",
            ResourceType::Image => "image",
            ResourceType::Link => "HTML",
            ResourceType::Other => {
                if url.ends_with(".woff")
                    || url.ends_with(".woff2")
                    || url.ends_with(".ttf")
                    || url.ends_with(".eot")
                {
                    "font"
                } else {
                    "resource"
                }
            }
        };

        let response = match client.get(url).send().await {
            Ok(resp) => resp,
            Err(e) => {
                log::error!("Failed to fetch {}: {} - {}", resource_type_str, url, e);
                // Mark errored in persistent state
                state.mark_errored(
                    url.to_string(),
                    format!("Request failed: {}", e),
                    resource_type.clone(),
                    source_url.clone(),
                );
                run_logger.track_error();
                return Ok(ProcessResult::Error(format!("Request failed: {}", e)));
            }
        };

        if response.status() != StatusCode::OK {
            log::warn!(
                "HTTP {}: {} ({})",
                response.status(),
                url,
                resource_type_str
            );
            // Mark errored in persistent state
            state.mark_errored(
                url.to_string(),
                format!("HTTP {} for {}", response.status(), url),
                resource_type.clone(),
                source_url.clone(),
            );
            run_logger.track_error();
            return Ok(ProcessResult::Error(format!(
                "HTTP {} for {}",
                response.status(),
                url
            )));
        }

        let _content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_string();

        let content = match response.bytes().await {
            Ok(bytes) => bytes,
            Err(e) => {
                log::error!("Failed to read response: {} - {}", url, e);
                // Mark errored in persistent state
                state.mark_errored(
                    url.to_string(),
                    format!("Failed to read response body: {}", e),
                    resource_type.clone(),
                    source_url.clone(),
                );
                run_logger.track_error();
                return Ok(ProcessResult::Error(format!(
                    "Failed to read response body: {}",
                    e
                )));
            }
        };

        let final_content = content.to_vec();

        let _saved_path = match file_manager.save_file(&local_path, &final_content) {
            Ok(path) => path,
            Err(e) => {
                log::error!("Failed to save {}: {} - {}", resource_type_str, url, e);
                // Mark errored in persistent state
                state.mark_errored(
                    url.to_string(),
                    format!("Failed to save file: {}", e),
                    resource_type.clone(),
                    source_url.clone(),
                );
                run_logger.track_error();
                return Ok(ProcessResult::Error(format!("Failed to save file: {}", e)));
            }
        };

        // Track successful download
        let size_bytes = final_content.len() as u64;

        // Track downloaded in run logger
        run_logger.track_downloaded(size_bytes);

        // Mark completed in persistent state
        state.mark_completed(
            url.to_string(),
            local_path.to_string_lossy().to_string(),
            resource_type.clone(),
            size_bytes,
            source_url,
        );

        log::info!("Downloaded {}: {}", resource_type_str, url);

        Ok(ProcessResult::Downloaded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tempfile::tempdir;

    #[test]
    fn test_website_mirror_new() {
        let temp_dir = tempdir().unwrap();
        let mirror = WebsiteMirror::new(
            "https://example.com",
            temp_dir.path(),
            3,
            false,
            false,
            None,
            true, // no_proxy
            None, // ignore_patterns
        )
        .unwrap();

        assert_eq!(mirror.base_url.as_str(), "https://example.com");
        assert_eq!(mirror.max_depth, 3);
        assert_eq!(mirror.ignore_robots, false);
        assert_eq!(mirror.download_external, false);
    }

    #[test]
    fn test_website_mirror_new_with_options() {
        let temp_dir = tempdir().unwrap();
        let mirror = WebsiteMirror::new(
            "https://example.com",
            temp_dir.path(),
            5,
            true,
            true,
            Some(vec!["images".to_string()]),
            false, // no_proxy
            None,  // ignore_patterns
        )
        .unwrap();

        assert_eq!(mirror.max_depth, 5);
        assert_eq!(mirror.ignore_robots, true);
        assert_eq!(mirror.download_external, true);
        assert_eq!(mirror.only_resources, Some(vec!["images".to_string()]));
    }

    #[test]
    fn test_should_process_resource_type() {
        let temp_dir = tempdir().unwrap();
        let mirror = WebsiteMirror::new(
            "https://example.com",
            temp_dir.path(),
            3,
            false,
            false,
            None,
            true, // no_proxy
            None, // ignore_patterns
        )
        .unwrap();

        // Test with no restrictions
        assert!(mirror.should_process_resource_type(&ResourceType::CSS));
        assert!(mirror.should_process_resource_type(&ResourceType::JavaScript));
        assert!(mirror.should_process_resource_type(&ResourceType::Image));
        assert!(mirror.should_process_resource_type(&ResourceType::Link));

        // Test with specific restrictions
        let mirror = WebsiteMirror::new(
            "https://example.com",
            temp_dir.path(),
            3,
            false,
            false,
            Some(vec!["images".to_string(), "css".to_string()]),
            true, // no_proxy
            None, // ignore_patterns
        )
        .unwrap();

        assert!(mirror.should_process_resource_type(&ResourceType::CSS));
        assert!(!mirror.should_process_resource_type(&ResourceType::JavaScript));
        assert!(mirror.should_process_resource_type(&ResourceType::Image));
        assert!(!mirror.should_process_resource_type(&ResourceType::Link));
    }

    #[test]
    fn test_download_task_ordering() {
        let task1 = DownloadTask {
            url: "https://example.com/style.css".to_string(),
            depth: 1,
            priority: DownloadPriority::Critical,
            resource_type: Some(ResourceType::CSS),
        };

        let task2 = DownloadTask {
            url: "https://example.com/page.html".to_string(),
            depth: 1,
            priority: DownloadPriority::High,
            resource_type: Some(ResourceType::Link),
        };

        let task3 = DownloadTask {
            url: "https://example.com/image.jpg".to_string(),
            depth: 1,
            priority: DownloadPriority::Normal,
            resource_type: Some(ResourceType::Image),
        };

        let task4 = DownloadTask {
            url: "https://example.com/script.js".to_string(),
            depth: 2,
            priority: DownloadPriority::Critical,
            resource_type: Some(ResourceType::JavaScript),
        };

        // Critical should come before High
        assert!(task1 > task2);

        // High should come before Normal
        assert!(task2 > task3);

        // Same priority, lower depth should come first
        assert!(task1 > task4);

        // Test PartialOrd
        assert!(task1 >= task2);
        assert!(task2 <= task1);
    }

    #[test]
    fn test_download_task_equality() {
        let task1 = DownloadTask {
            url: "https://example.com/style.css".to_string(),
            depth: 1,
            priority: DownloadPriority::Critical,
            resource_type: Some(ResourceType::CSS),
        };

        let task2 = DownloadTask {
            url: "https://example.com/style.css".to_string(),
            depth: 1,
            priority: DownloadPriority::Critical,
            resource_type: Some(ResourceType::CSS),
        };

        let task3 = DownloadTask {
            url: "https://example.com/script.js".to_string(),
            depth: 1,
            priority: DownloadPriority::Critical,
            resource_type: Some(ResourceType::JavaScript),
        };

        assert_eq!(task1, task2);
        assert_ne!(task1, task3);
    }

    #[test]
    fn test_download_priority_ordering() {
        assert!(DownloadPriority::Critical > DownloadPriority::High);
        assert!(DownloadPriority::High > DownloadPriority::Normal);
        // Normal is the lowest priority we have
    }

    #[test]
    fn test_download_priority_debug() {
        assert_eq!(format!("{:?}", DownloadPriority::Critical), "Critical");
        assert_eq!(format!("{:?}", DownloadPriority::High), "High");
        assert_eq!(format!("{:?}", DownloadPriority::Normal), "Normal");
        // We only have 3 priority levels
    }

    #[test]
    fn test_download_priority_clone() {
        let priority = DownloadPriority::Critical;
        let cloned = priority.clone();
        assert_eq!(cloned, priority);
    }

    #[test]
    fn test_website_mirror_debug() {
        let temp_dir = tempdir().unwrap();
        let mirror = WebsiteMirror::new(
            "https://example.com",
            temp_dir.path(),
            3,
            false,
            false,
            None,
            true, // no_proxy
            None, // ignore_patterns
        )
        .unwrap();

        let debug_str = format!("{:?}", mirror);
        assert!(debug_str.contains("WebsiteMirror"));
        assert!(debug_str.contains("example.com"));
    }

    #[test]
    fn test_website_mirror_clone() {
        let temp_dir = tempdir().unwrap();
        let mirror = WebsiteMirror::new(
            "https://example.com",
            temp_dir.path(),
            3,
            false,
            false,
            None,
            true, // no_proxy
            None, // ignore_patterns
        )
        .unwrap();

        let cloned = mirror.clone();
        assert_eq!(mirror.base_url, cloned.base_url);
        assert_eq!(mirror.max_depth, cloned.max_depth);
        assert_eq!(mirror.ignore_robots, cloned.ignore_robots);
        assert_eq!(mirror.download_external, cloned.download_external);
    }

    // Note: WebsiteMirror doesn't implement PartialEq, Eq, or Hash due to complex fields
    // These tests are removed as they're not essential for functionality
}
