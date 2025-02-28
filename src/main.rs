use blackbox::{AppConfig, AudioRecorder, CpalAudioProcessor, PerformanceTracker};
use std::path::Path;
use std::process;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;
use std::time::Duration;

fn main() {
    // Load configuration from file and environment variables
    let config = AppConfig::load();

    // Check if this is the first run and create a config file if needed
    if !Path::new("blackbox.toml").exists() {
        println!("First run detected. Creating default configuration file.");
        if let Err(e) = config.create_config_file("blackbox.toml") {
            eprintln!("Failed to create config file: {}", e);
        } else {
            println!("Created default configuration file: blackbox.toml");
            println!("You can edit this file to customize the recorder's behavior.");
        }
    }

    // Get configuration values
    let continuous_mode = config.get_continuous_mode();
    let performance_logging = config.get_performance_logging();
    let output_dir = config.get_output_dir();

    // Create output directory if it doesn't exist
    if !Path::new(&output_dir).exists() {
        if let Err(e) = std::fs::create_dir_all(&output_dir) {
            eprintln!("Failed to create output directory: {}", e);
            process::exit(1);
        }
    }

    // Start performance monitoring if enabled
    let performance_log_path = format!("{}/performance_log.csv", output_dir);
    let performance_tracker = PerformanceTracker::new(
        performance_logging,
        &performance_log_path,
        100, // Keep the last 100 measurements
        60,  // Sample every 60 seconds
    );

    if performance_logging {
        println!(
            "Performance monitoring enabled. Logs will be written to {}",
            performance_log_path
        );
        performance_tracker.start();
    }

    // Create a flag for handling clean shutdown
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();

    // Set up signal handling for clean shutdown
    ctrlc::set_handler(move || {
        println!("Received shutdown signal, stopping recording...");
        r.store(false, Ordering::SeqCst);
    })
    .expect("Error setting Ctrl-C handler");

    // Create the audio processor with CPAL implementation
    let processor = match CpalAudioProcessor::new() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Error initializing audio processor: {}", e);
            process::exit(1);
        }
    };

    // Create the recorder with our processor and config
    let mut recorder = AudioRecorder::with_config(processor, config);

    // Start recording
    match recorder.start_recording() {
        Ok(message) => println!("{}", message),
        Err(e) => {
            eprintln!("Error during recording: {}", e);
            process::exit(1);
        }
    }

    // In normal mode, wait for the recording to complete based on the duration
    // In continuous mode, keep running until we receive a shutdown signal
    if continuous_mode {
        println!("Running in continuous mode. Press Ctrl+C to stop.");

        // Keep the application running until a shutdown signal is received
        while running.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_secs(1));

            // Periodically report performance if monitoring is enabled
            if performance_logging && running.load(Ordering::SeqCst) {
                if let Some(metrics) = performance_tracker.get_current_metrics() {
                    if metrics.cpu_usage > 20.0 || metrics.memory_percent > 5.0 {
                        println!(
                            "Performance alert: CPU: {:.2}%, Memory: {:.2}MB ({:.2}%)",
                            metrics.cpu_usage,
                            metrics.memory_usage as f32 / 1024.0 / 1024.0,
                            metrics.memory_percent
                        );
                    }
                }
            }
        }

        println!("Shutting down recorder...");
        // The recorder will be dropped at the end of the function,
        // which will cause the audio processor to be finalized
    } else {
        // In normal mode, the recorder will automatically stop after the specified duration
        let duration = recorder.config.get_duration();

        println!("Recording for {} seconds...", duration);
        thread::sleep(Duration::from_secs(duration));
        println!("Recording complete.");
    }

    // Stop performance tracking if it was enabled
    if performance_logging {
        performance_tracker.stop();
        println!("Performance monitoring stopped.");

        // Print final performance statistics
        if let Some(avg_metrics) = performance_tracker.get_average_metrics() {
            println!(
                "Average performance: CPU: {:.2}%, Memory: {:.2}MB ({:.2}%)",
                avg_metrics.cpu_usage,
                avg_metrics.memory_usage as f32 / 1024.0 / 1024.0,
                avg_metrics.memory_percent
            );
        }
    }

    // All resources will be cleaned up when the program exits
}
