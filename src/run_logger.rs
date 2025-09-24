use anyhow::Result;
use chrono::Local;
use indicatif::{HumanBytes, HumanCount, MultiProgress, ProgressBar, ProgressStyle};
use log::{Level, LevelFilter};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::mirror_state::MirrorState;

/// Runtime statistics for the current run only
#[derive(Debug, Default, Clone)]
pub struct RunStats {
    pub downloaded: usize, // Files actually downloaded this run
    pub skipped: usize,    // Files skipped (already existed)
    pub errors: usize,     // Errors this run
    pub bytes: u64,        // Bytes downloaded this run
}

/// A custom logger that writes to both terminal and file
pub struct RunLogger {
    log_file: Arc<Mutex<File>>,
    run_dir: PathBuf,
    start_time: Instant,
    mirror_state: Arc<Mutex<Option<Arc<MirrorState>>>>,
    multi_progress: MultiProgress,
    stats_bar: ProgressBar,
    run_stats: Arc<Mutex<RunStats>>, // Stats for this run only
}

impl RunLogger {
    /// Initialize the logging system with dual output to console and file
    pub fn init(output_dir: &Path) -> Result<Self> {
        // Create run directory with timestamp
        let timestamp = Local::now().format("%Y-%m-%d_%H-%M-%S");
        let run_dir = output_dir
            .join(".mirror")
            .join("runs")
            .join(timestamp.to_string());
        fs::create_dir_all(&run_dir)?;

        // Create log file
        let log_path = run_dir.join("log.txt");
        let log_file = File::create(&log_path)?;
        let log_file = Arc::new(Mutex::new(log_file));

        // Create MultiProgress for dashboard
        let multi_progress = MultiProgress::new();

        // Create a sticky stats bar at the top
        let stats_bar = multi_progress.add(ProgressBar::new(0));
        stats_bar.set_style(ProgressStyle::with_template("{msg}").unwrap());
        stats_bar.set_message("📊 Initializing mirror...");

        // Clone for the logger closure
        let file_for_logger = Arc::clone(&log_file);
        let mp_for_logger = multi_progress.clone();

        // Set up env_logger with custom format
        env_logger::Builder::new()
            .filter_level(LevelFilter::Info)
            .format(move |_buf, record| {
                // Format message for terminal (with colors/emojis)
                let terminal_msg = match record.level() {
                    Level::Error => format!("❌ {}", record.args()),
                    Level::Warn => format!("⚠️  {}", record.args()),
                    Level::Info => format!("{}", record.args()),
                    Level::Debug => format!("🔍 {}", record.args()),
                    Level::Trace => format!("🔬 {}", record.args()),
                };

                // Print to terminal above the progress bar
                mp_for_logger.println(&terminal_msg).unwrap_or(());

                // Write to file (plain text with timestamp)
                if let Ok(mut file) = file_for_logger.lock() {
                    let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
                    let file_msg =
                        format!("[{}] [{}] {}\n", timestamp, record.level(), record.args());
                    let _ = file.write_all(file_msg.as_bytes());
                    let _ = file.flush();
                }

                Ok(())
            })
            .init();

        // Log initial messages to file only
        if let Ok(mut file) = log_file.lock() {
            let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S");
            let msg = format!(
                "[{}] [INFO] ========================================\n\
                 [{}] [INFO] Website Mirror - Run Started\n\
                 [{}] [INFO] Timestamp: {}\n\
                 [{}] [INFO] Log directory: {}\n\
                 [{}] [INFO] ========================================\n",
                timestamp,
                timestamp,
                timestamp,
                timestamp,
                timestamp,
                run_dir.display(),
                timestamp
            );
            let _ = file.write_all(msg.as_bytes());
            let _ = file.flush();
        }

        let logger = RunLogger {
            log_file,
            run_dir,
            start_time: Instant::now(),
            mirror_state: Arc::new(Mutex::new(None)),
            multi_progress,
            stats_bar,
            run_stats: Arc::new(Mutex::new(RunStats::default())),
        };

        // Start the stats updater thread
        logger.start_stats_updater();

        Ok(logger)
    }

    /// Get the run directory path
    pub fn run_dir(&self) -> &Path {
        &self.run_dir
    }

    /// Set the mirror state for pulling statistics
    pub fn set_mirror_state(&self, state: Arc<MirrorState>) {
        let mut mirror_state = self.mirror_state.lock().unwrap();
        *mirror_state = Some(state);
    }

    /// Track a file that was actually downloaded this run
    pub fn track_downloaded(&self, bytes: u64) {
        let mut stats = self.run_stats.lock().unwrap();
        stats.downloaded += 1;
        stats.bytes += bytes;
    }

    /// Track a file that was skipped (already existed)
    pub fn track_skipped(&self) {
        let mut stats = self.run_stats.lock().unwrap();
        stats.skipped += 1;
    }

    /// Track a download error this run
    pub fn track_error(&self) {
        let mut stats = self.run_stats.lock().unwrap();
        stats.errors += 1;
    }

