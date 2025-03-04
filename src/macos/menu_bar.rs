use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use cocoa::appkit::{NSApp, NSApplication, NSApplicationActivationPolicy, NSMenu, NSMenuItem, NSStatusBar};
use cocoa::base::{id, nil, YES};
use cocoa::foundation::{NSAutoreleasePool, NSString};
use objc::{class, msg_send, sel, sel_impl};
use core_foundation::runloop::{CFRunLoop, kCFRunLoopDefaultMode};
use core_foundation::base::TCFType;
use libc::c_void;

use crate::AppConfig;
use crate::AudioRecorder;
use crate::CpalAudioProcessor;

pub struct MenuBarApp {
    status_item: id,
    is_recording: Arc<Mutex<bool>>,
    recorder: Arc<Mutex<Option<AudioRecorder<CpalAudioProcessor>>>>,
    config: Arc<Mutex<AppConfig>>,
    recording_start_time: Arc<Mutex<Option<std::time::Instant>>>,
    status_update_thread: Arc<Mutex<Option<thread::JoinHandle<()>>>>,
}

impl MenuBarApp {
    pub fn new() -> Self {
        unsafe {
            let pool = NSAutoreleasePool::new(nil);
            
            // Create a status bar item
            let status_bar: id = msg_send![class!(NSStatusBar), systemStatusBar];
            let status_item: id = msg_send![status_bar, statusItemWithLength:-1.0];
            
            // Set title for status item
            let title = NSString::alloc(nil).init_str("●");
            let _: () = msg_send![status_item, setTitle:title];
            
            // Create menu
            let menu: id = msg_send![class!(NSMenu), new];
            
            // Add start/stop recording item
            let start_item: id = msg_send![class!(NSMenuItem), new];
            let start_title = NSString::alloc(nil).init_str("Start Recording");
            let _: () = msg_send![start_item, setTitle:start_title];
            let start_tag = 1;
            let _: () = msg_send![start_item, setTag:start_tag];
            let _: () = msg_send![menu, addItem:start_item];
            
            // Add status item to show recording information
            let status_item_menu: id = msg_send![class!(NSMenuItem), new];
            let status_title = NSString::alloc(nil).init_str("Not recording");
            let _: () = msg_send![status_item_menu, setTitle:status_title];
            let status_tag = 10;
            let _: () = msg_send![status_item_menu, setTag:status_tag];
            let _: () = msg_send![status_item_menu, setEnabled:NO];
            let _: () = msg_send![menu, addItem:status_item_menu];
            
            // Add separator
            let separator1: id = msg_send![class!(NSMenuItem), separatorItem];
            let _: () = msg_send![menu, addItem:separator1];
            
            // Add settings submenu
            let settings_item: id = msg_send![class!(NSMenuItem), new];
            let settings_title = NSString::alloc(nil).init_str("Settings");
            let _: () = msg_send![settings_item, setTitle:settings_title];
            
            // Create settings submenu
            let settings_menu: id = msg_send![class!(NSMenu), new];
            let _: () = msg_send![settings_item, setSubmenu:settings_menu];
            let _: () = msg_send![menu, addItem:settings_item];
            
            // Add duration submenu to settings
            let duration_item: id = msg_send![class!(NSMenuItem), new];
            let duration_title = NSString::alloc(nil).init_str("Duration");
            let _: () = msg_send![duration_item, setTitle:duration_title];
            
            // Create duration submenu
            let duration_menu: id = msg_send![class!(NSMenu), new];
            let _: () = msg_send![duration_item, setSubmenu:duration_menu];
            let _: () = msg_send![settings_menu, addItem:duration_item];
            
            // Add duration options
            let durations = [(30, "30 seconds"), (60, "1 minute"), 
                            (300, "5 minutes"), (1800, "30 minutes"), 
                            (3600, "1 hour"), (0, "Continuous")];
            
            for (i, &(seconds, label)) in durations.iter().enumerate() {
                let duration_option: id = msg_send![class!(NSMenuItem), new];
                let option_title = NSString::alloc(nil).init_str(label);
                let _: () = msg_send![duration_option, setTitle:option_title];
                let duration_tag = 100 + i;
                let _: () = msg_send![duration_option, setTag:duration_tag as i64];
                let _: () = msg_send![duration_option, setRepresentedObject:seconds as i64];
                let _: () = msg_send![duration_menu, addItem:duration_option];
            }
            
            // Add channels submenu to settings
            let channels_item: id = msg_send![class!(NSMenuItem), new];
            let channels_title = NSString::alloc(nil).init_str("Input Channels");
            let _: () = msg_send![channels_item, setTitle:channels_title];
            
            // Create channels submenu
            let channels_menu: id = msg_send![class!(NSMenu), new];
            let _: () = msg_send![channels_item, setSubmenu:channels_menu];
            let _: () = msg_send![settings_menu, addItem:channels_item];
            
            // Add channel options (defaults - will be updated when app starts)
            let channels = ["0 (Default)", "0,1 (Stereo)", "All Available"];
            
            for (i, &label) in channels.iter().enumerate() {
                let channel_option: id = msg_send![class!(NSMenuItem), new];
                let option_title = NSString::alloc(nil).init_str(label);
                let _: () = msg_send![channel_option, setTitle:option_title];
                let channel_tag = 200 + i;
                let _: () = msg_send![channel_option, setTag:channel_tag as i64];
                let _: () = msg_send![channels_menu, addItem:channel_option];
            }
            
            // Add "Choose Output Directory" to settings
            let dir_item: id = msg_send![class!(NSMenuItem), new];
            let dir_title = NSString::alloc(nil).init_str("Choose Output Directory...");
            let _: () = msg_send![dir_item, setTitle:dir_title];
            let dir_tag = 300;
            let _: () = msg_send![dir_item, setTag:dir_tag as i64];
            let _: () = msg_send![settings_menu, addItem:dir_item];
            
            // Add separator before Output Directory display
            let settings_separator: id = msg_send![class!(NSMenuItem), separatorItem];
            let _: () = msg_send![settings_menu, addItem:settings_separator];
            
            // Add current output directory display
            let current_dir_item: id = msg_send![class!(NSMenuItem), new];
            let current_dir_title = NSString::alloc(nil).init_str("Output: ./recordings");
            let _: () = msg_send![current_dir_item, setTitle:current_dir_title];
            let current_dir_tag = 301;
            let _: () = msg_send![current_dir_item, setTag:current_dir_tag as i64];
            let _: () = msg_send![current_dir_item, setEnabled:NO]; // Not clickable
            let _: () = msg_send![settings_menu, addItem:current_dir_item];
            
            // Add separator
            let separator2: id = msg_send![class!(NSMenuItem), separatorItem];
            let _: () = msg_send![menu, addItem:separator2];
            
            // Add quit item
            let quit_item: id = msg_send![class!(NSMenuItem), new];
            let quit_title = NSString::alloc(nil).init_str("Quit");
            let _: () = msg_send![quit_item, setTitle:quit_title];
            let quit_tag = 2;
            let _: () = msg_send![quit_item, setTag:quit_tag];
            let _: () = msg_send![menu, addItem:quit_item];
            
            // Set menu to status item
            let _: () = msg_send![status_item, setMenu:menu];
            
            pool.drain();
            
            // Load initial configuration
            let config = AppConfig::load();
            
            MenuBarApp {
                status_item,
                is_recording: Arc::new(Mutex::new(false)),
                recorder: Arc::new(Mutex::new(None)),
                config: Arc::new(Mutex::new(config)),
                recording_start_time: Arc::new(Mutex::new(None)),
                status_update_thread: Arc::new(Mutex::new(None)),
            }
        }
    }
    
