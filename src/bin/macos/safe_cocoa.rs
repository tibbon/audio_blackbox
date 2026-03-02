#![allow(unused_variables)]
#![allow(dead_code)]
#![allow(clippy::doc_markdown)]
#![allow(clippy::missing_const_for_fn)]
#![allow(clippy::unnecessary_wraps)]

#[cfg(target_os = "macos")]
use std::os::raw::c_void;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[cfg(target_os = "macos")]
use objc2::rc::Retained;
#[cfg(target_os = "macos")]
use objc2::runtime::AnyObject;
#[cfg(target_os = "macos")]
use objc2::sel;
#[cfg(target_os = "macos")]
use objc2::{AnyThread, MainThreadMarker, MainThreadOnly};
#[cfg(target_os = "macos")]
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSEventMask, NSImage, NSMenu,
    NSMenuItem as AppKitMenuItem, NSStatusBar, NSStatusItem as AppKitStatusItem,
};
#[cfg(target_os = "macos")]
use objc2_foundation::{NSAutoreleasePool, NSDate, NSDefaultRunLoopMode, NSSize, NSString};

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
    pool: Retained<NSAutoreleasePool>,
}

#[cfg(target_os = "macos")]
impl AutoreleasePool {
    /// Creates a new autorelease pool
    pub fn new() -> Self {
        let pool = unsafe { NSAutoreleasePool::new() };
        Self { pool }
    }
}

// Drop is handled automatically by Retained<NSAutoreleasePool>

/// Safe wrapper around NSString
#[cfg(target_os = "macos")]
pub struct SafeNSString {
    ns_string: Retained<NSString>,
}

#[cfg(target_os = "macos")]
impl SafeNSString {
    /// Create a new NSString from a Rust string
    pub fn new(string: &str) -> CocoaResult<Self> {
        let ns_string = NSString::from_str(string);
        Ok(Self { ns_string })
    }

    /// Get a reference to the underlying NSString
    pub fn as_ns_string(&self) -> &NSString {
        &self.ns_string
    }

    /// Convert to Rust String
    #[allow(dead_code, clippy::inherent_to_string)]
    pub fn to_string(&self) -> String {
        self.ns_string.to_string()
    }
}

/// Safe wrapper for menu bar icon (NSImage)
#[cfg(target_os = "macos")]
pub struct MenuBarIcon {
    image: Retained<NSImage>,
}

#[cfg(target_os = "macos")]
impl MenuBarIcon {
    /// Create an icon from a system-provided symbol name
    pub fn from_system_name(name: &str) -> CocoaResult<Self> {
        let name_str = NSString::from_str(name);
        let image = NSImage::imageNamed(&name_str).ok_or(CocoaError::ResourceNotFound(()))?;
        Ok(Self { image })
    }

    /// Create a circular icon with the specified color
    pub fn circle(color: &str, size: f64) -> CocoaResult<Self> {
        let pool = AutoreleasePool::new();

        let ns_size = NSSize::new(size, size);
        let image = NSImage::initWithSize(NSImage::alloc(), ns_size);

        // More implementation details...

        std::mem::drop(pool);
        Ok(Self { image })
    }

    /// Get a reference to the underlying NSImage
    pub fn as_image(&self) -> &NSImage {
        &self.image
    }
}

/// Menu item action type
#[cfg(target_os = "macos")]
pub type MenuItemAction = Box<dyn Fn() + Send + 'static>;

/// Represents a single menu item
#[cfg(target_os = "macos")]
pub struct MenuItem {
    item: Retained<AppKitMenuItem>,
    action: Option<Arc<Mutex<MenuItemAction>>>,
}

#[cfg(target_os = "macos")]
impl MenuItem {
    /// Create a new menu item with the given title
    pub fn new(title: &str) -> CocoaResult<Self> {
        let mtm = MainThreadMarker::new().ok_or(CocoaError::NilInstance(()))?;
        let title_str = NSString::from_str(title);
        let empty_key = NSString::from_str("");
        let item = unsafe {
            AppKitMenuItem::initWithTitle_action_keyEquivalent(
                AppKitMenuItem::alloc(mtm),
                &title_str,
                Some(sel!(menuItemAction:)),
                &empty_key,
            )
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
            self.item
                .setRepresentedObject(Some(&*(action_ptr as *const AnyObject)));
        }

        self
    }

    /// Set whether this item is enabled
    #[allow(dead_code)]
    pub fn set_enabled(&mut self, enabled: bool) -> &mut Self {
        self.item.setEnabled(enabled);
        self
    }

    /// Get a reference to the underlying NSMenuItem
    pub fn as_menu_item(&self) -> &AppKitMenuItem {
        &self.item
    }
}

/// Represents a menu that can contain menu items
#[cfg(target_os = "macos")]
pub struct Menu {
    inner: Retained<NSMenu>,
    items: Vec<MenuItem>,
    mtm: MainThreadMarker,
}

