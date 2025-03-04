// Implementation of the macOS menu bar app using safe Cocoa/AppKit wrappers

use std::sync::{Arc, Mutex};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};
use std::process::Command;

use crate::AppConfig;
use crate::AudioRecorder;
use crate::AudioProcessor;
use crate::CpalAudioProcessor;
use crate::macos::safe_cocoa::{
    self, Application, StatusItem, Menu, MenuItem, MenuBarIcon,
    CocoaResult, CocoaError, setup_exception_handling
};

// Messages that can be sent from the UI thread to the app thread
pub enum UiToAppMessage {
    StartRecording,
    StopRecording,
    UpdateOutputDir(String),
    Quit,
}

// Messages that can be sent from the app thread to the UI thread
pub enum AppToUiMessage {
    RecordingStarted,
    RecordingStopped,
    StatusUpdate(String),
    TimeUpdate(String),
    UpdateOutputDir(String),
    Quit,
}

/// Main implementation of the macOS menu bar application
pub struct MenuBarAppImpl {
    // Communication channels
    ui_to_app_sender: Sender<UiToAppMessage>,
    ui_to_app_receiver: Receiver<UiToAppMessage>,
    app_to_ui_sender: Sender<AppToUiMessage>,
    app_to_ui_receiver: Receiver<AppToUiMessage>,
    
    // Shared state (thread-safe)
    is_recording: Arc<Mutex<bool>>,
    recorder: Arc<Mutex<Option<AudioRecorder<CpalAudioProcessor>>>>,
    config: Arc<Mutex<AppConfig>>,
    recording_start_time: Arc<Mutex<Option<Instant>>>,
    
    // UI thread handle
    ui_thread: Option<thread::JoinHandle<()>>,
}

impl MenuBarAppImpl {
    /// Create a new instance of the macOS menu bar application
    pub fn new() -> Self {
        println!("Creating MenuBarApp (safe implementation)");
        
        // Create communication channels
        let (ui_to_app_sender, ui_to_app_receiver) = mpsc::channel::<UiToAppMessage>();
        let (app_to_ui_sender, app_to_ui_receiver) = mpsc::channel::<AppToUiMessage>();
        
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
        
        // Create app
        let ui_thread = Self::start_ui_thread(
            ui_to_app_sender.clone(),
            app_to_ui_receiver,
            output_dir,
        );
        
        MenuBarAppImpl {
            ui_to_app_sender,
            ui_to_app_receiver,
            app_to_ui_sender,
            app_to_ui_receiver,
            recorder,
            config,
            is_recording,
            recording_start_time,
            ui_thread: Some(ui_thread),
        }
    }
    
    /// Run the main application loop
    pub fn run(&mut self) {
        println!("Running MenuBarApp (safe implementation)");
        
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
        
        // Print initial status
        println!("Menu bar initialized and ready");
        println!("Recording will be saved to: {}", 
            self.config.lock().unwrap().get_output_dir());
        
        // Send notification
        Self::send_notification("BlackBox Audio Recorder", 
            "App is running. Use the menu bar icon to control recording.");
        
        // Main application loop - process messages from the UI
        loop {
            // Check for messages from the UI
            match self.ui_to_app_receiver.recv() {
                Ok(UiToAppMessage::StartRecording) => {
                    self.start_recording();
                },
                Ok(UiToAppMessage::StopRecording) => {
                    self.stop_recording();
                },
                Ok(UiToAppMessage::UpdateOutputDir(dir)) => {
                    self.update_output_dir(&dir);
                },
                Ok(UiToAppMessage::Quit) => {
                    println!("Quitting application...");
                    // Stop recording if active
                    if *self.is_recording.lock().unwrap() {
                        self.stop_recording();
                    }
                    
                    // Send notification
                    Self::send_notification("BlackBox Audio Recorder", 
                        "Application terminated");
                    
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
        
        // Wait for UI thread to terminate
        if let Some(handle) = self.ui_thread.take() {
            let _ = handle.join();
        }
        
        println!("Application exited.");
    }
    
    /// Start recording
    fn start_recording(&mut self) {
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
                        let _ = self.app_to_ui_sender.send(AppToUiMessage::RecordingStarted);
                        
                        // Send notification
                        Self::send_notification("BlackBox Audio Recorder", "Recording started");
                        
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
    }
    
    /// Stop recording
    fn stop_recording(&mut self) {
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
        let _ = self.app_to_ui_sender.send(AppToUiMessage::RecordingStopped);
        
        // Send notification
        Self::send_notification("BlackBox Audio Recorder", "Recording stopped");
    }
    
    /// Update output directory
    fn update_output_dir(&mut self, dir: &str) {
        println!("Updating output directory to: {}", dir);
        
        if let Ok(mut cfg) = self.config.lock() {
            cfg.output_dir = Some(dir.to_string());
        }
        
        // Send UI update
        let _ = self.app_to_ui_sender.send(AppToUiMessage::UpdateOutputDir(dir.to_string()));
    }
    
    /// Start a thread to update the recording time display
    fn start_time_updates(&self) {
        let sender = self.app_to_ui_sender.clone();
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
                        
                        let time_str = format!("{:02}:{:02}:{:02}", hours, minutes, secs);
                        
                        // Send UI update
                        let _ = sender.send(AppToUiMessage::TimeUpdate(time_str));
                    }
                }
                
                // Update once per second
                thread::sleep(Duration::from_secs(1));
            }
        });
    }
    
    /// Helper to send a system notification
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
    
    /// Start the UI thread with the safe Cocoa wrapper
    fn start_ui_thread(
        sender: Sender<UiToAppMessage>,
        receiver: Receiver<AppToUiMessage>,
        output_dir: String
    ) -> thread::JoinHandle<()> {
        thread::spawn(move || {
            // Set up exception handling
            setup_exception_handling();
            
            // Create application instance
            let app_result = MenuBarUi::run(sender, receiver, output_dir);
            
            // Handle any errors
            if let Err(e) = app_result {
                eprintln!("Error in UI thread: {:?}", e);
            }
            
            println!("UI thread terminated");
        })
    }
    
    // Update the recording status (for external callers)
    pub fn update_status(&self, is_recording: bool) {
        println!("MenuBarAppImpl: Updating status to: {}", is_recording);
        
        if is_recording {
            let _ = self.ui_to_app_sender.send(UiToAppMessage::StartRecording);
        } else {
            let _ = self.ui_to_app_sender.send(UiToAppMessage::StopRecording);
        }
    }
    
    // Set the output directory (for external callers)
    pub fn set_output_directory(&self, dir: &str) {
        println!("MenuBarAppImpl: Setting output directory to: {}", dir);
        let _ = self.ui_to_app_sender.send(UiToAppMessage::UpdateOutputDir(dir.to_string()));
    }
}

