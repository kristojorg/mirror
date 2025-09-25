# PERSISTED_STATE.md - Implementation Plan for Persistent Crawl State

## Background & Context

### The Core Problem
The current resumption approach has a fundamental flaw: when HTML files are saved, all URLs are rewritten to local paths (e.g., `https://example.com/image.jpg` → `example.com/image.jpg`). When resuming a crawl:
1. We read the saved HTML from disk
2. Try to extract links from it, but they're now local paths
3. Attempt to convert local paths back to URLs for queueing
4. **This fails** because the URL → local path transformation is lossy and can't be reliably reversed

Example: Both `https://example.com/page` and `https://example.com/page/` might map to `example.com/page/index.html`, making reverse mapping impossible.

### Why Persistent State Solves This
Instead of trying to reconstruct state from saved files, we persist the actual crawl state continuously:
- The download queue is saved to disk - no reconstruction needed
- URLs in progress are tracked and recovered on restart
- The state file becomes the authoritative source - we never need to check the filesystem
- Resume simply means: load state, continue processing the queue

### Key Design Decisions

1. **No separate "visited" set needed**: The union of `downloaded` and `errored` keys serves as the visited check. If a URL is in either map, it's been processed.

2. **Processing set for crash recovery**: URLs being actively downloaded are tracked in a `processing` set. On startup, these automatically move back to the queue, ensuring no work is lost during interruptions.

3. **State over filesystem**: Once implemented, we NEVER check the filesystem to determine if something was downloaded. The state file is the single source of truth.

4. **Atomic operations**: When dequeuing, the URL atomically moves from `queue` to `processing`. This ensures consistency even if interrupted.

5. **Priority without complexity**: We use VecDeque with insert positions instead of BinaryHeap. This allows priority ordering while maintaining JSON serializability.

## Overview
Replace the current in-memory state management (visited_urls HashSet, download_queue BinaryHeap) with a fully persistent state that enables reliable crawl resumption without complex URL reconstruction.

## Implementation Strategy
Build the new `persistent_state.rs` module first, then go through and remove old unnecessary code and replace it with calls to the new state module.

## Phase 1: Build New Persistent State Module

### 1.1 Create new state module (src/persistent_state.rs)
```rust
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct StateData {
    pub statistics: Statistics,
    pub queue: VecDeque<DownloadTask>,  // Use VecDeque for FIFO with priority
    pub processing: HashSet<String>,     // URLs currently being processed
    pub downloaded: HashMap<String, ResourceInfo>,  // URL -> local path + metadata
    pub errored: HashMap<String, ErrorInfo>,        // URL -> error details
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ResourceInfo {
    pub local_path: String,
    pub resource_type: String,
    pub size_bytes: u64,
    pub downloaded_at: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ErrorInfo {
    pub error_message: String,
    pub attempted_at: String,
    pub resource_type: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DownloadTask {
    pub url: String,
    pub depth: usize,
    pub priority: DownloadPriority,
    pub resource_type: Option<ResourceType>,
}
```

### 1.2 Implement persistence methods
```rust
impl PersistentState {
    fn save_to_disk(&self) -> Result<()>
    fn load_from_disk(path: &Path) -> Result<Self>
    fn dequeue(&self) -> Option<DownloadTask>  // Atomically moves from queue to processing
    fn enqueue(&self, task: DownloadTask)      // Checks if already visited before adding
    fn mark_completed(&self, url: String, info: ResourceInfo)  // Moves from processing to downloaded
    fn mark_errored(&self, url: String, error: ErrorInfo)      // Moves from processing to errored
    fn is_visited(&self, url: &str) -> bool     // Checks downloaded + errored (no separate visited set!)
    fn move_processing_to_queue(&mut self)      // For crash recovery on startup
}
```

### 1.3 Implement load/save operations
```rust
impl PersistentState {
    pub fn new(output_dir: &Path) -> Result<Self> {
        let cache_file = output_dir.join(".mirror/state.json");

        let data = if cache_file.exists() {
            let mut loaded: StateData = serde_json::from_str(&fs::read_to_string(&cache_file)?)?;
            // CRASH RECOVERY: Move all processing URLs back to queue
            // These were being downloaded when the program was interrupted
            for url in loaded.processing.drain() {
                // Re-queue at high priority to resume quickly
                // Note: We lose the original depth/resource_type, but that's acceptable
                loaded.queue.push_back(DownloadTask {
                    url: url.clone(),
                    depth: 0,  // Conservative depth to prevent deep recursion
                    priority: DownloadPriority::High,
                    resource_type: None,
                });
            }
            log::info!("Resumed crawl with {} queued URLs, {} downloaded, {} errors",
                loaded.queue.len(), loaded.downloaded.len(), loaded.errored.len());
            loaded
        } else {
            StateData::default()
        };

        Ok(Self {
            data: Arc::new(Mutex::new(data)),
            cache_file,
        })
    }

    fn save(&self) -> Result<()> {
        let data = self.data.lock().unwrap();
        let json = serde_json::to_string_pretty(&*data)?;
        fs::write(&self.cache_file, json)?;
        Ok(())
    }
}
```

