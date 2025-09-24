use anyhow::Result;
use chrono::Local;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::html_parser::ResourceType;

/// Thread-safe state tracker for the mirror operation
/// Persists to disk as JSON for crawl resumption and statistics
#[derive(Clone, Debug)]
pub struct MirrorState {
    /// Full state data including stats and resources
    data: Arc<Mutex<StateData>>,
    /// Path to the JSON file
    cache_file: PathBuf,
}

/// The JSON structure we save to disk
#[derive(Serialize, Deserialize, Debug)]
struct StateData {
    stats: Statistics,
    resources: HashMap<String, ResourceInfo>,
}

impl Default for StateData {
    fn default() -> Self {
        Self {
            stats: Statistics::default(),
            resources: HashMap::new(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Statistics {
    pub urls_discovered: usize,
    pub downloads: DownloadStats,
    pub total_bytes: u64,
    pub last_updated: String,
}

impl Default for Statistics {
    fn default() -> Self {
        Self {
            urls_discovered: 0,
            downloads: DownloadStats::default(),
            total_bytes: 0,
            last_updated: Local::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct DownloadStats {
    pub html: ResourceStats,
    pub css: ResourceStats,
    pub js: ResourceStats,
    pub images: ResourceStats,
    pub other: ResourceStats,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct ResourceStats {
    pub success: usize,
    pub error: usize,
    pub bytes: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct ResourceInfo {
    path: String,
    resource_type: String,
    size: u64,
    downloaded_at: String,
    status: String,
    error_message: Option<String>,
}

impl MirrorState {
    /// Create or load mirror state from the output directory
    pub fn new(output_dir: &Path) -> Result<Self> {
        let cache_file = output_dir.join(".mirror/state.json");

        // Ensure the .mirror directory exists
        if let Some(parent) = cache_file.parent() {
            fs::create_dir_all(parent)?;
        }

        // Load existing state or start empty
        let data = if cache_file.exists() {
            let content = fs::read_to_string(&cache_file)?;
            let state_data: StateData = serde_json::from_str(&content).unwrap_or_default();
            log::info!("📂 Loaded state: {} resources, {} discovered URLs",
                state_data.resources.len(),
                state_data.stats.urls_discovered);
            state_data
        } else {
            StateData::default()
        };

        Ok(Self {
            data: Arc::new(Mutex::new(data)),
            cache_file,
        })
    }

    /// Track that a URL was discovered (for crawling)
    pub fn track_url_discovered(&self) -> Result<()> {
        {
            let mut data = self.data.lock().unwrap();
            data.stats.urls_discovered += 1;
            data.stats.last_updated = Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
        }
        self.save()?;
        Ok(())
    }

    /// Track a successful download
    pub fn track_download_success(
        &self,
        url: String,
        local_path: String,
        resource_type: ResourceType,
        size_bytes: u64,
    ) -> Result<()> {
        {
            let mut data = self.data.lock().unwrap();

            // Update resource info
            let type_str = Self::resource_type_to_string(&resource_type);
            data.resources.insert(url, ResourceInfo {
                path: local_path,
                resource_type: type_str.clone(),
                size: size_bytes,
                downloaded_at: Local::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
                status: "success".to_string(),
                error_message: None,
            });

            // Update statistics
            let stats = Self::get_resource_stats(&mut data.stats.downloads, &resource_type);
            stats.success += 1;
            stats.bytes += size_bytes;
            data.stats.total_bytes += size_bytes;
            data.stats.last_updated = Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
        }

        self.save()?;
        Ok(())
    }

    /// Track a download error
    pub fn track_download_error(
        &self,
        url: String,
        resource_type: ResourceType,
        error_msg: &str,
    ) -> Result<()> {
        {
            let mut data = self.data.lock().unwrap();

            // Update resource info
            let type_str = Self::resource_type_to_string(&resource_type);
            data.resources.insert(url, ResourceInfo {
                path: String::new(),
                resource_type: type_str.clone(),
                size: 0,
                downloaded_at: Local::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
                status: "error".to_string(),
                error_message: Some(error_msg.to_string()),
            });

            // Update statistics
            let stats = Self::get_resource_stats(&mut data.stats.downloads, &resource_type);
            stats.error += 1;
            data.stats.last_updated = Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
        }

        self.save()?;
        Ok(())
    }

    /// Get local path for a URL (for checking if already downloaded)
    pub fn get_path(&self, url: &str) -> Option<String> {
        let data = self.data.lock().unwrap();
        data.resources
            .get(url)
            .filter(|info| info.status == "success")
            .map(|info| info.path.clone())
    }

    /// Get URL for a local path (reverse lookup - linear search)
    pub fn get_url(&self, local_path: &str) -> Option<String> {
        let data = self.data.lock().unwrap();
        data.resources
            .iter()
            .find(|(_, info)| info.path == local_path && info.status == "success")
            .map(|(url, _)| url.clone())
    }

    /// Check if URL has been successfully downloaded
    pub fn contains_url(&self, url: &str) -> bool {
        let data = self.data.lock().unwrap();
        data.resources
            .get(url)
            .map(|info| info.status == "success")
            .unwrap_or(false)
    }

    /// Check if we have a mapping for this local path
    pub fn contains_path(&self, path: &str) -> bool {
        let data = self.data.lock().unwrap();
        data.resources
            .values()
            .any(|info| info.path == path && info.status == "success")
    }

    /// Get total number of resources (successful downloads only)
    pub fn len(&self) -> usize {
        let data = self.data.lock().unwrap();
        data.resources
            .values()
            .filter(|info| info.status == "success")
            .count()
    }

    /// Get full statistics
    pub fn get_statistics(&self) -> Statistics {
        let data = self.data.lock().unwrap();
        data.stats.clone()
    }

    /// Save current state to disk
    fn save(&self) -> Result<()> {
        let data = self.data.lock().unwrap();
        let json = serde_json::to_string_pretty(&*data)?;
        fs::write(&self.cache_file, json)?;
        Ok(())
    }

    /// Helper to convert ResourceType to string
    fn resource_type_to_string(resource_type: &ResourceType) -> String {
        match resource_type {
            ResourceType::Link => "html".to_string(),
            ResourceType::CSS => "css".to_string(),
            ResourceType::JavaScript => "js".to_string(),
            ResourceType::Image => "images".to_string(),
            ResourceType::Other => "other".to_string(),
        }
    }

    /// Helper to get mutable reference to the right resource stats
    fn get_resource_stats<'a>(
        downloads: &'a mut DownloadStats,
        resource_type: &ResourceType,
    ) -> &'a mut ResourceStats {
        match resource_type {
            ResourceType::Link => &mut downloads.html,
            ResourceType::CSS => &mut downloads.css,
            ResourceType::JavaScript => &mut downloads.js,
            ResourceType::Image => &mut downloads.images,
            ResourceType::Other => &mut downloads.other,
        }
    }

    /// Get all successful URL to path mappings (for debugging/compatibility)
    pub fn all_mappings(&self) -> HashMap<String, String> {
        let data = self.data.lock().unwrap();
        data.resources
            .iter()
            .filter(|(_, info)| info.status == "success")
            .map(|(url, info)| (url.clone(), info.path.clone()))
            .collect()
    }

    /// Temporary compatibility method - will be removed
    pub fn insert(&self, url: String, local_path: String) -> Result<()> {
        // For now, just call track_download_success with ResourceType::Other and size 0
        self.track_download_success(url, local_path, ResourceType::Other, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_track_downloads() {
        let temp_dir = tempdir().unwrap();
        let state = MirrorState::new(temp_dir.path()).unwrap();

        // Track some discoveries
        state.track_url_discovered().unwrap();
        state.track_url_discovered().unwrap();

        // Track successful downloads
        state.track_download_success(
            "https://example.com/".to_string(),
            "example.com/index.html".to_string(),
            ResourceType::Link,
            1024,
        ).unwrap();

        state.track_download_success(
            "https://example.com/style.css".to_string(),
            "example.com/style.css".to_string(),
            ResourceType::CSS,
            2048,
        ).unwrap();

        // Track an error
        state.track_download_error(
            "https://example.com/missing.js".to_string(),
            ResourceType::JavaScript,
            "404 Not Found",
        ).unwrap();

        // Check statistics
        let stats = state.get_statistics();
        assert_eq!(stats.urls_discovered, 2);
        assert_eq!(stats.downloads.html.success, 1);
        assert_eq!(stats.downloads.css.success, 1);
        assert_eq!(stats.downloads.js.error, 1);
        assert_eq!(stats.total_bytes, 3072);
    }

    #[test]
    fn test_persistence() {
        let temp_dir = tempdir().unwrap();

        // Create and populate state
        {
            let state = MirrorState::new(temp_dir.path()).unwrap();
            state.track_download_success(
                "https://example.com/".to_string(),
                "example.com/index.html".to_string(),
                ResourceType::Link,
                1024,
            ).unwrap();
            state.track_url_discovered().unwrap();
        }

        // Load in new instance
        {
            let state = MirrorState::new(temp_dir.path()).unwrap();
            assert_eq!(state.len(), 1);
            assert!(state.contains_url("https://example.com/"));
            assert!(state.contains_path("example.com/index.html"));

            let stats = state.get_statistics();
            assert_eq!(stats.urls_discovered, 1);
            assert_eq!(stats.downloads.html.success, 1);
            assert_eq!(stats.total_bytes, 1024);
        }
    }

    #[test]
    fn test_error_tracking() {
        let temp_dir = tempdir().unwrap();
        let state = MirrorState::new(temp_dir.path()).unwrap();

        // Track an error
        state.track_download_error(
            "https://example.com/404.html".to_string(),
            ResourceType::Link,
            "404 Not Found",
        ).unwrap();

        // Should not be in successful lookups
        assert_eq!(state.get_path("https://example.com/404.html"), None);
        assert!(!state.contains_url("https://example.com/404.html"));

        // But should be in resources with error status
        let stats = state.get_statistics();
        assert_eq!(stats.downloads.html.error, 1);
    }
}