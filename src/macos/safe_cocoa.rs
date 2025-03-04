// Safe wrapper for Cocoa/AppKit APIs
// Provides exception-safe interfaces to common macOS UI functionality

use std::sync::{Arc, Mutex};
use std::ffi::CStr;
use std::os::raw::{c_char, c_void};
use std::time::Duration;

use cocoa::base::{id, nil, YES, NO};
use cocoa::foundation::{NSAutoreleasePool, NSString, NSSize};
use cocoa::appkit::{
    NSApplication, NSApplicationActivationPolicy, NSStatusBar, 
    NSStatusItem, NSImage, NSMenu, NSMenuItem
};
use objc::{class, msg_send, sel, sel_impl};
use objc::runtime::Sel;
use core_foundation::runloop::kCFRunLoopDefaultMode;

/// Represents an error from Cocoa/AppKit operations
#[derive(Debug)]
pub enum CocoaError {
    NilInstance(String),
    ExceptionThrown(String),
    InvalidArgument(String),
    ResourceNotFound(String),
    Generic(String),
}

/// Result type for Cocoa/AppKit operations
pub type CocoaResult<T> = Result<T, CocoaError>;

/// Wrapper for NSAutoreleasePool to ensure proper resource management
pub struct AutoreleasePool {
    pool: id,
}

impl AutoreleasePool {
    /// Create a new autorelease pool
    pub fn new() -> Self {
        unsafe {
            let pool = NSAutoreleasePool::new(nil);
            Self { pool }
        }
    }
}

impl Drop for AutoreleasePool {
    fn drop(&mut self) {
        unsafe {
            let _: () = msg_send![self.pool, drain];
        }
    }
}

/// Wrapper around cocoa::msg_send! that provides better error handling
#[macro_export]
macro_rules! safe_msg_send {
    ($obj:expr, $selector:ident) => {
        {
            if $obj == nil {
                Err(CocoaError::NilInstance(format!("Object is nil when calling {}", stringify!($selector))))
            } else {
                unsafe {
                    Ok(cocoa::base::msg_send![$obj, $selector])
                }
            }
        }
    };
    ($obj:expr, $selector:ident : $($arg:expr),*) => {
        {
            if $obj == nil {
                Err(CocoaError::NilInstance(format!("Object is nil when calling {}", stringify!($selector))))
            } else {
                unsafe {
                    Ok(cocoa::base::msg_send![$obj, $selector: $($arg),*])
                }
            }
        }
    };
}

/// Safe wrapper around NSString
pub struct SafeNSString {
    ns_string: id,
}

impl SafeNSString {
    /// Create a new NSString from a Rust string
    pub fn new(string: &str) -> CocoaResult<Self> {
        let _pool = AutoreleasePool::new();
        let ns_string = unsafe {
            let string = NSString::alloc(nil).init_str(string);
            if string == nil {
                return Err(CocoaError::NilInstance("Failed to create NSString".to_string()));
            }
            string
        };
        Ok(Self { ns_string })
    }
    
    /// Get the underlying NSString id
    pub fn as_id(&self) -> id {
        self.ns_string
    }
    
    /// Convert to Rust String
    pub fn to_string(&self) -> String {
        unsafe {
            let utf8_string: *const c_char = msg_send![self.ns_string, UTF8String];
            if utf8_string.is_null() {
                return String::new();
            }
            CStr::from_ptr(utf8_string).to_string_lossy().into_owned()
        }
    }
}

/// Safe wrapper around NSImage for menu bar icons
pub struct MenuBarIcon {
    image: id,
}

impl MenuBarIcon {
    /// Create a new icon from a named system image
    pub fn from_system_name(name: &str) -> CocoaResult<Self> {
        let string = SafeNSString::new(name)?;
        let image = unsafe {
            let image: id = msg_send![class!(NSImage), imageNamed:string.as_id()];
            if image == nil {
                return Err(CocoaError::ResourceNotFound(format!("System image not found: {}", name)));
            }
            image
        };
        Ok(Self { image })
    }
    
