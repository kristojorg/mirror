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

use crate::persistent_state::{IgnoredInfo, PersistentState};

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
    persistent_state: Arc<Mutex<Option<Arc<PersistentState>>>>,
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
        stats_bar.enable_steady_tick(Duration::from_millis(100));
        stats_bar.set_message("📊 Initializing mirror...");

        let start_time = Instant::now();
        let persistent_state: Arc<Mutex<Option<Arc<PersistentState>>>> = Arc::new(Mutex::new(None));
        let run_stats: Arc<Mutex<RunStats>> = Arc::new(Mutex::new(RunStats::default()));

        // Clone for the logger closure
        let file_for_logger = Arc::clone(&log_file);
        let mp_for_logger = multi_progress.clone();

        // Set up env_logger with custom format
        env_logger::Builder::new()
            .filter_level(LevelFilter::Info)
            .format(move |_buf, record| {
                use colored::*;

                // Format message for terminal with colors (no emojis)
                let terminal_msg = match record.level() {
                    Level::Error => format!("{}", record.args()).red().to_string(),
                    Level::Warn => format!("{}", record.args()).yellow().to_string(),
                    Level::Info => format!("{}", record.args()),
                    Level::Debug => format!("{}", record.args()).dimmed().to_string(),
                    Level::Trace => format!("{}", record.args()).dimmed().to_string(),
                };

                // Print to terminal above the progress bar
                // The stats bar is updated by the background thread, not here
                let _ = mp_for_logger.println(&terminal_msg);

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
            start_time,
            persistent_state,
            multi_progress,
            stats_bar,
            run_stats,
        };

        // Start the stats updater thread
        logger.start_stats_updater();

        Ok(logger)
    }

    /// Get the run directory path
    pub fn run_dir(&self) -> &Path {
        &self.run_dir
    }

    /// Set the persistent state for pulling statistics
    pub fn set_persistent_state(&self, state: Arc<PersistentState>) {
        let mut persistent_state = self.persistent_state.lock().unwrap();
        *persistent_state = Some(state);
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
        let persistent_state = Arc::clone(&self.persistent_state);
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

            let message = if let Some(ref state) = *persistent_state.lock().unwrap() {
                let stats = state.get_statistics();
                let total_files: usize = stats.downloads.values().sum();
                let current_run_ignored = state.get_current_run_ignored_count();

                format!(
                    "⏱  {} │ THIS RUN: ⬇️  {} new │ ⏭️  {} skipped │ 🚫 {} ignored │ ❌ {} errors │ 💾 {} │ TOTAL: 📁 {} files │ 💾 {}",
                    duration,
                    HumanCount(run.downloaded as u64),
                    HumanCount(run.skipped as u64),
                    HumanCount(current_run_ignored as u64),
                    run.errors,
                    HumanBytes(run.bytes),
                    HumanCount(total_files as u64),
                    HumanBytes(stats.total_bytes)
                )
            } else {
                format!("⏱  {} │ Waiting for persistent state...", duration)
            };

            stats_bar.set_message(message);
        });
    }

    /// Write summary at the end of the run - creates summary from PersistentState
    pub fn write_summary(
        &self,
        start_time_str: String,
        duration_str: String,
        base_url: String,
    ) -> Result<()> {
        // Create summary from persistent state
        let summary = if let Some(ref state) = *self.persistent_state.lock().unwrap() {
            let stats = state.get_statistics();
            let ignored_entries = state.get_ignored_entries();

            // Extract data from the native statistics format
            let pages_crawled = stats.downloads.get("html").copied().unwrap_or(0)
                + stats.downloads.get("link").copied().unwrap_or(0);
            let css_files = stats.downloads.get("css").copied().unwrap_or(0);
            let js_files = stats.downloads.get("js").copied().unwrap_or(0)
                + stats.downloads.get("javascript").copied().unwrap_or(0);
            let images = stats.downloads.get("image").copied().unwrap_or(0);
            let other_files = stats
                .downloads
                .iter()
                .filter(|(key, _)| {
                    !["html", "link", "css", "js", "javascript", "image"].contains(&key.as_str())
                })
                .map(|(_, count)| count)
                .sum();
            let total_successful: usize = stats.downloads.values().sum();
            let total_failed: usize = stats.errors.values().sum();
            let total_ignored: usize = stats.ignored.values().sum();
            let error_messages = state.get_error_messages();

            RunSummary {
                start_time: start_time_str,
                end_time: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
                duration: duration_str,
                base_url,
                pages_crawled,
                css_files,
                js_files,
                images,
                other_files,
                successful_downloads: total_successful,
                failed_downloads: total_failed,
                ignored_urls: total_ignored,
                ignored_entries,
                total_bytes: stats.total_bytes,
                errors: error_messages,
            }
        } else {
            // Fallback if no persistent state (shouldn't happen)
            RunSummary {
                start_time: start_time_str,
                end_time: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
                duration: duration_str,
                base_url,
                pages_crawled: 0,
                css_files: 0,
                js_files: 0,
                images: 0,
                other_files: 0,
                successful_downloads: 0,
                failed_downloads: 0,
                ignored_urls: 0,
                ignored_entries: Vec::new(),
                total_bytes: 0,
                errors: Vec::new(),
            }
        };

        self.write_summary_internal(summary)
    }

    /// Internal method to write an already-created summary
    fn write_summary_internal(&self, summary: RunSummary) -> Result<()> {
        // Get run stats for this session
        let run_stats = self.run_stats.lock().unwrap().clone();

        // Get current run ignored count from persistent state
        let current_run_ignored = if let Some(ref state) = *self.persistent_state.lock().unwrap() {
            state.get_current_run_ignored_count()
        } else {
            0
        };

        // Print final summary to terminal
        println!("\n========================================");
        println!("Site Summary");
        println!("========================================");
        println!(
            "Total files: {}",
            HumanCount(summary.successful_downloads as u64)
        );
        println!("  HTML pages: {}", HumanCount(summary.pages_crawled as u64));
        println!("  CSS files: {}", HumanCount(summary.css_files as u64));
        println!("  JS files: {}", HumanCount(summary.js_files as u64));
        println!("  Images: {}", HumanCount(summary.images as u64));
        println!("  Other: {}", HumanCount(summary.other_files as u64));
        if summary.ignored_urls > 0 {
            println!(
                "  Ignored (rules): {}",
                HumanCount(summary.ignored_urls as u64)
            );
        }
        if summary.failed_downloads > 0 {
            println!(
                "Failed downloads: {}",
                HumanCount(summary.failed_downloads as u64)
            );
        }
        println!("Total size: {}", HumanBytes(summary.total_bytes));
        println!("========================================");

        // Print run-specific summary
        println!("\n========================================");
        println!("This Run Summary");
        println!("========================================");
        println!("Duration: {}", summary.duration);
        println!(
            "Downloaded: {} files",
            HumanCount(run_stats.downloaded as u64)
        );
        println!(
            "Skipped: {} files (already existed)",
            HumanCount(run_stats.skipped as u64)
        );
        if current_run_ignored > 0 {
            println!("Ignored (rules): {}", HumanCount(current_run_ignored as u64));
        }
        if run_stats.errors > 0 {
            println!("Errors: {}", HumanCount(run_stats.errors as u64));
        }
        println!("Data downloaded: {}", HumanBytes(run_stats.bytes));
        println!("========================================");

        // Write detailed summary to file
        if let Ok(mut file) = self.log_file.lock() {
            let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S");
            let msg = format!(
                "[{}] [INFO] ========================================\n\
                 [{}] [INFO] Site Summary\n\
                 [{}] [INFO] ========================================\n\
                 [{}] [INFO] Total files: {}\n\
                 [{}] [INFO]   HTML pages: {}\n\
                 [{}] [INFO]   CSS files: {}\n\
                 [{}] [INFO]   JS files: {}\n\
                 [{}] [INFO]   Images: {}\n\
                 [{}] [INFO]   Other: {}\n\
                 [{}] [INFO]   Ignored (rules): {}\n\
                 [{}] [INFO] Failed downloads: {}\n\
                 [{}] [INFO] Total size: {} ({} bytes)\n",
                timestamp,
                timestamp,
                timestamp,
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
                HumanCount(summary.ignored_urls as u64),
                timestamp,
                HumanCount(summary.failed_downloads as u64),
                timestamp,
                HumanBytes(summary.total_bytes),
                summary.total_bytes
            );
            let _ = file.write_all(msg.as_bytes());

            // Add run-specific summary to log file
            let run_msg = format!(
                "[{}] [INFO] ========================================\n\
                 [{}] [INFO] This Run Summary\n\
                 [{}] [INFO] ========================================\n\
                 [{}] [INFO] Duration: {}\n\
                 [{}] [INFO] Downloaded: {} files\n\
                 [{}] [INFO] Skipped: {} files (already existed)\n\
                 [{}] [INFO] Ignored (rules): {}\n\
                 [{}] [INFO] Errors: {}\n\
                 [{}] [INFO] Data downloaded: {} ({} bytes)\n",
                timestamp,
                timestamp,
                timestamp,
                timestamp,
                summary.duration,
                timestamp,
                HumanCount(run_stats.downloaded as u64),
                timestamp,
                HumanCount(run_stats.skipped as u64),
                timestamp,
                HumanCount(current_run_ignored as u64),
                timestamp,
                HumanCount(run_stats.errors as u64),
                timestamp,
                HumanBytes(run_stats.bytes),
                run_stats.bytes
            );
            let _ = file.write_all(run_msg.as_bytes());

            if !summary.errors.is_empty() {
                let errors_msg =
                    format!("[{}] [INFO] Errors: {}\n", timestamp, summary.errors.len());
                let _ = file.write_all(errors_msg.as_bytes());
                for error in &summary.errors {
                    let error_msg = format!("[{}] [ERROR]   - {}\n", timestamp, error);
                    let _ = file.write_all(error_msg.as_bytes());
                }
            }

            if !summary.ignored_entries.is_empty() {
                let ignored_msg = format!(
                    "[{}] [INFO] Ignored URLs: {}\n",
                    timestamp,
                    summary.ignored_entries.len()
                );
                let _ = file.write_all(ignored_msg.as_bytes());
                for (url, info) in &summary.ignored_entries {
                    let detail = format!("[{}] [INFO]   - {} :: {}\n", timestamp, url, info.reason);
                    let _ = file.write_all(detail.as_bytes());
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
    pub ignored_urls: usize,
    pub ignored_entries: Vec<(String, IgnoredInfo)>,
    pub total_bytes: u64,
    pub errors: Vec<String>,
}
