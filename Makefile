# Makefile for BlackBox Audio Recorder

# Configuration
APP_NAME = BlackBox Audio Recorder
APP_VERSION = 0.1.0
BUNDLE_ID = com.dollhousemediatech.blackbox
CARGO_BIN = cargo

# Directories
TARGET_DIR = target
RELEASE_DIR = $(TARGET_DIR)/release

# Binary
BIN_NAME = blackbox

# Development team ID for code signing (set via env: export TEAM_ID=...)
TEAM_ID ?= $(DEVELOPMENT_TEAM)

# --- Rust ---

# Default target
.PHONY: all
all: build

# Build Rust in debug mode
.PHONY: build
build:
	$(CARGO_BIN) build

# Build Rust in release mode
.PHONY: release
release:
	$(CARGO_BIN) build --release

# Run tests
.PHONY: test
test:
	$(CARGO_BIN) test

# Run linting (matches CI)
.PHONY: lint
lint:
	$(CARGO_BIN) clippy --all-targets --no-default-features -- -D warnings
	$(CARGO_BIN) fmt --all -- --check

# Run the CLI directly
.PHONY: run
run:
	$(CARGO_BIN) run

# Clean build files
.PHONY: clean
clean:
	$(CARGO_BIN) clean

# Verify: fmt + clippy + test + build (run before committing)
.PHONY: verify
verify:
	$(CARGO_BIN) fmt --all -- --check
	$(CARGO_BIN) clippy --all-targets --no-default-features -- -D warnings
	$(CARGO_BIN) test -- --test-threads=1
	$(CARGO_BIN) build

# --- SwiftUI Menu Bar App ---

XCODE_PROJECT = BlackBoxApp/BlackBoxApp.xcodeproj
XCODE_SCHEME = BlackBoxApp
XCODE_CONFIG = Release
SWIFT_APP_DIR = BlackBoxApp
SWIFT_APP_BUNDLE = $(RELEASE_DIR)/BlackBox Audio Recorder.app

# Build the Rust static library with FFI exports
.PHONY: rust-lib
rust-lib:
	$(CARGO_BIN) build --release --features ffi

# Build universal (fat) Rust static library for aarch64 + x86_64
.PHONY: rust-lib-universal
rust-lib-universal:
	$(CARGO_BIN) build --release --features ffi --target=aarch64-apple-darwin
	$(CARGO_BIN) build --release --features ffi --target=x86_64-apple-darwin
	@mkdir -p $(TARGET_DIR)/universal
	lipo -create \
		$(TARGET_DIR)/aarch64-apple-darwin/release/libblackbox.a \
		$(TARGET_DIR)/x86_64-apple-darwin/release/libblackbox.a \
		-output $(TARGET_DIR)/universal/libblackbox.a
	@echo "Universal library created at $(TARGET_DIR)/universal/libblackbox.a"

# Build the SwiftUI app (depends on rust-lib)
.PHONY: swift-app
swift-app: rust-lib
	@if command -v xcodebuild >/dev/null 2>&1 && xcodebuild -version >/dev/null 2>&1; then \
		echo "Building with xcodebuild..."; \
		xcodebuild -project $(XCODE_PROJECT) -scheme $(XCODE_SCHEME) -configuration $(XCODE_CONFIG) build; \
		BUILT_APP=$$(xcodebuild -project $(XCODE_PROJECT) -scheme $(XCODE_SCHEME) -configuration $(XCODE_CONFIG) -showBuildSettings 2>/dev/null | grep ' BUILT_PRODUCTS_DIR' | sed 's/.*= //'); \
		if [ -d "$$BUILT_APP/BlackBox Audio Recorder.app" ]; then \
			rm -rf "$(SWIFT_APP_BUNDLE)"; \
			cp -R "$$BUILT_APP/BlackBox Audio Recorder.app" "$(SWIFT_APP_BUNDLE)"; \
			echo "Copied app bundle to $(SWIFT_APP_BUNDLE)"; \
		fi; \
	else \
		echo "Error: xcodebuild is required to build the SwiftUI app."; \
		exit 1; \
	fi

# Build both Rust lib + Swift app
.PHONY: app
app: swift-app

# Build and run the SwiftUI menu bar app
.PHONY: run-app
run-app: swift-app
	@open "$(SWIFT_APP_BUNDLE)"

# Code sign the SwiftUI app
.PHONY: sign
sign: swift-app
	@echo "Signing app bundle..."
	@codesign --force --deep --sign "Developer ID Application: $(TEAM_ID)" --options runtime "$(SWIFT_APP_BUNDLE)"
	@echo "App signed."

# Create DMG installer from the SwiftUI app
.PHONY: dmg
dmg: sign
	@echo "Creating DMG installer..."
	@hdiutil create -volname "$(APP_NAME)" -srcfolder "$(SWIFT_APP_BUNDLE)" -ov -format UDZO $(TARGET_DIR)/$(BIN_NAME)-$(APP_VERSION).dmg
	@echo "DMG created at $(TARGET_DIR)/$(BIN_NAME)-$(APP_VERSION).dmg"

# Notarize macOS app (requires Apple Developer account)
.PHONY: notarize
notarize: dmg
	@echo "Notarizing DMG..."
	@xcrun notarytool submit $(TARGET_DIR)/$(BIN_NAME)-$(APP_VERSION).dmg --apple-id "YOUR_APPLE_ID" --password "YOUR_APP_PASSWORD" --team-id "$(TEAM_ID)" --wait
	@echo "Notarization complete"

# Regenerate Xcode project from project.yml (requires xcodegen)
.PHONY: xcodegen
xcodegen:
	cd $(SWIFT_APP_DIR) && xcodegen generate

# Help
.PHONY: help
help:
	@echo "BlackBox Audio Recorder Makefile"
	@echo ""
	@echo "Rust:"
	@echo "  build           - Build debug version"
	@echo "  release         - Build release version"
	@echo "  test            - Run tests"
	@echo "  lint            - Run linting checks (matches CI)"
	@echo "  verify          - Run fmt + clippy + test + build"
	@echo "  run             - Run the CLI directly"
	@echo "  clean           - Clean build files"
	@echo ""
	@echo "SwiftUI App:"
	@echo "  rust-lib        - Build Rust static library with FFI"
	@echo "  rust-lib-universal - Build universal (arm64 + x86_64) static library"
	@echo "  swift-app       - Build SwiftUI menu bar app"
	@echo "  app             - Build Rust lib + Swift app (alias for swift-app)"
	@echo "  run-app         - Build and run the SwiftUI app"
	@echo "  sign            - Code sign the app"
	@echo "  dmg             - Create DMG installer"
	@echo "  notarize        - Notarize the app with Apple"
	@echo "  xcodegen        - Regenerate Xcode project from project.yml"
	@echo "  help            - Show this help"
