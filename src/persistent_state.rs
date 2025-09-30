use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::downloader::DownloadPriority;
use crate::html_parser::ResourceType;
use crate::url_mapper::UrlMapper;

/// Statistics computed from the state data
#[derive(Debug, Clone, Default)]
pub struct Statistics {
    pub urls_discovered: usize,
    pub downloads: HashMap<String, usize>,     // resource_type -> success count
    pub errors: HashMap<String, usize>,        // resource_type -> error count
    pub bytes_per_type: HashMap<String, u64>,  // resource_type -> total bytes
    pub total_bytes: u64,
}


/// Core state data that gets persisted to disk
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct StateData {
    pub queue: VecDeque<DownloadTask>,
    pub processing: HashSet<String>,
    pub downloaded: HashMap<String, ResourceInfo>,
    pub errored: HashMap<String, ErrorInfo>,
}

/// Information about a successfully downloaded resource
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ResourceInfo {
    pub local_path: String,
    pub resource_type: String,
    pub size_bytes: u64,
    pub downloaded_at: String,
}

/// Information about a failed download
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ErrorInfo {
    pub error_message: String,
    pub attempted_at: String,
    pub resource_type: String,
}

/// A task in the download queue
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DownloadTask {
    pub url: String,
    pub depth: usize,
    pub priority: DownloadPriority,
    pub resource_type: Option<ResourceType>,
}

/// Main persistent state manager
#[derive(Debug)]
pub struct PersistentState {
    data: Arc<Mutex<StateData>>,
    cache_file: PathBuf,
}

impl PersistentState {
    /// Creates or loads persistent state from the given output directory
    pub fn new(output_dir: &Path) -> Result<Self> {
        let mirror_dir = output_dir.join(".mirror");
        fs::create_dir_all(&mirror_dir)?;
        let cache_file = mirror_dir.join("state.json");

        let data = if cache_file.exists() {
            let contents = fs::read_to_string(&cache_file)?;
            let mut loaded: StateData = serde_json::from_str(&contents)?;

            // CRASH RECOVERY: Move all processing URLs back to queue
            // These were being downloaded when the program was interrupted
            let processing_count = loaded.processing.len();
            for url in loaded.processing.drain() {
                // Re-queue at high priority to resume quickly
                // Note: We lose the original depth/resource_type, but that's acceptable
                loaded.queue.push_front(DownloadTask {
                    url: url.clone(),
                    depth: 0,  // Conservative depth to prevent deep recursion
                    priority: DownloadPriority::High,
                    resource_type: None,
                });
            }

            if processing_count > 0 {
                log::info!(
                    "Recovered {} in-progress URLs and added them back to the queue",
                    processing_count
                );
            }

            log::info!(
                "Resumed crawl with {} queued URLs, {} downloaded, {} errors",
                loaded.queue.len(),
                loaded.downloaded.len(),
                loaded.errored.len()
            );
            loaded
        } else {
            log::info!("Starting new crawl - no existing state found");
            StateData::default()
        };

        Ok(Self {
            data: Arc::new(Mutex::new(data)),
            cache_file,
        })
    }

    /// Saves the current state to disk
    pub fn save(&self) -> Result<()> {
        let data = self.data.lock().unwrap();
        let json = serde_json::to_string_pretty(&*data)?;
        fs::write(&self.cache_file, json)?;
        Ok(())
    }

    /// Loads state from disk (used internally during new())
    pub fn load_from_disk(path: &Path) -> Result<Self> {
        Self::new(path.parent().unwrap())
    }

    /// Atomically dequeues a task and moves it to processing
    pub fn dequeue(&self) -> Option<DownloadTask> {
        let mut data = self.data.lock().unwrap();

        // Pop from front (respecting priority order maintained by enqueue)
        let task = data.queue.pop_front()?;

        // ATOMIC: Move from queue to processing (normalize URL for consistency)
        let normalized_url = UrlMapper::normalize_root_url(&task.url);
        data.processing.insert(normalized_url);

        drop(data);  // Release lock before I/O
        self.save().ok();  // Best effort save
        Some(task)
    }

    /// Adds a task to the queue if it hasn't been visited
    pub fn enqueue(&self, task: DownloadTask) {
        let mut data = self.data.lock().unwrap();

        // Normalize URL to prevent duplicates (e.g., example.com vs example.com/)
        let normalized_url = UrlMapper::normalize_root_url(&task.url);

        // Check if already visited (downloaded or errored) - no separate visited set!
        if data.downloaded.contains_key(&normalized_url) || data.errored.contains_key(&normalized_url) {
            return;
        }

        // Also check if already in queue or processing
        if data.processing.contains(&normalized_url) {
            return;
        }
        if data.queue.iter().any(|t| UrlMapper::normalize_root_url(&t.url) == normalized_url) {
            return;
        }

        // Insert based on priority (this replaces BinaryHeap's automatic ordering)
        let insert_pos = match task.priority {
            DownloadPriority::Critical => 0,  // Front of queue
            DownloadPriority::High => {
                // After all critical items
                data.queue
                    .iter()
                    .position(|t| !matches!(t.priority, DownloadPriority::Critical))
                    .unwrap_or(data.queue.len())
            }
            DownloadPriority::Normal => data.queue.len(),  // Back of queue
        };

        data.queue.insert(insert_pos, task);
        drop(data);
        self.save().ok();
    }