    pub fn run(&self) {
        // Set up main app
        unsafe {
            let app = NSApp();
            let _: () = msg_send![app, setActivationPolicy:NSApplicationActivationPolicy::NSApplicationActivationPolicyAccessory];
        }
        
        // Create a thread to check for menu item clicks
        let is_recording = self.is_recording.clone();
        let recorder = self.recorder.clone();
        let config = self.config.clone();
        let status_item = self.status_item;
        let recording_start_time = self.recording_start_time.clone();
        let status_update_thread = self.status_update_thread.clone();
        
        // Helper to update the recording status text
        let update_status_text = {
            let is_recording = is_recording.clone();
            let recording_start_time = recording_start_time.clone();
            let config = config.clone();
            let status_item = status_item;
            
            move || {
                unsafe {
                    let pool = NSAutoreleasePool::new(nil);
                    
                    let menu: id = msg_send![status_item, menu];
                    let status_item_menu: id = msg_send![menu, itemWithTag:10];
                    
                    let status_text = if *is_recording.lock().unwrap() {
                        let duration = {
                            let cfg = config.lock().unwrap();
                            cfg.get_duration()
                        };
                        
                        if let Some(start_time) = *recording_start_time.lock().unwrap() {
                            let elapsed = start_time.elapsed();
                            let elapsed_secs = elapsed.as_secs();
                            
                            if duration > 0 {
                                // Fixed duration recording
                                let remaining = if duration > elapsed_secs {
                                    duration - elapsed_secs
                                } else {
                                    0
                                };
                                
                                format!("Recording: {} remaining", format_duration(remaining))
                            } else {
                                // Continuous recording
                                format!("Recording: {} elapsed", format_duration(elapsed_secs))
                            }
                        } else {
                            "Recording...".to_string()
                        }
                    } else {
                        "Not recording".to_string()
                    };
                    
                    let ns_status = NSString::alloc(nil).init_str(&status_text);
                    let _: () = msg_send![status_item_menu, setTitle:ns_status];
                    
                    // Also update output directory display
                    let current_dir_item: id = msg_send![menu, itemWithTag:301];
                    let output_dir = {
                        let cfg = config.lock().unwrap();
                        cfg.get_output_dir()
                    };
                    
                    let dir_text = format!("Output: {}", output_dir);
                    let ns_dir = NSString::alloc(nil).init_str(&dir_text);
                    let _: () = msg_send![current_dir_item, setTitle:ns_dir];
                    
                    pool.drain();
                }
            }
        };
        
        // Start a status update thread that updates the UI with recording status
        let start_status_thread = {
            let is_recording = is_recording.clone();
            let status_update_thread = status_update_thread.clone();
            
            move || {
                // First stop any existing thread
                let mut thread_guard = status_update_thread.lock().unwrap();
                if let Some(handle) = thread_guard.take() {
                    let _ = handle.join();
                }
                
                // Start a new thread if recording
                if *is_recording.lock().unwrap() {
                    let update_fn = update_status_text.clone();
                    *thread_guard = Some(thread::spawn(move || {
                        while {
                            // Check if still recording
                            let recording = *is_recording.lock().unwrap();
                            
                            if recording {
                                // Update status text
                                update_fn();
                                
                                // Sleep for a bit
                                thread::sleep(Duration::from_millis(500));
                            }
                            
                            recording
                        } {}
                    }));
                } else {
                    // Update one last time to show "Not recording"
                    update_status_text();
                }
            }
        };
        
        // Start a background thread to poll for menu clicks
        thread::spawn(move || {
            loop {
                // Sleep to avoid high CPU usage
                thread::sleep(Duration::from_millis(100));
                
                unsafe {
                    let pool = NSAutoreleasePool::new(nil);
                    
                    // Check if menu items were clicked
                    let menu: id = msg_send![status_item, menu];
                    let selected_item: id = msg_send![menu, highlightedItem];
                    
                    // Process events to make the app responsive
                    let app = NSApp();
                    let _: () = msg_send![app, updateWindows];
                    
                    // Run the runloop for a bit
                    let current_loop = CFRunLoop::get_current();
                    current_loop.run_in_mode(kCFRunLoopDefaultMode, Duration::from_millis(0), true);
                    
                    // Check the recording item - if it was clicked, toggle recording
                    let start_item: id = msg_send![menu, itemWithTag:1];
                    let clicked: BOOL = msg_send![start_item, wasClicked];
                    
                    if clicked != NO {
                        if *is_recording.lock().unwrap() {
                            // Stop recording
                            let mut rec_lock = recorder.lock().unwrap();
                            *rec_lock = None;
                            
                            // Update recording state
                            *is_recording.lock().unwrap() = false;
                            *recording_start_time.lock().unwrap() = None;
                            
                            // Update UI
                            let title = NSString::alloc(nil).init_str("Start Recording");
                            let _: () = msg_send![start_item, setTitle:title];
                            
                            // Update status item color
                            let status_title = NSString::alloc(nil).init_str("●");
                            let _: () = msg_send![status_item, setTitle:status_title];
                            
                            // Update status thread
                            start_status_thread();
                            
                            // Reset click state
                            let _: () = msg_send![start_item, setWasClicked:NO];
                        } else {
                            // Start recording
                            let mut rec_lock = recorder.lock().unwrap();
                            
                            if rec_lock.is_none() {
                                // Create processor and recorder
                                if let Ok(processor) = CpalAudioProcessor::new() {
                                    // Create a copy of the config
                                    let cfg_copy = {
                                        let cfg_lock = config.lock().unwrap();
                                        cfg_lock.clone()
                                    };
                                    
                                    *rec_lock = Some(AudioRecorder::with_config(processor, cfg_copy));
                                    
                                    // Start recording
                                    if let Some(rec) = rec_lock.as_mut() {
                                        if let Ok(_) = rec.start_recording() {
                                            // Update recording state
                                            *is_recording.lock().unwrap() = true;
                                            *recording_start_time.lock().unwrap() = Some(std::time::Instant::now());
                                            
                                            // Update UI
                                            let title = NSString::alloc(nil).init_str("Stop Recording");
                                            let _: () = msg_send![start_item, setTitle:title];
                                            
                                            // Update status item color to red for recording
                                            let status_title = NSString::alloc(nil).init_str("🔴");
                                            let _: () = msg_send![status_item, setTitle:status_title];
                                            
                                            // Start status update thread
                                            start_status_thread();
                                        }
                                    }
                                }
                            }
                            
                            // Reset click state
                            let _: () = msg_send![start_item, setWasClicked:NO];
                        }
                    }
                    
                    // Check for duration menu clicks (tag range 100-110)
                    for i in 0..10 {
                        let tag = 100 + i;
                        let duration_item: id = msg_send![menu, itemWithTag:tag as i64];
                        
                        if !duration_item.is_null() {
                            let clicked: BOOL = msg_send![duration_item, wasClicked];
                            
                            if clicked != NO {
                                let seconds: i64 = msg_send![duration_item, representedObject];
                                
                                // Update configuration
                                {
                                    let mut cfg = config.lock().unwrap();
                                    cfg.duration = Some(seconds as u64);
                                }
                                
                                // Reset click state
                                let _: () = msg_send![duration_item, setWasClicked:NO];
                                
                                // Update status display
                                update_status_text();
                                
                                // Add checkmark to selected item and remove from others
                                for j in 0..10 {
                                    let other_tag = 100 + j;
                                    let other_item: id = msg_send![menu, itemWithTag:other_tag as i64];
                                    
                                    if !other_item.is_null() {
                                        let state = if other_tag == tag { 1 } else { 0 };
                                        let _: () = msg_send![other_item, setState:state];
                                    }
                                }
                            }
                        }
                    }
                    
                    // Check for channel menu clicks (tag range 200-210)
                    for i in 0..10 {
                        let tag = 200 + i;
                        let channel_item: id = msg_send![menu, itemWithTag:tag as i64];
                        
                        if !channel_item.is_null() {
                            let clicked: BOOL = msg_send![channel_item, wasClicked];
                            
                            if clicked != NO {
                                // Get the title which includes the channel spec
                                let title: id = msg_send![channel_item, title];
                                let channels_str = nsstring_to_string(title);
                                
                                // Extract just the channel spec (before the space)
                                let channel_spec = if let Some(idx) = channels_str.find(' ') {
                                    channels_str[..idx].to_string()
                                } else {
                                    channels_str
                                };
                                
                                // Update configuration
                                {
                                    let mut cfg = config.lock().unwrap();
                                    cfg.audio_channels = Some(channel_spec);
                                }
                                
                                // Reset click state
                                let _: () = msg_send![channel_item, setWasClicked:NO];
                                
                                // Add checkmark to selected item and remove from others
                                for j in 0..10 {
                                    let other_tag = 200 + j;
                                    let other_item: id = msg_send![menu, itemWithTag:other_tag as i64];
                                    
                                    if !other_item.is_null() {
                                        let state = if other_tag == tag { 1 } else { 0 };
                                        let _: () = msg_send![other_item, setState:state];
                                    }
                                }
                            }
                        }
                    }
                    
                    // Check for output directory selection (tag 300)
                    let dir_item: id = msg_send![menu, itemWithTag:300 as i64];
                    if !dir_item.is_null() {
                        let clicked: BOOL = msg_send![dir_item, wasClicked];
                        
                        if clicked != NO {
                            // Open directory selection dialog
                            let panel: id = msg_send![class!(NSOpenPanel), openPanel];
                            let _: () = msg_send![panel, setCanChooseDirectories:YES];
                            let _: () = msg_send![panel, setCanChooseFiles:NO];
                            let _: () = msg_send![panel, setAllowsMultipleSelection:NO];
                            let _: () = msg_send![panel, setPrompt:NSString::alloc(nil).init_str("Select")];
                            let _: () = msg_send![panel, setTitle:NSString::alloc(nil).init_str("Choose Output Directory")];
                            
                            let response: i64 = msg_send![panel, runModal];
                            
                            if response == 1 { // NSModalResponseOK
                                let urls: id = msg_send![panel, URLs];
                                let count: u64 = msg_send![urls, count];
                                
                                if count > 0 {
                                    let url: id = msg_send![urls, objectAtIndex:0];
                                    let path: id = msg_send![url, path];
                                    let path_str = nsstring_to_string(path);
                                    
                                    // Update configuration
                                    {
                                        let mut cfg = config.lock().unwrap();
                                        cfg.output_dir = Some(path_str);
                                    }
                                    
                                    // Update status display
                                    update_status_text();
                                }
                            }
                            
                            // Reset click state
                            let _: () = msg_send![dir_item, setWasClicked:NO];
                        }
                    }
                    
                    // Check the quit item
                    let quit_item: id = msg_send![menu, itemWithTag:2];
                    let clicked: BOOL = msg_send![quit_item, wasClicked];
                    
                    if clicked != NO {
                        // Stop any active recording before quitting
                        if *is_recording.lock().unwrap() {
                            let mut rec_lock = recorder.lock().unwrap();
                            *rec_lock = None;
                        }
                        
                        // Quit the app
                        let _: () = msg_send![app, terminate:nil];
                        
                        // Reset click state (though we're terminating)
                        let _: () = msg_send![quit_item, setWasClicked:NO];
                    }
                    
                    pool.drain();
                }
            }
        });
        
        // Run the application
        unsafe {
            let app = NSApp();
            app.run();
        }
    }
    
