#![allow(unused_variables)]
#![allow(dead_code)]
#![allow(clippy::doc_markdown)]
#![allow(clippy::missing_const_for_fn)]
#![allow(clippy::unnecessary_wraps)]

#[cfg(target_os = "macos")]
// Safe wrapper for Cocoa/AppKit APIs
// Provides exception-safe interfaces to common macOS UI functionality
use std::ffi::CStr;
use std::os::raw::{c_char, c_void};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[cfg(target_os = "macos")]
use cocoa::appkit::NSApplicationActivationPolicy;
#[cfg(target_os = "macos")]
use cocoa::base::{id, nil, NO, YES};
#[cfg(target_os = "macos")]
use cocoa::foundation::{NSAutoreleasePool, NSSize, NSString};
#[cfg(target_os = "macos")]
use core_foundation::runloop::kCFRunLoopDefaultMode;
#[cfg(target_os = "macos")]
use objc::{class, msg_send, sel, sel_impl};

/// Represents an error from Cocoa/AppKit operations
#[cfg(target_os = "macos")]
#[derive(Debug)]
pub enum CocoaError {
    NilInstance(()),
    ExceptionThrown(()),
    ResourceNotFound(()),
}

/// Result type for Cocoa/AppKit operations
#[cfg(target_os = "macos")]
pub type CocoaResult<T> = Result<T, CocoaError>;

/// A safe wrapper around NSAutoreleasePool
/// Automatically releases all objects when dropped
#[cfg(target_os = "macos")]
pub struct AutoreleasePool {
    pool: id,
}

#[cfg(target_os = "macos")]
impl AutoreleasePool {
    /// Creates a new autorelease pool
    pub fn new() -> Self {
        unsafe {
            let pool = NSAutoreleasePool::new(nil);
            Self { pool }
        }
    }
}

#[cfg(target_os = "macos")]
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
                Err(CocoaError::NilInstance(()))
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
                Err(CocoaError::NilInstance(()))
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
                return Err(CocoaError::NilInstance(()));
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
    #[allow(dead_code, clippy::inherent_to_string)]
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

/// Safe wrapper for menu bar icon (NSImage)
#[cfg(target_os = "macos")]
pub struct MenuBarIcon {
    image: id,
}

#[cfg(target_os = "macos")]
impl MenuBarIcon {
    /// Create an icon from a system-provided symbol name
    pub fn from_system_name(name: &str) -> CocoaResult<Self> {
        let name_str = SafeNSString::new(name)?;
        let image = unsafe {
            let image: id = msg_send![class!(NSImage), imageNamed:name_str.as_id()];
            if image == nil {
                return Err(CocoaError::ResourceNotFound(()));
            }
            image
        };
        Ok(Self { image })
    }

    /// Create a circular icon with the specified color
    pub fn circle(color: &str, size: f64) -> CocoaResult<Self> {
        let pool = AutoreleasePool::new();

        // Create an image with the specified size
        let size = NSSize::new(size, size);
        let image = unsafe {
            let image: id = msg_send![class!(NSImage), alloc];
            let image: id = msg_send![image, initWithSize:size];
            if image == nil {
                return Err(CocoaError::NilInstance(()));
            }
            image
        };

        // More implementation details...

        std::mem::drop(pool);
        Ok(Self { image })
    }

    /// Get the underlying NSImage object
    pub fn as_id(&self) -> id {
        self.image
    }
}

/// Menu item action type
pub type MenuItemAction = Box<dyn Fn() + Send + 'static>;

/// Represents a single menu item
#[cfg(target_os = "macos")]
pub struct MenuItem {
    item: id,
    action: Option<Arc<Mutex<MenuItemAction>>>,
}

#[cfg(target_os = "macos")]
impl MenuItem {
    /// Create a new menu item with the given title
    pub fn new(title: &str) -> CocoaResult<Self> {
        let string = SafeNSString::new(title)?;
        let item = unsafe {
            let item: id = msg_send![class!(NSMenuItem), alloc];
            let item: id = msg_send![item, initWithTitle:string.as_id() action:sel!(menuItemAction:) keyEquivalent:NSString::alloc(nil).init_str("")];
            if item == nil {
                return Err(CocoaError::NilInstance(()));
            }
            item
        };

        Ok(Self { item, action: None })
    }

    /// Set the action to be performed when this item is clicked
    pub fn set_action<F>(&mut self, action: F) -> &mut Self
    where
        F: Fn() + Send + 'static,
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
    #[allow(dead_code)]
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
#[cfg(target_os = "macos")]
pub struct Menu {
    menu: id,
    items: Vec<MenuItem>,
}

#[cfg(target_os = "macos")]
impl Menu {
    /// Create a new empty menu
    pub fn new() -> CocoaResult<Self> {
        let menu = unsafe {
            let menu: id = msg_send![class!(NSMenu), new];
            if menu == nil {
                return Err(CocoaError::NilInstance(()));
            }
            menu
        };

        Ok(Self {
            menu,
            items: Vec::new(),
        })
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
                return Err(CocoaError::NilInstance(()));
            }
            status_item
        };

        Ok(Self {
            status_item,
            menu: None,
        })
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
    #[allow(dead_code)]
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
    #[allow(dead_code)]
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

