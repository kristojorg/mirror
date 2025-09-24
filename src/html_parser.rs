use anyhow::{Context, Result};
use select::document::Document;
use select::predicate::{Attr, Name};
use url::Url;

/// Represents a resource found in HTML content
#[derive(Debug, Clone)]
pub struct ResourceLink {
    pub original_url: String, // Original URL as found in HTML
    pub absolute_url: String, // Resolved absolute URL
    pub resource_type: ResourceType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceType {
    CSS,
    JavaScript,
    Image,
    Link,
    Other,
}

/// Parser for extracting resources from HTML content
/// Only responsible for finding and resolving URLs, not for path generation
#[derive(Clone, Debug)]
pub struct HtmlParser {
    base_url: Url,
}

impl HtmlParser {
    pub fn new(base_url: &str) -> Result<Self> {
        let base_url = Url::parse(base_url)
            .with_context(|| format!("Failed to parse base URL: {}", base_url))?;

        Ok(Self { base_url })
    }

    /// Extract all resources from HTML content
    /// If skip_url_resolution is true, relative paths are kept as-is (for saved HTML files)
    pub fn extract_resources_with_mode(
        &self,
        html_content: &str,
        skip_url_resolution: bool,
    ) -> Result<Vec<ResourceLink>> {
        let document = Document::from(html_content);
        let mut resources = Vec::new();

        // Extract CSS files
        for link in document.find(Name("link")) {
            if let Some(href) = link.attr("href") {
                if let Some(rel) = link.attr("rel") {
                    if rel.contains("stylesheet") {
                        if let Ok(resource) =
                            self.create_resource_link(href, ResourceType::CSS, skip_url_resolution)
                        {
                            resources.push(resource);
                        }
                    }
                }
            }
        }

        // Extract JavaScript files
        for script in document.find(Name("script")) {
            if let Some(src) = script.attr("src") {
                if let Ok(resource) =
                    self.create_resource_link(src, ResourceType::JavaScript, skip_url_resolution)
                {
                    resources.push(resource);
                }
            }
        }

        // Extract images
        for img in document.find(Name("img")) {
            if let Some(src) = img.attr("src") {
                if let Ok(resource) =
                    self.create_resource_link(src, ResourceType::Image, skip_url_resolution)
                {
                    resources.push(resource);
                }
            }
        }

        // Extract background images from inline styles
        for element in document.find(Attr("style", ())) {
            if let Some(style) = element.attr("style") {
                self.extract_background_images_from_css(style, &mut resources);
            }
        }

        // Extract links
        for link in document.find(Name("a")) {
            if let Some(href) = link.attr("href") {
                if let Ok(resource) =
                    self.create_resource_link(href, ResourceType::Link, skip_url_resolution)
                {
                    resources.push(resource);
                }
            }
        }

        Ok(resources)
    }

    /// Extract all resources from HTML content (convenience method)
    pub fn extract_resources(&self, html_content: &str) -> Result<Vec<ResourceLink>> {
        self.extract_resources_with_mode(html_content, false)
    }

    /// Create a resource link with resolved absolute URL (or keep relative if skip_resolution is true)
    fn create_resource_link(
        &self,
        url: &str,
        resource_type: ResourceType,
        skip_url_resolution: bool,
    ) -> Result<ResourceLink> {
        // Skip data URLs and other special schemes
        if url.starts_with("data:")
            || url.starts_with("javascript:")
            || url.starts_with("mailto:")
            || url.starts_with("tel:")
            || url.starts_with("#")
        {
            return Err(anyhow::anyhow!("Special URL scheme not supported"));
        }

        let absolute_url = if skip_url_resolution {
            // When processing saved HTML files, keep the relative paths as-is
            // These will be looked up in mirror_state to find the original URLs
            url.to_string()
        } else {
            // Normal mode: resolve to absolute URL
            self.resolve_url(url)?.to_string()
        };

        Ok(ResourceLink {
            original_url: url.to_string(),
            absolute_url,
            resource_type,
        })
    }

