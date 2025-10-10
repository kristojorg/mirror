# Simplified URL Rewriting Fix

## Current State Analysis

The `download_cache: Arc<Mutex<HashMap<String, String>>>` in `downloader.rs` is **already doing what we need**:
- Maps URLs to local paths
- Prevents duplicate downloads
- Checks if files exist on disk

## The ONLY Issues Are:

1. **Not Persistent** - Lost when program exits, can't resume crawls
2. **Navigation Links Not Added** - HTML `<a href>` links are queued but NOT added to cache/mappings

## Simplified Solution

### Step 1: Make download_cache Persistent

Add simple JSON persistence to existing cache:

```rust
// In WebsiteMirror struct, add path for cache file
cache_file_path: PathBuf,  // output_dir/url_cache.json

// In WebsiteMirror::new()
let cache_file_path = output_dir.join("url_cache.json");
let download_cache = if cache_file_path.exists() {
    // Load existing cache
    let content = fs::read_to_string(&cache_file_path)?;
    let cache: HashMap<String, String> = serde_json::from_str(&content)?;
    Arc::new(Mutex::new(cache))
} else {
    Arc::new(Mutex::new(HashMap::new()))
};

// Add method to save cache
fn save_cache(&self) -> Result<()> {
    let cache = self.download_cache.lock().unwrap();
    let json = serde_json::to_string_pretty(&*cache)?;
    fs::write(&self.cache_file_path, json)?;
    Ok(())
}

// Call save_cache() after each successful download in download_resource()
```

### Step 2: Fix Navigation Link Rewriting

In `process_html_resources()` around line 570, the bug is that navigation links are queued but NOT added to url_mappings:

```rust
// Current code (BROKEN):
for resource in &high_resources {
    // Just queues for crawling, doesn't add to mappings!
    let normalized_url = UrlMapper::normalize_root_url(&resource.absolute_url);
    if !visited_urls.lock().unwrap().contains(&normalized_url) {
        queue.push(...);
    }
}

// Fixed code:
for resource in &high_resources {
    // FIRST: Add to url_mappings so links get rewritten
    if resource.absolute_url.starts_with(base_url) {
        if let Ok(local_path) = url_mapper.url_to_local_path(&resource.absolute_url, &ResourceType::Link) {
            if let Ok(current_html_path) = url_mapper.url_to_local_path(current_page_url, &ResourceType::Link) {
                let relative_path = Self::calculate_relative_path(
                    &current_html_path.to_string_lossy(),
                    &local_path.to_string_lossy(),
                );
                url_mappings.insert(resource.original_url.clone(), relative_path);
            }
        }
    }
    
    // THEN: Queue for crawling as before
    let normalized_url = UrlMapper::normalize_root_url(&resource.absolute_url);
    if !visited_urls.lock().unwrap().contains(&normalized_url) {
        queue.push(...);
    }
}
```

### Step 3: Fix Crawl Resumption

When HTML exists on disk, we need to handle already-rewritten links:

```rust
// In download_and_process_html() when file exists:
if file_manager.file_exists(&local_html_path) {
    // Check if we've seen this URL before
    {
        let cache = download_cache.lock().unwrap();
        if !cache.contains_key(url) {
            // Add to cache since file exists
            cache.insert(url.to_string(), local_html_path.to_string_lossy().to_string());
        }
    }
    
    // The current code that extracts links is actually fine!
    // It creates a new HtmlParser with the ORIGINAL URL as base
    // So relative links will be resolved correctly
    
    // Continue with existing logic...
}
```

### Step 4: Remove WebP (Optional Simplification)

This can be done separately, but would simplify everything:
1. Remove WebP conversion functions
2. Remove `convert_to_webp` parameter everywhere
3. Remove `image` and `webp` dependencies from Cargo.toml

## Benefits of This Approach

1. **Minimal Changes** - Reuses existing `download_cache`
2. **Simple Implementation** - Just add JSON save/load
3. **Solves All Issues**:
   - ✅ Navigation links get rewritten (fixing the mappings bug)
   - ✅ Crawls can resume (persistent cache)
   - ✅ No duplicate downloads (cache persists across runs)

## Implementation Time

- Step 1 (Persistent Cache): 1 hour
- Step 2 (Fix Link Rewriting): 30 minutes  
- Step 3 (Fix Resumption): 30 minutes
- Step 4 (Remove WebP): 1 hour
- Testing: 1 hour

**Total: ~4 hours** vs 15-20 hours for the full rewrite

## The Core Insight

We don't need a new URL cache module! The `download_cache` is already the URL cache - it just needs:
1. Persistence (JSON file)
2. Navigation links added to mappings (one-line fix)
3. Save after downloads

This is a much simpler, more pragmatic solution that leverages existing code.