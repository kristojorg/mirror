use anyhow::{Context, Result};
use select::document::Document;
use select::predicate::{Attr, Name};
use url::Url;

/// Represents a resource found in HTML content
#[derive(Debug, Clone)]
pub struct ResourceLink {
    pub original_url: String, // Original URL as found in HTML
    pub resolved: String,     // Either absolute URL or output-relative path
    pub resource_type: ResourceType,
}

/// Resolve a URL relative to a base URL into an absolute URL
pub fn resolve_url(base_url: &Url, url: &str) -> Result<Url> {
    if url.starts_with("http://") || url.starts_with("https://") {
        Ok(Url::parse(url)?)
    } else if url.starts_with("//") {
        // Protocol-relative URL
        let scheme = base_url.scheme();
        let url_with_scheme = format!("{}:{}", scheme, url);
        Ok(Url::parse(&url_with_scheme)?)
    } else {
        // Relative URL
        Ok(base_url.join(url)?)
    }
}

/// Create a resource link with resolved URL
fn create_resource_link(
    base_url: &Url,
    url: &str,
    resource_type: ResourceType,
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

    // If it's already absolute, use as-is
    if url.starts_with("http://") || url.starts_with("https://") {
        return Ok(ResourceLink {
            original_url: url.to_string(),
            resolved: url.to_string(),
            resource_type,
        });
    }

    // Resolve relative URLs to absolute URLs using the base_url
    let resolved = resolve_url(base_url, url)?.to_string();

    Ok(ResourceLink {
        original_url: url.to_string(),
        resolved,
        resource_type,
    })
}

/// Extract background images from CSS content
fn extract_background_images_from_css(base_url: &Url, css_content: &str) -> Vec<ResourceLink> {
    let mut resources = Vec::new();

    // Extract background-image URLs from CSS content
    let background_patterns = [
        r#"background-image:\s*url\(['"]?([^'")\s]+)['"]?\)"#,
        r#"background:\s*url\(['"]?([^'")\s]+)['"]?\)"#,
    ];

    for pattern in &background_patterns {
        if let Ok(regex) = regex::Regex::new(pattern) {
            for cap in regex.captures_iter(css_content) {
                if let Some(url) = cap.get(1) {
                    if let Ok(resource) = create_resource_link(base_url, url.as_str(), ResourceType::Image) {
                        resources.push(resource);
                    }
                }
            }
        }
    }

    resources
}

/// Parser for extracting resources from CSS content
#[derive(Clone, Debug)]
pub struct CssParser {
    base_url: Url,
}

impl CssParser {
    pub fn new(base_url: &str) -> Result<Self> {
        let base_url = Url::parse(base_url)
            .with_context(|| format!("Failed to parse base URL: {}", base_url))?;

        Ok(Self { base_url })
    }

    /// Extract background images from CSS content
    pub fn extract_background_images(&self, css_content: &str) -> Vec<ResourceLink> {
        extract_background_images_from_css(&self.base_url, css_content)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub enum ResourceType {
    CSS,
    JavaScript,
    Image,
    Link,
    Other,
}

/// Parser for extracting resources from HTML content
/// Only responsible for finding and resolving URLs, not for path generation
pub struct HtmlParser {
    base_url: Url,
    document: Document,
}

impl std::fmt::Debug for HtmlParser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HtmlParser")
            .field("base_url", &self.base_url)
            .field("document", &"<Document>")
            .finish()
    }
}

impl HtmlParser {
    pub fn new(page_url: &str, html_content: &str) -> Result<Self> {
        let mut base_url = Url::parse(page_url)
            .with_context(|| format!("Failed to parse page URL: {}", page_url))?;

        let document = Document::from(html_content);

        // Look for <base href> element and update base_url if found
        if let Some(base_href) = document
            .find(Name("base"))
            .filter_map(|base| base.attr("href"))
            .next()
        {
            // If base href is absolute, use it. If relative, join to base_url
            if let Some(effective_base) =
                if base_href.starts_with("http://") || base_href.starts_with("https://") {
                    Url::parse(base_href).ok()
                } else {
                    base_url.join(base_href).ok()
                }
            {
                base_url = effective_base;
            }
        }

        Ok(Self { base_url, document })
    }

