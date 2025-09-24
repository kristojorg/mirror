# Logging and Statistics Plan

## Overview
This document outlines the phased approach to implementing comprehensive logging, statistics tracking, and state management for the website-mirror tool. The goal is to provide visibility into long-running operations while maintaining simplicity.

## Phase 1: Basic File Logging ✅ COMPLETED
**Goal**: Capture all existing output to log files without changing the current logging approach.

### Implementation (DONE)
- ✅ Create run directories: `.mirror/runs/YYYY-MM-DD_HH-MM-SS/`
- ✅ Redirect all println!/eprintln! output to both terminal and `log.txt`
- ✅ Write a basic summary to the log at completion
- ✅ Use the `log` crate with `env_logger` for dual output

### Deliverables (ACHIEVED)
- ✅ All existing console output captured to file with timestamps
- ✅ Run-specific directories for organization
- ✅ Basic summary statistics using existing data (visited_urls.len(), etc.)

### Current Limitations (Expected)
- Total bytes shows 0 (not tracked yet)
- Successful downloads shows 0 (only tracking HTML pages in visited_urls)
- Total resources shows 0 (download_cache only populated for non-HTML resources)

## Phase 2: Statistics Tracking (NEXT)
**Goal**: Add proper counters for different resource types and operations.

### Implementation
- Add atomic counters to WebsiteMirror:
  - Pages crawled, HTML/CSS/JS/Image downloads
  - Success/error counts per resource type
  - Queue size tracking
  - **Total bytes downloaded (sum of all file sizes)**
- Track each download's size when saving files
- Properly track both HTML and non-HTML resources in statistics

### Deliverables
- Accurate statistics per resource type
- Proper success/failure counts
- Total download size in bytes
- Performance metrics (duration, throughput)

## Phase 3: Live Terminal Dashboard
**Goal**: Replace simple progress bar with rich, real-time statistics display.

### Implementation
- Use `indicatif::MultiProgress` for multi-line terminal UI
- Display live statistics from Phase 2 counters
- Show current downloads in progress (last 3-5 URLs)
- Update every 100ms without flicker

### Example Display
```
═══════════════════════════════════════
 Website Mirror - Running (2h 34m)
═══════════════════════════════════════
 Pages:     234 crawled, 89 queued
 HTML:      234 ✓   2 ✗
 CSS:        45 ✓   0 ✗
 JS:         78 ✓   1 ✗
 Images:    523 ✓  12 ✗

 Downloading (3/10):
 • /assets/style.css
 • /images/banner.jpg
 • /page/about.html
═══════════════════════════════════════
```

## Phase 4: Manifest System
**Goal**: Create a persistent inventory of all downloaded resources across runs.

### Implementation
- Maintain `.mirror/manifest.json` with:
  - List of all runs with timestamps
  - Complete resource inventory
  - URL to local file mappings
- Update incrementally during downloads
- Include error details and retry information

### Structure
```json
{
  "runs": [
    {
      "id": "2024-01-20_10-30-00",
      "start_time": "...",
      "end_time": "...",
      "stats": { ... },
      "log_file": "runs/2024-01-20_10-30-00/log.txt"
    }
  ],
  "resources": {
    "https://example.com/page": {
      "local_path": "example.com/page.html",
      "type": "html",
      "size": 12345,
      "downloaded_at": "...",
      "run_id": "2024-01-20_10-30-00"
    }
  }
}
```

## Phase 5: Resumable Downloads
**Goal**: Support continuing interrupted downloads using the manifest.

### Implementation
- On startup with `--continue`:
  - Load manifest to identify incomplete downloads
  - Skip already downloaded resources
  - Resume from last crawl depth
- Mark resources with status: pending|downloading|success|error
- Implement retry logic for failed resources

## Phase 6: Enhanced Reporting
**Goal**: Generate detailed reports for analysis.

### Implementation
- HTML report generation with:
  - Statistics visualization
  - Error analysis
  - Download timeline
  - Resource breakdown by domain
- Export options: CSV, JSON, Markdown

## Future Considerations

### Nice to Have
- Web UI for browsing manifest
- SQLite for URL mapping (better performance at scale)
- Download speed tracking and ETA
- Bandwidth throttling controls
- Resource deduplication detection

### Not Planned
- Complex structured logging frameworks
- Real-time metrics servers
- Database-backed state management

## Implementation Notes

### Threading Considerations
- Use `Arc<Mutex<T>>` for shared mutable state
- Use `AtomicUsize` for simple counters (lock-free)
- Keep critical sections small to avoid contention

### File Organization
```
output_dir/
├── .mirror/
│   ├── manifest.json          # Global manifest (Phase 4)
│   ├── url_map.db             # SQLite URL cache (Already implemented)
│   └── runs/
│       ├── 2024-01-20_10-30-00/
│       │   └── log.txt        # Full log output (Phase 1 ✅)
│       └── 2024-01-20_16-45-00/
│           └── log.txt
└── [downloaded content]
```

### Dependencies by Phase
- Phase 1: `log`, `env_logger`, `chrono`
- Phase 2: (none - uses std::sync::atomic)
- Phase 3: (already have `indicatif`)
- Phase 4: (already have `serde_json`)
- Phase 5: (none)
- Phase 6: `handlebars` or raw HTML generation