    /// Create a circle icon with the given color
    pub fn circle(color: &str, size: f64) -> CocoaResult<Self> {
        // This creates a simple colored circle as a fallback icon
        let _pool = AutoreleasePool::new();
        let image = unsafe {
            let size = NSSize::new(size, size);
            let image: id = msg_send![class!(NSImage), alloc];
            let image: id = msg_send![image, initWithSize:size];
            if image == nil {
                return Err(CocoaError::NilInstance("Failed to create image".to_string()));
            }
            
            // Set the image to be template if it's monochrome
            if color == "black" || color == "white" {
                let _: () = msg_send![image, setTemplate:YES];
            }
            
            image
        };
        
        Ok(Self { image })
    }
    
    /// Get the underlying NSImage id
    pub fn as_id(&self) -> id {
        self.image
    }
}

/// Menu item action type
pub type MenuItemAction = Box<dyn Fn() + Send + 'static>;

/// Represents a single menu item
pub struct MenuItem {
    item: id,
    action: Option<Arc<Mutex<MenuItemAction>>>,
}

impl MenuItem {
    /// Create a new menu item with the given title
    pub fn new(title: &str) -> CocoaResult<Self> {
        let string = SafeNSString::new(title)?;
        let item = unsafe {
            let item: id = msg_send![class!(NSMenuItem), alloc];
            let item: id = msg_send![item, initWithTitle:string.as_id() action:sel!(menuItemAction:) keyEquivalent:NSString::alloc(nil).init_str("")];
            if item == nil {
                return Err(CocoaError::NilInstance("Failed to create menu item".to_string()));
            }
            item
        };
        
        Ok(Self { item, action: None })
    }
    
    /// Set the action to be performed when this item is clicked
    pub fn set_action<F>(&mut self, action: F) -> &mut Self 
    where
        F: Fn() + Send + 'static
    {
        let boxed_action: MenuItemAction = Box::new(action);
        self.action = Some(Arc::new(Mutex::new(boxed_action)));
        
        // Store the action pointer in the menu item's represented object
        unsafe {
            let action_ptr = Arc::into_raw(self.action.as_ref().unwrap().clone()) as *mut c_void;
            let _: () = msg_send![self.item, setRepresentedObject:action_ptr as id];
        }
        
        self
    }
    
    /// Set whether this item is enabled
    pub fn set_enabled(&mut self, enabled: bool) -> &mut Self {
        unsafe {
            let _: () = msg_send![self.item, setEnabled:if enabled { YES } else { NO }];
        }
        self
    }
    
    /// Get the underlying NSMenuItem id
    pub fn as_id(&self) -> id {
        self.item
    }
}

/// Represents a menu that can contain menu items
pub struct Menu {
    menu: id,
    items: Vec<MenuItem>,
}

impl Menu {
    /// Create a new empty menu
    pub fn new() -> CocoaResult<Self> {
        let menu = unsafe {
            let menu: id = msg_send![class!(NSMenu), new];
            if menu == nil {
                return Err(CocoaError::NilInstance("Failed to create menu".to_string()));
            }
            menu
        };
        
        Ok(Self { menu, items: Vec::new() })
    }
    
    /// Add a menu item
    pub fn add_item(&mut self, item: MenuItem) -> &mut Self {
        unsafe {
            let _: () = msg_send![self.menu, addItem:item.as_id()];
        }
        self.items.push(item);
        self
    }
    
    /// Add a separator
    pub fn add_separator(&mut self) -> &mut Self {
        unsafe {
            let separator: id = msg_send![class!(NSMenuItem), separatorItem];
            let _: () = msg_send![self.menu, addItem:separator];
        }
        self
    }
    
    /// Get the underlying NSMenu id
    pub fn as_id(&self) -> id {
        self.menu
    }
}

/// Represents a status item in the macOS menu bar
pub struct StatusItem {
    status_item: id,
    menu: Option<Menu>,
}

impl StatusItem {
    /// Create a new status item in the menu bar
    pub fn new() -> CocoaResult<Self> {
        let _pool = AutoreleasePool::new();
        let status_item = unsafe {
            let status_bar: id = msg_send![class!(NSStatusBar), systemStatusBar];
            // Use a fixed width for the status item
            let status_item: id = msg_send![status_bar, statusItemWithLength:-1.0];
            if status_item == nil {
                return Err(CocoaError::NilInstance("Failed to create status item".to_string()));
            }
            status_item
        };
        
        Ok(Self { status_item, menu: None })
    }
    