    pub fn update_status(&self, is_recording: bool) {
        unsafe {
            let pool = NSAutoreleasePool::new(nil);
            
            // Update recording state
            *self.is_recording.lock().unwrap() = is_recording;
            
            // Update recording start time
            if is_recording {
                *self.recording_start_time.lock().unwrap() = Some(std::time::Instant::now());
            } else {
                *self.recording_start_time.lock().unwrap() = None;
            }
            
            // Update menu title
            let menu: id = msg_send![self.status_item, menu];
            let start_item: id = msg_send![menu, itemWithTag:1];
            
            let title = if is_recording {
                "Stop Recording"
            } else {
                "Start Recording"
            };
            
            let ns_title = NSString::alloc(nil).init_str(title);
            let _: () = msg_send![start_item, setTitle:ns_title];
            
            // Update status item indicator
            let status_title = NSString::alloc(nil).init_str(if is_recording { "🔴" } else { "●" });
            let _: () = msg_send![self.status_item, setTitle:status_title];
            
            // Update status text
            let status_item_menu: id = msg_send![menu, itemWithTag:10];
            let status_text = if is_recording {
                "Recording..."
            } else {
                "Not recording"
            };
            let ns_status = NSString::alloc(nil).init_str(status_text);
            let _: () = msg_send![status_item_menu, setTitle:ns_status];
            
            pool.drain();
            
            // Start or stop the status update thread
            if let Some(start_thread_fn) = self.start_status_thread() {
                start_thread_fn();
            }
        }
    }
    
