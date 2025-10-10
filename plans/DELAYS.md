# Request Delay Implementation Plan

## Problem Statement

The tool currently downloads files sequentially with no delays between requests, which:
1. Could trigger rate limiting on target servers
2. Appears bot-like and suspicious
3. May result in IP blocks during large mirrors

Additionally, the `max_concurrent` parameter is defined but **completely unused** - the code never implements parallel downloads.

## Current Behavior

```rust
// src/downloader.rs:283-330
loop {
    let download_task = self.state.dequeue();
    if let Some(task) = download_task {
        // Process directly with .await (blocks until complete)
        match Self::download_and_process_url(...).await {
            // No delay after completion
        }
    }
}
```

**Downloads are sequential:** file1 → wait → file2 → wait → file3...
**No parallelism:** Despite having `max_concurrent` parameter
**No delays:** Requests sent as fast as the network allows

## Solution

### Part 1: Remove Unused `max_concurrent` Parameter

Since we never use this for concurrency control and it's misleading:

**Files to modify:**
1. `src/cli.rs` - Remove the CLI argument
2. `src/downloader.rs` - Remove from struct and constructor
3. `src/main.rs` - Remove from initialization call
4. `tests/integration_tests.rs` - Update test signatures

**Search for:** `max_concurrent` (18 occurrences across 4 files)

### Part 2: Add Hardcoded Random Delay

Implement human-like delays between downloads using randomized intervals (200-2000ms).

**Add dependency to `Cargo.toml`:**
```toml
[dependencies]
rand = "0.8"
```

**Add delay logic in download loop (`src/downloader.rs`):**
```rust
use rand::Rng;

// After processing each download (line ~350):
// Apply delay only when we made a network request
match Self::download_and_process_url(...).await {
    Ok(ProcessResult::Downloaded) |
    Ok(ProcessResult::Error(_)) |
    Err(_) => {
        // We hit the network - apply rate limiting delay
        let delay_ms = rand::thread_rng().gen_range(200..=2000);
        log::debug!("Waiting {}ms before next request", delay_ms);
        tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
    }
    Ok(ProcessResult::SkippedAlreadyExists) |
    Ok(ProcessResult::SkippedFiltered) |
    Ok(ProcessResult::AlreadyVisited) => {
        // No network request made - no delay needed
    }
}
```

**Delay only applies when we touch the network:**
- `Downloaded` - ✅ YES (successful download from server)
- `Error(msg)` - ✅ YES (server returned error: 404, 500, timeout, etc.)
- Unexpected `Err(_)` - ✅ YES (might have hit server before error)
- `SkippedAlreadyExists` - ❌ NO (file exists locally, no network request)
- `SkippedFiltered` - ❌ NO (filtered by CLI, no network request)
- `AlreadyVisited` - ❌ NO (deduplicated, no network request)

**Benefit:** Resumption is fast because we skip through already-downloaded files without artificial delays, while still rate-limiting actual server requests.

## Implementation Steps

### Step 1: Remove `max_concurrent`
- [ ] Remove CLI arg from `src/cli.rs:24-26`
- [ ] Remove field from `WebsiteMirror` struct in `src/downloader.rs:61`
- [ ] Remove from Debug impl in `src/downloader.rs:79`
- [ ] Remove parameter from `new()` function in `src/downloader.rs:147`
- [ ] Remove from struct initialization in `src/downloader.rs:192`
- [ ] Remove from log statement in `src/downloader.rs:257-259`
- [ ] Remove from main.rs initialization in `src/main.rs:19`
- [ ] Update all test signatures (just remove parameter, keep tests passing)

### Step 2: Add Delay Logic
- [ ] Add `rand = "0.8"` dependency to `Cargo.toml`
- [ ] Add `use rand::Rng;` import to `src/downloader.rs`
- [ ] Add delay after downloads in `mirror_website()` loop (line ~350)
- [ ] Add debug log showing delay duration

### Step 3: Testing
- [ ] Verify tests still pass (no new tests needed)
- [ ] Manual test: verify delays are applied during downloads
- [ ] Manual test: check logs show random delays between 200-2000ms

## Default Behavior

**Hardcoded delay:** 200-2000ms random interval between network requests only

This gives an average of ~1.1 seconds per network request with human-like variation:
- Fast enough for reasonable download speeds (~3600 files/hour)
- Slow enough to avoid rate limiting
- Random enough to appear human-like
- **Smart resumption:** No delays when skipping already-downloaded files

**Note:** No CLI flags to configure delays. If you need faster/slower speeds in the future, edit the hardcoded range in `src/downloader.rs`.

## Rationale

### Why Random Delays (200-2000ms)?
1. **Human-like behavior:** Real users don't click at constant intervals
2. **Rate limit avoidance:** Random timing harder to detect than fixed-interval requests
3. **Respectful crawling:** Gives server breathing room (~1 req/second average)
4. **Simple implementation:** No CLI complexity, no validation logic

### Why Remove `max_concurrent`?
1. **Unused code:** Misleading and confusing to users
2. **Sequential design:** Current architecture is explicitly single-threaded
3. **Simplicity:** Easier to reason about delays in sequential context
4. **Future-proof:** If we add parallelism later, we'll redesign properly

### Why Not Implement Actual Concurrency?
1. **Complexity:** Requires semaphores, task spawning, and careful state management
2. **Overkill:** For one-time mirrors, sequential + delays is sufficient
3. **Harder rate limiting:** Multiple parallel streams are harder to throttle correctly
4. **Future work:** Can be added later if needed

### Why Hardcode Instead of CLI Flags?
1. **YAGNI:** Don't need configurability for the current use case
2. **Less testing:** No validation logic, no edge cases
3. **Simpler UX:** One less thing for users to think about
4. **Easy to change:** If needed later, it's a one-line code change

## Alternative Considered: Keep Sequential, Add Rate Limiter

Instead of random delays, we could use a proper rate limiter (e.g., `governor` crate):
```rust
use governor::{Quota, RateLimiter};

let limiter = RateLimiter::direct(Quota::per_second(nonzero!(1u32)));
limiter.until_ready().await;
```

**Decision:** Random delays preferred because:
- Simpler implementation (no extra crate)
- More human-like behavior
- Easier to explain to users
- Good enough for the use case

## Success Criteria

- [ ] `max_concurrent` completely removed from codebase
- [ ] All tests pass without `max_concurrent` parameter
- [ ] Random delays (200-2000ms) work correctly
- [ ] Delays are logged at debug level for visibility
- [ ] Manual testing confirms human-like request patterns
