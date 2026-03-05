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
        let byte_rate = spec.sample_rate * u32::from(spec.channels) * u32::from(byte_width);
        let block_align = spec.channels * u16::from(byte_width);

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
        self.writer
            .write_all(&bytes[..self.byte_width as usize])?;
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
        let data_size = self.data_bytes_written.min(u64::from(u32::MAX)) as u32;
        let file_size = data_size + 36; // 44-byte header minus 8-byte RIFF preamble

        let pos = self.writer.stream_position()?;
        self.writer.seek(SeekFrom::Start(4))?;
        self.writer.write_all(&file_size.to_le_bytes())?;
        self.writer.seek(SeekFrom::Start(40))?;
        self.writer.write_all(&data_size.to_le_bytes())?;
        self.writer.seek(SeekFrom::Start(pos))?;
        Ok(())
    }
}
