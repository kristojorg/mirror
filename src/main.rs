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

    let mut mirror = WebsiteMirror::new(
        &args.url,
        &args.output_dir,
        args.max_depth,
        !args.respect_robots, // ignore_robots is the inverse of respect_robots
        true,                 // download_external is always true to ensure no 404s
        args.only_resources.clone(),
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
    fn test_parse_args_with_depth() {
        let args = vec![
            "website-mirror".to_string(),
            "https://example.com".to_string(),
            "-o".to_string(),
            "./output".to_string(),
            "-d".to_string(),
            "5".to_string(),
        ];

        let result = MirrorCommand::try_parse_from(args);
        assert!(result.is_ok());

        let cmd = result.unwrap();
        assert_eq!(cmd.url, "https://example.com");
        assert_eq!(cmd.output_dir.to_string_lossy(), "./output");
        assert_eq!(cmd.max_depth, 5);
    }

    #[test]
    fn test_parse_args_with_respect_robots() {
        let args = vec![
            "website-mirror".to_string(),
            "https://example.com".to_string(),
            "-o".to_string(),
            "./output".to_string(),
            "--respect-robots".to_string(),
        ];

        let result = MirrorCommand::try_parse_from(args);
        assert!(result.is_ok());

        let cmd = result.unwrap();
        assert_eq!(cmd.url, "https://example.com");
        assert_eq!(cmd.output_dir.to_string_lossy(), "./output");
        assert!(cmd.respect_robots);
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
}
