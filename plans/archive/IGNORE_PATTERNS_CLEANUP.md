**Findings**
- **Blocking** `WebsiteMirror::new` now takes an `ignore_patterns` argument, but several existing call sites still pass the old arity, so the project no longer compiles. Examples: `tests/integration_tests.rs:11`, `tests/webp_extension_tests.rs:33`, and `benches/benchmarks.rs:138` are still calling the nine-argument signature. Every caller needs to supply the new parameter (probably `None` when the feature isn’t used).
- **Blocking** URLs that were already persisted in the queue before the run (or before the new flag was introduced) are still fetched even if they match an ignore pattern. In `mirror_website` the dequeued task at `src/downloader.rs:373-404` flows straight into `download_and_process_url` without a pattern check, so a resumed crawl won’t respect the filter. You already have `should_ignore_url`; calling it before download (and marking the task as skipped in state/run logger) would prevent the unwanted request.

**Suggestions**
- **Non-blocking** The CLI uses `value_delimiter='|'` (`src/cli.rs:66-70`), but `|` is a very common regex operator. A single argument like `https://example.com/(foo|bar)` gets split into two broken patterns before the regex engine ever sees it. Allow repeated `--ignore-patterns` flags (the default clap behaviour) or choose a delimiter that won’t collide with valid regex syntax.
- **Non-blocking** `ignore_patterns` is cloned on every loop iteration (`src/downloader.rs:388-403`). Since `Regex` is cheap to clone this isn’t disastrous, but you can pass `&self.ignore_patterns` directly and tighten the async signature to avoid the extra allocation.

**Next Steps**
- 1. Patch all remaining `WebsiteMirror::new` callers to pass the new `ignore_patterns` argument, then `cargo test` to confirm the build is green.
- 2. Add an early skip for dequeued URLs that match the ignore list so resumptions honour the filter.

No tests were run for this review.
