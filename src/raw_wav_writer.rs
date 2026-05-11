//! Minimal WAV writer optimised for the BlackBox write path.
//!
//! Replaces `hound::WavWriter` on the hot path to eliminate per-sample
//! dynamic dispatch: no runtime match on `bits_per_sample`, no range
//! check, and a single `write_all` per sample instead of three
//! separate `write_u8` calls for 24-bit.

use std::fs::File;
use std::io::{self, BufWriter, Seek, SeekFrom, Write};

/// WAV spec — mirrors the subset of `hound::WavSpec` we actually use.
#[derive(Debug, Clone, Copy)]
pub struct WavSpec {
    pub channels: u16,
    pub sample_rate: u32,
    pub bits_per_sample: u16,
}

/// Lightweight WAV writer that writes PCM data directly to a `BufWriter<File>`.
///
/// Unlike `hound::WavWriter`, the per-sample write compiles down to a single
/// `to_le_bytes()` slice + `write_all` — no match, no range check.
pub struct RawWavWriter {
    writer: BufWriter<File>,
    /// Total PCM data bytes written so far.
    data_bytes_written: u64,
    /// Bytes per sample (2 for 16-bit, 3 for 24-bit, 4 for 32-bit).
    byte_width: u8,
}

/// 64 KB write buffer — same as the constant in writer_thread.rs.
const WAV_BUF_CAPACITY: usize = 65_536;

impl RawWavWriter {
    /// Create a new WAV file at `path` with the given spec.
    pub fn create(path: &str, spec: WavSpec) -> io::Result<Self> {
        let file = File::create(path)?;
        let mut writer = BufWriter::with_capacity(WAV_BUF_CAPACITY, file);
        let byte_width = (spec.bits_per_sample / 8) as u8;

        // Write the 44-byte RIFF/WAV header with placeholder sizes.
        // Saturating arithmetic so an extreme spec (e.g. 384 kHz × 32-bit
        // × hundreds of channels) caps the header value rather than
        // wrapping silently into an OS-accepted-but-misinterpreted u32
        // (DOLL-111). Defense-in-depth alongside DOLL-95's data_size fix.
        let byte_rate = spec
            .sample_rate
            .saturating_mul(u32::from(spec.channels))
            .saturating_mul(u32::from(byte_width));
        // Widen the multiply to u32 to avoid wrapping inside u16 for
        // hypothetical >16k-channel specs; truncate via try_into. With
        // MAX_CHANNELS = 255 this can't actually exceed u16, but the
        // explicit widening documents the invariant.
        let block_align: u16 = u32::from(spec.channels)
            .saturating_mul(u32::from(byte_width))
            .try_into()
            .unwrap_or(u16::MAX);

        writer.write_all(b"RIFF")?;
        writer.write_all(&0_u32.to_le_bytes())?; // placeholder file size
        writer.write_all(b"WAVE")?;
        writer.write_all(b"fmt ")?;
        writer.write_all(&16_u32.to_le_bytes())?; // PCM fmt chunk size
        writer.write_all(&1_u16.to_le_bytes())?; // PCM format tag
        writer.write_all(&spec.channels.to_le_bytes())?;
        writer.write_all(&spec.sample_rate.to_le_bytes())?;
        writer.write_all(&byte_rate.to_le_bytes())?;
        writer.write_all(&block_align.to_le_bytes())?;
        writer.write_all(&spec.bits_per_sample.to_le_bytes())?;
        writer.write_all(b"data")?;
        writer.write_all(&0_u32.to_le_bytes())?; // placeholder data size

        Ok(Self {
            writer,
            data_bytes_written: 0,
            byte_width,
        })
    }

    /// Write a single i32 sample as little-endian bytes.
    ///
    /// For 24-bit: writes the low 3 bytes.  For 16-bit: low 2 bytes.
    /// For 32-bit: all 4 bytes.  No match — the slice length is constant
    /// per writer instance and the compiler optimises accordingly.
    #[inline]
    pub fn write_sample(&mut self, sample: i32) -> io::Result<()> {
        let bytes = sample.to_le_bytes();
        self.writer.write_all(&bytes[..self.byte_width as usize])?;
        self.data_bytes_written += u64::from(self.byte_width);
        Ok(())
    }

