# Concurrent Download Implementation Plan

## Current State

The code downloads files **sequentially** - one file at a time:
```rust
loop {
    let task = self.state.dequeue();
    if let Some(task) = task {
        // Process with .await (blocks until complete)
        Self::download_and_process_url(...).await;
    }
}
```

**Performance:** 200,000 files × 2s avg = **111 hours** (with network latency)

## Goal

Enable parallel downloads to reduce total time:
- 5 workers: **~22 hours** (5x speedup)
- 10 workers: **~11 hours** (10x speedup)
- 20 workers: **~5.5 hours** (20x speedup)

## Simplest Implementation

### Option 1: Spawn Workers (Recommended)

Use Tokio's task spawning with a semaphore to limit concurrency.

**Key changes:**
1. Add `tokio::sync::Semaphore` to limit concurrent downloads
2. Spawn tasks with `tokio::spawn` instead of `.await`
3. Track active tasks with `JoinSet` for proper cleanup
4. Keep existing queue-based architecture

**Code structure:**
```rust
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

pub async fn mirror_website(&mut self, max_concurrent: usize) -> Result<()> {
    let semaphore = Arc::new(Semaphore::new(max_concurrent));
    let mut join_set = JoinSet::new();

    loop {
        // Dequeue next task
        if let Some(task) = self.state.dequeue() {
            // Wait for available slot
            let permit = semaphore.clone().acquire_owned().await?;

            // Clone what we need for the spawned task
            let client = self.client.clone();
            let file_manager = self.file_manager.clone();
            let state = self.state.clone();
            // ... clone other needed fields

            // Spawn task
            join_set.spawn(async move {
                let result = Self::download_and_process_url(...).await;
                drop(permit); // Release semaphore slot
                result
            });
        } else {
            // No more tasks in queue - wait for active tasks to finish
            if join_set.is_empty() {
                break;
            }
            // Wait for at least one task to complete
            join_set.join_next().await;
        }
    }

    // Wait for all remaining tasks
    while let Some(result) = join_set.join_next().await {
        // Handle result
    }

    Ok(())
}
```

**Complexity:** Moderate
- Requires understanding async task spawning
- Need to handle task results properly
- Must ensure state is thread-safe (already using `Arc`)

---

### Option 2: FuturesUnordered (Alternative)

Use `futures::stream::FuturesUnordered` to manage concurrent futures without spawning tasks.

**Code structure:**
```rust
use futures::stream::{FuturesUnordered, StreamExt};
use tokio::sync::Semaphore;

pub async fn mirror_website(&mut self, max_concurrent: usize) -> Result<()> {
    let semaphore = Arc::new(Semaphore::new(max_concurrent));
    let mut futures = FuturesUnordered::new();

    loop {
        // Fill up to max_concurrent
        while futures.len() < max_concurrent {
            if let Some(task) = self.state.dequeue() {
                let permit = semaphore.clone().acquire_owned().await?;
                let fut = Self::download_and_process_url(...);
                futures.push(async move {
                    let result = fut.await;
                    drop(permit);
                    result
                });
            } else {
                break;
            }
        }

        if futures.is_empty() {
            break;
        }

        // Wait for next completion
        futures.next().await;
    }

    Ok(())
}
```

**Complexity:** Moderate
- Requires `futures` crate
- Slightly more complex stream handling
- No task spawning needed

---

## Recommended Approach: Option 1 (Spawn Workers)

### Step-by-Step Implementation

#### Step 1: Add Dependencies
```toml
# Already have tokio with full features
[dependencies]
tokio = { version = "1", features = ["full"] }
```

#### Step 2: Modify `mirror_website()` Loop

**Location:** `src/downloader.rs:275-367`

**Current code:**
```rust
loop {
    let download_task = self.state.dequeue();
    if let Some(task) = download_task {
        // ... setup ...
        match Self::download_and_process_url(...).await {
            // ... handle result ...
        }
    } else {
        // ... check if done ...
    }
}
```

