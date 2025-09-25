use anyhow::Result;
use colored::*;
use indicatif::{ProgressBar, ProgressStyle};

use reqwest::{Client, ClientBuilder, StatusCode};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::file_manager::FileManager;
use crate::html_parser::{HtmlParser, ResourceType};
use crate::html_rewriter::HtmlRewriter;
use crate::mirror_state::MirrorState;
use crate::persistent_state::{PersistentState, DownloadTask};
use crate::run_logger::RunLogger;
use crate::url_mapper::UrlMapper;
use webp::Encoder;

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

#[derive(Clone, Debug)]
pub struct WebsiteMirror {
    pub base_url: String,
    pub output_dir: PathBuf,
    pub max_depth: usize,
    pub max_concurrent: usize,
    pub ignore_robots: bool,
    pub download_external: bool,
    pub only_resources: Option<Vec<String>>,
    pub convert_to_webp: bool,
    client: Client,
    file_manager: FileManager,
    html_parser: HtmlParser,
    state: Arc<PersistentState>,  // Unified persistent state management
    run_logger: Arc<RunLogger>, // Runtime logger for tracking this run
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

    /// Perform comprehensive WebP extension replacement for any remaining image references
    pub fn perform_comprehensive_webp_replacement(html_content: &str) -> String {
        let mut updated_content = html_content.to_string();

        // First, do simple string replacements for all image extensions
        // This catches most cases including those in JavaScript, CSS, and HTML
        // Use a two-pass approach to avoid double-converting already .webp files

        // Pass 1: Mark already-converted .webp files with a temporary marker
        updated_content = updated_content.replace(".webp", "___WEBP_MARKER___");

        // Pass 2: Convert remaining image extensions to .webp
        let simple_replacements = vec![
            (".jpg", ".webp"),
            (".jpeg", ".webp"),
            (".png", ".webp"),
            (".JPG", ".webp"),
            (".JPEG", ".webp"),
            (".PNG", ".webp"),
        ];

        for (old_ext, new_ext) in simple_replacements {
            let before_count = updated_content.matches(old_ext).count();
            updated_content = updated_content.replace(old_ext, new_ext);
            let after_count = updated_content.matches(new_ext).count();

            if before_count > 0 {
                log::info!(
                    "🔍 Simple WebP replacement: {} -> {} ({} replacements)",
                    old_ext,
                    new_ext,
                    after_count,
                );
            }
        }

        // Pass 3: Restore the original .webp files
        updated_content = updated_content.replace("___WEBP_MARKER___", ".webp");

        // Then use regex patterns for more specific cases that might have been missed
        // These patterns will now work correctly since we've already handled the double-conversion issue
        let patterns = vec![
            // URLs in quotes that might have been missed
            (
                r#"url\(["']?([^"']*\.(?:jpg|jpeg|png|JPG|JPEG|PNG))["']?\)"#,
                r#"url($1.webp)"#,
            ),
            // Src attributes that might have been missed
            (
                r#"src=["']([^"']*\.(?:jpg|jpeg|png|JPG|JPEG|PNG))["']"#,
                r#"src="$1.webp""#,
            ),
            // Background image URLs that might have been missed
            (
                r#"background-image:\s*url\(["']?([^"']*\.(?:jpg|jpeg|png|JPG|JPEG|PNG))["']?\)"#,
                r#"background-image: url($1.webp)"#,
            ),
        ];

        for (pattern, replacement) in patterns {
            let regex = regex::Regex::new(pattern).unwrap();
            let before_count = regex.find_iter(&updated_content).count();
            updated_content = regex.replace_all(&updated_content, replacement).to_string();
            let after_count = regex.find_iter(&updated_content).count();

            if before_count > 0 {
                log::info!(
                    "🔍 Regex WebP replacement: {} -> {} ({} replacements)",
                    pattern,
                    replacement,
                    after_count
                );
            }
        }

        updated_content
    }

