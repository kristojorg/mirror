# PENDING_RESOURCE_PLAN.md

## Priority: HIGH - Blocking Resume Functionality

### Problem Statement

When the mirror tool processes an HTML page, it immediately rewrites all resource URLs to local paths and saves the modified HTML. However, resource mappings are only added to the state file (`state.json`) AFTER successful download. This creates a critical issue:

1. HTML page is downloaded and processed
2. Resources are discovered and queued
3. HTML is rewritten with local paths (e.g., `example.com/image.jpg`)
4. Modified HTML is saved to disk
5. **If interrupted before resources download**, the local paths in HTML cannot be mapped back to URLs
6. **Resume fails** because `mirror_state.get_url()` can't find mappings for pending resources

### Current Behavior (Broken)

```
❌ Resource URL: www.scrapethissite.com/index.html
🔍 Processing link from existing file: current_page_url=https://www.scrapethissite.com, link=www.scrapethissite.com/index.html
⚠️  Could not find URL for local path: www.scrapethissite.com/lessons/index.html (searched in state)
```

### Proposed Solution

Track pending resources in the state file immediately when discovered, before downloading.

#### State File Structure

```json
{
  "stats": {
    "urls_discovered": 100,
    "downloads": { ... },
    "total_bytes": 12345678,
    "last_updated": "2025-09-24T15:15:41"
  },
  "resources": {
    // Successfully downloaded resources
    "https://example.com/page1": {
      "path": "example.com/page1.html",
      "resource_type": "html",
      "size": 10674,
      "downloaded_at": "2025-09-24T15:15:41",
      "status": "success",
      "error_message": null
    }
  },
  "pending": {
    // NEW: Resources discovered but not yet downloaded
    "https://example.com/image.jpg": {
      "path": "example.com/image.jpg",
      "resource_type": "image",
      "discovered_at": "2025-09-24T15:15:41",
      "status": "pending"
    },
    "https://example.com/style.css": {
      "path": "example.com/style.css",
      "resource_type": "css",
      "discovered_at": "2025-09-24T15:15:41",
      "status": "pending"
    }
  }
}
```

### Implementation Tasks

#### 1. Update `src/mirror_state.rs`

- [ ] Add `pending: HashMap<String, PendingResourceInfo>` field to `StateData` struct
- [ ] Create `PendingResourceInfo` struct:
  ```rust
  #[derive(Serialize, Deserialize, Debug, Clone)]
  struct PendingResourceInfo {
      path: String,
      resource_type: String,
      discovered_at: String,
      status: String, // Always "pending"
  }
  ```
- [ ] Add method `track_pending_resource(url: String, local_path: String, resource_type: ResourceType) -> Result<()>`
- [ ] Update `get_url(&self, local_path: &str) -> Option<String>` to search BOTH:
  - First check `resources` (completed downloads)
  - Then check `pending` (discovered but not downloaded)
- [ ] Add method `promote_pending_to_success(url: String, size: u64) -> Result<()>` to:
  - Remove from `pending`
  - Add to `resources` with full info and `status: "success"`
- [ ] Update `track_download_success` to check if resource exists in pending first

#### 2. Update `src/downloader.rs`

- [ ] In `process_html_resources()`, when building `url_mappings`:
  - **Before** adding to `url_mappings`
  - Call `mirror_state.track_pending_resource(url, local_path, resource_type)`
  - This ensures the mapping exists even if download fails

- [ ] In `download_resource()`:
  - On success, call `mirror_state.promote_pending_to_success()` instead of `track_download_success()`
  - On error, leave in pending (so resume can retry)

#### 3. Update Resume Logic in `src/downloader.rs`

- [ ] In `extract_and_queue_html_links()`:
  - The existing `state.get_url(&local_path)` call will now work because it checks both pending and completed
  - No changes needed here - it will automatically work once `get_url()` is updated

### Success Criteria

1. **Immediate tracking**: Resources are added to state as "pending" as soon as discovered
2. **Resume works**: Even if interrupted, all local paths in HTML can be mapped back to URLs
3. **State transitions**: Resources move from `pending` → `resources` on successful download
4. **Backward compatible**: Existing state files without `pending` section still work

### Testing Plan

1. Start a mirror operation on a site with many resources
2. Interrupt after HTML is saved but before resources download
3. Resume the operation
4. Verify:
   - No "Could not find URL for local path" warnings
   - Pending resources are correctly queued for download
   - Successfully downloaded resources move from pending to completed

### Future Enhancements

Once this is working, the `pending` section could eventually become the foundation for a persistent download queue, allowing:
- Better progress tracking
- Smarter resume (know exactly what's left to download)
- Priority-based downloading
- Retry failed downloads

### Notes for Implementation

- The critical fix for resume is updating `get_url()` to check both `resources` and `pending`
- Make sure to handle the case where old state files don't have a `pending` section (use `unwrap_or_default()`)
- Consider adding a `retry_count` field to pending resources for future retry logic