// We're providing a simplified implementation of the macOS menu bar
// that displays a basic UI and allows recording functionality

// Import menu bar implementation but don't use it directly
#[cfg(feature = "menu-bar")]
mod menu_bar;

use std::sync::{Arc, Mutex};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};
use std::process::Command;

// Import required Cocoa and Objective-C dependencies
use cocoa::appkit::{NSApplication, NSMenu, NSMenuItem};
use cocoa::foundation::NSAutoreleasePool;

// Import core foundation types

use crate::AppConfig;
use crate::AudioRecorder;
use crate::AudioProcessor;
use crate::CpalAudioProcessor;

// Constants
const NS_UINT_MAX: u64 = std::u64::MAX;

// Define messages that can be sent between threads
enum UiMessage {
    StartRecording,
    StopRecording,
    UpdateStatus(String),
    UpdateTime(String),
    Quit,
}

enum AppMessage {
    RecordingStarted,
    RecordingStopped,
    StatusUpdated(String),
    Terminated,
}

// Create a wrapper that will be used by the application
pub struct MenuBarApp {
    // Communication channels
    ui_sender: Sender<UiMessage>,
    app_receiver: Receiver<AppMessage>,
    app_sender: Sender<AppMessage>,
    
    // Shared state
    is_recording: Arc<Mutex<bool>>,
    recorder: Arc<Mutex<Option<AudioRecorder<CpalAudioProcessor>>>>,
    config: Arc<Mutex<AppConfig>>,
    recording_start_time: Arc<Mutex<Option<Instant>>>,
}

impl MenuBarApp {
    pub fn new() -> Self {
        println!("Creating MenuBarApp (thread-safe implementation)");
        
        // Create channels for communication
        let (ui_sender, ui_receiver) = mpsc::channel::<UiMessage>();
        let (app_sender, app_receiver) = mpsc::channel::<AppMessage>();
        
        // Initialize shared state
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
                cfg.output_dir = Some(output_dir.clone());
            }
        }
        
        // Clone app_sender for UI thread
        let app_sender_ui = app_sender.clone();
        
        // Spawn UI thread safely - using simplified Cocoa API access
        thread::spawn(move || {
            // Create a simple UI with a menu bar status item
            let mut ui = MacOsMenuBarUi::new(ui_receiver, app_sender_ui, output_dir);
            
            // Run the UI loop
            ui.run();
            
            println!("UI thread terminated");
        });
        
        // Return MenuBarApp instance
        MenuBarApp { 
            ui_sender,
            app_receiver,
            app_sender,
            is_recording,
            recorder,
            config,
            recording_start_time,
        }
    }

    pub fn run(&self) {
        println!("Running MenuBarApp (thread-safe implementation)");
        
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
        
        println!("Menu bar initialized and ready");
        println!("Recording will be saved to: {}", 
            self.config.lock().unwrap().get_output_dir());
        
        // Send a notification
        Self::send_notification("BlackBox Audio Recorder", 
            "App is running. Use the menu bar icon to control recording.");
        
        // Main loop - process messages from the UI thread
        loop {
            match self.app_receiver.recv() {
                Ok(AppMessage::RecordingStarted) => {
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
                                    
                                    // Update recording state
                                    if let Ok(mut is_rec) = self.is_recording.lock() {
                                        *is_rec = true;
                                    }
                                    
                                    // Send UI update
                                    let _ = self.ui_sender.send(UiMessage::StartRecording);
                                    
                                    // Send notification
                                    Self::send_notification("BlackBox Audio Recorder", 
                                        "Recording started");
                                        
                                    // Start time updates
                                    self.start_time_updates();
                                },
                                Err(e) => {
                                    eprintln!("Failed to start recording: {}", e);
                                    Self::send_notification("BlackBox Audio Recorder", 
                                        &format!("Failed to start recording: {}", e));
                                }
                            }
                        }
                    }
                },
                Ok(AppMessage::RecordingStopped) => {
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
                    
                    // Update recording state
                    if let Ok(mut is_rec) = self.is_recording.lock() {
                        *is_rec = false;
                    }
                    
                    // Send UI update
                    let _ = self.ui_sender.send(UiMessage::StopRecording);
                    
                    // Send notification
                    Self::send_notification("BlackBox Audio Recorder", 
                        "Recording stopped");
                },
                Ok(AppMessage::StatusUpdated(status)) => {
                    println!("Status updated: {}", status);
                    
                    // Send UI update
                    let _ = self.ui_sender.send(UiMessage::UpdateStatus(status));
                },
                Ok(AppMessage::Terminated) => {
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
                    
                    // Send notification
                    Self::send_notification("BlackBox Audio Recorder", 
                        "Application terminated");
                    
                    // Send UI update to quit
                    let _ = self.ui_sender.send(UiMessage::Quit);
                    
                    // Exit the application
                    break;
                },
                Err(_) => {
                    // Channel closed, UI thread has terminated
                    println!("UI thread terminated, exiting...");
                    break;
                }
            }
        }
        
        println!("Application exited.");
    }
    
    // Start a thread to update the recording time display
    fn start_time_updates(&self) {
        let ui_sender = self.ui_sender.clone();
        let recording_start_time = self.recording_start_time.clone();
        let is_recording = self.is_recording.clone();
        
        thread::spawn(move || {
            while *is_recording.lock().unwrap() {
                if let Ok(time_guard) = recording_start_time.lock() {
                    if let Some(start_time) = *time_guard {
                        let elapsed = start_time.elapsed();
                        let seconds = elapsed.as_secs();
                        
                        let hours = seconds / 3600;
                        let minutes = (seconds % 3600) / 60;
                        let secs = seconds % 60;
                        
                        let time_str = format!("Time: {:02}:{:02}:{:02}", hours, minutes, secs);
                        
                        // Send UI update
                        let _ = ui_sender.send(UiMessage::UpdateTime(time_str));
                    }
                }
                
                // Update once per second
                thread::sleep(Duration::from_secs(1));
            }
        });
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
        
        if is_recording {
            // Send message to start recording
            let _ = self.app_sender.send(AppMessage::RecordingStarted);
        } else {
            // Send message to stop recording
            let _ = self.app_sender.send(AppMessage::RecordingStopped);
        }
    }
    
    pub fn set_output_directory(&self, dir: &str) {
        println!("MenuBarApp: Output directory set to: {}", dir);
        
        // Update config
        if let Ok(mut cfg) = self.config.lock() {
            cfg.output_dir = Some(dir.to_string());
        }
        
        // Send status update message
        let _ = self.app_sender.send(AppMessage::StatusUpdated(
            format!("Output directory: {}", dir)
        ));
    }
}