### 1.4 Implement atomic state operations
```rust
pub fn dequeue(&self) -> Option<DownloadTask> {
    let mut data = self.data.lock().unwrap();

    // Pop from front (respecting priority order maintained by enqueue)
    let task = data.queue.pop_front()?;

    // ATOMIC: Move from queue to processing
    data.processing.insert(task.url.clone());

    drop(data);  // Release lock before I/O
    self.save().ok();  // Best effort save
    Some(task)
}

pub fn mark_completed(&self, url: String, local_path: String, resource_type: ResourceType, size: u64) {
    let mut data = self.data.lock().unwrap();

    // ATOMIC: Move from processing to downloaded
    data.processing.remove(&url);
    data.downloaded.insert(url, ResourceInfo {
        local_path,
        resource_type: format!("{:?}", resource_type),
        size_bytes: size,
        downloaded_at: chrono::Local::now().to_rfc3339(),
    });

    // Update statistics
    data.statistics.urls_discovered = data.downloaded.len() + data.errored.len();

    drop(data);
    self.save().ok();
}

pub fn is_visited(&self, url: &str) -> bool {
    let data = self.data.lock().unwrap();
    // No separate visited set - just check downloaded + errored
    data.downloaded.contains_key(url) || data.errored.contains_key(url)
}
```

### 1.5 Implement priority queue handling
Since VecDeque doesn't maintain priority order naturally (unlike BinaryHeap), we implement priority ordering through insert positions:
```rust
pub fn enqueue(&self, task: DownloadTask) {
    let mut data = self.data.lock().unwrap();

    // Check if already visited (downloaded or errored) - no separate visited set!
    if data.downloaded.contains_key(&task.url) || data.errored.contains_key(&task.url) {
        return;
    }

    // Also check if already in queue or processing
    if data.processing.contains(&task.url) {
        return;
    }
    if data.queue.iter().any(|t| t.url == task.url) {
        return;
    }

    // Insert based on priority (this replaces BinaryHeap's automatic ordering)
    let insert_pos = match task.priority {
        DownloadPriority::Critical => 0,  // Front of queue
        DownloadPriority::High => {
            // After all critical items
            data.queue.iter()
                .position(|t| t.priority > DownloadPriority::Critical)
                .unwrap_or(0)
        },
        DownloadPriority::Normal => data.queue.len(),  // Back of queue
    };

    data.queue.insert(insert_pos, task);
    drop(data);
    self.save().ok();
}
```

### Why VecDeque Instead of BinaryHeap?
- BinaryHeap doesn't serialize well to JSON (loses order)
- VecDeque with insert positions gives us full control
- Easier to inspect and debug in the JSON state file
- Performance is fine since queue operations are infrequent (5 req/s max)

### 1.6 Add to lib.rs
```rust
pub mod persistent_state;
```

## Phase 2: Remove Complex Resumption Logic

### 2.1 Simplify `download_and_process_html` (src/downloader.rs:817-984)
- Remove lines 837-884 (the entire "HTML already exists on disk" block that tries to extract links)
- Keep simple file exists check for now

### 2.2 Simplify `extract_and_queue_html_links` (src/downloader.rs:445-577)
- Remove the `existing_file_mode` parameter
- Remove the `mirror_state` parameter for URL reverse lookup
- Remove lines 468-555 (existing file mode handling)
- Keep only the fresh download path

## Phase 3: Replace In-Memory Structures with PersistentState

### 3.1 Update WebsiteMirror struct (src/downloader.rs)
- Remove line 98: `visited_urls: Arc<Mutex<HashSet<String>>>`
- Remove line 99: `download_queue: Arc<Mutex<BinaryHeap<DownloadTask>>>`
- Remove line 100: `mirror_state: MirrorState`
- Add: `state: Arc<PersistentState>`