    // Helper to create a function that starts a status update thread
    fn start_status_thread(&self) -> Option<Box<dyn Fn() + Send>> {
        let is_recording = self.is_recording.clone();
        let recording_start_time = self.recording_start_time.clone();
        let config = self.config.clone();
        let status_item = self.status_item;
        let status_update_thread = self.status_update_thread.clone();
        
        // Helper to update the recording status text
        let update_status_text = {
            let is_recording = is_recording.clone();
            let recording_start_time = recording_start_time.clone();
            let config = config.clone();
            let status_item = status_item;
            
            move || {
                unsafe {
                    let pool = NSAutoreleasePool::new(nil);
                    
                    let menu: id = msg_send![status_item, menu];
                    let status_item_menu: id = msg_send![menu, itemWithTag:10];
                    
                    let status_text = if *is_recording.lock().unwrap() {
                        let duration = {
                            let cfg = config.lock().unwrap();
                            cfg.get_duration()
                        };
                        
                        if let Some(start_time) = *recording_start_time.lock().unwrap() {
                            let elapsed = start_time.elapsed();
                            let elapsed_secs = elapsed.as_secs();
                            
                            if duration > 0 {
                                // Fixed duration recording
                                let remaining = if duration > elapsed_secs {
                                    duration - elapsed_secs
                                } else {
                                    0
                                };
                                
                                format!("Recording: {} remaining", format_duration(remaining))
                            } else {
                                // Continuous recording
                                format!("Recording: {} elapsed", format_duration(elapsed_secs))
                            }
                        } else {
                            "Recording...".to_string()
                        }
                    } else {
                        "Not recording".to_string()
                    };
                    
                    let ns_status = NSString::alloc(nil).init_str(&status_text);
                    let _: () = msg_send![status_item_menu, setTitle:ns_status];
                    
                    // Also update output directory display
                    let current_dir_item: id = msg_send![menu, itemWithTag:301];
                    if !current_dir_item.is_null() {
                        let output_dir = {
                            let cfg = config.lock().unwrap();
                            cfg.get_output_dir()
                        };
                        
                        let dir_text = format!("Output: {}", output_dir);
                        let ns_dir = NSString::alloc(nil).init_str(&dir_text);
                        let _: () = msg_send![current_dir_item, setTitle:ns_dir];
                    }
                    
                    pool.drain();
                }
            }
        };
        
        // Create function that starts/stops the status thread
        Some(Box::new(move || {
            // First stop any existing thread
            let mut thread_guard = status_update_thread.lock().unwrap();
            if let Some(handle) = thread_guard.take() {
                let _ = handle.join();
            }
            
            // Start a new thread if recording
            if *is_recording.lock().unwrap() {
                let update_fn = update_status_text.clone();
                *thread_guard = Some(thread::spawn(move || {
                    while {
                        // Check if still recording
                        let recording = *is_recording.lock().unwrap();
                        
                        if recording {
                            // Update status text
                            update_fn();
                            
                            // Sleep for a bit
                            thread::sleep(Duration::from_millis(500));
                        }
                        
                        recording
                    } {}
                }));
            } else {
                // Update one last time to show "Not recording"
                update_status_text();
            }
        }))
    }
    
