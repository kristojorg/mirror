# WebP Removal Plan

## Goals
- Eliminate the optional WebP conversion feature without breaking core mirroring functionality.
- Preserve existing behaviour for downloading and rewriting resources, apart from no longer transforming image formats or extensions.
- Leave the codebase, tests, and docs consistent and free of dead WebP references.

## Current WebP Touchpoints
- CLI flag plumbing (`src/cli.rs`, `src/main.rs`) exposes the `--convert-to-webp` option and threads it through to `WebsiteMirror`.
- `WebsiteMirror` stores and propagates the flag, calls `perform_comprehensive_webp_replacement`, and performs image transcoding via `convert_to_webp_static` (`src/downloader.rs`).
- `UrlMapper` rewrites image extensions to `.webp` when the flag is set (`src/url_mapper.rs`).
- HTML rewriting tests expect `.webp` extensions (`src/html_rewriter.rs`).
- Dedicated WebP test coverage in `tests/webp_extension_tests.rs` and related integration tests.
- Dependencies on `image` and `webp` crates (`Cargo.toml`).
- Documentation references in `README.md`, `CLAUDE.md`, `LOGGING_FORMAT.md`, `TODO.md`, and design docs (`URL_REWRITING_SIMPLIFIED.md`).

## Implementation Steps
1. **Remove CLI Surface Area**
   - Delete the `convert_to_webp` field, help text, and assertions from `MirrorCommand` in `src/cli.rs` and its unit tests.
   - Update `src/main.rs` to stop reading the flag or passing it into `WebsiteMirror::new`, and drop the related test case (`test_parse_args_with_convert_to_webp`).

2. **Simplify `WebsiteMirror` Configuration**
   - Remove the `convert_to_webp` field from `WebsiteMirror` (`src/downloader.rs:68`) and from the constructor parameters/signature.
   - Adjust all internal method signatures (`mirror_website`, `download_and_process_html`, `download_and_process_css`, `download_resource`, etc.) to stop accepting or forwarding the flag.
   - Delete `perform_comprehensive_webp_replacement` and `convert_to_webp_static`, along with any call sites.
   - Replace the image conversion branch in `download_resource` with a straight pass-through of the original bytes and remove the log statements tied to conversion ratios.

3. **Update URL Mapping Logic**
   - Remove the `convert_to_webp` field from `UrlMapper` (`src/url_mapper.rs`) and simplify `UrlMapper::new` accordingly.
   - Strip the `.webp` extension swapping logic from `image_url_to_path` so image paths retain their original file extensions.
   - Ensure the rest of the path sanitisation code still works without the flag (no other behavioural changes expected).

4. **Adjust HTML Rewriting Expectations**
   - Update `src/html_rewriter.rs` tests so mappings expect original image extensions rather than `.webp`.
   - Confirm no additional WebP-specific rewriting remains in the implementation; if any residual logic exists, remove it.

5. **Clean Up Tests**
   - Delete (or repurpose) `tests/webp_extension_tests.rs`; retain only the portions that still add value, such as verifying that HTML rewriting honours local mappings without format conversion.
   - Update integration tests that rely on the flag, notably `test_webp_conversion_flag` and the WebP workflow scenario in `tests/integration_tests.rs`, so they either disappear or assert the new behaviour (images keep their source extensions).
   - Prune unit tests in `src/downloader.rs` that cover the removed helpers (`convert_to_webp_static`, flag propagation, etc.).

6. **Prune Dependencies and Imports**
   - Remove `image` and `webp` from `[dependencies]` in `Cargo.toml` and drop corresponding `use` statements (`use webp::Encoder;`, `image::load_from_memory`, etc.).
   - Run `cargo update -p image -p webp` if needed after editing `Cargo.toml` to ensure Cargo.lock remains consistent (or regenerate the lock file per repo standards).

7. **Documentation & Guidance Updates**
   - Excise WebP references from user-facing docs: `README.md`, option tables, examples, feature lists, and usage notes.
   - Update contributor guidance in `CLAUDE.md`, `LOGGING_FORMAT.md`, and any design doc sections (e.g., remove Step 4 in `URL_REWRITING_SIMPLIFIED.md`).
   - Remove the WebP bullet from `TODO.md` and add any migration notes about the removal if necessary.

8. **Sanity Checks & Backwards Compatibility**
   - Validate that persistent state handling (`state.json`) and previously cached entries keep working. Document that older runs may have `.webp` files on disk but future runs will leave them unchanged; no migration is strictly required but mention it in release notes if appropriate.
   - Ensure logging still reads sensibly after removing conversion-specific messages.

9. **Verification**
   - Run `cargo fmt` and `cargo clippy` to confirm formatting and linting.
   - Execute the full test suite (`cargo test`), paying particular attention to integration coverage that touches resource downloads.
   - Optionally perform a manual smoke test against a small site to confirm images download with their original extensions and HTML rewriting still points to the saved assets.

## Deliverables
- Updated source, tests, and documentation free of WebP-specific code.
- Clean dependency tree (no `image`/`webp`).
- Confirmation in release notes or CHANGELOG (if maintained) that WebP conversion has been removed.