### 3.2 Update WebsiteMirror::new (src/downloader.rs:252-291)
- Remove creation of `visited_urls` HashSet
- Remove creation of `download_queue` BinaryHeap
- Remove creation of `mirror_state`
- Create `PersistentState::new(output_dir)`

### 3.3 Update mirror_website method (src/downloader.rs:322-438)
- Replace `self.download_queue.lock().unwrap().push()` → `self.state.enqueue()`
- Replace `self.download_queue.lock().unwrap().pop()` → `self.state.dequeue()`

## Phase 4: Update Download Processing Methods

### 4.1 Update `download_and_process_url` (src/downloader.rs:1110-1192)
- Remove visited_urls parameter
- Remove download_queue parameter
- Remove mirror_state parameter
- Pass state instead
- Remove the visited check at beginning (dequeue handles this)

### 4.2 Update `download_resource` (src/downloader.rs:1194-1360)
- Replace `mirror_state.get_path()` check with `state.downloaded.contains_key()`
- Replace `mirror_state.track_download_success()` with `state.mark_completed()`
- Replace `mirror_state.track_download_error()` with `state.mark_errored()`
- Replace file exists check with state check

### 4.3 Update `process_html_resources` (src/downloader.rs:580-815)
- Remove visited_urls parameter
- Remove download_queue parameter
- Remove mirror_state parameter
- Pass state instead
- Replace queue operations with `state.enqueue()`
- Check `state.is_visited()` before enqueueing

### 4.4 Update `download_and_process_html`
- Replace file exists check with `state.is_visited()` check

### 4.5 Update `extract_and_queue_html_links`
- Remove visited_urls parameter
- Remove download_queue parameter
- Pass state instead
- Use `state.enqueue()` for queueing

## Phase 5: Clean Up Old Code

### 5.1 Remove old MirrorState
- Delete src/mirror_state.rs entirely
- Remove all imports and usage
- Update lib.rs to remove mirror_state module export

### 5.2 Update RunLogger
- Update to use PersistentState instead of MirrorState
- Or simplify/remove if redundant with new state

### 5.3 Final cleanup
- Verify all file exists checks replaced with state checks
- Remove any remaining references to old state management

## Phase 6: Testing & Verification

### 6.1 Test resumption
1. Start a crawl
2. Kill it mid-way (Ctrl+C)
3. Restart with same command
4. Verify it continues from where it left off

### 6.2 Test crash recovery
1. Start a crawl
2. Kill -9 the process (simulate crash)
3. Check that "processing" URLs are back in queue
4. Verify no URLs are lost

### 6.3 Test deduplication
1. Verify URLs aren't downloaded twice
2. Verify queue doesn't contain duplicates
3. Verify visited check works correctly

## Phase 7: Add Convenience Features (Optional)

### 7.1 Add state inspection command
```bash
website-mirror --inspect-state output_dir
# Shows: queue size, downloaded count, error count, processing items
```

### 7.2 Add resume confirmation
```rust
if state_exists {
    println!("Found existing crawl with {} pending URLs. Resume? [Y/n]", queue.len());
    // Allow user to start fresh if desired
}
```

### 7.3 Add state reset option
```bash
website-mirror --reset-state https://example.com -o output
# Clears state but keeps downloaded files
```

## Benefits of This Approach

1. **Simpler code**: No complex URL reverse engineering
2. **Reliable resumption**: Complete state is always preserved
3. **Crash recovery**: Processing URLs returned to queue
4. **No data loss**: Every state change is persisted
5. **Debuggable**: JSON state file is human-readable
6. **Future-proof**: Easy to add new features like --recrawl

## Implementation Notes

### Build-First Approach
1. **Phase 1**: Build complete `persistent_state.rs` module
2. **Phase 2**: Remove complex resumption logic that won't be needed
3. **Phase 3-4**: Replace old state management with new module
4. **Phase 5**: Clean up and remove old code

This approach keeps the code working throughout the transition

### Key Principle
**The state file becomes authoritative**: Once fully implemented, we never check the filesystem to determine if something was downloaded. The state file tracks:
- What's been downloaded (`downloaded` map)
- What failed (`errored` map)
- What's in progress (`processing` set)
- What's pending (`queue`)

## Migration Notes

- No backward compatibility needed - clean break
- Existing crawls will need to restart from scratch
- State file location: `output_dir/.mirror/state.json`
- Can coexist with existing code during development
- Add TODO comments for any temporary code that needs cleanup