use std::collections::HashMap;

/// Rewrites URLs in HTML content to use local file paths
pub struct HtmlRewriter;

impl HtmlRewriter {
    pub fn new() -> Self {
        Self
    }

    /// Rewrite all URLs in HTML content using the provided mappings
    ///
    /// # Arguments
    /// * `html_content` - The HTML content to rewrite
    /// * `url_mappings` - Map of original URL -> local file path
    ///
    /// # Returns
    /// The HTML content with all URLs replaced with local paths
    pub fn rewrite_urls(
        &self,
        html_content: &str,
        url_mappings: &HashMap<String, String>,
    ) -> String {
        let mut result = html_content.to_string();

        // Sort mappings by length (longest first) to avoid partial replacements
        // e.g., replace "https://example.com/page.html" before "https://example.com/"
        let mut sorted_mappings: Vec<(&String, &String)> = url_mappings.iter().collect();
        sorted_mappings.sort_by(|a, b| b.0.len().cmp(&a.0.len()));

        for (original_url, local_path) in sorted_mappings {
            // Replace in common HTML attributes
            // Handle both single and double quotes

            // href="url" and href='url'
            result = result.replace(
                &format!(r#"href="{}""#, original_url),
                &format!(r#"href="{}""#, local_path),
            );
            result = result.replace(
                &format!(r#"href='{}'"#, original_url),
                &format!(r#"href='{}'"#, local_path),
            );

            // src="url" and src='url'
            result = result.replace(
                &format!(r#"src="{}""#, original_url),
                &format!(r#"src="{}""#, local_path),
            );
            result = result.replace(
                &format!(r#"src='{}'"#, original_url),
                &format!(r#"src='{}'"#, local_path),
            );

            // action="url" and action='url' (for forms)
            result = result.replace(
                &format!(r#"action="{}""#, original_url),
                &format!(r#"action="{}""#, local_path),
            );
            result = result.replace(
                &format!(r#"action='{}'"#, original_url),
                &format!(r#"action='{}'"#, local_path),
            );

            // Background images in inline styles
            // style="background: url(url)" or style="background-image: url(url)"
            // Handle with and without quotes
            result = result.replace(
                &format!(r#"url({})"#, original_url),
                &format!(r#"url({})"#, local_path),
            );
            result = result.replace(
                &format!(r#"url("{}")"#, original_url),
                &format!(r#"url("{}")"#, local_path),
            );
            result = result.replace(
                &format!(r#"url('{}')"#, original_url),
                &format!(r#"url('{}')"#, local_path),
            );

            // data attributes that might contain URLs
            result = result.replace(
                &format!(r#"data-src="{}""#, original_url),
                &format!(r#"data-src="{}""#, local_path),
            );
            result = result.replace(
                &format!(r#"data-src='{}'"#, original_url),
                &format!(r#"data-src='{}'"#, local_path),
            );
        }

        result
    }

    /// Create relative paths for better portability
    /// Converts absolute local paths to relative paths from the HTML file location
    pub fn make_paths_relative(
        &self,
        html_content: &str,
        html_file_path: &str,
        url_mappings: &HashMap<String, String>,
    ) -> String {
        // Calculate relative paths
        let mut relative_mappings = HashMap::new();
        for (original_url, local_path) in url_mappings {
            let relative = calculate_relative_path(html_file_path, local_path);
            relative_mappings.insert(original_url.clone(), relative);
        }

        // Use the regular rewrite with relative paths
        self.rewrite_urls(html_content, &relative_mappings)
    }
}

/// Calculate relative path from HTML file to resource
fn calculate_relative_path(from_html: &str, to_resource: &str) -> String {
    use std::path::Path;

    let from = Path::new(from_html);
    let to = Path::new(to_resource);

    // Get the directory of the HTML file
    let from_dir = from.parent().unwrap_or(Path::new(""));

    // Calculate relative path
    match pathdiff::diff_paths(to, from_dir) {
        Some(relative) => {
            let path_str = relative.to_string_lossy();
            // Ensure forward slashes for web compatibility
            path_str.replace('\\', "/")
        }
        None => to_resource.to_string(), // Fallback to absolute
    }
}

impl Default for HtmlRewriter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rewrite_urls_basic() {
        let rewriter = HtmlRewriter::new();
        let html = r#"<html>
<head>
    <link rel="stylesheet" href="https://example.com/style.css">
    <script src="https://example.com/script.js"></script>
</head>
<body>
    <img src="https://example.com/image.jpg" alt="test">
    <a href="https://example.com/page.html">Link</a>
</body>
</html>"#;

        let mut mappings = HashMap::new();
        mappings.insert(
            "https://example.com/style.css".to_string(),
            "example.com/style.css".to_string(),
        );
        mappings.insert(
            "https://example.com/script.js".to_string(),
            "example.com/script.js".to_string(),
        );
        mappings.insert(
            "https://example.com/image.jpg".to_string(),
            "example.com/image.webp".to_string(),
        );
        mappings.insert(
            "https://example.com/page.html".to_string(),
            "example.com/page.html".to_string(),
        );

        let result = rewriter.rewrite_urls(html, &mappings);

        assert!(result.contains(r#"href="example.com/style.css""#));
        assert!(result.contains(r#"src="example.com/script.js""#));
        assert!(result.contains(r#"src="example.com/image.webp""#));
        assert!(result.contains(r#"href="example.com/page.html""#));
    }

    #[test]
    fn test_rewrite_urls_single_quotes() {
        let rewriter = HtmlRewriter::new();
        let html = r#"<img src='https://example.com/image.jpg' alt='test'>"#;

        let mut mappings = HashMap::new();
        mappings.insert(
            "https://example.com/image.jpg".to_string(),
            "example.com/image.webp".to_string(),
        );

        let result = rewriter.rewrite_urls(html, &mappings);
        assert!(result.contains(r#"src='example.com/image.webp'"#));
    }

    #[test]
    fn test_rewrite_urls_inline_styles() {
        let rewriter = HtmlRewriter::new();
        let html = r#"<div style="background-image: url(https://example.com/bg.jpg);"></div>"#;

        let mut mappings = HashMap::new();
        mappings.insert(
            "https://example.com/bg.jpg".to_string(),
            "example.com/bg.webp".to_string(),
        );

        let result = rewriter.rewrite_urls(html, &mappings);
        assert!(result.contains("url(example.com/bg.webp)"));
    }

    #[test]
    fn test_rewrite_urls_quoted_css() {
        let rewriter = HtmlRewriter::new();
        let html = r#"<div style="background: url('https://example.com/bg.jpg');"></div>"#;

        let mut mappings = HashMap::new();
        mappings.insert(
            "https://example.com/bg.jpg".to_string(),
            "example.com/bg.webp".to_string(),
        );

        let result = rewriter.rewrite_urls(html, &mappings);
        assert!(result.contains("url('example.com/bg.webp')"));
    }

    #[test]
    fn test_rewrite_urls_data_attributes() {
        let rewriter = HtmlRewriter::new();
        let html = r#"<img data-src="https://example.com/lazy.jpg" alt="lazy">"#;

        let mut mappings = HashMap::new();
        mappings.insert(
            "https://example.com/lazy.jpg".to_string(),
            "example.com/lazy.webp".to_string(),
        );

        let result = rewriter.rewrite_urls(html, &mappings);
        assert!(result.contains(r#"data-src="example.com/lazy.webp""#));
    }

    #[test]
    fn test_rewrite_urls_form_action() {
        let rewriter = HtmlRewriter::new();
        let html = r#"<form action="https://example.com/submit"></form>"#;

        let mut mappings = HashMap::new();
        mappings.insert(
            "https://example.com/submit".to_string(),
            "example.com/submit.html".to_string(),
        );

        let result = rewriter.rewrite_urls(html, &mappings);
        assert!(result.contains(r#"action="example.com/submit.html""#));
    }

    #[test]
    fn test_rewrite_urls_no_partial_replacement() {
        let rewriter = HtmlRewriter::new();
        let html = r#"<a href="https://example.com/page">Link</a>
<a href="https://example.com/">Home</a>"#;

        let mut mappings = HashMap::new();
        mappings.insert(
            "https://example.com/".to_string(),
            "example.com/index.html".to_string(),
        );
        mappings.insert(
            "https://example.com/page".to_string(),
            "example.com/page.html".to_string(),
        );

        let result = rewriter.rewrite_urls(html, &mappings);

        // Should not partially replace the longer URL
        assert!(result.contains(r#"href="example.com/page.html""#));
        assert!(result.contains(r#"href="example.com/index.html""#));
        // Should not contain any partial replacements like "example.com/index.htmlpage"
        assert!(!result.contains("index.htmlpage"));
    }

    #[test]
    fn test_calculate_relative_path() {
        // Same directory
        assert_eq!(
            calculate_relative_path("example.com/index.html", "example.com/style.css"),
            "style.css"
        );

        // Resource in subdirectory
        assert_eq!(
            calculate_relative_path("example.com/index.html", "example.com/css/style.css"),
            "css/style.css"
        );

        // HTML in subdirectory, resource in parent
        assert_eq!(
            calculate_relative_path("example.com/blog/post.html", "example.com/style.css"),
            "../style.css"
        );

        // Different subdomains
        assert_eq!(
            calculate_relative_path("example.com/index.html", "cdn.example.com/style.css"),
            "../cdn.example.com/style.css"
        );
    }
}