    /// Flush buffered data and update the WAV header so the file is valid
    /// up to this point (crash-safe recovery).
    pub fn flush(&mut self) -> io::Result<()> {
        // Flush the BufWriter first so all data reaches the file.
        self.writer.flush()?;
        self.update_header()?;
        self.writer.flush()?;
        Ok(())
    }

    /// Finalize the WAV file: update the header with final sizes.
    /// Consumes self, closing the file.
    pub fn finalize(mut self) -> io::Result<()> {
        self.writer.flush()?;
        self.update_header()?;
        self.writer.flush()?;
        Ok(())
    }

    /// Seek back and write the correct RIFF and data chunk sizes.
    fn update_header(&mut self) -> io::Result<()> {
        // DOLL-204: WAV's chunk-size field is `u32`, so the on-disk header
        // can't represent more than 4 GiB of audio data. Files larger
        // than that keep growing on disk but the data-chunk-size cap
        // out at `u32::MAX` — readers fail to import or silently
        // truncate to the first 4 GiB. Log a warning so the operator
        // knows their recording will be partially unreadable;
        // upgrading to RF64 / W64 is out of scope.
        if self.data_bytes_written > u64::from(u32::MAX) {
            log::error!(
                "WAV file exceeds 4 GiB ({} bytes); header data-chunk-size capped at u32::MAX. \
                 Players may fail to import or truncate to the first 4 GiB. \
                 Reduce recording_cadence or channel count to stay under the cap.",
                self.data_bytes_written
            );
        }
        let data_size = self.data_bytes_written.min(u64::from(u32::MAX)) as u32;
        // Saturating add: data_size = u32::MAX (a single 4 GiB+ WAV) would wrap
        // in release and panic in debug. The header value can't represent more
        // than u32::MAX anyway, so saturating is the most-honest answer.
        let file_size = data_size.saturating_add(36); // 44-byte header minus 8-byte RIFF preamble

        let pos = self.writer.stream_position()?;
        self.writer.seek(SeekFrom::Start(4))?;
        self.writer.write_all(&file_size.to_le_bytes())?;
        self.writer.seek(SeekFrom::Start(40))?;
        self.writer.write_all(&data_size.to_le_bytes())?;
        self.writer.seek(SeekFrom::Start(pos))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// Reads byte_rate (offset 28-31) and block_align (offset 32-33) from a WAV header.
    fn read_header_fields(path: &str) -> (u32, u16) {
        let bytes = std::fs::read(path).unwrap();
        let byte_rate = u32::from_le_bytes(bytes[28..32].try_into().unwrap());
        let block_align = u16::from_le_bytes(bytes[32..34].try_into().unwrap());
        (byte_rate, block_align)
    }

    #[test]
    fn test_header_byte_rate_normal_spec() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("normal.wav").to_str().unwrap().to_string();
        let spec = WavSpec {
            channels: 2,
            sample_rate: 44100,
            bits_per_sample: 16,
        };
        drop(RawWavWriter::create(&path, spec).unwrap()); // close the file
        let (byte_rate, block_align) = read_header_fields(&path);
        assert_eq!(byte_rate, 44100 * 2 * 2);
        assert_eq!(block_align, 2 * 2);
    }

    #[test]
    fn test_header_byte_rate_extreme_spec_does_not_wrap() {
        // 384 kHz × 32-bit × 255 channels = ~3.92e8, well within u32 — but
        // verify the saturating chain doesn't accidentally break the
        // straightforward case (DOLL-111).
        let dir = tempdir().unwrap();
        let path = dir.path().join("extreme.wav").to_str().unwrap().to_string();
        let spec = WavSpec {
            channels: 255,
            sample_rate: 384_000,
            bits_per_sample: 32,
        };
        drop(RawWavWriter::create(&path, spec).unwrap());
        let (byte_rate, block_align) = read_header_fields(&path);
        assert_eq!(byte_rate, 384_000_u32 * 255 * 4);
        assert_eq!(block_align, 255 * 4);
    }
}
