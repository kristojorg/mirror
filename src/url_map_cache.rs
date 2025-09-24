use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Thread-safe cache that maps URLs to local file paths
/// Persists to disk as JSON for crawl resumption
#[derive(Clone, Debug)]
pub struct UrlMapCache {
    /// URL -> local path mapping
    mappings: Arc<Mutex<HashMap<String, String>>>,
    /// Path to the JSON file
    cache_file: PathBuf,
}

/// The JSON structure we save to disk
#[derive(Serialize, Deserialize, Debug, Default)]
struct CacheData {
    mappings: HashMap<String, String>,
}

impl UrlMapCache {
    /// Create or load a URL map cache from the output directory
    pub fn new(output_dir: &Path) -> Result<Self> {
        let cache_file = output_dir.join(".mirror/url_to_filename.json");

        // Load existing mappings or start empty
        let mappings = if cache_file.exists() {
            let content = fs::read_to_string(&cache_file)?;
            let data: CacheData = serde_json::from_str(&content).unwrap_or_default();
            println!("📂 Loaded {} cached URL mappings", data.mappings.len());
            data.mappings
        } else {
            HashMap::new()
        };

        Ok(Self {
            mappings: Arc::new(Mutex::new(mappings)),
            cache_file,
        })
    }

    /// Add a URL to local path mapping and save to disk
    pub fn insert(&self, url: String, local_path: String) -> Result<()> {
        {
            let mut cache = self.mappings.lock().unwrap();
            cache.insert(url, local_path);
        }

        // Save to disk
        self.save()?;
        Ok(())
    }

    /// Get local path for a URL
    pub fn get_path(&self, url: &str) -> Option<String> {
        let cache = self.mappings.lock().unwrap();
        cache.get(url).cloned()
    }

    /// Get URL for a local path (reverse lookup - linear search)
    pub fn get_url(&self, local_path: &str) -> Option<String> {
        let cache = self.mappings.lock().unwrap();
        cache
            .iter()
            .find(|(_, path)| path.as_str() == local_path)
            .map(|(url, _)| url.clone())
    }

    /// Check if URL has been downloaded
    pub fn contains_url(&self, url: &str) -> bool {
        let cache = self.mappings.lock().unwrap();
        cache.contains_key(url)
    }

    /// Check if we have a mapping for this local path
    pub fn contains_path(&self, path: &str) -> bool {
        let cache = self.mappings.lock().unwrap();
        cache.values().any(|p| p == path)
    }

    /// Get total number of mappings
    pub fn len(&self) -> usize {
        let cache = self.mappings.lock().unwrap();
        cache.len()
    }

    /// Save current mappings to disk
    fn save(&self) -> Result<()> {
        let cache = self.mappings.lock().unwrap();

        let data = CacheData {
            mappings: cache.clone(),
        };

        let json = serde_json::to_string_pretty(&data)?;
        fs::write(&self.cache_file, json)?;

        Ok(())
    }

    /// Get all URL to path mappings (for debugging)
    pub fn all_mappings(&self) -> HashMap<String, String> {
        let cache = self.mappings.lock().unwrap();
        cache.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_bidirectional_lookup() {
        let temp_dir = tempdir().unwrap();
        let cache = UrlMapCache::new(temp_dir.path()).unwrap();

        // Insert a mapping
        cache
            .insert(
                "https://example.com/page".to_string(),
                "example.com/page.html".to_string(),
            )
            .unwrap();

        // Test forward lookup (O(1))
        assert_eq!(
            cache.get_path("https://example.com/page"),
            Some("example.com/page.html".to_string())
        );

        // Test reverse lookup (O(n))
        assert_eq!(
            cache.get_url("example.com/page.html"),
            Some("https://example.com/page".to_string())
        );
    }

    #[test]
    fn test_persistence() {
        let temp_dir = tempdir().unwrap();

        // Create and populate cache
        {
            let cache = UrlMapCache::new(temp_dir.path()).unwrap();
            cache
                .insert(
                    "https://example.com/".to_string(),
                    "example.com/index.html".to_string(),
                )
                .unwrap();
        }

        // Load in new instance
        {
            let cache = UrlMapCache::new(temp_dir.path()).unwrap();
            assert_eq!(cache.len(), 1);
            assert!(cache.contains_url("https://example.com/"));
            assert!(cache.contains_path("example.com/index.html"));
        }
    }

    #[test]
    fn test_multiple_mappings() {
        let temp_dir = tempdir().unwrap();
        let cache = UrlMapCache::new(temp_dir.path()).unwrap();

        // Add multiple mappings
        let mappings = vec![
            ("https://example.com/", "example.com/index.html"),
            ("https://example.com/about", "example.com/about.html"),
            ("https://example.com/contact", "example.com/contact.html"),
        ];

        for (url, path) in mappings.iter() {
            cache.insert(url.to_string(), path.to_string()).unwrap();
        }

        assert_eq!(cache.len(), 3);

        // Verify all forward lookups work
        for (url, path) in mappings.iter() {
            assert_eq!(cache.get_path(url), Some(path.to_string()));
        }

        // Verify all reverse lookups work
        for (url, path) in mappings.iter() {
            assert_eq!(cache.get_url(path), Some(url.to_string()));
        }
    }
}