    /// Resolve URLs relative to the file into absolute URLs.
    pub fn resolve_url(&self, url: &str) -> Result<Url> {
        if url.starts_with("http://") || url.starts_with("https://") {
            Ok(Url::parse(url)?)
        } else if url.starts_with("//") {
            // Protocol-relative URL
            let scheme = self.base_url.scheme();
            let url_with_scheme = format!("{}:{}", scheme, url);
            Ok(Url::parse(&url_with_scheme)?)
        } else {
            // Relative URL
            Ok(self.base_url.join(url)?)
        }
    }

    /// Extract background images from CSS content
    pub fn extract_background_images_from_css(
        &self,
        css_content: &str,
        resources: &mut Vec<ResourceLink>,
    ) {
        // Extract background-image URLs from CSS content
        let background_patterns = [
            r#"background-image:\s*url\(['"]?([^'")\s]+)['"]?\)"#,
            r#"background:\s*url\(['"]?([^'")\s]+)['"]?\)"#,
        ];

        for pattern in &background_patterns {
            if let Ok(regex) = regex::Regex::new(pattern) {
                for cap in regex.captures_iter(css_content) {
                    if let Some(url) = cap.get(1) {
                        if let Ok(resource) =
                            self.create_resource_link(url.as_str(), ResourceType::Image, false)
                        {
                            resources.push(resource);
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_html_parser() {
        let parser = HtmlParser::new("https://example.com/page").unwrap();
        assert_eq!(parser.base_url.as_str(), "https://example.com/page");
    }

    #[test]
    fn test_new_html_parser_invalid_url() {
        let result = HtmlParser::new("not-a-url");
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_resources() {
        let html_content = r#"
            <html>
                <head>
                    <link rel="stylesheet" href="/static/style.css">
                    <script src="/static/script.js"></script>
                </head>
                <body>
                    <img src="/static/image.jpg" alt="test">
                    <a href="/page">Link</a>
                </body>
            </html>
        "#;

        let parser = HtmlParser::new("https://example.com").unwrap();
        let resources = parser.extract_resources(html_content).unwrap();

        assert_eq!(resources.len(), 4);

        let css_resource = resources
            .iter()
            .find(|r| r.resource_type == ResourceType::CSS)
            .unwrap();
        assert_eq!(css_resource.original_url, "/static/style.css");
        assert_eq!(
            css_resource.absolute_url,
            "https://example.com/static/style.css"
        );

        let js_resource = resources
            .iter()
            .find(|r| r.resource_type == ResourceType::JavaScript)
            .unwrap();
        assert_eq!(js_resource.original_url, "/static/script.js");
        assert_eq!(
            js_resource.absolute_url,
            "https://example.com/static/script.js"
        );

        let img_resource = resources
            .iter()
            .find(|r| r.resource_type == ResourceType::Image)
            .unwrap();
        assert_eq!(img_resource.original_url, "/static/image.jpg");
        assert_eq!(
            img_resource.absolute_url,
            "https://example.com/static/image.jpg"
        );

        let link_resource = resources
            .iter()
            .find(|r| r.resource_type == ResourceType::Link)
            .unwrap();
        assert_eq!(link_resource.original_url, "/page");
        assert_eq!(link_resource.absolute_url, "https://example.com/page");
    }

    #[test]
    fn test_extract_resources_with_absolute_urls() {
        let html_content = r#"
            <html>
                <head>
                    <link rel="stylesheet" href="https://cdn.example.com/style.css">
                    <script src="https://cdn.example.com/script.js"></script>
                </head>
                <body>
                    <img src="https://cdn.example.com/image.jpg" alt="test">
                </body>
            </html>
        "#;

        let parser = HtmlParser::new("https://example.com").unwrap();
        let resources = parser.extract_resources(html_content).unwrap();

        assert_eq!(resources.len(), 3);

        let css_resource = resources
            .iter()
            .find(|r| r.resource_type == ResourceType::CSS)
            .unwrap();
        assert_eq!(
            css_resource.original_url,
            "https://cdn.example.com/style.css"
        );
        assert_eq!(
            css_resource.absolute_url,
            "https://cdn.example.com/style.css"
        );
    }

    #[test]
    fn test_extract_resources_with_relative_urls() {
        let html_content = r#"
            <html>
                <head>
                    <link rel="stylesheet" href="../style.css">
                    <script src="./script.js"></script>
                </head>
                <body>
                    <img src="images/photo.jpg" alt="test">
                </body>
            </html>
        "#;

        let parser = HtmlParser::new("https://example.com/subdir/").unwrap();
        let resources = parser.extract_resources(html_content).unwrap();

        assert_eq!(resources.len(), 3);

        let css_resource = resources
            .iter()
            .find(|r| r.resource_type == ResourceType::CSS)
            .unwrap();
        assert_eq!(css_resource.original_url, "../style.css");
        assert_eq!(css_resource.absolute_url, "https://example.com/style.css");
    }

    #[test]
    fn test_extract_resources_with_data_urls() {
        let html_content = r#"
            <html>
                <body>
                    <img src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==" alt="test">
                </body>
            </html>
        "#;

        let parser = HtmlParser::new("https://example.com").unwrap();
        let resources = parser.extract_resources(html_content).unwrap();

        // Data URLs should be ignored
        assert_eq!(resources.len(), 0);
    }

    #[test]
    fn test_extract_resources_with_malformed_html() {
        let html_content = r#"
            <html>
                <head>
                    <link rel="stylesheet" href="/static/style.css
                    <script src="/static/script.js
                </head>
                <body>
                    <img src="/static/image.jpg alt="test">
                </body>
            </html>
        "#;

        let parser = HtmlParser::new("https://example.com").unwrap();
        let resources = parser.extract_resources(html_content).unwrap();

        // Should still extract what it can
        assert!(resources.len() > 0);
    }

    #[test]
    fn test_resolve_url_absolute() {
        let parser = HtmlParser::new("https://example.com").unwrap();
        let result = parser
            .resolve_url("https://cdn.example.com/style.css")
            .unwrap();
        assert_eq!(result.as_str(), "https://cdn.example.com/style.css");
    }

    #[test]
    fn test_resolve_url_relative() {
        let parser = HtmlParser::new("https://example.com/subdir/").unwrap();
        let result = parser.resolve_url("../style.css").unwrap();
        assert_eq!(result.as_str(), "https://example.com/style.css");
    }

    #[test]
    fn test_resolve_url_protocol_relative() {
        let parser = HtmlParser::new("https://example.com").unwrap();
        let result = parser.resolve_url("//cdn.example.com/style.css").unwrap();
        assert_eq!(result.as_str(), "https://cdn.example.com/style.css");
    }

    #[test]
    fn test_extract_background_images_from_css() {
        let css_content = r#"
            .bg1 { background-image: url('/images/bg1.jpg'); }
            .bg2 { background: url('/images/bg2.jpg'); }
            .bg3 { background-image: url('/images/bg3.jpg'); }
        "#;

        let parser = HtmlParser::new("https://example.com").unwrap();
        let mut resources = Vec::new();
        parser.extract_background_images_from_css(css_content, &mut resources);

        assert_eq!(resources.len(), 3);

        let urls: Vec<String> = resources.iter().map(|r| r.original_url.clone()).collect();
        assert!(urls.contains(&"/images/bg1.jpg".to_string()));
        assert!(urls.contains(&"/images/bg2.jpg".to_string()));
        assert!(urls.contains(&"/images/bg3.jpg".to_string()));
    }

    #[test]
    fn test_extract_background_images_from_css_with_quotes() {
        let css_content = r#"
            .bg1 { background-image: url("/images/bg1.jpg"); }
            .bg2 { background: url('/images/bg2.jpg'); }
        "#;

        let parser = HtmlParser::new("https://example.com").unwrap();
        let mut resources = Vec::new();
        parser.extract_background_images_from_css(css_content, &mut resources);

        assert_eq!(resources.len(), 2);
    }

    #[test]
    fn test_extract_background_images_from_css_no_matches() {
        let css_content = r#"
            .bg1 { background-color: red; }
            .bg2 { color: blue; }
        "#;

        let parser = HtmlParser::new("https://example.com").unwrap();
        let mut resources = Vec::new();
        parser.extract_background_images_from_css(css_content, &mut resources);

        assert_eq!(resources.len(), 0);
    }

    #[test]
    fn test_create_resource_link() {
        let parser = HtmlParser::new("https://example.com").unwrap();

        let resource = parser
            .create_resource_link("/style.css", ResourceType::CSS, false)
            .unwrap();
        assert_eq!(resource.original_url, "/style.css");
        assert_eq!(resource.absolute_url, "https://example.com/style.css");
        assert_eq!(resource.resource_type, ResourceType::CSS);

        let resource = parser
            .create_resource_link("/script.js", ResourceType::JavaScript, false)
            .unwrap();
        assert_eq!(resource.original_url, "/script.js");
        assert_eq!(resource.absolute_url, "https://example.com/script.js");
        assert_eq!(resource.resource_type, ResourceType::JavaScript);

        let resource = parser
            .create_resource_link("/image.jpg", ResourceType::Image, false)
            .unwrap();
        assert_eq!(resource.original_url, "/image.jpg");
        assert_eq!(resource.absolute_url, "https://example.com/image.jpg");
        assert_eq!(resource.resource_type, ResourceType::Image);

        let resource = parser
            .create_resource_link("/page", ResourceType::Link, false)
            .unwrap();
        assert_eq!(resource.original_url, "/page");
        assert_eq!(resource.absolute_url, "https://example.com/page");
        assert_eq!(resource.resource_type, ResourceType::Link);
    }

    #[test]
    fn test_create_resource_link_with_data_url() {
        let parser = HtmlParser::new("https://example.com").unwrap();
        let result =
            parser.create_resource_link("data:image/png;base64,data", ResourceType::Image, false);
        assert!(result.is_err());
    }

    #[test]
    fn test_create_resource_link_with_fragment() {
        let parser = HtmlParser::new("https://example.com").unwrap();
        let result = parser.create_resource_link("#fragment", ResourceType::Link, false);
        assert!(result.is_err());
    }

    #[test]
    fn test_create_resource_link_with_mailto() {
        let parser = HtmlParser::new("https://example.com").unwrap();
        let result =
            parser.create_resource_link("mailto:test@example.com", ResourceType::Link, false);
        assert!(result.is_err());
    }

    #[test]
    fn test_create_resource_link_with_tel() {
        let parser = HtmlParser::new("https://example.com").unwrap();
        let result = parser.create_resource_link("tel:+1234567890", ResourceType::Link, false);
        assert!(result.is_err());
    }

    #[test]
    fn test_create_resource_link_with_javascript() {
        let parser = HtmlParser::new("https://example.com").unwrap();
        let result =
            parser.create_resource_link("javascript:alert('test')", ResourceType::Link, false);
        assert!(result.is_err());
    }

    #[test]
    fn test_resource_link_clone() {
        let resource = ResourceLink {
            original_url: "/test.css".to_string(),
            absolute_url: "https://example.com/test.css".to_string(),
            resource_type: ResourceType::CSS,
        };

        let cloned = resource.clone();
        assert_eq!(cloned.original_url, resource.original_url);
        assert_eq!(cloned.absolute_url, resource.absolute_url);
        assert_eq!(cloned.resource_type, resource.resource_type);
    }

    #[test]
    fn test_resource_type_debug() {
        assert_eq!(format!("{:?}", ResourceType::CSS), "CSS");
        assert_eq!(format!("{:?}", ResourceType::JavaScript), "JavaScript");
        assert_eq!(format!("{:?}", ResourceType::Image), "Image");
        assert_eq!(format!("{:?}", ResourceType::Link), "Link");
        assert_eq!(format!("{:?}", ResourceType::Other), "Other");
    }

    #[test]
    fn test_resource_type_clone() {
        let css_type = ResourceType::CSS;
        let cloned = css_type.clone();
        assert_eq!(cloned, css_type);
    }

    #[test]
    fn test_html_parser_debug() {
        let parser = HtmlParser::new("https://example.com").unwrap();
        let debug_str = format!("{:?}", parser);
        assert!(debug_str.contains("HtmlParser"));
        assert!(debug_str.contains("example.com"));
    }
}