    /// Extract all resources from HTML content
    pub fn extract_resources(&self) -> Result<Vec<ResourceLink>> {
        let mut resources = Vec::new();

        // Extract CSS files
        for link in self.document.find(Name("link")) {
            if let Some(href) = link.attr("href") {
                if let Some(rel) = link.attr("rel") {
                    if rel.contains("stylesheet") {
                        if let Ok(resource) = create_resource_link(&self.base_url, href, ResourceType::CSS) {
                            resources.push(resource);
                        }
                    }
                }
            }
        }

        // Extract JavaScript files
        for script in self.document.find(Name("script")) {
            if let Some(src) = script.attr("src") {
                if let Ok(resource) = create_resource_link(&self.base_url, src, ResourceType::JavaScript) {
                    resources.push(resource);
                }
            }
        }

        // Extract images
        for img in self.document.find(Name("img")) {
            if let Some(src) = img.attr("src") {
                if let Ok(resource) = create_resource_link(&self.base_url, src, ResourceType::Image) {
                    resources.push(resource);
                }
            }
        }

        // Extract background images from inline styles
        for element in self.document.find(Attr("style", ())) {
            if let Some(style) = element.attr("style") {
                resources.extend(extract_background_images_from_css(&self.base_url, style));
            }
        }

        // Extract links
        for link in self.document.find(Name("a")) {
            if let Some(href) = link.attr("href") {
                if let Ok(resource) = create_resource_link(&self.base_url, href, ResourceType::Link) {
                    resources.push(resource);
                }
            }
        }

        Ok(resources)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_html_parser() {
        let parser = HtmlParser::new("https://example.com/page", "<html></html>").unwrap();
        assert_eq!(parser.base_url.as_str(), "https://example.com/page");
    }

    #[test]
    fn test_new_html_parser_invalid_url() {
        let result = HtmlParser::new("not-a-url", "<html></html>");
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

        let parser = HtmlParser::new("https://example.com", html_content).unwrap();
        let resources = parser.extract_resources().unwrap();

        assert_eq!(resources.len(), 4);

        let css_resource = resources
            .iter()
            .find(|r| r.resource_type == ResourceType::CSS)
            .unwrap();
        assert_eq!(css_resource.original_url, "/static/style.css");
        assert_eq!(
            css_resource.resolved,
            "https://example.com/static/style.css"
        );

        let js_resource = resources
            .iter()
            .find(|r| r.resource_type == ResourceType::JavaScript)
            .unwrap();
        assert_eq!(js_resource.original_url, "/static/script.js");
        assert_eq!(js_resource.resolved, "https://example.com/static/script.js");

        let img_resource = resources
            .iter()
            .find(|r| r.resource_type == ResourceType::Image)
            .unwrap();
        assert_eq!(img_resource.original_url, "/static/image.jpg");
        assert_eq!(
            img_resource.resolved,
            "https://example.com/static/image.jpg"
        );

        let link_resource = resources
            .iter()
            .find(|r| r.resource_type == ResourceType::Link)
            .unwrap();
        assert_eq!(link_resource.original_url, "/page");
        assert_eq!(link_resource.resolved, "https://example.com/page");
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

        let parser = HtmlParser::new("https://example.com", html_content).unwrap();
        let resources = parser.extract_resources().unwrap();

        assert_eq!(resources.len(), 3);

        let css_resource = resources
            .iter()
            .find(|r| r.resource_type == ResourceType::CSS)
            .unwrap();
        assert_eq!(
            css_resource.original_url,
            "https://cdn.example.com/style.css"
        );
        assert_eq!(css_resource.resolved, "https://cdn.example.com/style.css");
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

        let parser = HtmlParser::new("https://example.com/subdir/", html_content).unwrap();
        let resources = parser.extract_resources().unwrap();

        assert_eq!(resources.len(), 3);

        let css_resource = resources
            .iter()
            .find(|r| r.resource_type == ResourceType::CSS)
            .unwrap();
        assert_eq!(css_resource.original_url, "../style.css");
        assert_eq!(css_resource.resolved, "https://example.com/style.css");
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

        let parser = HtmlParser::new("https://example.com", html_content).unwrap();
        let resources = parser.extract_resources().unwrap();

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

        let parser = HtmlParser::new("https://example.com", html_content).unwrap();
        let resources = parser.extract_resources().unwrap();

        // Should still extract what it can
        assert!(resources.len() > 0);
    }

    #[test]
    fn test_resolve_url_absolute() {
        let base_url = Url::parse("https://example.com").unwrap();
        let result = resolve_url(&base_url, "https://cdn.example.com/style.css").unwrap();
        assert_eq!(result.as_str(), "https://cdn.example.com/style.css");
    }

    #[test]
    fn test_resolve_url_relative() {
        let base_url = Url::parse("https://example.com/subdir/").unwrap();
        let result = resolve_url(&base_url, "../style.css").unwrap();
        assert_eq!(result.as_str(), "https://example.com/style.css");
    }

    #[test]
    fn test_resolve_url_protocol_relative() {
        let base_url = Url::parse("https://example.com").unwrap();
        let result = resolve_url(&base_url, "//cdn.example.com/style.css").unwrap();
        assert_eq!(result.as_str(), "https://cdn.example.com/style.css");
    }

    #[test]
    fn test_extract_background_images_from_css() {
        let css_content = r#"
            .bg1 { background-image: url('/images/bg1.jpg'); }
            .bg2 { background: url('/images/bg2.jpg'); }
            .bg3 { background-image: url('/images/bg3.jpg'); }
        "#;

        let parser = CssParser::new("https://example.com").unwrap();
        let resources = parser.extract_background_images(css_content);

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

        let parser = CssParser::new("https://example.com").unwrap();
        let resources = parser.extract_background_images(css_content);

        assert_eq!(resources.len(), 2);
    }

    #[test]
    fn test_extract_background_images_from_css_no_matches() {
        let css_content = r#"
            .bg1 { background-color: red; }
            .bg2 { color: blue; }
        "#;

        let parser = CssParser::new("https://example.com").unwrap();
        let resources = parser.extract_background_images(css_content);

        assert_eq!(resources.len(), 0);
    }

    #[test]
    fn test_resource_link_clone() {
        let resource = ResourceLink {
            original_url: "/test.css".to_string(),
            resolved: "https://example.com/test.css".to_string(),
            resource_type: ResourceType::CSS,
        };

        let cloned = resource.clone();
        assert_eq!(cloned.original_url, resource.original_url);
        assert_eq!(cloned.resolved, resource.resolved);
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
        let parser = HtmlParser::new("https://example.com", "<html></html>").unwrap();
        let debug_str = format!("{:?}", parser);
        assert!(debug_str.contains("HtmlParser"));
        assert!(debug_str.contains("example.com"));
    }

    #[test]
    fn test_base_tag_with_directory() {
        let html_content = r#"
            <html>
                <head>
                    <base href="https://example.com/assets/">
                </head>
                <body>
                    <img src="pics/A.jpg">
                    <link rel="stylesheet" href="styles/main.css">
                </body>
            </html>
        "#;

        let parser = HtmlParser::new("https://example.com/page", html_content).unwrap();
        let resources = parser.extract_resources().unwrap();

        let img_resource = resources
            .iter()
            .find(|r| r.resource_type == ResourceType::Image)
            .unwrap();
        assert_eq!(
            img_resource.resolved,
            "https://example.com/assets/pics/A.jpg"
        );

        let css_resource = resources
            .iter()
            .find(|r| r.resource_type == ResourceType::CSS)
            .unwrap();
        assert_eq!(
            css_resource.resolved,
            "https://example.com/assets/styles/main.css"
        );
    }

    #[test]
    fn test_base_tag_with_filename() {
        // Test the PCS bug case: base href with a filename gets the filename dropped
        let html_content = r#"
            <html>
                <head>
                    <base href="https://www.procyclingstats.com/info.php">
                </head>
                <body>
                    <img src="images/uploads/Bert-Lip.jpg">
                </body>
            </html>
        "#;

        let parser = HtmlParser::new("https://www.procyclingstats.com/info/about-us", html_content).unwrap();
        let resources = parser.extract_resources().unwrap();

        let img_resource = resources
            .iter()
            .find(|r| r.resource_type == ResourceType::Image)
            .unwrap();
        // The base has a filename (info.php), so joining drops it and appends to the directory
        assert_eq!(
            img_resource.resolved,
            "https://www.procyclingstats.com/images/uploads/Bert-Lip.jpg"
        );
    }

    #[test]
    fn test_base_tag_relative() {
        let html_content = r#"
            <html>
                <head>
                    <base href="../assets/">
                </head>
                <body>
                    <img src="pic.jpg">
                </body>
            </html>
        "#;

        let parser = HtmlParser::new("https://example.com/pages/subdir/", html_content).unwrap();
        let resources = parser.extract_resources().unwrap();

        let img_resource = resources
            .iter()
            .find(|r| r.resource_type == ResourceType::Image)
            .unwrap();
        assert_eq!(
            img_resource.resolved,
            "https://example.com/pages/assets/pic.jpg"
        );
    }

    #[test]
    fn test_no_base_tag() {
        let html_content = r#"
            <html>
                <body>
                    <img src="images/pic.jpg">
                </body>
            </html>
        "#;

        let parser = HtmlParser::new("https://example.com/page", html_content).unwrap();
        let resources = parser.extract_resources().unwrap();

        let img_resource = resources
            .iter()
            .find(|r| r.resource_type == ResourceType::Image)
            .unwrap();
        // Without a base tag, should resolve relative to the page URL
        assert_eq!(img_resource.resolved, "https://example.com/images/pic.jpg");
    }
}