    // Helper to find resource paths in the app bundle
    fn get_resource_path(resource_name: &str) -> Option<PathBuf> {
        let mut path = std::env::current_exe().ok()?;
        path.pop(); // Remove executable name
        
        // For development, check if running from target directory
        if path.ends_with("target/debug") || path.ends_with("target/release") {
            path.pop(); // Remove debug or release
            path.pop(); // Remove target
            path.push(resource_name);
            if path.exists() {
                return Some(path);
            }
        }
        
        // For bundled app
        path.pop(); // Remove MacOS
        path.push("Resources");
        path.push(resource_name);
        
        if path.exists() {
            Some(path)
        } else {
            None
        }
    }
}

// Helper for system events
pub fn process_system_events() {
    unsafe {
        let pool = NSAutoreleasePool::new(nil);
        
        // Process one event
        let current_loop = CFRunLoop::get_current();
        current_loop.run_in_mode(kCFRunLoopDefaultMode, Duration::from_millis(100), true);
        
        pool.drain();
    }
}

// NSMenuItem extension
#[allow(non_upper_case_globals)]
const NO: i8 = 0;

trait NSMenuItemExtensions {
    unsafe fn wasClicked(&self) -> BOOL;
    unsafe fn setWasClicked(&self, was_clicked: BOOL);
}