    /// Static version for use in functions without self access
    fn convert_to_webp_static(image_data: &[u8], original_url: &str) -> Result<Vec<u8>> {
        // Decode the image
        let img = match image::load_from_memory(image_data) {
            Ok(img) => img,
            Err(e) => {
                log::error!("⚠️  Failed to decode image {}: {}", original_url, e);
                return Ok(image_data.to_vec()); // Return original data if conversion fails
            }
        };

        // Convert to RGB8 if needed (WebP encoder expects RGB)
        let rgb_img = img.to_rgb8();

        // Create WebP encoder with good quality (80/100)
        let encoder = Encoder::from_rgb(&rgb_img, rgb_img.width(), rgb_img.height());

        // Encode with quality 80 (good balance between size and quality)
        let webp_data = encoder.encode(80.0);

        let original_size = image_data.len();
        let webp_size = webp_data.len();
        let compression_ratio = (original_size as f64 / webp_size as f64 * 100.0) as u32;

        log::info!(
            "🔄 Converted {} to WebP: {} -> {} bytes ({}% of original size)",
            original_url,
            original_size,
            webp_size,
            compression_ratio
        );

        Ok(webp_data.to_vec())
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
        max_concurrent: usize,
        ignore_robots: bool,
        download_external: bool,
        only_resources: Option<Vec<String>>,
        convert_to_webp: bool,
    ) -> Result<Self> {
        let client = Self::build_http_client()?;
        let file_manager = FileManager::new(output_dir)?;
        let html_parser = HtmlParser::new(base_url)?;

        // Create persistent state (automatically loads existing state or creates new)
        let state = Arc::new(PersistentState::new(output_dir)?);

        // Create the run logger internally
        let run_logger = RunLogger::init(output_dir)?;
        let run_logger = Arc::new(run_logger);

        // TODO: Phase 5 - Update RunLogger to work with PersistentState directly
        // For now, we'll create a temporary mirror_state for compatibility
        let mirror_state = MirrorState::new(output_dir)?;
        run_logger.set_mirror_state(Arc::new(mirror_state));

        Ok(Self {
            base_url: base_url.to_string(),
            output_dir: output_dir.to_path_buf(),
            max_depth,
            max_concurrent,
            ignore_robots,
            download_external,
            only_resources,
            convert_to_webp,
            client,
            file_manager,
            html_parser,
            state,
            run_logger,
        })
    }

    fn build_http_client() -> Result<Client> {
        // Build a simple HTTP client with default SSL handling
        let _proxy =
            reqwest::Proxy::all("https://user-spxihizegc:wk0c88X0N~nRibgUxm@gate.decodo.com:7000")?;
        let client = ClientBuilder::new()
            .use_rustls_tls()
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
            // .proxy(proxy)
            .timeout(std::time::Duration::from_secs(480))
            .build()?;

        Ok(client)
    }

    /// Get statistics from the PersistentState
    pub fn get_mirror_state_statistics(&self) -> crate::mirror_state::Statistics {
        // TODO: Phase 5 - This entire method will be removed when RunLogger uses PersistentState directly
        let stats = self.state.get_statistics();

        // Convert HashMap to DownloadStats structure (temporary compatibility layer)
        let mut download_stats = crate::mirror_state::DownloadStats::default();
        for (resource_type, count) in stats.downloads {
            let resource_stats = crate::mirror_state::ResourceStats {
                success: count,
                error: 0,
                bytes: 0,  // TODO: Phase 5 - Track bytes per resource type in PersistentState
            };
            match resource_type.as_str() {
                "html" | "link" => download_stats.html = resource_stats,
                "css" => download_stats.css = resource_stats,
                "javascript" | "js" => download_stats.js = resource_stats,
                "image" => download_stats.images = resource_stats,
                _ => download_stats.other = resource_stats,
            }
        }

        crate::mirror_state::Statistics {
            urls_discovered: stats.urls_discovered,
            downloads: download_stats,
            total_bytes: stats.total_bytes,
            last_updated: chrono::Local::now().to_rfc3339(),
        }
    }

    /// Get a reference to the MirrorState for sharing
    pub fn get_mirror_state(&self) -> Arc<MirrorState> {
        // TODO: Phase 5 - Remove this method entirely
        Arc::new(MirrorState::new(&self.output_dir).unwrap())
    }

    /// Get a reference to the RunLogger for summary writing
    pub fn get_run_logger(&self) -> Arc<RunLogger> {
        self.run_logger.clone()
    }