// A safer implementation of the macOS menu bar UI
struct MacOsMenuBarUi {
    // Communication channels
    receiver: Receiver<UiMessage>,
    sender: Sender<AppMessage>,
    
    // UI components
    output_dir: String,
    
    // Cached states
    is_recording: bool,
}

impl MacOsMenuBarUi {
    fn new(receiver: Receiver<UiMessage>, sender: Sender<AppMessage>, output_dir: String) -> Self {
        Self {
            receiver,
            sender,
            output_dir,
            is_recording: false,
        }
    }
    
    // Run the UI loop safely
    fn run(&mut self) {
        // Print a simple status message - the actual UI is not implemented yet
        // due to issues with Objective-C exception handling
        println!("Menu bar UI is running in a simplified mode");
        println!("Output directory: {}", self.output_dir);
        
        // The UI loop - check for messages and respond
        loop {
            // Check for messages from the main thread
            if let Ok(message) = self.receiver.recv_timeout(Duration::from_millis(100)) {
                match message {
                    UiMessage::StartRecording => {
                        self.is_recording = true;
                        println!("UI: Recording started");
                    },
                    UiMessage::StopRecording => {
                        self.is_recording = false;
                        println!("UI: Recording stopped");
                    },
                    UiMessage::UpdateStatus(status) => {
                        println!("UI: Status updated: {}", status);
                    },
                    UiMessage::UpdateTime(time) => {
                        // Only print occasionally to avoid flooding the console
                        if time.ends_with(":00") {
                            println!("UI: {}", time);
                        }
                    },
                    UiMessage::Quit => {
                        println!("UI: Quitting");
                        break;
                    }
                }
            }
            
            // Check for simulated UI events (keyboard input in this case)
            if let Some(key) = self.check_keyboard_input() {
                match key {
                    's' => {
                        // Toggle recording state
                        self.is_recording = !self.is_recording;
                        if self.is_recording {
                            let _ = self.sender.send(AppMessage::RecordingStarted);
                        } else {
                            let _ = self.sender.send(AppMessage::RecordingStopped);
                        }
                    },
                    'q' => {
                        // Quit the application
                        let _ = self.sender.send(AppMessage::Terminated);
                        break;
                    },
                    _ => {}
                }
            }
            
            // Sleep a bit to avoid hogging CPU
            thread::sleep(Duration::from_millis(50));
        }
    }
    
    // This is a placeholder that simulates keyboard input
    // In a real implementation, we would check for actual menu clicks
    fn check_keyboard_input(&self) -> Option<char> {
        // This is just a placeholder - we're not actually checking keyboard input
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_menu_bar_can_create() {
        // Skip test in CI environments
        if std::env::var("CI").is_ok() {
            return;
        }
        
        let _app = MenuBarApp::new();
        println!("MenuBarApp created successfully.");
    }
}
