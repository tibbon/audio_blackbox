# BlackBox Audio Recorder

**Always-on multichannel audio recording for macOS.** BlackBox sits in your menu bar and records every channel of your audio interface to WAV, like a flight recorder for your studio. Start it once and never lose a take again.

[![Download on the Mac App Store](https://toolbox.marketingtools.apple.com/api/v2/badges/download-on-the-mac-app-store/black/en-us)](https://apps.apple.com/us/app/blackbox-audio-recorder/id6759437659?mt=12)

<!-- TODO: hero screenshot of the menu bar dropdown with live meters -->

## Why BlackBox

- **Don't lose a take.** Files are written crash-safe and rotated on a schedule, so a crash or power cut costs you seconds of audio, not the whole session.
- **Set and forget.** Launch at login, start recording on launch, and resume after the Mac wakes from sleep. Start and stop from the menu bar or a global keyboard shortcut.
- **Built for real interfaces.** Multi-channel recording: tested to 64 channels, supports up to 255 simultaneously, from any Core Audio device.
- **Stays out of your DAW's way.** The audio thread never touches the disk, takes a lock, or allocates. The production write path records 64 channels at 192 kHz into a single multichannel file at about 14× real time on Apple Silicon.
- **Private.** No network access and no accounts. Recordings stay in the folder you choose.

## Features

- 16-, 24- (default), or 32-bit WAV, with dither on 16-bit
- One multichannel file, or one file per channel
- Automatic file rotation (hourly by default; anywhere from 1 second to 24 hours)
- Silence gate: stops writing during silence, resumes on signal without clipping the onset
- Per-channel peak meters with clip indication
- Low-disk safe stop
- Automatic restart when the device's stream fails or its sample rate changes
- VoiceOver support

Requires macOS 15 or later. $49.99 one-time, no subscription. Mac App Store: [BlackBox Audio Recorder](https://apps.apple.com/us/app/blackbox-audio-recorder/id6759437659?mt=12), or search "BlackBox Audio Recorder" in the Mac App Store. Support and privacy policy: [dollhousemediatech.com/blackbox](https://dollhousemediatech.com/blackbox/).

## Build from source

[![CI](https://github.com/tibbon/audio_blackbox/actions/workflows/rust.yml/badge.svg)](https://github.com/tibbon/audio_blackbox/actions/workflows/rust.yml)

The app is SwiftUI over a Rust recording engine. You need Xcode and a stable Rust toolchain; [SETUP.md](SETUP.md) has the full list.

```bash
git clone https://github.com/tibbon/audio_blackbox.git
cd audio_blackbox
make run-app                # build and launch the menu-bar app
cargo build --release       # or just the CLI recorder: target/release/blackbox
make check                  # the same gate CI runs
```

The CLI reads `blackbox.toml` from the working directory, and every setting can be overridden with a `BLACKBOX_*` environment variable. The variables are listed in [AGENTS.md](AGENTS.md#environment-variables-doll-198).

## How it works

```
Audio device → Core Audio callback → lock-free ring buffer → writer thread → WAV files
```

The real-time callback only copies samples into a ring buffer. A dedicated writer thread does all encoding, metering, and disk I/O, with seconds of buffer runway so brief disk stalls don't cause dropouts. [ARCHITECTURE.md](ARCHITECTURE.md) covers the threading model and invariants.

## Project docs

- [CHANGELOG.md](CHANGELOG.md): release notes
- [AGENTS.md](AGENTS.md): contributor workflow and invariants
- [SETUP.md](SETUP.md): development and release setup
- [ACKNOWLEDGMENTS.md](ACKNOWLEDGMENTS.md): third-party licenses

## License

[Business Source License 1.1](LICENSE). Personal, non-commercial use is permitted; commercial use requires a license from the author. The code converts to Apache License 2.0 on 2030-03-01.
