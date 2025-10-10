# Agent Instructions.

This file provides guidance to coding agents when working with code in this repository.

## Project Overview

This is a Rust CLI utility called `website-mirror` that downloads static copies of websites with all their resources (HTML, CSS, JavaScript, images) for offline viewing. The tool ensures "zero 404 guarantee" by downloading all media files regardless of their hosting location.

## Development Commands

### Building and Testing

```bash
# Build the project
cargo build

# Run tests (not fully working currently)
cargo test

# Run specific test file
cargo test integration_tests
cargo test webp_extension_tests

# Run with verbose output
cargo test -- --nocapture

# Install globally
cargo install --path .
```

### Running the Tool

```bash
# Basic usage (after building)
./target/debug/website-mirror https://example.com -o output

# Release version
./target/release/website-mirror https://example.com -o output

# If installed globally
website-mirror https://example.com -o output
```

### Development Tools

```bash
# Check for lint issues
cargo clippy

# Format code
cargo fmt

# Check dependencies
cargo tree

# Update dependencies
cargo update
```

## Architecture Overview

### Core Modules

- **`main.rs`** - CLI entry point, argument parsing integration
- **`cli.rs`** - Command-line interface using clap with comprehensive options
- **`downloader.rs`** - Core mirroring engine with HTTP client, concurrent downloads, and caching
- **`html_parser.rs`** - HTML parsing and resource extraction using html5ever
- **`file_manager.rs`** - File system operations, directory management, and WebP conversion
- **`lib.rs`** - Module exports and public API
- **`persistent_state.rs`** (planned) - Persistent crawl state management for reliable resumption
- **`mirror_state.rs`** (to be replaced) - Current state tracking, will be replaced by persistent_state.rs

### Key Components

1. **WebsiteMirror** (`downloader.rs`) - Main orchestrator that:
   - Manages HTTP client with SSL/TLS support
   - Implements concurrent download queue with priority system
   - Handles resource deduplication and caching
   - Processes robots.txt when not ignored

2. **HtmlParser** (`html_parser.rs`) - Extracts resources from HTML:
   - Finds CSS, JS, image, and font links
   - Handles both relative and absolute URLs
   - Categorizes resources by type (CSS, JS, images, fonts, other)

3. **FileManager** (`file_manager.rs`) - Manages file operations:
   - Creates directory structures
   - Saves downloaded content with proper extensions
   - Converts images to WebP when requested
   - Updates HTML content with local resource paths

### Resource Processing Flow

1. **Priority-based processing**: CSS/JS (critical, downloaded before the HTML page they're found on) → HTML (high) → Images (normal)
2. **Smart caching**: Each unique URL downloaded only once
3. **External resource handling**: Downloads from CDNs, AWS S3, etc.
4. **Path resolution**: Converts all resource and HTML URLs to relative local paths, and keeps a log of the url -> local path map in the state file.

### Persistent State & Resumption (New Architecture)

The system is transitioning to a fully persistent state model that enables reliable crawl resumption:

#### Current Issues (Being Replaced)
The current approach tries to reconstruct URLs from saved HTML files with rewritten local paths, which is fragile and complex because:
- URL → local path transformation is lossy and can't be reliably reversed
- State is only saved after successful downloads, missing pending resources
- Complex logic needed to extract and reverse-map links from saved HTML

#### New Persistent State Model (See PERSISTED_STATE.md)
The new architecture persists ALL crawl state to disk continuously:
- **Queue persistence**: Crawl queue saved to disk, no need to reconstruct from HTML
- **Visited tracking**: Downloaded and errored URLs tracked persistently
- **Processing state**: URLs being processed are tracked, moved back to queue on restart
- **Automatic resumption**: Run the same command again to continue from where it left off
- **Crash recovery**: Handles interruptions gracefully with no lost work

State file structure (`output_dir/.mirror/state.json`):
```json
{
  "statistics": {
    "urls_discovered": 1500,
    "downloads": { "html": 50, "css": 20, "js": 15, "images": 200 },
    "total_bytes": 104857600
  },
  "queue": [
    { "url": "https://example.com/page2", "depth": 1, "priority": "High", "resource_type": "Link" }
  ],
  "processing": [ "https://example.com/page3" ],
  "downloaded": {
    "https://example.com/index.html": {
      "local_path": "example.com/index.html",
      "resource_type": "html",
      "size_bytes": 10240,
      "downloaded_at": "2025-09-25T10:30:00Z"
    }
  },
  "errored": {
    "https://example.com/broken.jpg": {
      "error_message": "404 Not Found",
      "attempted_at": "2025-09-25T10:31:00Z",
      "resource_type": "image"
    }
  }
}
```

This eliminates all the complex resumption logic and provides true fault tolerance.

### Testing Strategy

- **Unit tests**: Embedded in each module using `#[cfg(test)]`
- **Integration tests**: Full workflow testing in `tests/integration_tests.rs`
- **WebP tests**: Specific testing for image conversion in `tests/webp_extension_tests.rs`
- **Benchmarks**: Performance testing in `benches/` directory

## Key Features

### CLI Options
- `--full-mirror`: Unlimited depth crawling with all external resources
- `--only-resources [types]`: Filter to specific resource types (images, css, js, html)
- `--convert-to-webp`: Convert JPEG/PNG to WebP for better compression
- `--ignore-robots`: Bypass robots.txt restrictions

### Technical Capabilities
- **Zero 404 guarantee**: All referenced resources are downloaded
- **Smart deduplication**: Prevents duplicate downloads using in-memory and disk caching
- **SSL/TLS support**: Handles modern certificate chains with rustls
- **Concurrent downloads**: Configurable parallelism with proper backpressure
- **Progress tracking**: Real-time status updates and resource type logging
- **Resumption**: Ability to resume interrupted downloads from the last completed resource

## Dependencies

### Core Dependencies
- **tokio**: Async runtime for concurrent operations
- **reqwest**: HTTP client with SSL support via rustls
- **clap**: Command-line argument parsing with derive macros
- **html5ever**: HTML parsing and DOM manipulation
- **url**: URL parsing and manipulation
- **anyhow**: Error handling and context

### Utility Dependencies
- **pathdiff**: Relative path calculation for HTML resource links
- **mime_guess**: MIME type detection for downloaded files
- **image + webp**: Image format conversion capabilities
- **indicatif + console + colored**: Progress bars and terminal output

## Common Development Tasks

### Adding New Resource Types
1. Update `ResourceType` enum in `html_parser.rs`
2. Add detection logic in `HtmlParser::extract_resources()`
3. Update resource categorization in downloader priority system

### Testing New Features
- Add unit tests directly in the relevant module
- Mock HTTP responses with `mockall` for isolated testing
