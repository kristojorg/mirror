# PERSISTED_STATE.md - Implementation Plan for Persistent Crawl State

## Background & Context

### The Core Problem
The current resumption approach has a fundamental flaw: when HTML files are saved, all URLs are rewritten to local paths (e.g., `https://example.com/image.jpg` → `example.com/image.jpg`). When resuming a crawl:
1. We read the saved HTML from disk
2. Try to extract links from it, but they're now local paths
3. Attempt to convert local paths back to URLs for queueing
4. **This fails** because the URL → local path transformation is lossy and can't be reliably reversed

### Why Persistent State Solves This
Instead of trying to reconstruct state from saved files, we persist the actual crawl state continuously:
- The download queue is saved to disk - no reconstruction needed
- URLs in progress are tracked and recovered on restart
- The state file becomes the authoritative source - we never need to check the filesystem
- Resume simply means: load state, continue processing the queue

## ✅ Phase 1: COMPLETED - Persistent State Module

The `persistent_state.rs` module has been implemented and provides a complete API for managing crawl state persistently.

### API Overview

```rust
use website_mirror::persistent_state::PersistentState;

// Initialize (automatically loads existing state or creates new)
let state = PersistentState::new(&output_dir)?;

// Queue Management
state.enqueue(DownloadTask {
    url, depth, priority, resource_type
});
let task = state.dequeue(); // Returns Option<DownloadTask>

// State Transitions
state.mark_completed(url, local_path, resource_type, size_bytes);
state.mark_errored(url, error_message, resource_type);

// Query Methods
state.is_visited(&url) -> bool;           // Check if URL was processed
state.is_queued(&url) -> bool;            // Check if URL is in queue
state.is_processing(&url) -> bool;        // Check if URL is being processed
state.get_local_path(&url) -> Option<String>; // Get local file path

// Statistics
state.queue_size() -> usize;
state.downloaded_count() -> usize;
state.errored_count() -> usize;
state.processing_count() -> usize;
state.total_seen_urls() -> usize;
state.get_statistics() -> Statistics;     // Computed on-the-fly
```

### Key Features Implemented

1. **Automatic Persistence**: Every state change is saved to `output_dir/.mirror/state.json`
2. **Crash Recovery**: URLs in `processing` automatically move back to `queue` on restart
3. **Priority Queue**: VecDeque with proper insertion ordering (Critical → High → Normal)
4. **Duplicate Prevention**: Built-in deduplication across queue, processing, downloaded, and errored
5. **No Separate Visited Set**: Uses union of `downloaded` and `errored` maps
6. **Statistics On-Demand**: No redundant statistics field; computed from actual data

### State File Structure

```json
{
  "queue": [
    { "url": "...", "depth": 1, "priority": "High", "resource_type": "Link" }
  ],
  "processing": ["https://example.com/processing.html"],
  "downloaded": {
    "https://example.com/index.html": {
      "local_path": "example.com/index.html",
      "resource_type": "Link",
      "size_bytes": 10240,
      "downloaded_at": "2025-09-25T10:30:00Z"
    }
  },
  "errored": {
    "https://example.com/404.jpg": {
      "error_message": "404 Not Found",
      "attempted_at": "2025-09-25T10:31:00Z",
      "resource_type": "Image"
    }
  }
}
```

## ✅ Phase 2: COMPLETED - Remove Complex Resumption Logic

**Goal**: Remove the complex code that tries to reconstruct URLs from saved HTML files. This code is no longer needed since PersistentState tracks everything.

### 2.1 Simplify `download_and_process_html` (src/downloader.rs:~817-984)
- Find and remove the entire "HTML already exists on disk" block that tries to extract links from saved HTML
- This is the code that attempts to reconstruct URLs from local paths (the problematic behavior)
- Keep simple file exists checks for now (will be replaced in Phase 4)

### 2.2 Simplify `extract_and_queue_html_links` (src/downloader.rs:~445-577)
- Remove the `existing_file_mode` parameter completely
- Remove any code paths that handle existing files differently
- Keep only the fresh download path (parsing from live HTML content)

## ✅ Phase 3: COMPLETED - Replace In-Memory Structures with PersistentState

**Goal**: Replace HashSet/BinaryHeap with PersistentState throughout WebsiteMirror.

### 3.1 ✅ Update WebsiteMirror struct
- Removed `visited_urls: Arc<Mutex<HashSet<String>>>`
- Removed `download_queue: Arc<Mutex<BinaryHeap<DownloadTask>>>`
- Added `state: Arc<PersistentState>`

### 3.2 ✅ Update WebsiteMirror::new
- Created `PersistentState::new(output_dir)` instead of in-memory structures
- Removed initialization of visited_urls, download_queue
- Kept temporary MirrorState for RunLogger compatibility (will be removed in Phase 5)

### 3.3 ✅ Update main crawl loop in `mirror_website`
- Replaced `self.download_queue.lock().unwrap().push()` → `self.state.enqueue()`
- Replaced `self.download_queue.lock().unwrap().pop()` → `self.state.dequeue()`
- Removed visited_urls checks (PersistentState handles this internally)

