# Makefile for BlackBox Audio Recorder

# Configuration
APP_NAME = BlackBox Audio Recorder
APP_VERSION = 0.1.0
BUNDLE_ID = com.blackbox.audiorecorder
MACOS_MIN_VERSION = 10.14
CARGO_BIN = cargo
RUSTC_BIN = rustc

# Directories
TARGET_DIR = target
RELEASE_DIR = $(TARGET_DIR)/release
DEBUG_DIR = $(TARGET_DIR)/debug
APP_BUNDLE_DIR = $(TARGET_DIR)/$(APP_NAME).app
IMAGES_DIR = images
RESOURCES_DIR = $(APP_BUNDLE_DIR)/Contents/Resources

# Binary
BIN_NAME = blackbox

# Development team ID for code signing (replace with your own)
TEAM_ID = YOURDEVELOPMENTTEAMID

# Default target
.PHONY: all
all: build

# Build the app in debug mode
.PHONY: build
build:
	$(CARGO_BIN) build

# Build the app in release mode
.PHONY: release
release:
	$(CARGO_BIN) build --release

# Run tests
.PHONY: test
test:
	$(CARGO_BIN) test

# Run linting
.PHONY: lint
lint:
	$(CARGO_BIN) clippy -- -D warnings
	$(CARGO_BIN) fmt --all -- --check

# Create macOS .app bundle (release mode)
.PHONY: app-bundle
app-bundle: release
	@echo "Creating macOS app bundle..."
	@mkdir -p $(APP_BUNDLE_DIR)/Contents/MacOS
	@mkdir -p $(APP_BUNDLE_DIR)/Contents/Resources/images
	@cp $(RELEASE_DIR)/$(BIN_NAME) $(APP_BUNDLE_DIR)/Contents/MacOS/
	@cp Info.plist $(APP_BUNDLE_DIR)/Contents/
	@cp -R $(IMAGES_DIR)/* $(APP_BUNDLE_DIR)/Contents/Resources/images/
	@[ -f $(IMAGES_DIR)/App.icns ] && cp $(IMAGES_DIR)/App.icns $(APP_BUNDLE_DIR)/Contents/Resources/ || echo "Warning: App.icns not found"
	@defaults write $(APP_BUNDLE_DIR)/Contents/Info.plist CFBundleShortVersionString $(APP_VERSION)
	@plutil -convert xml1 $(APP_BUNDLE_DIR)/Contents/Info.plist
	@echo "App bundle created at $(APP_BUNDLE_DIR)"

# Create debug app bundle
.PHONY: app-bundle-debug
app-bundle-debug: build
	@echo "Creating macOS debug app bundle..."
	@mkdir -p $(APP_BUNDLE_DIR)/Contents/MacOS
	@mkdir -p $(APP_BUNDLE_DIR)/Contents/Resources/images
	@cp $(DEBUG_DIR)/$(BIN_NAME) $(APP_BUNDLE_DIR)/Contents/MacOS/
	@cp Info.plist $(APP_BUNDLE_DIR)/Contents/
	@cp -R $(IMAGES_DIR)/* $(APP_BUNDLE_DIR)/Contents/Resources/images/
	@[ -f $(IMAGES_DIR)/App.icns ] && cp $(IMAGES_DIR)/App.icns $(APP_BUNDLE_DIR)/Contents/Resources/ || echo "Warning: App.icns not found"
	@defaults write $(APP_BUNDLE_DIR)/Contents/Info.plist CFBundleShortVersionString $(APP_VERSION)
	@plutil -convert xml1 $(APP_BUNDLE_DIR)/Contents/Info.plist
	@echo "Debug app bundle created at $(APP_BUNDLE_DIR)"

# Code sign the app (macOS)
.PHONY: sign
sign: app-bundle
	@echo "Signing app bundle..."
	@codesign --force --deep --sign "Developer ID Application: $(TEAM_ID)" --options runtime $(APP_BUNDLE_DIR)
	@echo "App signed."

# Create DMG installer
.PHONY: dmg
dmg: sign
	@echo "Creating DMG installer..."
	@hdiutil create -volname "$(APP_NAME)" -srcfolder $(APP_BUNDLE_DIR) -ov -format UDZO $(TARGET_DIR)/$(BIN_NAME)-$(APP_VERSION).dmg
	@echo "DMG created at $(TARGET_DIR)/$(BIN_NAME)-$(APP_VERSION).dmg"

# Notarize macOS app (requires Apple Developer account)
.PHONY: notarize
notarize: dmg
	@echo "Notarizing DMG..."
	@xcrun notarytool submit $(TARGET_DIR)/$(BIN_NAME)-$(APP_VERSION).dmg --apple-id "YOUR_APPLE_ID" --password "YOUR_APP_PASSWORD" --team-id "$(TEAM_ID)" --wait
	@echo "Notarization complete"

# Run the app directly
.PHONY: run
run:
	$(CARGO_BIN) run

# Clean build files
.PHONY: clean
clean:
	$(CARGO_BIN) clean
	rm -rf $(APP_BUNDLE_DIR)
	rm -f $(TARGET_DIR)/$(BIN_NAME)-$(APP_VERSION).dmg

# Create images directory
.PHONY: create-image-dirs
create-image-dirs:
	mkdir -p $(IMAGES_DIR)
	@echo "Created images directory. Please add idle_icon.png and recording_icon.png (16x16 PNG format)"

# Help
.PHONY: help
help:
	@echo "BlackBox Audio Recorder Makefile"
	@echo ""
	@echo "Targets:"
	@echo "  build           - Build debug version"
	@echo "  release         - Build release version"
	@echo "  test            - Run tests"
	@echo "  lint            - Run linting checks"
	@echo "  app-bundle      - Create macOS app bundle (release)"
	@echo "  app-bundle-debug - Create macOS app bundle (debug)"
	@echo "  sign            - Code sign the macOS app bundle"
	@echo "  dmg             - Create DMG installer"
	@echo "  notarize        - Notarize the app with Apple"
	@echo "  run             - Run the app directly"
	@echo "  clean           - Clean build files"
	@echo "  create-image-dirs - Create images directory for app icons"
	@echo "  help            - Show this help" 