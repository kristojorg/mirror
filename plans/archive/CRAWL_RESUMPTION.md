# Crawl Resumption Implementation Plan

## Problem Statement

When running the website-mirror tool against a site that has already been partially downloaded, the tool currently has a critical bug:

1. **Current Behavior**: When an HTML file exists on disk, the code tries to extract links using `HtmlParser::extract_resources()` which expects absolute URLs
2. **The Issue**: Saved HTML files have URLs already rewritten to relative local paths (e.g., `../css/style.css` instead of `https://example.com/css/style.css`)
3. **The Result**: Links cannot be properly extracted and crawling doesn't continue correctly

## Root Cause Analysis

### Current Flow (Broken)
```
1. Check if HTML file exists on disk
2. Read the HTML content
3. Try to extract resources using HtmlParser (expects absolute URLs)
4. Fails because HTML contains relative local paths
5. Cannot properly queue links for continued crawling
```

### How HTML Files Are Saved
1. HTML is downloaded with original absolute URLs
2. Resources are downloaded and saved locally
3. `HtmlRewriter::rewrite_urls()` replaces all URLs with local paths
4. HTML is saved with these local paths
5. URL mappings are stored in `UrlMapCache`

Example transformation:
```html
<!-- Original HTML -->
<a href="https://example.com/about">About</a>
<link rel="stylesheet" href="https://example.com/css/style.css">

<!-- Saved HTML (after rewriting) -->
<a href="about.html">About</a>
<link rel="stylesheet" href="css/style.css">
```

## Solution Design

### Core Strategy
1. Extract local paths from saved HTML (as they are)
2. Convert relative paths to absolute filesystem paths
3. Look up original URLs using `UrlMapCache::get_url()`
4. Queue the original URLs for crawling

### Implementation Components

#### 1. New Method: `HtmlParser::extract_local_links()`
```rust
// Extract href values from <a> tags without URL resolution
pub fn extract_local_links(&self, html_content: &str) -> Vec<String> {
    let document = Document::from(html_content);
    let mut links = Vec::new();
    
    for link in document.find(Name("a")) {
        if let Some(href) = link.attr("href") {
            // Skip special schemes and anchors
            if !href.starts_with("javascript:") 
               && !href.starts_with("mailto:")
               && !href.starts_with("tel:")
               && !href.starts_with("#") {
                links.push(href.to_string());
            }
        }
    }
    
    links
}
```

#### 2. Updated `download_and_process_html()` Logic
```rust
// When HTML file exists on disk:
if file_manager.file_exists(&local_html_path) {
    // Read existing HTML
    let content = file_manager.read_file(&local_html_path)?;
    let html_content = String::from_utf8_lossy(&content);
    
    // Extract local paths (not URLs)
    let local_links = page_html_parser.extract_local_links(&html_content);
    
    // Convert local paths back to original URLs
    for local_link in local_links {
        // Resolve relative path to absolute filesystem path
        let absolute_path = resolve_local_path(
            &local_html_path,  // Current HTML file
            &local_link        // Link found in HTML
        );
        
        // Look up original URL in cache
        if let Some(original_url) = url_cache.get_url(&absolute_path) {
            // Queue for crawling if not visited
            if !visited_urls.contains(&original_url) {
                queue.push(DownloadTask {
                    url: original_url,
                    depth: depth + 1,
                    priority: DownloadPriority::High,
                    resource_type: Some(ResourceType::Link),
                });
            }
        }
    }
}
```

#### 3. Helper Function: `resolve_local_path()`
```rust
fn resolve_local_path(current_file: &Path, link_path: &str) -> String {
    // Handle different types of paths:
    // - Relative: "../page.html" -> resolve relative to current file
    // - Root-relative: "/page.html" -> relative to output directory
    // - Absolute URLs: "https://..." -> return as-is (external links)
    
    if link_path.starts_with("http://") || link_path.starts_with("https://") {
        // External link that wasn't rewritten
        return link_path.to_string();
    }
    
    let current_dir = current_file.parent().unwrap_or(Path::new(""));
    
    if link_path.starts_with("/") {
        // Root-relative path
        link_path.trim_start_matches('/').to_string()
    } else {
        // Relative path
        let resolved = current_dir.join(link_path);
        resolved.to_string_lossy().to_string()
    }
}
```