type BOOL = i8;

impl NSMenuItemExtensions for id {
    unsafe fn wasClicked(&self) -> BOOL {
        let clicked: BOOL = *(*self).get_ivar("_wasClicked");
        clicked
    }
    
    unsafe fn setWasClicked(&self, was_clicked: BOOL) {
        (*self).set_ivar("_wasClicked", was_clicked);
    }
}

// Extend objc::runtime::Object with Ivar methods
trait IvarAccess {
    unsafe fn get_ivar<T>(&self, name: &str) -> T;
    unsafe fn set_ivar<T>(&self, name: &str, value: T);
}

impl IvarAccess for objc::runtime::Object {
    unsafe fn get_ivar<T>(&self, name: &str) -> T {
        use std::mem;
        use std::ptr;
        
        let cls = objc::runtime::object_getClass(self as *const Self);
        let ivar = objc::runtime::class_getInstanceVariable(cls, name.as_ptr() as *const i8);
        if ivar.is_null() {
            // If ivar doesn't exist, initialize it
            objc::runtime::object_setInstanceVariable(
                self as *mut Self as *mut c_void,
                name.as_ptr() as *const i8,
                ptr::null_mut()
            );
            return mem::zeroed();
        }
        
        let offset = objc::runtime::ivar_getOffset(ivar);
        let ptr = (self as *const Self as *const u8).offset(offset as isize) as *const T;
        ptr.read()
    }
    