    /// Marks a URL as successfully downloaded
    pub fn mark_completed(
        &self,
        url: String,
        local_path: String,
        resource_type: ResourceType,
        size: u64,
    ) {
        let mut data = self.data.lock().unwrap();

        // Normalize URL for consistency
        let normalized_url = UrlMapper::normalize_root_url(&url);

        // ATOMIC: Move from processing to downloaded
        data.processing.remove(&normalized_url);
        data.downloaded.insert(
            normalized_url,
            ResourceInfo {
                local_path,
                resource_type: format!("{:?}", resource_type),
                size_bytes: size,
                downloaded_at: chrono::Local::now().to_rfc3339(),
            },
        );
        drop(data);
        self.save().ok();
    }

    /// Marks a URL as errored
    pub fn mark_errored(&self, url: String, error: String, resource_type: ResourceType) {
        let mut data = self.data.lock().unwrap();

        // Normalize URL for consistency
        let normalized_url = UrlMapper::normalize_root_url(&url);

        // ATOMIC: Move from processing to errored
        data.processing.remove(&normalized_url);
        data.errored.insert(
            normalized_url,
            ErrorInfo {
                error_message: error,
                attempted_at: chrono::Local::now().to_rfc3339(),
                resource_type: format!("{:?}", resource_type),
            },
        );
        drop(data);
        self.save().ok();
    }

    /// Checks if a URL has been visited (downloaded or errored)
    pub fn is_visited(&self, url: &str) -> bool {
        let data = self.data.lock().unwrap();
        let normalized_url = UrlMapper::normalize_root_url(url);
        // No separate visited set - just check downloaded + errored
        data.downloaded.contains_key(&normalized_url) || data.errored.contains_key(&normalized_url)
    }

    /// Moves all processing URLs back to the queue (for crash recovery)
    pub fn move_processing_to_queue(&self) {
        let mut data = self.data.lock().unwrap();

        let processing_urls: Vec<String> = data.processing.drain().collect();
        for url in processing_urls {
            data.queue.push_front(DownloadTask {
                url,
                depth: 0,
                priority: DownloadPriority::High,
                resource_type: None,
            });
        }

        drop(data);
        self.save().ok();
    }

    /// Gets the current queue size
    pub fn queue_size(&self) -> usize {
        let data = self.data.lock().unwrap();
        data.queue.len()
    }

    /// Gets the number of downloaded URLs
    pub fn downloaded_count(&self) -> usize {
        let data = self.data.lock().unwrap();
        data.downloaded.len()
    }

    /// Gets the number of errored URLs
    pub fn errored_count(&self) -> usize {
        let data = self.data.lock().unwrap();
        data.errored.len()
    }

    /// Gets the number of URLs currently being processed
    pub fn processing_count(&self) -> usize {
        let data = self.data.lock().unwrap();
        data.processing.len()
    }

    /// Computes statistics from the current state
    pub fn get_statistics(&self) -> Statistics {
        let data = self.data.lock().unwrap();

        let mut downloads: HashMap<String, usize> = HashMap::new();
        let mut errors: HashMap<String, usize> = HashMap::new();
        let mut bytes_per_type: HashMap<String, u64> = HashMap::new();
        let mut total_bytes = 0u64;

        // Count successful downloads and bytes
        for info in data.downloaded.values() {
            let resource_type = info.resource_type.to_lowercase();
            *downloads.entry(resource_type.clone()).or_insert(0) += 1;
            *bytes_per_type.entry(resource_type).or_insert(0) += info.size_bytes;
            total_bytes += info.size_bytes;
        }

        // Count errors by resource type
        for error_info in data.errored.values() {
            let resource_type = error_info.resource_type.to_lowercase();
            *errors.entry(resource_type).or_insert(0) += 1;
        }

        Statistics {
            urls_discovered: data.downloaded.len() + data.errored.len() + data.queue.len() + data.processing.len(),
            downloads,
            errors,
            bytes_per_type,
            total_bytes,
        }
    }

    /// Get error messages for summary reporting
    pub fn get_error_messages(&self) -> Vec<String> {
        let data = self.data.lock().unwrap();
        data.errored.values().map(|err| err.error_message.clone()).collect()
    }


    /// Checks if a URL is already in the queue
    pub fn is_queued(&self, url: &str) -> bool {
        let data = self.data.lock().unwrap();
        let normalized_url = UrlMapper::normalize_root_url(url);
        data.queue.iter().any(|t| UrlMapper::normalize_root_url(&t.url) == normalized_url)
    }

    /// Checks if a URL is currently being processed
    pub fn is_processing(&self, url: &str) -> bool {
        let data = self.data.lock().unwrap();
        let normalized_url = UrlMapper::normalize_root_url(url);
        data.processing.contains(&normalized_url)
    }

    /// Gets the local path for a downloaded URL, if it exists
    pub fn get_local_path(&self, url: &str) -> Option<String> {
        let data = self.data.lock().unwrap();
        let normalized_url = UrlMapper::normalize_root_url(url);
        data.downloaded.get(&normalized_url).map(|info| info.local_path.clone())
    }

    /// Gets total count of URLs that have been seen/discovered
    pub fn total_seen_urls(&self) -> usize {
        let data = self.data.lock().unwrap();
        data.downloaded.len() + data.errored.len() + data.queue.len() + data.processing.len()
    }

    /// Clears the state (for testing or reset)
    pub fn clear(&self) {
        let mut data = self.data.lock().unwrap();
        *data = StateData::default();
        drop(data);
        self.save().ok();
    }
}