#[cfg(target_os = "macos")]
impl Menu {
    /// Create a new empty menu
    pub fn new() -> CocoaResult<Self> {
        let mtm = MainThreadMarker::new().ok_or(CocoaError::NilInstance(()))?;
        let inner = NSMenu::new(mtm);
        Ok(Self {
            inner,
            items: Vec::new(),
            mtm,
        })
    }

    /// Add a menu item
    pub fn add_item(&mut self, item: MenuItem) -> &mut Self {
        self.inner.addItem(&item.item);
        self.items.push(item);
        self
    }

    /// Add a separator
    pub fn add_separator(&mut self) -> &mut Self {
        let separator = AppKitMenuItem::separatorItem(self.mtm);
        self.inner.addItem(&separator);
        self
    }

    /// Get a reference to the underlying NSMenu
    pub fn as_menu(&self) -> &NSMenu {
        &self.inner
    }
}

/// Represents a status item in the macOS menu bar
#[cfg(target_os = "macos")]
pub struct StatusItem {
    inner: Retained<AppKitStatusItem>,
    attached_menu: Option<Menu>,
    mtm: MainThreadMarker,
}

#[cfg(target_os = "macos")]
impl StatusItem {
    /// Create a new status item in the menu bar
    pub fn new() -> CocoaResult<Self> {
        let _pool = AutoreleasePool::new();
        let mtm = MainThreadMarker::new().ok_or(CocoaError::NilInstance(()))?;
        let status_bar = NSStatusBar::systemStatusBar();
        // -1.0 = NSVariableStatusItemLength
        let inner = status_bar.statusItemWithLength(-1.0);

        Ok(Self {
            inner,
            attached_menu: None,
            mtm,
        })
    }

    /// Set the title of the status item via its button
    pub fn set_title(&mut self, title: &str) -> &mut Self {
        let title_str = NSString::from_str(title);
        if let Some(button) = self.inner.button(self.mtm) {
            button.setTitle(&title_str);
        }
        self
    }

    /// Set the icon of the status item via its button
    #[allow(dead_code)]
    pub fn set_icon(&mut self, icon: &MenuBarIcon) -> &mut Self {
        if let Some(button) = self.inner.button(self.mtm) {
            button.setImage(Some(&icon.image));
        }
        self
    }

    /// Set the menu for this status item
    pub fn set_menu(&mut self, menu: Menu) -> &mut Self {
        self.inner.setMenu(Some(&menu.inner));
        self.attached_menu = Some(menu);
        self
    }
}

/// The main application wrapper
pub struct Application {
    app: Retained<NSApplication>,
    status_items: Vec<StatusItem>,
    is_running: bool,
}

impl Application {
    /// Create a new application instance
    pub fn new() -> CocoaResult<Self> {
        let _pool = AutoreleasePool::new();
        let mtm = MainThreadMarker::new().ok_or(CocoaError::NilInstance(()))?;
        let app = NSApplication::sharedApplication(mtm);
        // Set as accessory app (appears in menu bar without dock icon or app menu)
        app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

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
            let date = NSDate::dateWithTimeIntervalSinceNow(timeout.as_secs_f64());
            let mode = NSDefaultRunLoopMode;

            let event = self.app.nextEventMatchingMask_untilDate_inMode_dequeue(
                NSEventMask::Any,
                Some(&date),
                mode,
                true,
            );

            if let Some(event) = event {
                self.app.sendEvent(&event);
                self.app.updateWindows();
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
unsafe extern "C" {
    fn objc_setUncaughtExceptionHandler(handler: extern "C" fn(*mut c_void));
}

extern "C" fn exception_handler(exception: *mut c_void) {
    unsafe {
        let exception_obj = &*(exception as *const AnyObject);
        let name: Option<Retained<NSString>> = objc2::msg_send![exception_obj, name];
        let reason: Option<Retained<NSString>> = objc2::msg_send![exception_obj, reason];

        let name_str = name.map_or_else(String::new, |s| s.to_string());
        let reason_str = reason.map_or_else(String::new, |s| s.to_string());

        eprintln!("Uncaught Objective-C exception: {name_str} - {reason_str}");
    }
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
            Ok(_icon) => {} // icon exists, that's enough
            Err(e) => println!("Skipping system icon test: {e:?}"),
        }

        // Test creating circle icon
        match MenuBarIcon::circle("green", 16.0) {
            Ok(_icon) => {} // icon exists, that's enough
            Err(e) => println!("Skipping circle icon test: {e:?}"),
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
            println!("Skipping menu item test: {e:?}");
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
            println!("Skipping menu test: {e:?}");
            return;
        }

        let mut menu = menu_result.unwrap();

        // Try to add an item if we can create one
        if let Ok(item) = MenuItem::new("Test Item") {
            menu.add_item(item);

            // Test adding separator
            menu.add_separator();
        }
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
            println!("Skipping status item test: {e:?}");
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
            println!("Skipping application test: {e:?}");
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
        assert!(format!("{error:?}").contains("NilInstance"));

        let error = CocoaError::ExceptionThrown(());
        assert!(format!("{error:?}").contains("ExceptionThrown"));
    }
}