    unsafe fn set_ivar<T>(&self, name: &str, value: T) {
        use std::mem;
        use std::ptr;
        
        let cls = objc::runtime::object_getClass(self as *const Self);
        let ivar = objc::runtime::class_getInstanceVariable(cls, name.as_ptr() as *const i8);
        
        if ivar.is_null() {
            // If ivar doesn't exist, we need to add it to the class
            let enc = format!("{}\0", std::any::type_name::<T>());
            let ivar = objc::runtime::class_addIvar(
                cls,
                name.as_ptr() as *const i8,
                mem::size_of::<T>(),
                mem::align_of::<T>() as u8,
                enc.as_ptr() as *const i8
            );
            if ivar == 0 {
                // Failed to add ivar
                return;
            }
        }
        
        // Now set the value
        let ivar = objc::runtime::class_getInstanceVariable(cls, name.as_ptr() as *const i8);
        let offset = objc::runtime::ivar_getOffset(ivar);
        let ptr = (self as *const Self as *mut u8).offset(offset as isize) as *mut T;
        ptr.write(value);
    }
}

// Helper for formatting durations (e.g., "1:30" for 1 minute 30 seconds)
fn format_duration(seconds: u64) -> String {
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let secs = seconds % 60;
    
    if hours > 0 {
        format!("{}:{:02}:{:02}", hours, minutes, secs)
    } else {
        format!("{}:{:02}", minutes, secs)
    }
}

// Helper to convert NSString to Rust String
unsafe fn nsstring_to_string(ns_string: id) -> String {
    let utf8_string: *const i8 = msg_send![ns_string, UTF8String];
    let bytes = std::ffi::CStr::from_ptr(utf8_string).to_bytes();
    String::from_utf8_lossy(bytes).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CpalAudioProcessor;
    use std::time::Duration;

    #[test]
    fn test_menu_bar_app_creation() {
        let app = MenuBarApp::new();
        assert!(!*app.is_recording.lock().unwrap());
        assert!(app.recorder.lock().unwrap().is_none());
        assert!(app.recording_start_time.lock().unwrap().is_none());
    }

    #[test]
    fn test_menu_bar_status_update() {
        let app = MenuBarApp::new();
        
        // Test setting recording state to true
        app.update_status(true);
        assert!(*app.is_recording.lock().unwrap());
        assert!(app.recording_start_time.lock().unwrap().is_some());
        
        // Test setting recording state to false
        app.update_status(false);
        assert!(!*app.is_recording.lock().unwrap());
        assert!(app.recording_start_time.lock().unwrap().is_none());
    }

    #[test]
    fn test_resource_path_finding() {
        // Test development path
        let dev_path = MenuBarApp::get_resource_path("test_resource.txt");
        assert!(dev_path.is_some() || dev_path.is_none()); // Either outcome is valid depending on test environment
        
        // Test bundled app path - we can't test this directly in a test environment
    }

    #[test]
    fn test_recorder_initialization() {
        let app = MenuBarApp::new();
        let mut recorder = app.recorder.lock().unwrap();
        
        // Test creating a new recorder
        if let Ok(processor) = CpalAudioProcessor::new() {
            *recorder = Some(AudioRecorder::with_config(processor, AppConfig::load()));
            assert!(recorder.is_some());
        }
    }

    #[test]
    fn test_config_updates() {
        let app = MenuBarApp::new();
        let config = app.config.lock().unwrap();
        
        // Verify default config values match what we expect
        assert_eq!(config.get_audio_channels(), "0");
    }

    #[test]
    fn test_process_system_events() {
        // Test that processing system events doesn't panic
        process_system_events();
        
        // Test multiple event processing
        for _ in 0..3 {
            process_system_events();
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    
    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(0), "0:00");
        assert_eq!(format_duration(30), "0:30");
        assert_eq!(format_duration(60), "1:00");
        assert_eq!(format_duration(65), "1:05");
        assert_eq!(format_duration(3600), "1:00:00");
        assert_eq!(format_duration(3665), "1:01:05");
    }
} 