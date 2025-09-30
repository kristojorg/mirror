use anyhow::Result;
use chrono::Local;
use clap::Parser;
use std::time::Instant;

use website_mirror::{cli::MirrorCommand, downloader::WebsiteMirror};

#[tokio::main]
async fn main() -> Result<()> {
    let start_time = Instant::now();
    let start_time_str = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let args = MirrorCommand::parse();

    // Handle full mirror option
    let (max_depth, max_concurrent, ignore_robots, download_external) = if args.full_mirror {
        // Full mirror: unlimited depth crawling of target site + all media files from any site
        (0, 100, true, true)
    } else {
        // Standard mirror: limited depth + all media files from any site (ensures no 404s)
        (
            args.max_depth,
            args.max_concurrent,
            args.ignore_robots,
            true,
        )
    };

    let mut mirror = WebsiteMirror::new(
        &args.url,
        &args.output_dir,
        max_depth,
        max_concurrent,
        ignore_robots,
        download_external,
        args.only_resources.clone(),
        args.convert_to_webp,
        args.no_proxy,
        args.ignore_patterns.clone(),
    )?;

    // Perform the mirroring
    let result = mirror.mirror_website().await;

    // Calculate duration
    let duration = start_time.elapsed();
    let duration_str = format!(
        "{}h {}m {}s",
        duration.as_secs() / 3600,
        (duration.as_secs() % 3600) / 60,
        duration.as_secs() % 60
    );

    // Write summary using the logger from the mirror - RunLogger creates summary from PersistentState
    let logger = mirror.get_run_logger();
    logger.write_summary(start_time_str, duration_str, args.url.clone())?;

    // Log final status
    if result.is_ok() {
        log::info!("Mirror completed successfully");
    } else {
        log::error!("Mirror completed with errors: {:?}", result);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_args() {
        let args = vec![
            "website-mirror".to_string(),
            "https://example.com".to_string(),
            "-o".to_string(),
            "./output".to_string(),
        ];

        let result = MirrorCommand::try_parse_from(args);
        assert!(result.is_ok());

        let cmd = result.unwrap();
        assert_eq!(cmd.url, "https://example.com");
        assert_eq!(cmd.output_dir.to_string_lossy(), "./output");
    }

    #[test]
    fn test_parse_args_with_full_mirror() {
        let args = vec![
            "website-mirror".to_string(),
            "https://example.com".to_string(),
            "-o".to_string(),
            "./output".to_string(),
            "--full-mirror".to_string(),
        ];

        let result = MirrorCommand::try_parse_from(args);
        assert!(result.is_ok());

        let cmd = result.unwrap();
        assert_eq!(cmd.url, "https://example.com");
        assert_eq!(cmd.output_dir.to_string_lossy(), "./output");
        assert!(cmd.full_mirror);
    }

    #[test]
    fn test_parse_args_with_convert_to_webp() {
        let args = vec![
            "website-mirror".to_string(),
            "https://example.com".to_string(),
            "-o".to_string(),
            "./output".to_string(),
            "--convert-to-webp".to_string(),
        ];

        let result = MirrorCommand::try_parse_from(args);
        assert!(result.is_ok());

        let cmd = result.unwrap();
        assert_eq!(cmd.url, "https://example.com");
        assert_eq!(cmd.output_dir.to_string_lossy(), "./output");
        assert!(cmd.convert_to_webp);
    }

    #[test]
    fn test_parse_args_with_only_resources() {
        let args = vec![
            "website-mirror".to_string(),
            "https://example.com".to_string(),
            "-o".to_string(),
            "./output".to_string(),
            "--only-resources".to_string(),
            "images,css".to_string(),
        ];

        let result = MirrorCommand::try_parse_from(args);
        assert!(result.is_ok());

        let cmd = result.unwrap();
        assert_eq!(cmd.url, "https://example.com");
        assert_eq!(cmd.output_dir.to_string_lossy(), "./output");
        assert_eq!(
            cmd.only_resources,
            Some(vec!["images".to_string(), "css".to_string()])
        );
    }

    #[test]
    fn test_parse_args_with_depth_and_concurrent() {
        let args = vec![
            "website-mirror".to_string(),
            "https://example.com".to_string(),
            "-o".to_string(),
            "./output".to_string(),
            "-d".to_string(),
            "5".to_string(),
            "-c".to_string(),
            "20".to_string(),
        ];

        let result = MirrorCommand::try_parse_from(args);
        assert!(result.is_ok());

        let cmd = result.unwrap();
        assert_eq!(cmd.url, "https://example.com");
        assert_eq!(cmd.output_dir.to_string_lossy(), "./output");
        assert_eq!(cmd.max_depth, 5);
        assert_eq!(cmd.max_concurrent, 20);
    }

    #[test]
    fn test_parse_args_with_ignore_robots() {
        let args = vec![
            "website-mirror".to_string(),
            "https://example.com".to_string(),
            "-o".to_string(),
            "./output".to_string(),
            "--ignore-robots".to_string(),
        ];

        let result = MirrorCommand::try_parse_from(args);
        assert!(result.is_ok());

        let cmd = result.unwrap();
        assert_eq!(cmd.url, "https://example.com");
        assert_eq!(cmd.output_dir.to_string_lossy(), "./output");
        assert!(cmd.ignore_robots);
    }

    #[test]
    fn test_parse_args_with_download_external() {
        let args = vec![
            "website-mirror".to_string(),
            "https://example.com".to_string(),
            "-o".to_string(),
            "./output".to_string(),
            "--download-external".to_string(),
        ];

        let result = MirrorCommand::try_parse_from(args);
        assert!(result.is_ok());

        let cmd = result.unwrap();
        assert_eq!(cmd.url, "https://example.com");
        assert_eq!(cmd.output_dir.to_string_lossy(), "./output");
        assert!(cmd.download_external);
    }

    #[test]
    fn test_parse_args_missing_url() {
        let args = vec![
            "website-mirror".to_string(),
            "-o".to_string(),
            "./output".to_string(),
        ];

        let result = MirrorCommand::try_parse_from(args);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_args_missing_output() {
        let args = vec![
            "website-mirror".to_string(),
            "https://example.com".to_string(),
        ];

        let result = MirrorCommand::try_parse_from(args);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_args_invalid_depth() {
        let args = vec![
            "website-mirror".to_string(),
            "https://example.com".to_string(),
            "-o".to_string(),
            "./output".to_string(),
            "-d".to_string(),
            "0".to_string(),
        ];

        let result = MirrorCommand::try_parse_from(args);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_args_invalid_concurrent() {
        let args = vec![
            "website-mirror".to_string(),
            "https://example.com".to_string(),
            "-o".to_string(),
            "./output".to_string(),
            "-c".to_string(),
            "0".to_string(),
        ];

        let result = MirrorCommand::try_parse_from(args);
        assert!(result.is_err());
    }
}
