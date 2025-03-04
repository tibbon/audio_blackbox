// We're providing a simplified implementation of the macOS menu bar
// that displays a basic UI and allows recording functionality

// Import menu bar implementation but don't use it directly
#[cfg(feature = "menu-bar")]
mod menu_bar;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use std::thread;
use std::process::Command;

use crate::AppConfig;
use crate::AudioRecorder;
use crate::AudioProcessor;
use crate::CpalAudioProcessor;

// Create a wrapper that will be used by the application
pub struct MenuBarApp {
    is_recording: Arc<Mutex<bool>>,
    recorder: Arc<Mutex<Option<AudioRecorder<CpalAudioProcessor>>>>,
    config: Arc<Mutex<AppConfig>>,
    recording_start_time: Arc<Mutex<Option<Instant>>>,
}

impl MenuBarApp {
    pub fn new() -> Self {
        println!("Creating MenuBarApp (simplified implementation)");
        
        // Initialize variables
        let is_recording = Arc::new(Mutex::new(false));
        let recorder = Arc::new(Mutex::new(None));
        let config = Arc::new(Mutex::new(AppConfig::new()));
        let recording_start_time = Arc::new(Mutex::new(None));
        
        // Get the current directory as the default output directory
        let output_dir = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| "recordings".to_string());
        
        if let Ok(mut cfg) = config.lock() {
            if cfg.output_dir.is_none() {
                cfg.output_dir = Some(output_dir);
            }
        }
        