/// The UI implementation using the safe Cocoa wrapper
struct MenuBarUi;

impl MenuBarUi {
    /// Run the menu bar UI
    fn run(
        sender: Sender<UiToAppMessage>,
        receiver: Receiver<AppToUiMessage>,
        output_dir: String
    ) -> CocoaResult<()> {
        // Create application
        let mut app = Application::new()?;
        
        // Create status item
        let mut status_item = StatusItem::new()?;
        
        // Set initial title and icon
        status_item.set_title("●");
        
        // Try to create an icon (fallback to text if this fails)
        if let Ok(icon) = MenuBarIcon::circle("black", 16.0) {
            status_item.set_icon(&icon);
        }
        
        // Create menu
        let mut menu = Menu::new()?;
        
        // Add start/stop recording item
        let mut start_stop_item = MenuItem::new("Start Recording")?;
        let sender_clone = sender.clone();
        start_stop_item.set_action(move || {
            let _ = sender_clone.send(UiToAppMessage::StartRecording);
        });
        menu.add_item(start_stop_item);
        
        // Add status item
        let mut status_item_menu = MenuItem::new(&format!("Status: Idle"))?;
        status_item_menu.set_enabled(false);
        menu.add_item(status_item_menu);
        
        // Add time item
        let mut time_item = MenuItem::new("Time: 00:00:00")?;
        time_item.set_enabled(false);
        menu.add_item(time_item);
        
        // Add separator
        menu.add_separator();
        
        // Add output directory item
        let mut output_dir_item = MenuItem::new(&format!("Output: {}", output_dir))?;
        output_dir_item.set_enabled(false);
        menu.add_item(output_dir_item);
        
        // Add separator
        menu.add_separator();
        
        // Add quit item
        let mut quit_item = MenuItem::new("Quit")?;
        let sender_clone = sender.clone();
        quit_item.set_action(move || {
            let _ = sender_clone.send(UiToAppMessage::Quit);
        });
        menu.add_item(quit_item);
        
        // Set menu for status item
        status_item.set_menu(menu);
        
        // Add status item to application
        app.add_status_item(status_item);
        
        // Process messages from app thread
        let sender_clone = sender.clone();
        let is_recording = Arc::new(Mutex::new(false));
        let is_recording_clone = is_recording.clone();
        
        // Spawn a thread to process app messages
        thread::spawn(move || {
            loop {
                match receiver.recv() {
                    Ok(AppToUiMessage::RecordingStarted) => {
                        *is_recording_clone.lock().unwrap() = true;
                        println!("UI: Recording started");
                    },
                    Ok(AppToUiMessage::RecordingStopped) => {
                        *is_recording_clone.lock().unwrap() = false;
                        println!("UI: Recording stopped");
                    },
                    Ok(AppToUiMessage::StatusUpdate(status)) => {
                        println!("UI: Status updated: {}", status);
                    },
                    Ok(AppToUiMessage::TimeUpdate(time)) => {
                        // Only print occasionally to avoid flooding the console
                        if time.ends_with(":00") {
                            println!("UI: Time: {}", time);
                        }
                    },
                    Ok(AppToUiMessage::UpdateOutputDir(dir)) => {
                        println!("UI: Output directory updated: {}", dir);
                    },
                    Ok(AppToUiMessage::Quit) => {
                        println!("UI: Quitting");
                        break;
                    },
                    Err(_) => {
                        // Channel closed, app thread has terminated
                        println!("App thread terminated, exiting UI...");
                        let _ = sender_clone.send(UiToAppMessage::Quit);
                        break;
                    }
                }
            }
        });
        
        // Run the application event loop
        app.run();
        
        Ok(())
    }
} 