    pub async fn mirror_website(&mut self) -> Result<()> {
        log::info!(
            "🚀 Starting website mirroring for: {}",
            self.base_url.blue()
        );
        log::info!("📁 Output directory: {:?}", self.output_dir);
        log::info!("🔗 Max depth: {}", self.max_depth);
        log::info!("⚡ Max concurrent downloads: {}", self.max_concurrent);

        // Add the base URL to the download queue with high priority (HTML page)
        // Only add HTML pages if we're not filtering to specific resource types
        if self.only_resources.is_none() || self.should_process_resource_type(&ResourceType::Link) {
            self.state.enqueue(DownloadTask {
                url: self.base_url.clone(),
                depth: 0,
                priority: DownloadPriority::High,
                resource_type: None,
            });
        } else {
            log::info!("🔍 Resource filter active: skipping HTML page crawling");
        }

        let progress_bar = ProgressBar::new_spinner();
        progress_bar.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner} {msg}")
                .unwrap(),
        );

        // Process the download queue
        loop {
            // Use PersistentState dequeue which automatically moves URL to processing
            let download_task = self.state.dequeue();

            if let Some(task) = download_task {
                let url = task.url.clone();
                let depth = task.depth;
                let priority = task.priority.clone();
                let resource_type = task.resource_type.clone();
                // Check depth limit (0 means unlimited)
                if self.max_depth > 0 && depth > self.max_depth {
                    continue;
                }

                let client = self.client.clone();
                let file_manager = self.file_manager.clone();
                let state = self.state.clone();
                // TODO: Phase 5 - Remove MirrorState dependency completely
                let mirror_state = MirrorState::new(&self.output_dir).unwrap();

                progress_bar.set_message(format!("Downloading: {}", url));

                let base_url = self.base_url.clone();
                let run_logger = Some(self.run_logger.clone());

                // Process the download directly instead of spawning a task
                match Self::download_and_process_url(
                    &client,
                    &file_manager,
                    &url,
                    depth,
                    &state,
                    &mirror_state,
                    &base_url,
                    priority,
                    resource_type,
                    &self.only_resources,
                    self.convert_to_webp,
                    &run_logger,
                )
                .await
                {
                    Ok(ProcessResult::Downloaded) => {
                        log::info!("✅ Downloaded: {}", url);
                    }
                    Ok(ProcessResult::SkippedAlreadyExists) => {
                        log::debug!("⏭️  Already exists: {}", url);
                    }
                    Ok(ProcessResult::SkippedFiltered) => {
                        log::debug!("🔍 Filtered out: {}", url);
                    }
                    Ok(ProcessResult::AlreadyVisited) => {
                        // This is normal, no log needed
                    }
                    Ok(ProcessResult::Error(msg)) => {
                        log::error!("❌ Error: {} - {}", url, msg);
                    }
                    Err(e) => {
                        log::error!("❌ Unexpected error downloading {}: {}", url, e);
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

        progress_bar.finish_with_message("✅ All downloads completed!");

        Ok(())
    }

    async fn download_and_process_html(
        client: &Client,
        file_manager: &FileManager,
        url: &str,
        depth: usize,
        state: &Arc<PersistentState>,
        mirror_state: &MirrorState,
        base_url: &str,
        only_resources: &Option<Vec<String>>,
        convert_to_webp: bool,
        run_logger: &Option<Arc<RunLogger>>,
    ) -> Result<ProcessResult> {
        log::debug!("📄 Processing HTML page: {}", url);

        // Note: Complex resumption logic removed - PersistentState will handle tracking
        // what has been downloaded. Simple file exists checks will be replaced in Phase 4.
        let url_mapper = UrlMapper::new(convert_to_webp)?;

        // Download the HTML page
        log::debug!("📥 Downloading HTML page: {}", url);
        let client_for_download = client.clone();
        let response = match client_for_download.get(url).send().await {
            Ok(resp) => resp,
            Err(e) => {
                log::error!("❌ Request failed: {}", e);
                mirror_state.track_download_error(
                    url.to_string(),
                    ResourceType::Link,
                    &format!("Request failed: {}", e),
                )?;
                // Track error in run logger
                if let Some(ref logger) = run_logger {
                    logger.track_error();
                }
                return Ok(ProcessResult::Error(format!("Request failed: {}", e)));
            }
        };

        if response.status() != StatusCode::OK {
            log::warn!("⚠️  HTTP {} for {}", response.status(), url);
            return Ok(ProcessResult::Error(format!(
                "HTTP {} for {}",
                response.status(),
                url
            )));
        }

        let content = match response.bytes().await {
            Ok(bytes) => bytes,
            Err(e) => {
                log::error!("❌ Failed to read response body: {}", e);
                return Ok(ProcessResult::Error(format!(
                    "Failed to read response body: {}",
                    e
                )));
            }
        };

        let html_content = String::from_utf8_lossy(&content);

        // Create a new HTML parser with the current page's base URL
        let page_html_parser = HtmlParser::new(url)?;
        let html_rewriter = HtmlRewriter::new();

        // Extract resources from HTML and process them
        let resources = page_html_parser.extract_resources(&html_content)?;
        let mut url_mappings = HashMap::new();

        // Helper function to check if a resource type should be processed
        let should_process_resource_type = |resource_type: &ResourceType| -> bool {
            if let Some(ref only_resources) = only_resources {
                let type_str = match resource_type {
                    ResourceType::Image => "images",
                    ResourceType::CSS => "css",
                    ResourceType::JavaScript => "js",
                    ResourceType::Link => "html",
                    ResourceType::Other => "other",
                };
                only_resources.iter().any(|r| r.to_lowercase() == type_str)
            } else {
                true
            }
        };

        // Categorize resources by priority
        let mut critical_resources = Vec::new();
        let mut high_resources = Vec::new();
        let mut normal_resources = Vec::new();

        for resource in &resources {
            let priority = match resource.resource_type {
                ResourceType::CSS | ResourceType::JavaScript => DownloadPriority::Critical,
                ResourceType::Link => DownloadPriority::High,
                ResourceType::Image | ResourceType::Other => DownloadPriority::Normal,
            };

            let should_download = match resource.resource_type {
                ResourceType::Image | ResourceType::CSS | ResourceType::JavaScript => {
                    should_process_resource_type(&resource.resource_type)
                }
                ResourceType::Link => {
                    resource.resolved.starts_with(base_url)
                        && should_process_resource_type(&resource.resource_type)
                }
                ResourceType::Other => {
                    resource.resolved.starts_with(base_url)
                        && should_process_resource_type(&resource.resource_type)
                }
            };

            if should_download {
                match priority {
                    DownloadPriority::Critical => critical_resources.push(resource.clone()),
                    DownloadPriority::High => high_resources.push(resource.clone()),
                    DownloadPriority::Normal => normal_resources.push(resource.clone()),
                }
            } else if !resource.resolved.starts_with(base_url) {
                match resource.resource_type {
                    ResourceType::Link => log::info!(
                        "⏭️  Skipping external page: {} (but will download its media)",
                        resource.original_url
                    ),
                    _ => {}
                }
            } else if !should_process_resource_type(&resource.resource_type) {
                let resource_type_str = match resource.resource_type {
                    ResourceType::Image => "Image",
                    ResourceType::CSS => "CSS",
                    ResourceType::JavaScript => "JavaScript",
                    ResourceType::Link => "Link",
                    ResourceType::Other => "Other",
                };
                log::info!(
                    "🔍 Skipping {} due to resource filter: {}",
                    resource_type_str,
                    resource.original_url
                );
            }
        }

        // Download critical resources (CSS/JS)
        for resource in &critical_resources {
            let resource_type_str = match resource.resource_type {
                ResourceType::CSS => "CSS",
                ResourceType::JavaScript => "JavaScript",
                _ => "Critical",
            };
            log::info!(
                "🔥 Processing CRITICAL {} resource: {}",
                resource_type_str,
                resource.original_url
            );

            if let Err(e) = Self::download_resource(
                client,
                file_manager,
                &url_mapper,
                &resource.resolved,
                &resource.resource_type,
                mirror_state,
                convert_to_webp,
                run_logger,
            )
            .await
            {
                log::error!(
                    "⚠️  Failed to download CRITICAL {} resource {}: {}",
                    resource_type_str,
                    resource.resolved,
                    e
                );
            } else {
                // Map original URL to local path for HTML rewriting
                if let Ok(local_path) =
                    url_mapper.url_to_local_path(&resource.resolved, &resource.resource_type)
                {
                    // Calculate relative path from current HTML to resource
                    if let Ok(current_html_path) =
                        url_mapper.url_to_local_path(url, &ResourceType::Link)
                    {
                        let relative_path = Self::calculate_relative_path(
                            &current_html_path.to_string_lossy(),
                            &local_path.to_string_lossy(),
                        );
                        url_mappings.insert(resource.original_url.clone(), relative_path);
                    }
                }
            }
        }

        // Process HTML links - add to mappings AND queue for crawling
        for resource in &high_resources {
            // First, add to url_mappings so links get rewritten
            if resource.resolved.starts_with(base_url) {
                if let Ok(local_path) =
                    url_mapper.url_to_local_path(&resource.resolved, &ResourceType::Link)
                {
                    if let Ok(current_html_path) =
                        url_mapper.url_to_local_path(url, &ResourceType::Link)
                    {
                        let relative_path = Self::calculate_relative_path(
                            &current_html_path.to_string_lossy(),
                            &local_path.to_string_lossy(),
                        );
                        url_mappings.insert(resource.original_url.clone(), relative_path);
                        log::info!(
                            "📝 Added link mapping: {} -> local path",
                            resource.original_url
                        );
                    }
                }
            }

            // Then queue for crawling as before
            // PersistentState enqueue handles deduplication internally
            state.enqueue(DownloadTask {
                url: resource.resolved.clone(),
                depth: depth + 1,
                priority: DownloadPriority::High,
                resource_type: Some(resource.resource_type.clone()),
            });
            log::info!(
                    "⚡ Queued HIGH priority HTML page: {}",
                    resource.resolved
                );
        }

        // Download normal priority resources (images, etc.)
        for resource in &normal_resources {
            let resource_type_str = match resource.resource_type {
                ResourceType::Image => "Image",
                ResourceType::Other => "Other",
                _ => "Normal",
            };
            log::info!(
                "📥 Processing NORMAL {} resource: {}",
                resource_type_str,
                resource.original_url
            );

            if let Err(e) = Self::download_resource(
                client,
                file_manager,
                &url_mapper,
                &resource.resolved,
                &resource.resource_type,
                mirror_state,
                convert_to_webp,
                run_logger,
            )
            .await
            {
                log::error!(
                    "⚠️  Failed to download NORMAL {} resource {}: {}",
                    resource_type_str,
                    resource.resolved,
                    e
                );
            } else {
                // Map original URL to local path for HTML rewriting
                if let Ok(local_path) =
                    url_mapper.url_to_local_path(&resource.resolved, &resource.resource_type)
                {
                    // Calculate relative path from current HTML to resource
                    if let Ok(current_html_path) =
                        url_mapper.url_to_local_path(url, &ResourceType::Link)
                    {
                        let relative_path = Self::calculate_relative_path(
                            &current_html_path.to_string_lossy(),
                            &local_path.to_string_lossy(),
                        );
                        url_mappings.insert(resource.original_url.clone(), relative_path);
                    }
                }
            }
        }

        // Apply URL replacements using HtmlRewriter
        let mut html_content_updated = html_rewriter.rewrite_urls(&html_content, &url_mappings);

        // Additional comprehensive WebP extension replacement for any remaining image references
        if convert_to_webp {
            log::info!("🔍 Performing comprehensive WebP extension replacement...");
            html_content_updated =
                Self::perform_comprehensive_webp_replacement(&html_content_updated);
        }

        // Save the updated HTML with local paths for resources
        let local_html_path = url_mapper.url_to_local_path(url, &ResourceType::Link)?;
        log::debug!("💾 Saving HTML to: {}", local_html_path.display());
        let saved_path =
            file_manager.save_file(&local_html_path, html_content_updated.as_bytes())?;
        log::debug!("✅ Saved HTML to: {}", saved_path.display());

        // Track successful HTML download
        let size_bytes = html_content_updated.len() as u64;
        mirror_state.track_download_success(
            url.to_string(),
            local_html_path.to_string_lossy().to_string(),
            ResourceType::Link,
            size_bytes,
        )?;

        // Track downloaded in run logger
        if let Some(ref logger) = run_logger {
            logger.track_downloaded(size_bytes);
        }

        Ok(ProcessResult::Downloaded)
    }

    async fn download_and_process_css(
        client: &Client,
        file_manager: &FileManager,
        url: &str,
        mirror_state: &MirrorState,
        convert_to_webp: bool,
        run_logger: &Option<Arc<RunLogger>>,
    ) -> Result<ProcessResult> {
        log::debug!("🎨 Processing CSS file: {}", url);

        // Check if CSS already exists on disk
        let url_mapper = UrlMapper::new(convert_to_webp)?;
        let local_path = url_mapper.url_to_local_path(url, &ResourceType::CSS)?;

        if file_manager.file_exists(&local_path) {
            // File already exists, should already be in the manifest from previous run
            log::debug!(
                "⏭️  Skipping CSS (already exists on disk at {})",
                local_path.display()
            );
            // Track as skipped in run logger
            if let Some(ref logger) = run_logger {
                logger.track_skipped();
            }
            return Ok(ProcessResult::SkippedAlreadyExists);
        }

        // Download the URL
        let response = match client.get(url).send().await {
            Ok(resp) => resp,
            Err(e) => {
                log::error!("❌ Request failed: {}", e);
                mirror_state.track_download_error(
                    url.to_string(),
                    ResourceType::CSS,
                    &format!("Request failed: {}", e),
                )?;
                // Track error in run logger
                if let Some(ref logger) = run_logger {
                    logger.track_error();
                }
                return Ok(ProcessResult::Error(format!("Request failed: {}", e)));
            }
        };

        if response.status() != StatusCode::OK {
            log::warn!("⚠️  HTTP {} for {}", response.status(), url);
            return Ok(ProcessResult::Error(format!(
                "HTTP {} for {}",
                response.status(),
                url
            )));
        }

        let content = match response.bytes().await {
            Ok(bytes) => bytes,
            Err(e) => {
                log::error!("❌ Failed to read response body: {}", e);
                return Ok(ProcessResult::Error(format!(
                    "Failed to read response body: {}",
                    e
                )));
            }
        };

        // Process CSS files to extract background images
        let css_content = String::from_utf8_lossy(&content);
        let page_html_parser = HtmlParser::new(url)?;

        // Extract background images from CSS
        let mut background_resources = Vec::new();
        page_html_parser
            .extract_background_images_from_css(&css_content, &mut background_resources);

        // Download background images with normal priority (after CSS/JS)
        for resource in &background_resources {
            log::info!("📥 Processing background image: {}", resource.original_url);
            let url_mapper = UrlMapper::new(convert_to_webp)?;
            if let Err(e) = Self::download_resource(
                client,
                file_manager,
                &url_mapper,
                &resource.resolved,
                &ResourceType::Image,
                mirror_state,
                convert_to_webp,
                run_logger,
            )
            .await
            {
                log::error!(
                    "⚠️  Failed to download background image {}: {}",
                    resource.resolved,
                    e
                );
            }
        }

        // Save the CSS file
        let url_mapper = UrlMapper::new(convert_to_webp)?;
        let local_path = url_mapper.url_to_local_path(url, &ResourceType::CSS)?;
        log::debug!("💾 Saving CSS to: {}", local_path.display());
        let saved_path = file_manager.save_file(&local_path, &content)?;
        log::debug!("✅ Saved CSS to: {:?}", saved_path);

        // Track successful CSS download
        let size_bytes = content.len() as u64;
        mirror_state.track_download_success(
            url.to_string(),
            local_path.to_string_lossy().to_string(),
            ResourceType::CSS,
            size_bytes,
        )?;

        // Track downloaded in run logger
        if let Some(ref logger) = run_logger {
            logger.track_downloaded(size_bytes);
        }

        Ok(ProcessResult::Downloaded)
    }

    async fn download_and_process_url(
        client: &Client,
        file_manager: &FileManager,
        url: &str,
        depth: usize,
        state: &Arc<PersistentState>,
        mirror_state: &MirrorState,
        base_url: &str,
        priority: DownloadPriority,
        resource_type: Option<ResourceType>,
        only_resources: &Option<Vec<String>>,
        convert_to_webp: bool,
        run_logger: &Option<Arc<RunLogger>>,
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
                    mirror_state,
                    base_url,
                    only_resources,
                    convert_to_webp,
                    run_logger,
                )
                .await
            }
            Some(ResourceType::CSS) => {
                // CSS file
                Self::download_and_process_css(
                    client,
                    file_manager,
                    url,
                    mirror_state,
                    convert_to_webp,
                    run_logger,
                )
                .await
            }
            Some(resource_type) => {
                // Other resources (JS, images, etc.) - just download without processing
                log::debug!("📦 Processing resource: {}", url);
                let url_mapper = UrlMapper::new(convert_to_webp)?;
                Self::download_resource(
                    client,
                    file_manager,
                    &url_mapper,
                    url,
                    &resource_type,
                    mirror_state,
                    convert_to_webp,
                    run_logger,
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
        mirror_state: &MirrorState,
        convert_to_webp: bool,
        run_logger: &Option<Arc<RunLogger>>,
    ) -> Result<ProcessResult> {
        // Check if already downloaded using cache
        if let Some(cached_path) = mirror_state.get_path(url) {
            log::debug!(
                "⏭️  Skipping {} (already downloaded to {})",
                url,
                cached_path
            );
            // Track as skipped in run logger
            return Ok(ProcessResult::SkippedAlreadyExists);
        }

        // Check if file exists on disk
        // First convert URL to the local path where it would be saved
        let local_path = url_mapper.url_to_local_path(url, resource_type)?;
        if file_manager.file_exists(&local_path) {
            // File already exists, should already be in the manifest from previous run
            log::debug!(
                "⏭️  Skipping {} (already exists on disk at {})",
                url,
                local_path.display()
            );
            // Track as skipped in run logger
            if let Some(ref logger) = run_logger {
                logger.track_skipped();
            }
            return Ok(ProcessResult::SkippedAlreadyExists);
        }

        // Determine resource type string for better logging
        let resource_type_str = match resource_type {
            ResourceType::CSS => "CSS",
            ResourceType::JavaScript => "JavaScript",
            ResourceType::Image => "Image",
            ResourceType::Link => "HTML",
            ResourceType::Other => {
                if url.ends_with(".woff")
                    || url.ends_with(".woff2")
                    || url.ends_with(".ttf")
                    || url.ends_with(".eot")
                {
                    "Font"
                } else {
                    "Resource"
                }
            }
        };

        log::debug!("📥 Downloading {}: {}", resource_type_str, url);

        let response = match client.get(url).send().await {
            Ok(resp) => resp,
            Err(e) => {
                log::error!(
                    "❌ Failed to send request for {} {}: {}",
                    resource_type_str,
                    url,
                    e
                );
                mirror_state.track_download_error(
                    url.to_string(),
                    resource_type.clone(),
                    &format!("Request failed: {}", e),
                )?;
                // Track error in run logger
                if let Some(ref logger) = run_logger {
                    logger.track_error();
                }
                return Ok(ProcessResult::Error(format!("Request failed: {}", e)));
            }
        };

        if response.status() != StatusCode::OK {
            log::warn!(
                "⚠️  HTTP {} for {} {}",
                response.status(),
                resource_type_str,
                url
            );
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
                log::error!(
                    "❌ Failed to read {} body {}: {}",
                    resource_type_str,
                    url,
                    e
                );
                return Ok(ProcessResult::Error(format!(
                    "Failed to read response body: {}",
                    e
                )));
            }
        };

        // Convert images to WebP if they're JPEG or PNG and the flag is enabled
        let final_content = if convert_to_webp
            && matches!(resource_type, ResourceType::Image)
            && (url.ends_with(".jpg")
                || url.ends_with(".jpeg")
                || url.ends_with(".png")
                || url.ends_with(".JPG")
                || url.ends_with(".JPEG")
                || url.ends_with(".PNG"))
        {
            // Convert to WebP
            Self::convert_to_webp_static(&content, url)?
        } else {
            // Keep original content
            content.to_vec()
        };

        // Note: The local_path already has the correct extension (.webp if convert_to_webp is true)
        // because UrlMapper handles the conversion logic
        let saved_path = match file_manager.save_file(&local_path, &final_content) {
            Ok(path) => path,
            Err(e) => {
                log::error!("❌ Failed to save {} {}: {}", resource_type_str, url, e);
                return Ok(ProcessResult::Error(format!("Failed to save file: {}", e)));
            }
        };

        // Track successful download
        let size_bytes = final_content.len() as u64;
        mirror_state.track_download_success(
            url.to_string(),
            local_path.to_string_lossy().to_string(),
            resource_type.clone(),
            size_bytes,
        )?;

        // Track downloaded in run logger
        if let Some(ref logger) = run_logger {
            logger.track_downloaded(size_bytes);
        }

        log::info!(
            "✅ Downloaded {} to: {}",
            resource_type_str,
            saved_path.display()
        );

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
            10,
            false,
            false,
            None,
            false,
        )
        .unwrap();

        assert_eq!(mirror.base_url.as_str(), "https://example.com");
        assert_eq!(mirror.max_depth, 3);
        assert_eq!(mirror.max_concurrent, 10);
        assert_eq!(mirror.ignore_robots, false);
        assert_eq!(mirror.download_external, false);
        assert_eq!(mirror.convert_to_webp, false);
    }

    #[test]
    fn test_website_mirror_new_with_options() {
        let temp_dir = tempdir().unwrap();
        let mirror = WebsiteMirror::new(
            "https://example.com",
            temp_dir.path(),
            5,
            20,
            true,
            true,
            Some(vec!["images".to_string()]),
            true,
        )
        .unwrap();

        assert_eq!(mirror.max_depth, 5);
        assert_eq!(mirror.max_concurrent, 20);
        assert_eq!(mirror.ignore_robots, true);
        assert_eq!(mirror.download_external, true);
        assert_eq!(mirror.convert_to_webp, true);
        assert_eq!(mirror.only_resources, Some(vec!["images".to_string()]));
    }

    #[test]
    fn test_should_process_resource_type() {
        let temp_dir = tempdir().unwrap();
        let mirror = WebsiteMirror::new(
            "https://example.com",
            temp_dir.path(),
            3,
            10,
            false,
            false,
            None,
            false,
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
            10,
            false,
            false,
            Some(vec!["images".to_string(), "css".to_string()]),
            false,
        )
        .unwrap();

        assert!(mirror.should_process_resource_type(&ResourceType::CSS));
        assert!(!mirror.should_process_resource_type(&ResourceType::JavaScript));
        assert!(mirror.should_process_resource_type(&ResourceType::Image));
        assert!(!mirror.should_process_resource_type(&ResourceType::Link));
    }

    #[test]
    fn test_convert_to_webp_success() {
        // Create a simple test image (1x1 pixel PNG)
        let png_data = vec![
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00,
            0x00, 0x90, 0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08,
            0x99, 0x01, 0x01, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0x00, 0x00, 0x00, 0x02, 0x00, 0x01,
            0xE2, 0x21, 0xBC, 0x33, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42,
            0x60, 0x82,
        ];

        let result = WebsiteMirror::convert_to_webp_static(&png_data, "test.png");
        assert!(result.is_ok());

        let webp_data = result.unwrap();
        assert!(webp_data.len() > 0);
        assert!(webp_data.len() != png_data.len()); // Should be different size
    }

    #[test]
    fn test_convert_to_webp_invalid_image() {
        let invalid_data = b"not an image";
        let result = WebsiteMirror::convert_to_webp_static(invalid_data, "test.txt");
        assert!(result.is_ok()); // Should return original data on failure

        let returned_data = result.unwrap();
        assert_eq!(returned_data, invalid_data);
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
            10,
            false,
            false,
            None,
            false,
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
            10,
            false,
            false,
            None,
            false,
        )
        .unwrap();

        let cloned = mirror.clone();
        assert_eq!(mirror.base_url, cloned.base_url);
        assert_eq!(mirror.max_depth, cloned.max_depth);
        assert_eq!(mirror.max_concurrent, cloned.max_concurrent);
        assert_eq!(mirror.ignore_robots, cloned.ignore_robots);
        assert_eq!(mirror.download_external, cloned.download_external);
        assert_eq!(mirror.convert_to_webp, cloned.convert_to_webp);
    }

    // Note: WebsiteMirror doesn't implement PartialEq, Eq, or Hash due to complex fields
    // These tests are removed as they're not essential for functionality

}
