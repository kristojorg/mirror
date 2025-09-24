use crate::html_parser::ResourceType;
use anyhow::{Context, Result};
use std::path::PathBuf;
use url::Url;

/// Centralized URL to local file path mapping
///
/// This module is responsible for converting absolute URLs to local file paths,
/// handling different resource types and applying transformations like WebP conversion.
#[derive(Debug, Clone)]
pub struct UrlMapper {
    convert_to_webp: bool,
}

impl UrlMapper {
    pub fn new(convert_to_webp: bool) -> Result<Self> {
        Ok(Self { convert_to_webp })
    }

    /// Normalize root URL for consistent duplicate checking
    /// Only normalizes the root URL case: example.com → example.com/
    /// This ensures both example.com and example.com/ are treated as the same URL
    pub fn normalize_root_url(url: &str) -> String {
        if let Ok(mut parsed_url) = Url::parse(url) {
            let path = parsed_url.path();

            // Special case: root URLs without trailing slash should get one added
            // This makes example.com and example.com/ equivalent
            if path.is_empty() || path == "/" {
                parsed_url.set_path("/");
            }

            parsed_url.to_string()
        } else {
            // If parsing fails, return the original URL
            url.to_string()
        }
    }

    /// Convert an absolute URL to local file path based on resource type
    /// This is the main entry point for URL to path conversion
    pub fn url_to_local_path(&self, url: &str, resource_type: &ResourceType) -> Result<PathBuf> {
        // Skip special URL schemes
        if url.starts_with("data:")
            || url.starts_with("javascript:")
            || url.starts_with("mailto:")
            || url.starts_with("tel:")
            || url.starts_with("#")
        {
            return Err(anyhow::anyhow!("Special URL scheme not supported: {}", url));
        }

        // Parse the URL (it should already be absolute)
        let parsed_url =
            Url::parse(url).with_context(|| format!("Failed to parse URL: {}", url))?;

        match resource_type {
            ResourceType::Link => self.html_url_to_path(&parsed_url),
            ResourceType::CSS => self.css_url_to_path(&parsed_url),
            ResourceType::JavaScript => self.js_url_to_path(&parsed_url),
            ResourceType::Image => self.image_url_to_path(&parsed_url),
            ResourceType::Other => self.generic_url_to_path(&parsed_url),
        }
    }

    /// Convert HTML URLs to paths
    /// - URLs ending with / get index.html appended
    /// - URLs without extension get .html appended
    /// - URLs with .html/.htm keep their extension
    fn html_url_to_path(&self, url: &Url) -> Result<PathBuf> {
        let mut path = self.build_base_path(url)?;
        let url_path = url.path();

        // If ends with /, add index.html
        if url_path.ends_with('/') {
            path = path.join("index.html");
        }
        // If path doesn't have an extension, add .html
        else if path.extension().is_none() {
            path.set_extension("html");
        }
        // Otherwise keep the existing extension (.html, .htm, etc.)

        Ok(path)
    }

    /// Convert CSS URLs to paths
    fn css_url_to_path(&self, url: &Url) -> Result<PathBuf> {
        let mut path = self.build_base_path(url)?;

        // Always ensure .css extension for CSS files
        // This handles cases like fonts.googleapis.com/css?family=Lato
        // where query params get appended but we lose the extension
        if path.extension().is_none() {
            if url.path().ends_with('/') {
                path = path.join("index.css");
            } else {
                path.set_extension("css");
            }
        }

        Ok(path)
    }

    /// Convert JavaScript URLs to paths
    fn js_url_to_path(&self, url: &Url) -> Result<PathBuf> {
        let mut path = self.build_base_path(url)?;

        // Ensure .js extension if missing
        if path.extension().is_none() && !url.path().ends_with('/') {
            path.set_extension("js");
        }

        Ok(path)
    }

    /// Convert image URLs to paths (handles WebP conversion)
    fn image_url_to_path(&self, url: &Url) -> Result<PathBuf> {
        let mut path = self.build_base_path(url)?;

        // Handle WebP conversion if enabled
        if self.convert_to_webp {
            if let Some(ext) = path.extension() {
                let ext_str = ext.to_string_lossy().to_lowercase();
                if matches!(ext_str.as_str(), "jpg" | "jpeg" | "png") {
                    path.set_extension("webp");
                }
            }
        }

        Ok(path)
    }

    /// Convert generic URLs to paths (preserves extension if present)
    fn generic_url_to_path(&self, url: &Url) -> Result<PathBuf> {
        self.build_base_path(url)
    }

