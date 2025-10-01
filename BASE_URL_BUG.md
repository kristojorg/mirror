# Base URL Resolution Bug

## What is going wrong?
When the HTML parser scans a page, it turns every `src`/`href` it finds into an absolute URL by joining the attribute value with the page URL that was passed to `HtmlParser::new`. This works for most pages, but it ignores the HTML `<base href="…">` tag.

The PCS "About us" page sets `<base href="https://www.procyclingstats.com/info.php">`. Because the base ends with a filename, the browser (and `Url::join`) treat it as a document URL: the filename is dropped before the relative segment is appended. A link like:

```html
<img src="images/uploads/Bert-Lip.jpg">
```

should resolve to `https://www.procyclingstats.com/images/uploads/Bert-Lip.jpg`. Because our parser never checks the `<base>` element it instead joins against the page URL (`https://www.procyclingstats.com/info/about-us`). The parser therefore produces `https://www.procyclingstats.com/info/images/uploads/Bert-Lip.jpg`.

That bogus URL is then treated as an image download. The server serves HTML at that path, so the downloader logs the same URL twice (once as an image, once as HTML) and rewrites the mirrored page so that `src="account.php"`, which is what you’re seeing during resumption.

## Goal
Teach the parser to honour a page-level `<base href>` without changing any of the higher-level downloader logic.

## Proposed Fix (minimal changes)
1. **Cache the effective base URL inside `extract_resources`.**
   - Parse the DOM once at the top of `extract_resources`.
   - Look for the first `<base href>` element.
   - If `href` is absolute, use it. If it’s relative, join it to `self.base_url`. If parsing fails, ignore it.
   - Store the result in a local variable, e.g. `effective_base`.

2. **Resolve everything through that base.**
   - Add a tiny helper (e.g. `resolve_with_base(&Url, &str)`) that wraps the existing `resolve_url` logic but accepts a `&Url` to join against.
   - Inside the loops that build `ResourceLink`s, call `resolve_with_base(&effective_base, value)` instead of `self.resolve_url(value)`.
   - No need to thread the base URL through other modules; everything still happens inside `extract_resources`.

3. **Inline-style background images.**
   - `extract_background_images_from_css` currently calls `create_resource_link`. Give it a simple overload that accepts the `&Url` for the base so the CSS helper also resolves via `effective_base`.
   - Because `extract_background_images_from_css` is only used from `extract_resources` (for inline styles) and from the CSS downloader, pass `&effective_base` in those two spots.

4. **Tests.**
   - Add a unit test that feeds the parser an HTML string containing `<base href="https://example.com/assets/">` and a relative `<img src="pics/A.jpg">`, then assert that the resolved URL is `https://example.com/assets/pics/A.jpg`.
   - Verify that existing tests still pass (`cargo test`).

## Quick checklist for implementation
- [ ] Read `<base href>` in `extract_resources` and compute `effective_base`.
- [ ] Update `create_resource_link` usage to resolve via `effective_base`.
- [ ] Allow the CSS background-image helper to use the same base URL (pass the reference where needed).
- [ ] Add a regression unit test covering `<base>`.
- [ ] Run `cargo test`.

Once these steps are in place the parser will generate the correct absolute URLs, the downloader will fetch the right files, and the mirror won’t rewrite `<img>` tags to `.php` pages anymore.