### 3.4 ✅ Additional Improvements Made
- Added URL normalization to PersistentState to prevent duplicates (e.g., example.com vs example.com/)
- Removed duplicate DownloadTask struct from downloader.rs (now uses persistent_state::DownloadTask)
- Updated method signatures to use `state: Arc<PersistentState>` instead of visited_urls/download_queue

## Phase 4: Update All Download Methods

**Goal**: Add proper state transitions (mark_completed/mark_errored) to track downloads in PersistentState.

### ⚠️ CRITICAL ISSUE IDENTIFIED:
Downloads are being processed but **NOT being tracked in PersistentState**. URLs are moved from queue → processing but never moved to downloaded/errored.

### Required State Transitions:
Currently methods return `ProcessResult::Downloaded` but don't call `state.mark_completed()`. Need to add:

1. **`download_and_process_html`** (line 745) - Add `state.mark_completed()` before returning
2. **`download_and_process_css`** (line 867) - Add `state.mark_completed()` before returning
3. **`download_resource`** (line 1111) - Add `state.mark_completed()` before returning

### Error Handling:
Also need to add `state.mark_errored()` calls when downloads fail with errors.

### File Existence Checks:
Replace filesystem-based "already exists" checks with `state.is_visited()` - this is the core benefit of PersistentState.

## Phase 5: Update RunLogger to use PersistentState

**Goal**: Replace MirrorState dependencies with PersistentState in RunLogger.

### Current Temporary Compatibility Code:
**These are stopgap measures from Phase 3 that need to be removed:**

1. **Line 246-249** - `WebsiteMirror::new()` creates temporary MirrorState for RunLogger
2. **Line 284-309** - `get_mirror_state_statistics()` converts PersistentState → MirrorState format
3. **Line 314-316** - `get_mirror_state()` creates new MirrorState instances
4. **Line 370-371** - `mirror_website()` creates temporary MirrorState per download
5. **Line 293** - Missing bytes tracking per resource type in PersistentState

### RunLogger Integration:
- `src/run_logger.rs` expects MirrorState format for statistics display
- Need to update RunLogger to work directly with PersistentState statistics
- Or create a simple adapter that doesn't require MirrorState instantiation

### Statistics Format Differences:
```rust
// PersistentState format
struct Statistics {
    urls_discovered: usize,
    downloads: HashMap<String, usize>,  // resource_type -> count
    total_bytes: u64,
}

// MirrorState format (more complex)
struct Statistics {
    downloads: DownloadStats {  // nested structure
        html: ResourceStats { success, error, bytes },
        css: ResourceStats { success, error, bytes },
        // ...
    }
    // ...
}
```

## Phase 6: Clean Up Old Code

**Goal**: Remove all obsolete state management code.

### 6.1 Delete `src/mirror_state.rs` entirely
### 6.2 Remove module export from `lib.rs` (line 17: `pub use mirror_state::MirrorState;`)
### 6.3 Remove MirrorState imports from `src/downloader.rs`
### 6.4 Remove all compatibility methods:
   - `get_mirror_state_statistics()`
   - `get_mirror_state()`
### 6.5 ✅ RESOLVED - Removed duplicate DownloadTask struct from downloader.rs (Phase 3)

## Current State After Phase 3

### ✅ Working:
- PersistentState API is fully implemented with URL normalization
- Main crawl loop uses PersistentState queue operations
- Downloads are dequeued properly and moved to processing
- No more in-memory HashSet/BinaryHeap usage
- Code compiles and runs

### ❌ Not Working Yet:
- **Downloads aren't tracked** - URLs stay in processing forever (Phase 4)
- **No crash recovery** - Processing URLs don't get moved back to queue properly
- **RunLogger still uses MirrorState** - Temporary compatibility layer (Phase 5)
- **File exists checks** - Still using filesystem instead of state.is_visited()

### Next Developer Notes:
1. **Phase 4 is critical** - Without mark_completed()/mark_errored() calls, the persistent state doesn't work
2. **Test resumption early** - Start a crawl, kill it, restart to verify recovery
3. **The state.json file should show progress** - URLs moving between queue/processing/downloaded sections

## Testing Strategy

After integration is complete:
1. **Test Resume**: Start crawl → Ctrl+C → Restart → Should continue from queue
2. **Test Crash Recovery**: Kill -9 during download → Restart → Processing URLs should be re-queued
3. **Test No Duplicates**: Run full crawl → Check state.json → No URL should appear in multiple sections

## Key Implementation Notes

- **State is authoritative**: Never check filesystem to see if something was downloaded; always check `state.is_visited()`
- **Atomic transitions**: URLs move atomically between queue → processing → downloaded/errored
- **Best-effort saves**: State saves after each operation but continues even if save fails
- **Backward compatibility**: Not needed - this is a clean break from the old approach
