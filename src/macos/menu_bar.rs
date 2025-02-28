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
            
            // Add separator
            let separator: id = msg_send![class!(NSMenuItem), separatorItem];
            let _: () = msg_send![menu, addItem:separator];
            
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
            
            MenuBarApp {
                status_item,
                is_recording: Arc::new(Mutex::new(false)),
                recorder: Arc::new(Mutex::new(None)),
                config: Arc::new(Mutex::new(AppConfig::load())),
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
                            
                            // Update UI
                            *is_recording.lock().unwrap() = false;
                            let title = NSString::alloc(nil).init_str("Start Recording");
                            let _: () = msg_send![start_item, setTitle:title];
                            
                            // Update status item color
                            let status_title = NSString::alloc(nil).init_str("●");
                            let _: () = msg_send![status_item, setTitle:status_title];
                            
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
                                            // Update UI
                                            *is_recording.lock().unwrap() = true;
                                            let title = NSString::alloc(nil).init_str("Stop Recording");
                                            let _: () = msg_send![start_item, setTitle:title];
                                            
                                            // Update status item color to red for recording
                                            let status_title = NSString::alloc(nil).init_str("🔴");
                                            let _: () = msg_send![status_item, setTitle:status_title];
                                        }
                                    }
                                }
                            }
                            
                            // Reset click state
                            let _: () = msg_send![start_item, setWasClicked:NO];
                        }
                    }
                    
                    // Check the quit item
                    let quit_item: id = msg_send![menu, itemWithTag:2];
                    let clicked: BOOL = msg_send![quit_item, wasClicked];
                    
                    if clicked != NO {
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
            
            pool.drain();
        }
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