    /// Build the base path from URL (includes host and sanitized path)
    /// Returns a relative path (without output_dir prefix)
    fn build_base_path(&self, url: &Url) -> Result<PathBuf> {
        let mut path = PathBuf::new();

        // Add host to path
        if let Some(host) = url.host_str() {
            path.push(host);
        } else {
            path.push("localhost");
        }

        // Add URL path segments
        let url_path = url.path();

        // Remove leading slash and process path
        let clean_path = if url_path.starts_with('/') {
            &url_path[1..]
        } else {
            url_path
        };

        // Handle root path
        if clean_path.is_empty() || clean_path == "/" {
            return Ok(path);
        }

        // Add path segments
        for segment in clean_path.split('/').filter(|s| !s.is_empty()) {
            path.push(self.sanitize_path_segment(segment));
        }

        // Handle query parameters if present
        // Use -- as separator and replace special chars with _
        if let Some(query) = url.query() {
            if !query.is_empty() {
                let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("index");
                let sanitized_query = self.sanitize_path_segment(query);
                let new_name = format!("{}--{}", filename, sanitized_query);
                path.set_file_name(new_name);
            }
        }

        Ok(path)
    }

    /// Sanitize path segment for filesystem compatibility
    /// Simple approach: replace problematic chars with underscore
    fn sanitize_path_segment(&self, segment: &str) -> String {
        segment
            .chars()
            .map(|c| match c {
                // Filesystem-problematic characters
                '?' | '&' | '=' | '#' | '*' | '<' | '>' | '|' | ':' | '"' | '\\' | '/' => '_',
                // Spaces
                ' ' => '_',
                // Keep alphanumeric, dots, dashes, underscores
                c if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' => c,
                // Replace everything else
                _ => '_',
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_html_url_to_path() {
        let mapper = UrlMapper::new(false).unwrap();

        // Test root URL
        let path = mapper
            .url_to_local_path("https://example.com/", &ResourceType::Link)
            .unwrap();
        assert!(path.ends_with("example.com/index.html"));

        // Test directory URL
        let path = mapper
            .url_to_local_path("https://example.com/docs/", &ResourceType::Link)
            .unwrap();
        assert!(path.ends_with("example.com/docs/index.html"));

        // Test page without extension - should now be .html not /index.html
        let path = mapper
            .url_to_local_path("https://example.com/about", &ResourceType::Link)
            .unwrap();
        assert!(path.ends_with("example.com/about.html"));

        // Test page with .html extension
        let path = mapper
            .url_to_local_path("https://example.com/page.html", &ResourceType::Link)
            .unwrap();
        assert!(path.ends_with("example.com/page.html"));
    }

    #[test]
    fn test_image_url_to_path_with_webp() {
        let mapper = UrlMapper::new(true).unwrap();

        // Test JPEG conversion
        let path = mapper
            .url_to_local_path("https://example.com/photo.jpg", &ResourceType::Image)
            .unwrap();
        assert!(path.ends_with("example.com/photo.webp"));

        // Test PNG conversion
        let path = mapper
            .url_to_local_path("https://example.com/logo.png", &ResourceType::Image)
            .unwrap();
        assert!(path.ends_with("example.com/logo.webp"));

        // Test GIF (no conversion)
        let path = mapper
            .url_to_local_path("https://example.com/anim.gif", &ResourceType::Image)
            .unwrap();
        assert!(path.ends_with("example.com/anim.gif"));
    }

    #[test]
    fn test_css_url_to_path() {
        let mapper = UrlMapper::new(false).unwrap();

        // Test CSS with extension
        let path = mapper
            .url_to_local_path("https://example.com/style.css", &ResourceType::CSS)
            .unwrap();
        assert!(path.ends_with("example.com/style.css"));

        // Test CSS without extension
        let path = mapper
            .url_to_local_path("https://example.com/style", &ResourceType::CSS)
            .unwrap();
        assert!(path.ends_with("example.com/style.css"));
    }

    #[test]
    fn test_query_parameter_handling() {
        let mapper = UrlMapper::new(false).unwrap();

        // Now using -- as separator
        let path = mapper
            .url_to_local_path("https://example.com/style.css?v=1.2.3", &ResourceType::CSS)
            .unwrap();
        assert!(path.to_string_lossy().contains("style.css--v_1.2.3"));
    }

    #[test]
    fn test_sanitize_path_segment() {
        let mapper = UrlMapper::new(false).unwrap();

        assert_eq!(
            mapper.sanitize_path_segment("normal-file.txt"),
            "normal-file.txt"
        );
        assert_eq!(
            mapper.sanitize_path_segment("file with spaces"),
            "file_with_spaces"
        );
        assert_eq!(
            mapper.sanitize_path_segment("file?query=value"),
            "file_query_value"
        );
        assert_eq!(
            mapper.sanitize_path_segment("file#fragment"),
            "file_fragment"
        );
    }
}