**New code:**
```rust
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

let semaphore = Arc::new(Semaphore::new(self.max_concurrent));
let mut join_set = JoinSet::new();

loop {
    // Try to spawn new tasks
    while join_set.len() < self.max_concurrent {
        let download_task = self.state.dequeue();
        if let Some(task) = download_task {
            let permit = semaphore.clone().acquire_owned().await.unwrap();

            // Clone all needed fields
            let client = self.client.clone();
            let file_manager = self.file_manager.clone();
            let state = self.state.clone();
            let base_url = self.base_url.clone();
            let run_logger = self.run_logger.clone();
            let ignore_patterns = self.ignore_patterns.clone();

            let url = task.url.clone();
            let depth = task.depth;
            let priority = task.priority.clone();
            let resource_type = task.resource_type.clone();
            let source_url = task.source_url.clone();

            // Spawn task
            join_set.spawn(async move {
                let result = Self::download_and_process_url(
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
                ).await;

                drop(permit); // Release semaphore
                (url, result)
            });
        } else {
            break; // No more tasks in queue
        }
    }

    // If no active tasks and no queue items, we're done
    if join_set.is_empty() {
        let queue_size = self.state.queue_size();
        if queue_size == 0 {
            break;
        }
        // Give queue a moment to populate
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        continue;
    }

    // Wait for at least one task to complete
    if let Some(result) = join_set.join_next().await {
        match result {
            Ok((url, Ok(ProcessResult::Downloaded))) => {
                // Success logged in download_resource
            }
            Ok((url, Ok(ProcessResult::Error(msg)))) => {
                log::error!("Error downloading {}: {}", url, msg);
            }
            Ok((url, Err(e))) => {
                log::error!("Unexpected error: {} - {}", url, e);
            }
            Ok((_, Ok(_))) => {
                // Other results (skipped, etc.)
            }
            Err(e) => {
                log::error!("Task join error: {}", e);
            }
        }
    }
}

// Wait for all remaining tasks
while let Some(result) = join_set.join_next().await {
    // Handle remaining results
}
```

#### Step 3: Re-add `max_concurrent` CLI Argument

**Location:** `src/cli.rs:24`

```rust
/// Maximum concurrent downloads
#[arg(short = 'c', long, default_value = "5")]
pub max_concurrent: usize,
```

#### Step 4: Add Back to WebsiteMirror

**Location:** `src/downloader.rs:60`

```rust
pub struct WebsiteMirror {
    pub base_url: String,
    pub output_dir: PathBuf,
    pub max_depth: usize,
    pub max_concurrent: usize,  // Add back
    // ... rest of fields
}
```

#### Step 5: Pass Through Constructor

Update all the places we just removed it from:
- `new()` parameter
- Struct initialization
- main.rs call
- All tests

---

## Complexity Assessment

### Medium Difficulty
- **Lines of code:** ~50 lines changed in `mirror_website()`
- **New concepts:** Tokio spawning, semaphores, JoinSet
- **Risk:** Medium - need to handle task results correctly
- **Time estimate:** 1-2 hours implementation + testing

### Challenges
1. **All clones must be Arc-safe:** Already done (client, state, run_logger are Arc)
2. **Error handling:** Need to handle both task join errors and download errors
3. **Graceful shutdown:** Must wait for all tasks before exiting
4. **State consistency:** Already thread-safe with Arc<PersistentState>

---

## Testing Strategy

1. **Unit test:** Verify semaphore limits concurrent tasks
2. **Integration test:** Compare speed sequential vs concurrent (5 workers)
3. **Manual test:** Download small site with `--max-concurrent 10`
4. **Stress test:** Download with `--max-concurrent 20` and verify no deadlocks

---

## Performance Expectations

**Assuming 2s average per file (including network latency):**

| Workers | 1,000 files | 10,000 files | 100,000 files |
|---------|-------------|--------------|---------------|
| 1       | 33 min      | 5.5 hrs      | 55 hrs        |
| 5       | 6.7 min     | 1.1 hrs      | 11 hrs        |
| 10      | 3.3 min     | 33 min       | 5.5 hrs       |
| 20      | 1.7 min     | 17 min       | 2.8 hrs       |

**Diminishing returns:** Beyond 10-20 workers, you hit other bottlenecks:
- Network bandwidth
- Server rate limits
- Disk I/O
- DNS lookups

---

## Alternative: Keep It Simple

**If concurrency seems too complex right now:**
1. Keep sequential downloads
2. Use VPN + no delays
3. Let it run overnight (60 hours for 200k files)
4. Add concurrency later only if needed

**Pro tip:** Many large scraping projects run fine with sequential downloads. The real bottleneck is usually:
- Getting blocked (solved with VPN/proxy)
- Network latency (solved with good connection)
- Not total throughput

---

## Decision Matrix

| Approach | Speed | Complexity | Risk | Recommendation |
|----------|-------|------------|------|----------------|
| Sequential (current) | 1x | Low | Low | ✅ Start here |
| 5 workers | 5x | Medium | Medium | Good for production |
| 10 workers | 10x | Medium | Medium | Sweet spot |
| 20+ workers | 15-20x | Medium-High | High | Overkill for most |

---

## Recommendation

1. **Start with sequential** (current state) - try the download and see how long it actually takes
2. **If too slow**, implement Option 1 with **5-10 workers**
3. **Monitor for rate limiting** - may need to add delays even with concurrency
4. **Scale up slowly** - test with 5, then 10, then 20 if needed

The complexity of adding concurrency is **moderate** but very doable. The code structure is already well-suited for it since everything uses `Arc` for shared state.