    /// Start a background thread to update the stats display
    fn start_stats_updater(&self) {
        let stats_bar = self.stats_bar.clone();
        let mirror_state = Arc::clone(&self.mirror_state);
        let run_stats = Arc::clone(&self.run_stats);
        let start_time = self.start_time;

        thread::spawn(move || loop {
            thread::sleep(Duration::from_millis(500));

            let elapsed = start_time.elapsed();
            let duration = format!(
                "{:02}:{:02}:{:02}",
                elapsed.as_secs() / 3600,
                (elapsed.as_secs() % 3600) / 60,
                elapsed.as_secs() % 60
            );

            let run = run_stats.lock().unwrap().clone();

            let message = if let Some(ref state) = *mirror_state.lock().unwrap() {
                let stats = state.get_statistics();
                let downloads = &stats.downloads;
                let total_success = downloads.html.success
                    + downloads.css.success
                    + downloads.js.success
                    + downloads.images.success
                    + downloads.other.success;

                format!(
                        "⏱  {} │ THIS RUN: ⬇️  {} new │ ⏭️  {} skipped │ ❌ {} errors │ 💾 {} │ TOTAL: 📁 {} files │ 💾 {}",
                        duration,
                        HumanCount(run.downloaded as u64),
                        HumanCount(run.skipped as u64),
                        run.errors,
                        HumanBytes(run.bytes),
                        HumanCount(total_success as u64),
                        HumanBytes(stats.total_bytes)
                    )
            } else {
                format!("⏱  {} │ Waiting for mirror state...", duration)
            };

            stats_bar.set_message(message);
        });
    }

    /// Write summary at the end of the run
    pub fn write_summary(&self, summary: RunSummary) -> Result<()> {
        // Print final summary to terminal
        println!("\n========================================");
        println!("Site Summary");
        println!("========================================");
        println!("Duration: {}", summary.duration);
        println!(
            "Total files: {}",
            HumanCount(summary.successful_downloads as u64)
        );
        println!("  HTML pages: {}", HumanCount(summary.pages_crawled as u64));
        println!("  CSS files: {}", HumanCount(summary.css_files as u64));
        println!("  JS files: {}", HumanCount(summary.js_files as u64));
        println!("  Images: {}", HumanCount(summary.images as u64));
        println!("  Other: {}", HumanCount(summary.other_files as u64));
        if summary.failed_downloads > 0 {
            println!(
                "Failed downloads: {}",
                HumanCount(summary.failed_downloads as u64)
            );
        }
        println!("Total size: {}", HumanBytes(summary.total_bytes));
        println!("========================================");

        // Write detailed summary to file
        if let Ok(mut file) = self.log_file.lock() {
            let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S");
            let msg = format!(
                "[{}] [INFO] ========================================\n\
                 [{}] [INFO] Site Summary\n\
                 [{}] [INFO] ========================================\n\
                 [{}] [INFO] Duration: {}\n\
                 [{}] [INFO] Total files: {}\n\
                 [{}] [INFO]   HTML pages: {}\n\
                 [{}] [INFO]   CSS files: {}\n\
                 [{}] [INFO]   JS files: {}\n\
                 [{}] [INFO]   Images: {}\n\
                 [{}] [INFO]   Other: {}\n\
                 [{}] [INFO] Failed downloads: {}\n\
                 [{}] [INFO] Total size: {} ({} bytes)\n",
                timestamp,
                timestamp,
                timestamp,
                timestamp,
                summary.duration,
                timestamp,
                HumanCount(summary.successful_downloads as u64),
                timestamp,
                HumanCount(summary.pages_crawled as u64),
                timestamp,
                HumanCount(summary.css_files as u64),
                timestamp,
                HumanCount(summary.js_files as u64),
                timestamp,
                HumanCount(summary.images as u64),
                timestamp,
                HumanCount(summary.other_files as u64),
                timestamp,
                HumanCount(summary.failed_downloads as u64),
                timestamp,
                HumanBytes(summary.total_bytes),
                summary.total_bytes
            );
            let _ = file.write_all(msg.as_bytes());

            if !summary.errors.is_empty() {
                let errors_msg =
                    format!("[{}] [INFO] Errors: {}\n", timestamp, summary.errors.len());
                let _ = file.write_all(errors_msg.as_bytes());
                for error in &summary.errors {
                    let error_msg = format!("[{}] [ERROR]   - {}\n", timestamp, error);
                    let _ = file.write_all(error_msg.as_bytes());
                }
            }

            let separator = format!(
                "[{}] [INFO] ========================================\n",
                timestamp
            );
            let _ = file.write_all(separator.as_bytes());
            let _ = file.flush();
        }

        Ok(())
    }
}

impl std::fmt::Debug for RunLogger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunLogger")
            .field("run_dir", &self.run_dir)
            .field("start_time", &self.start_time)
            .field("run_stats", &self.run_stats)
            .finish()
    }
}

pub struct RunSummary {
    pub start_time: String,
    pub end_time: String,
    pub duration: String,
    pub base_url: String,
    pub pages_crawled: usize,
    pub css_files: usize,
    pub js_files: usize,
    pub images: usize,
    pub other_files: usize,
    pub successful_downloads: usize,
    pub failed_downloads: usize,
    pub total_bytes: u64,
    pub errors: Vec<String>,
}