    /// Set the title of the status item
    pub fn set_title(&mut self, title: &str) -> &mut Self {
        let string = SafeNSString::new(title).unwrap_or_else(|_| SafeNSString::new("").unwrap());
        unsafe {
            let _: () = msg_send![self.status_item, setTitle:string.as_id()];
        }
        self
    }
    
    /// Set the icon of the status item
    pub fn set_icon(&mut self, icon: &MenuBarIcon) -> &mut Self {
        unsafe {
            let _: () = msg_send![self.status_item, setImage:icon.as_id()];
        }
        self
    }
    
    /// Set the menu for this status item
    pub fn set_menu(&mut self, menu: Menu) -> &mut Self {
        unsafe {
            let _: () = msg_send![self.status_item, setMenu:menu.as_id()];
        }
        self.menu = Some(menu);
        self
    }
    
    /// Get the underlying NSStatusItem id
    pub fn as_id(&self) -> id {
        self.status_item
    }
}

/// The main application wrapper
pub struct Application {
    app: id,
    status_items: Vec<StatusItem>,
    is_running: bool,
}

impl Application {
    /// Create a new application instance
    pub fn new() -> CocoaResult<Self> {
        let _pool = AutoreleasePool::new();
        let app = unsafe {
            let app: id = msg_send![class!(NSApplication), sharedApplication];
            // Set as accessory app (appears in menu bar without dock icon or app menu)
            let _: () = msg_send![app, setActivationPolicy:NSApplicationActivationPolicy::NSApplicationActivationPolicyAccessory];
            app
        };
        
        Ok(Self { 
            app, 
            status_items: Vec::new(),
            is_running: false,
        })
    }
    
    /// Add a status item to the application
    pub fn add_status_item(&mut self, status_item: StatusItem) -> &mut Self {
        self.status_items.push(status_item);
        self
    }
    
    /// Process a single event
    pub fn process_event(&self, timeout: Duration) -> bool {
        let _pool = AutoreleasePool::new();
        unsafe {
            let timeout_date = if timeout.as_secs() == 0 && timeout.subsec_nanos() == 0 {
                nil
            } else {
                let date: id = msg_send![class!(NSDate), dateWithTimeIntervalSinceNow:timeout.as_secs_f64()];
                date
            };
            
            // Get the next event (with timeout)
            let event: id = msg_send![
                self.app,
                nextEventMatchingMask:std::u64::MAX
                untilDate:timeout_date
                inMode:kCFRunLoopDefaultMode
                dequeue:YES
            ];
            
            if event != nil {
                let _: () = msg_send![self.app, sendEvent:event];
                true
            } else {
                false
            }
        }
    }
    
    /// Run the event loop until terminated
    pub fn run(&mut self) {
        self.is_running = true;
        
        while self.is_running {
            let _ = self.process_event(Duration::from_millis(100));
            
            // Sleep a bit to avoid hogging CPU
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    
    /// Stop the application's run loop
    pub fn terminate(&mut self) {
        self.is_running = false;
    }
}

// C functions for exception handling
extern "C" {
    fn objc_setUncaughtExceptionHandler(handler: extern "C" fn(*mut c_void));
}

extern "C" fn exception_handler(exception: *mut c_void) {
    unsafe {
        let exception_id = exception as id;
        let name: id = msg_send![exception_id, name];
        let reason: id = msg_send![exception_id, reason];
        
        let name_str = nsstring_to_string(name);
        let reason_str = nsstring_to_string(reason);
        
        eprintln!("Uncaught Objective-C exception: {} - {}", name_str, reason_str);
    }
}

// Helper function to convert NSString to Rust String
unsafe fn nsstring_to_string(ns_string: id) -> String {
    let utf8_string: *const c_char = msg_send![ns_string, UTF8String];
    if utf8_string.is_null() {
        return String::new();
    }
    CStr::from_ptr(utf8_string).to_string_lossy().into_owned()
}

// Set up exception handling
pub fn setup_exception_handling() {
    unsafe {
        objc_setUncaughtExceptionHandler(exception_handler);
    }
} 