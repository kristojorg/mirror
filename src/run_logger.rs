use anyhow::Result;
use chrono::Local;
use log::{Level, LevelFilter};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// A custom logger that writes to both terminal and file
pub struct RunLogger {
    log_file: Arc<Mutex<File>>,
    run_dir: PathBuf,
}

impl RunLogger {
    /// Initialize the logging system with dual output to console and file
    pub fn init(output_dir: &Path) -> Result<Self> {
        // Create run directory with timestamp
        let timestamp = Local::now().format("%Y-%m-%d_%H-%M-%S");
        let run_dir = output_dir.join(".mirror").join("runs").join(timestamp.to_string());
        fs::create_dir_all(&run_dir)?;

        // Create log file
        let log_path = run_dir.join("log.txt");
        let log_file = File::create(&log_path)?;
        let log_file = Arc::new(Mutex::new(log_file));

        // Clone for the logger closure
        let file_for_logger = Arc::clone(&log_file);

        // Set up env_logger with custom format
        env_logger::Builder::new()
            .filter_level(LevelFilter::Info)
            .format(move |buf, record| {
                // Format message for terminal (with colors/emojis)
                let terminal_msg = match record.level() {
                    Level::Error => format!("❌ {}", record.args()),
                    Level::Warn => format!("⚠️  {}", record.args()),
                    Level::Info => format!("{}", record.args()),
                    Level::Debug => format!("🔍 {}", record.args()),
                    Level::Trace => format!("🔬 {}", record.args()),
                };

                // Write to terminal
                writeln!(buf, "{}", terminal_msg)?;

                // Write to file (plain text with timestamp)
                if let Ok(mut file) = file_for_logger.lock() {
                    let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
                    let file_msg = format!(
                        "[{}] [{}] {}\n",
                        timestamp,
                        record.level(),
                        record.args()
                    );
                    let _ = file.write_all(file_msg.as_bytes());
                    let _ = file.flush();
                }

                Ok(())
            })
            .init();

        log::info!("========================================");
        log::info!("Website Mirror - Run Started");
        log::info!("Timestamp: {}", Local::now().format("%Y-%m-%d %H:%M:%S"));
        log::info!("Log directory: {}", run_dir.display());
        log::info!("========================================");

        Ok(RunLogger {
            log_file,
            run_dir,
        })
    }

    /// Get the run directory path
    pub fn run_dir(&self) -> &Path {
        &self.run_dir
    }

    /// Write summary at the end of the run
    pub fn write_summary(&self, summary: RunSummary) -> Result<()> {
        // Log summary to console and file
        log::info!("========================================");
        log::info!("Run Summary");
        log::info!("========================================");
        log::info!("Duration: {}", summary.duration);
        log::info!("Pages crawled: {}", summary.pages_crawled);
        log::info!("Total resources: {}", summary.total_resources);
        log::info!("Successful downloads: {}", summary.successful_downloads);
        log::info!("Failed downloads: {}", summary.failed_downloads);
        log::info!("Total size: {} bytes", summary.total_bytes);

        if !summary.errors.is_empty() {
            log::info!("Errors: {}", summary.errors.len());
            for error in &summary.errors {
                log::error!("  - {}", error);
            }
        }

        log::info!("========================================");
        Ok(())
    }
}

pub struct RunSummary {
    pub start_time: String,
    pub end_time: String,
    pub duration: String,
    pub base_url: String,
    pub pages_crawled: usize,
    pub total_resources: usize,
    pub successful_downloads: usize,
    pub failed_downloads: usize,
    pub total_bytes: u64,
    pub errors: Vec<String>,
}