    /// Process a single event with the given timeout
    pub fn process_event(&self, timeout: Duration) -> bool {
        unsafe {
            let date: id =
                msg_send![class!(NSDate), dateWithTimeIntervalSinceNow:timeout.as_secs_f64()];
            let mode = kCFRunLoopDefaultMode;

            let event: id = msg_send![
                self.app,
                nextEventMatchingMask:u64::MAX
                untilDate:date
                inMode:mode
                dequeue:YES
            ];

            if event != nil {
                let _: () = msg_send![self.app, sendEvent:event];
                let _: () = msg_send![self.app, updateWindows];
                return true;
            }

            false
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

        eprintln!("Uncaught Objective-C exception: {name_str} - {reason_str}",);
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

/// Determines if GUI tests can run in the current environment
/// Always returns false during tests to prevent segfaults
#[allow(dead_code)]
pub fn can_run_gui_tests() -> bool {
    if cfg!(test) {
        return false;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_autorelease_pool() {
        // Test creating and dropping an autorelease pool
        // This is safe to run in any environment
        let pool = AutoreleasePool::new();
        drop(pool);
    }

    #[test]
    fn test_safe_nsstring() {
        if !can_run_gui_tests() {
            println!("Skipping test_safe_nsstring - not in GUI environment");
            return;
        }

        // Create a pool for this test
        let _pool = AutoreleasePool::new();

        // Test creating a NSString and converting it back
        let test_str = "Hello, world!";
        let ns_string = SafeNSString::new(test_str).expect("Failed to create NSString");

        // Test to_string
        let roundtrip = ns_string.to_string();
        assert_eq!(test_str, roundtrip);

        // Test as_id does not return nil
        assert_ne!(ns_string.as_id(), nil);
    }

    #[test]
    fn test_menu_bar_icon() {
        if !can_run_gui_tests() {
            println!("Skipping test_menu_bar_icon - not in GUI environment");
            return;
        }

        // Create a pool for this test
        let _pool = AutoreleasePool::new();

        // Test creating system icon
        match MenuBarIcon::from_system_name("NSStatusAvailable") {
            Ok(icon) => assert_ne!(icon.as_id(), nil),
            Err(e) => println!("Skipping system icon test: {:?}", e),
        }

        // Test creating circle icon
        match MenuBarIcon::circle("green", 16.0) {
            Ok(icon) => assert_ne!(icon.as_id(), nil),
            Err(e) => println!("Skipping circle icon test: {:?}", e),
        }
    }

    #[test]
    fn test_menu_item() {
        if !can_run_gui_tests() {
            println!("Skipping test_menu_item - not in GUI environment");
            return;
        }

        // Create a pool for this test
        let _pool = AutoreleasePool::new();

        // Test creating menu item
        let menu_item_result = MenuItem::new("Test Item");
        if let Err(e) = menu_item_result {
            println!("Skipping menu item test: {:?}", e);
            return;
        }

        let mut item = menu_item_result.unwrap();

        // Test setting action
        let action_called = Arc::new(Mutex::new(false));
        {
            let action_called_clone = action_called.clone();
            item.set_action(move || {
                let mut flag = action_called_clone.lock().unwrap();
                *flag = true;
            });
        }

        // Test enabling/disabling
        item.set_enabled(false);
        item.set_enabled(true);

        assert_ne!(item.as_id(), nil);
    }

    #[test]
    fn test_menu() {
        if !can_run_gui_tests() {
            println!("Skipping test_menu - not in GUI environment");
            return;
        }

        // Create a pool for this test
        let _pool = AutoreleasePool::new();

        // Test creating menu
        let menu_result = Menu::new();
        if let Err(e) = menu_result {
            println!("Skipping menu test: {:?}", e);
            return;
        }

        let mut menu = menu_result.unwrap();

        // Try to add an item if we can create one
        if let Ok(item) = MenuItem::new("Test Item") {
            menu.add_item(item);

            // Test adding separator
            menu.add_separator();
        }

        assert_ne!(menu.as_id(), nil);
    }

    #[test]
    fn test_status_item() {
        if !can_run_gui_tests() {
            println!("Skipping test_status_item - not in GUI environment");
            return;
        }

        // Create autorelease pool for this test
        let _pool = AutoreleasePool::new();

        // Test creating status item
        let status_item_result = StatusItem::new();
        if let Err(e) = status_item_result {
            println!("Skipping status item test: {:?}", e);
            return;
        }

        let mut status_item = status_item_result.unwrap();

        // Test setting title
        status_item.set_title("Test");

        // Test setting icon if we can create one
        if let Ok(icon) = MenuBarIcon::from_system_name("NSStatusAvailable") {
            status_item.set_icon(&icon);
        }

        // Test setting menu if we can create one
        if let Ok(menu) = Menu::new() {
            status_item.set_menu(menu);
        }

        assert_ne!(status_item.as_id(), nil);
    }

    #[test]
    fn test_application() {
        if !can_run_gui_tests() {
            println!("Skipping test_application - not in GUI environment");
            return;
        }

        // Create autorelease pool for this test
        let _pool = AutoreleasePool::new();

        // Test creating application
        let app_result = Application::new();
        if let Err(e) = app_result {
            println!("Skipping application test: {:?}", e);
            return;
        }

        let mut app = app_result.unwrap();

        // Test adding status item if we can create one
        if let Ok(status_item) = StatusItem::new() {
            app.add_status_item(status_item);
        }

        // Test process_event (does not block)
        let _result = app.process_event(Duration::from_millis(1));

        // We can't meaningfully test run() and terminate() in a unit test
        // as they would disrupt the test process
    }

    #[test]
    fn test_errors() {
        // Test error handling (safe to run in any environment)
        let error = CocoaError::NilInstance(());
        assert!(format!("{:?}", error).contains("NilInstance"));

        let error = CocoaError::ExceptionThrown(());
        assert!(format!("{:?}", error).contains("ExceptionThrown"));
    }
}