        MenuBarApp { 
            is_recording,
            recorder,
            config,
            recording_start_time,
        }
    }

    pub fn run(&self) {
        println!("Running MenuBarApp (simplified implementation)");
        
        // Create processor and recorder
        let processor = match CpalAudioProcessor::new() {
            Ok(p) => p,
            Err(e) => {
                eprintln!("Failed to create audio processor: {}", e);
                return;
            }
        };
        
        if let Ok(mut rec_guard) = self.recorder.lock() {
            if let Ok(cfg_guard) = self.config.lock() {
                *rec_guard = Some(AudioRecorder::with_config(processor, cfg_guard.clone()));
            }
        }
        
        // Display instructions for terminal-based control
        println!("\n==== BlackBox Audio Recorder ====");
        println!("Commands available in separate terminal:");
        println!("- Start recording: `touch /tmp/blackbox_start`");
        println!("- Stop recording:  `touch /tmp/blackbox_stop`");
        println!("- Quit app:        `touch /tmp/blackbox_quit`");
        println!("- Check status:    `cat /tmp/blackbox_status`");
        println!("================================\n");
        
        // Create status file
        std::fs::write("/tmp/blackbox_status", "Idle").unwrap_or_else(|e| {
            eprintln!("Failed to write status file: {}", e);
        });
        
        // Remove control files if they exist
        let _ = std::fs::remove_file("/tmp/blackbox_start");
        let _ = std::fs::remove_file("/tmp/blackbox_stop");
        let _ = std::fs::remove_file("/tmp/blackbox_quit");
        
        println!("Menu bar initialized and ready");
        println!("Recording will be saved to: {}", 
            self.config.lock().unwrap().get_output_dir());
        println!("Press Ctrl+C to exit");
        
        // Secondary notification (native notification)
        Self::send_notification("BlackBox Audio Recorder", 
            "App is running. Use commands in a terminal to control recording.");
        
        // Main loop - poll for control files periodically
        loop {
            // Check for start recording command
            if std::path::Path::new("/tmp/blackbox_start").exists() {
                let _ = std::fs::remove_file("/tmp/blackbox_start");
                
                let mut is_rec = self.is_recording.lock().unwrap();
                if !*is_rec {
                    // Start recording
                    println!("Starting recording...");
                    if let Ok(mut rec_guard) = self.recorder.lock() {
                        if let Some(ref mut rec) = *rec_guard {
                            match rec.start_recording() {
                                Ok(_) => {
                                    println!("Recording started!");
                                    
                                    // Set recording start time
                                    if let Ok(mut time_guard) = self.recording_start_time.lock() {
                                        *time_guard = Some(Instant::now());
                                    }
                                    
                                    *is_rec = true;
                                    
                                    // Update status file
                                    let _ = std::fs::write("/tmp/blackbox_status", "Recording");
                                    
                                    // Send notification
                                    Self::send_notification("BlackBox Audio Recorder", 
                                        "Recording started");
                                },
                                Err(e) => {
                                    eprintln!("Failed to start recording: {}", e);
                                    Self::send_notification("BlackBox Audio Recorder", 
                                        &format!("Failed to start recording: {}", e));
                                }
                            }
                        }
                    }
                }
            }
            
            // Check for stop recording command
            if std::path::Path::new("/tmp/blackbox_stop").exists() {
                let _ = std::fs::remove_file("/tmp/blackbox_stop");
                
                let mut is_rec = self.is_recording.lock().unwrap();
                if *is_rec {
                    // Stop recording
                    println!("Stopping recording...");
                    if let Ok(mut rec_guard) = self.recorder.lock() {
                        if let Some(ref mut rec) = *rec_guard {
                            if let Err(e) = rec.processor.stop_recording() {
                                eprintln!("Error stopping recording: {:?}", e);
                            }
                        }
                    }
                    
                    // Reset recording start time
                    if let Ok(mut time_guard) = self.recording_start_time.lock() {
                        *time_guard = None;
                    }
                    
                    *is_rec = false;
                    
                    // Update status file
                    let _ = std::fs::write("/tmp/blackbox_status", "Idle");
                    
                    // Send notification
                    Self::send_notification("BlackBox Audio Recorder", 
                        "Recording stopped");
                }
            }
            
            // Check for quit command
            if std::path::Path::new("/tmp/blackbox_quit").exists() {
                let _ = std::fs::remove_file("/tmp/blackbox_quit");
                
                println!("Quitting application...");
                
                // Stop recording if active
                let is_rec = *self.is_recording.lock().unwrap();
                if is_rec {
                    if let Ok(mut rec_guard) = self.recorder.lock() {
                        if let Some(ref mut rec) = *rec_guard {
                            if let Err(e) = rec.processor.stop_recording() {
                                eprintln!("Error stopping recording: {:?}", e);
                            }
                        }
                    }
                }
                
                // Clean up status file
                let _ = std::fs::remove_file("/tmp/blackbox_status");
                
                // Send notification
                Self::send_notification("BlackBox Audio Recorder", 
                    "Application terminated");
                
                // Exit the application
                std::process::exit(0);
            }
            
            // Update status file with time if recording
            if let Ok(is_rec) = self.is_recording.lock() {
                if *is_rec {
                    if let Ok(time_guard) = self.recording_start_time.lock() {
                        if let Some(start_time) = *time_guard {
                            let elapsed = start_time.elapsed();
                            let seconds = elapsed.as_secs();
                            
                            let hours = seconds / 3600;
                            let minutes = (seconds % 3600) / 60;
                            let secs = seconds % 60;
                            
                            let time_str = format!("{:02}:{:02}:{:02}", hours, minutes, secs);
                            let status = format!("Recording ({})", time_str);
                            
                            let _ = std::fs::write("/tmp/blackbox_status", &status);
                        }
                    }
                }
            }
            
            // Sleep to avoid high CPU usage
            thread::sleep(Duration::from_millis(100));
        }
    }

    // Helper to send a system notification
    fn send_notification(title: &str, message: &str) {
        if cfg!(target_os = "macos") {
            // macOS notification using osascript
            let script = format!(
                r#"display notification "{}" with title "{}""#,
                message, title
            );
            let _ = Command::new("osascript")
                .args(["-e", &script])
                .output();
        }
    }

    pub fn update_status(&self, is_recording: bool) {
        println!("MenuBarApp: Updating status to: {}", is_recording);
        
        // Update our internal state
        if let Ok(mut is_rec) = self.is_recording.lock() {
            *is_rec = is_recording;
        }
        
        if is_recording {
            // Set recording start time
            if let Ok(mut time_guard) = self.recording_start_time.lock() {
                *time_guard = Some(Instant::now());
            }
            
            // Update status file
            let _ = std::fs::write("/tmp/blackbox_status", "Recording");
            
            // Send notification
            Self::send_notification("BlackBox Audio Recorder", "Recording started");
        } else {
            // Reset recording start time
            if let Ok(mut time_guard) = self.recording_start_time.lock() {
                *time_guard = None;
            }
            
            // Update status file
            let _ = std::fs::write("/tmp/blackbox_status", "Idle");
            
            // Send notification
            Self::send_notification("BlackBox Audio Recorder", "Recording stopped");
        }
    }
    
    pub fn set_output_directory(&self, dir: &str) {
        println!("MenuBarApp: Output directory set to: {}", dir);
        
        // Update config
        if let Ok(mut cfg) = self.config.lock() {
            cfg.output_dir = Some(dir.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_menu_bar_can_create() {
        let _app = MenuBarApp::new();
        println!("MenuBarApp created successfully.");
    }
}