## Implementation Steps

### Step 1: Add Local Link Extraction to HtmlParser
- File: `src/html_parser.rs`
- Add `extract_local_links()` method
- Extract href attributes without URL resolution
- Skip special schemes (javascript:, mailto:, etc.)

### Step 2: Create Path Resolution Helper
- File: `src/downloader.rs`
- Add `resolve_local_path()` function
- Handle relative paths (../, ./)
- Handle root-relative paths (/)
- Handle absolute URLs (http://, https://)

### Step 3: Update download_and_process_html
- File: `src/downloader.rs`
- Replace existing link extraction for cached files
- Use new `extract_local_links()` method
- Resolve paths and look up URLs in cache
- Queue original URLs for crawling

### Step 4: Ensure UrlMapCache Persistence
- Verify cache is loaded at startup
- Ensure all downloads update the cache
- Confirm cache is saved after each update

## Edge Cases to Handle

1. **Missing Cache Entries**: Links that were added to HTML after initial download
   - Solution: Skip or attempt to reconstruct URL from path structure

2. **External Links**: Some external links may not be rewritten
   - Solution: Detect absolute URLs and handle separately

3. **Hash Fragments**: Links with anchors (#section)
   - Solution: Strip fragments before processing

4. **Query Parameters**: Links with query strings
   - Solution: Preserve during path resolution

5. **Malformed Paths**: Invalid or broken links in HTML
   - Solution: Log and skip invalid paths

## Testing Plan

### Test Case 1: Resume Partial Download
1. Start downloading a site
2. Interrupt after a few pages
3. Restart the download
4. Verify:
   - Existing HTML files are loaded from disk
   - Links are properly extracted from local paths
   - Original URLs are recovered from cache
   - Crawling continues without re-downloading existing files

### Test Case 2: Mixed Content
1. Create a test site with:
   - Internal links (rewritten to local paths)
   - External links (may remain as absolute URLs)
   - Various path types (relative, root-relative)
2. Download partially
3. Resume and verify all link types are handled

### Test Case 3: Cache Integrity
1. Verify UrlMapCache persists between runs
2. Ensure bidirectional lookup works:
   - URL → local path (forward lookup)
   - Local path → URL (reverse lookup)
3. Test with large number of mappings

## Expected Outcome

After implementation:
1. **Efficiency**: No re-downloading of existing content
2. **Continuity**: Seamless resume from interruption point
3. **Completeness**: All links properly discovered and queued
4. **Performance**: Fast startup using cached mappings

## Code Locations

- `src/html_parser.rs`: Add `extract_local_links()` method
- `src/downloader.rs`: Update `download_and_process_html()` function
- `src/url_map_cache.rs`: Already has bidirectional lookup support
- `src/file_manager.rs`: Already has file existence checking

## Risks and Mitigations

### Risk 1: Performance Impact
- **Issue**: Reverse lookup in UrlMapCache is O(n)
- **Mitigation**: Consider adding reverse index if performance becomes issue

### Risk 2: Path Resolution Complexity
- **Issue**: Complex relative paths may be tricky to resolve
- **Mitigation**: Use Rust's Path utilities for robust resolution

### Risk 3: Cache Corruption
- **Issue**: Corrupted cache file could break resumption
- **Mitigation**: Add validation and fallback to full re-download

## Success Criteria

1. Tool successfully resumes from partial downloads
2. No duplicate downloads of existing files
3. All links properly extracted from saved HTML
4. Performance comparable to fresh crawl
5. No data loss or corruption

## Timeline

- Phase 1: Implement local link extraction (30 min)
- Phase 2: Add path resolution logic (30 min)
- Phase 3: Update download_and_process_html (45 min)
- Phase 4: Testing and debugging (45 min)
- Total estimated time: 2.5 hours