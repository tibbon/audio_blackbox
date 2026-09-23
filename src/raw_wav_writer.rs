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
pub(crate) struct WavSpec {
    pub channels: u16,
    pub sample_rate: u32,
    pub bits_per_sample: u16,
}

/// Lightweight WAV writer that writes PCM data directly to a `BufWriter<File>`.
///
/// Unlike `hound::WavWriter`, the per-sample write compiles down to a single
/// `to_le_bytes()` slice + `write_all` — no match, no range check.
pub(crate) struct RawWavWriter {
    writer: BufWriter<File>,
    /// Where the file was created, so callers can tell which file a failed
    /// `finalize` belongs to.
    path: String,
    /// Total PCM data bytes written so far.
    data_bytes_written: u64,
    /// Bytes per sample (2 for 16-bit, 3 for 24-bit, 4 for 32-bit).
    byte_width: u8,
    /// Bytes per frame (`byte_width * channels`).
    block_align: u16,
    /// Length of the header `create` wrote: where the audio starts
    /// (`PCM_HEADER_LEN` or `EXTENSIBLE_HEADER_LEN`).
    header_len: u64,
    /// Frames that failed to write and are still owed as silence, so this
    /// file stays in step with its sibling files in split mode (see
    /// `write_frame_in_step`). Always 0 for writers that use `write_frame`.
    owed_frames: u64,
    /// Test-only failure injection: the next this-many `write_frame` calls
    /// fail without writing anything, then writes succeed again.
    #[cfg(test)]
    fail_next_writes: u32,
}

/// Header length for plain `WAVE_FORMAT_PCM` (16-byte `fmt ` chunk).
pub(crate) const PCM_HEADER_LEN: u64 = 44;

/// Header length for `WAVE_FORMAT_EXTENSIBLE` (40-byte `fmt ` chunk).
pub(crate) const EXTENSIBLE_HEADER_LEN: u64 = 68;

/// Audio bytes after which the writer thread starts a new file. The RIFF
/// size field (`u32`) must hold the data plus up to 60 header bytes and a
/// pad byte; 1 MiB of margin covers the up to 64 KiB one writer-thread read
/// can add after the check.
pub(crate) const MAX_WAV_DATA_BYTES: u64 = u32::MAX as u64 - (1 << 20);

/// `WAVE_FORMAT_EXTENSIBLE` format tag.
const FORMAT_EXTENSIBLE: u16 = 0xFFFE;

/// `KSDATAFORMAT_SUBTYPE_PCM` (`00000001-0000-0010-8000-00AA00389B71`) as it
/// is stored on disk: the first three GUID fields little-endian.
const SUBTYPE_PCM: [u8; 16] = [
    0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71,
];

/// Whether `spec` needs `WAVE_FORMAT_EXTENSIBLE`. Microsoft's
/// `WAVEFORMATEX` documentation limits plain `WAVE_FORMAT_PCM` to 8- or
/// 16-bit samples and one or two channels; strict readers reject (or
/// misread) anything else under the plain tag.
pub(crate) const fn needs_extensible(spec: WavSpec) -> bool {
    spec.bits_per_sample > 16 || spec.channels > 2
}

/// `dwChannelMask` for `channels`: front center for mono, front left and
/// right for stereo, and 0 (speaker positions unassigned) above that, since
/// arbitrary interface inputs have no speaker positions.
pub(crate) const fn channel_mask(channels: u16) -> u32 {
    match channels {
        1 => 0x4, // SPEAKER_FRONT_CENTER
        2 => 0x3, // SPEAKER_FRONT_LEFT | SPEAKER_FRONT_RIGHT
        _ => 0,
    }
}

/// 64 KB write buffer — same as the constant in `writer_thread.rs`.
///
/// `write_frame` relies on every frame being smaller than this: `BufWriter`
/// either copies such a write into its buffer whole or, when making room
/// fails, returns the error before taking any of it, so a failed frame
/// leaves nothing behind. The largest frame is `MAX_CHANNELS` (255) 32-bit
/// samples, 1020 bytes.
const WAV_BUF_CAPACITY: usize = 65_536;

/// Zero bytes `write_frame_in_step` pays owed frames from. 1020 is a
/// multiple of every sample width (1 to 4 bytes) and is the largest frame.
const ZERO_CHUNK: [u8; 1020] = [0; 1020];

impl RawWavWriter {
    /// Create a new WAV file at `path` with the given spec.
    pub(crate) fn create(path: &str, spec: WavSpec) -> io::Result<Self> {
        // `write_sample` slices `byte_width` bytes out of an i32, so only whole
        // bytes from 1 to 4 work. Reject anything else before creating the
        // file: a wider spec used to panic on the first sample, one under 8
        // bits wrote no sample bytes, and a depth that isn't a multiple of 8
        // wrote a header that disagreed with the sample width.
        let byte_width = match spec.bits_per_sample {
            8 => 1_u8,
            16 => 2,
            24 => 3,
            32 => 4,
            other => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("unsupported bits_per_sample: {other}"),
                ));
            }
        };
        let file = File::create(path)?;
        lock_while_open(&file, path);
        let mut writer = BufWriter::with_capacity(WAV_BUF_CAPACITY, file);

        // Write the RIFF/WAVE header with placeholder sizes: 44 bytes for
        // plain PCM, 68 for WAVE_FORMAT_EXTENSIBLE (see `needs_extensible`).
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

        let extensible = needs_extensible(spec);
        writer.write_all(b"RIFF")?;
        writer.write_all(&0_u32.to_le_bytes())?; // placeholder file size
        writer.write_all(b"WAVE")?;
        writer.write_all(b"fmt ")?;
        if extensible {
            writer.write_all(&40_u32.to_le_bytes())?; // fmt chunk size
            writer.write_all(&FORMAT_EXTENSIBLE.to_le_bytes())?;
        } else {
            writer.write_all(&16_u32.to_le_bytes())?; // fmt chunk size
            writer.write_all(&1_u16.to_le_bytes())?; // WAVE_FORMAT_PCM
        }
        writer.write_all(&spec.channels.to_le_bytes())?;
        writer.write_all(&spec.sample_rate.to_le_bytes())?;
        writer.write_all(&byte_rate.to_le_bytes())?;
        writer.write_all(&block_align.to_le_bytes())?;
        // wBitsPerSample is the container size; every depth we write fills
        // its bytes, so it equals the valid bits below.
        writer.write_all(&spec.bits_per_sample.to_le_bytes())?;
        if extensible {
            writer.write_all(&22_u16.to_le_bytes())?; // cbSize
            writer.write_all(&spec.bits_per_sample.to_le_bytes())?; // wValidBitsPerSample
            writer.write_all(&channel_mask(spec.channels).to_le_bytes())?;
            writer.write_all(&SUBTYPE_PCM)?;
        }
        writer.write_all(b"data")?;
        writer.write_all(&0_u32.to_le_bytes())?; // placeholder data size

        Ok(Self {
            writer,
            path: path.to_owned(),
            data_bytes_written: 0,
            byte_width,
            block_align,
            header_len: if extensible {
                EXTENSIBLE_HEADER_LEN
            } else {
                PCM_HEADER_LEN
            },
            owed_frames: 0,
            #[cfg(test)]
            fail_next_writes: 0,
        })
    }

    /// Test-only: build a writer whose every `write_sample` fails.
    ///
    /// Creates a real file at `path`, reopens it READ-ONLY, and wraps the
    /// read-only fd in a 1-byte `BufWriter` — each multi-byte sample write
    /// bypasses the buffer and hits the fd directly, failing with EBADF.
    /// Lets tests drive the persistent-write-failure self-stop path
    /// (DOLL-349/437) deterministically, without filling a disk.
    #[cfg(test)]
    pub(crate) fn new_failing_for_tests(path: &str) -> Self {
        File::create(path).expect("create placeholder file");
        let read_only = File::open(path).expect("reopen read-only");
        Self {
            writer: BufWriter::with_capacity(1, read_only),
            path: path.to_owned(),
            data_bytes_written: 0,
            byte_width: 3,
            block_align: 3,
            header_len: EXTENSIBLE_HEADER_LEN,
            owed_frames: 0,
            fail_next_writes: 0,
        }
    }

    /// Test-only: make the next `n` `write_frame` calls fail without writing
    /// anything, as a transient I/O error would, after which writes succeed.
    #[cfg(test)]
    pub(crate) const fn fail_next_writes(&mut self, n: u32) {
        self.fail_next_writes = n;
    }

    /// Audio data bytes written so far (excluding the header).
    pub(crate) const fn data_bytes(&self) -> u64 {
        self.data_bytes_written
    }

    /// The path this writer was created at.
    pub(crate) fn path(&self) -> &str {
        &self.path
    }

    /// Write a single i32 sample as little-endian bytes.
    ///
    /// For 24-bit: writes the low 3 bytes.  For 16-bit: low 2 bytes.
    /// For 32-bit: all 4 bytes. Test helper for writing fixtures: the
    /// writer thread writes whole frames with `write_frame`, so a failed
    /// write can never leave part of a frame behind.
    #[cfg(test)]
    pub(crate) fn write_sample(&mut self, sample: i32) -> io::Result<()> {
        let bytes = sample.to_le_bytes();
        self.writer.write_all(&bytes[..self.byte_width as usize])?;
        self.data_bytes_written += u64::from(self.byte_width);
        Ok(())
    }

    /// Bytes per sample: 2, 3 or 4 (1 for 8-bit).
    pub(crate) const fn byte_width(&self) -> u8 {
        self.byte_width
    }

    /// Write one whole frame of already-encoded little-endian samples.
    ///
    /// The frame is written with a single `write_all`, so it either reaches
    /// the buffer whole or not at all (see `WAV_BUF_CAPACITY`). A transient
    /// error therefore drops a whole frame and the file stays frame-aligned;
    /// writing sample by sample used to skip just the failed sample, which
    /// shifted every later frame by one channel.
    #[inline]
    pub(crate) fn write_frame(&mut self, frame: &[u8]) -> io::Result<()> {
        #[cfg(test)]
        if self.fail_next_writes > 0 {
            self.fail_next_writes -= 1;
            return Err(io::Error::from(io::ErrorKind::Other));
        }
        self.writer.write_all(frame)?;
        self.data_bytes_written += frame.len() as u64;
        Ok(())
    }

    /// Write one frame, first paying any frames earlier calls failed to
    /// write as silence. For split mode, where each channel has its own
    /// file: a frame that fails on one channel's file is owed and written
    /// as zeros once that file accepts writes again, so every channel file
    /// keeps the same length and sample `n` of each is the same instant.
    /// If a failure never clears, that file ends short by the frames owed.
    #[inline]
    pub(crate) fn write_frame_in_step(&mut self, frame: &[u8]) -> io::Result<()> {
        let result = self
            .pay_owed_frames()
            .and_then(|()| self.write_frame(frame));
        if result.is_err() {
            self.owed_frames = self.owed_frames.saturating_add(1);
        }
        result
    }

    /// Write the owed frames as zeros, in chunks of at most `ZERO_CHUNK`.
    fn pay_owed_frames(&mut self) -> io::Result<()> {
        let frame_len = usize::from(self.block_align.max(1));
        // At least one frame per chunk: no frame is longer than ZERO_CHUNK.
        let per_chunk = (ZERO_CHUNK.len() / frame_len).max(1);
        while self.owed_frames > 0 {
            let frames = usize::try_from(self.owed_frames)
                .unwrap_or(usize::MAX)
                .min(per_chunk);
            let len = (frames * frame_len).min(ZERO_CHUNK.len());
            self.write_frame(&ZERO_CHUNK[..len])?;
            self.owed_frames -= frames as u64;
        }
        Ok(())
    }

    /// Flush buffered data and update the WAV header so the file is valid
    /// up to this point (crash-safe recovery).
    pub(crate) fn flush(&mut self) -> io::Result<()> {
        // Flush the BufWriter first so all data reaches the file.
        self.writer.flush()?;
        // No pad byte mid-recording: more samples will follow, and a pad here
        // would corrupt the data stream. The pad is written only at finalize.
        self.update_header(false)?;
        self.writer.flush()?;
        Ok(())
    }

    /// Finalize the WAV file: update the header with final sizes.
    /// Consumes self, closing the file.
    ///
    /// If the final flush fails (typically a full disk), the header is still
    /// rewritten to cover the whole frames that did reach the file, so the
    /// audio already on disk stays readable instead of keeping the
    /// placeholder "0 bytes of data" header. The flush error is returned.
    pub(crate) fn finalize(mut self) -> io::Result<()> {
        if let Err(flush_err) = self.writer.flush() {
            if let Err(e) = self.salvage_header() {
                log::error!("Could not rewrite the WAV header after a failed flush: {e}");
            }
            return Err(flush_err);
        }
        // DOLL-372: RIFF requires each chunk's data be padded to an even byte
        // count. A 24-bit-mono recording with an odd sample count ends the data
        // chunk on an odd boundary; append a single 0x00 pad byte so strict
        // parsers accept the file. The data-chunk size field stays unpadded
        // (the pad isn't data); the parent RIFF size includes it.
        let pad = self.data_bytes_written % 2 == 1;
        if pad {
            self.writer.write_all(&[0_u8])?;
        }
        self.update_header(pad)?;
        self.writer.flush()?;
        Ok(())
    }

    /// Point the header at the audio that reached the file after a flush
    /// failed. Writes through the `File` directly: the `BufWriter` would try
    /// to flush its stranded buffer again first.
    fn salvage_header(self) -> io::Result<()> {
        let block_align = self.block_align;
        let counted = self.data_bytes_written;
        let header_len = self.header_len;
        let (mut file, _unwritten) = self.writer.into_parts();
        let on_disk = file.metadata()?.len().saturating_sub(header_len);
        let data_bytes = salvaged_data_len(on_disk, counted, block_align);
        let (file_size, data_size) = header_sizes(data_bytes, false, header_len);
        file.seek(SeekFrom::Start(4))?;
        file.write_all(&file_size.to_le_bytes())?;
        file.seek(SeekFrom::Start(header_len - 4))?;
        file.write_all(&data_size.to_le_bytes())?;
        file.flush()
    }

    /// Seek back and write the correct RIFF and data chunk sizes. `pad` is true
    /// when a word-alignment pad byte has been appended after the data chunk
    /// (finalize only) — it's counted in the RIFF size but not the data size.
    fn update_header(&mut self, pad: bool) -> io::Result<()> {
        // DOLL-204: WAV's chunk-size field is `u32`, so the on-disk header
        // can't represent more than 4 GiB of audio data. Files larger
        // than that keep growing on disk but the data-chunk-size cap
        // out at `u32::MAX` — readers fail to import or silently
        // truncate to the first 4 GiB. Log a warning so the operator
        // knows their recording will be partially unreadable;
        // upgrading to RF64 / W64 is out of scope. The writer thread
        // rotates at MAX_WAV_DATA_BYTES, so this is a last resort.
        if self.data_bytes_written > u64::from(u32::MAX) {
            log::error!(
                "WAV file exceeds 4 GiB ({} bytes); header data-chunk-size capped at u32::MAX. \
                 Players may fail to import or truncate to the first 4 GiB. \
                 Reduce recording_cadence or channel count to stay under the cap.",
                self.data_bytes_written
            );
        }
        let (file_size, data_size) = header_sizes(self.data_bytes_written, pad, self.header_len);

        let pos = self.writer.stream_position()?;
        self.writer.seek(SeekFrom::Start(4))?;
        self.writer.write_all(&file_size.to_le_bytes())?;
        // The data chunk's size field is the last 4 bytes of the header.
        self.writer.seek(SeekFrom::Start(self.header_len - 4))?;
        self.writer.write_all(&data_size.to_le_bytes())?;
        self.writer.seek(SeekFrom::Start(pos))?;
        Ok(())
    }
}

/// Take an advisory exclusive lock (`flock`) on a file being recorded, held
/// until the file is closed.
///
/// `recover_recordings` skips a `.recording.wav` it can't lock, so a second
/// recorder (the CLI and the app, or two CLIs) sharing an output folder
/// doesn't finalize and rename a file this one is still writing. The kernel
/// drops the lock when the process exits, so a crashed recorder's files
/// become recoverable. A file system without `flock` support gets a warning;
/// recording goes on unlocked.
fn lock_while_open(file: &File, path: &str) {
    if let Err(e) = file.try_lock() {
        log::warn!(
            "Could not lock {path} while recording ({e}); crash recovery in another process could touch it"
        );
    }
}

/// The RIFF and data-chunk size fields for `data_bytes` of audio behind a
/// `header_len`-byte header.
fn header_sizes(data_bytes: u64, pad: bool, header_len: u64) -> (u32, u32) {
    let data_size = u32::try_from(data_bytes).unwrap_or(u32::MAX);
    // Saturating add: data_size = u32::MAX (a single 4 GiB+ WAV) would wrap
    // in release and panic in debug. The header value can't represent more
    // than u32::MAX anyway, so saturating is the most-honest answer.
    // The RIFF size counts the header minus its 8-byte preamble (36 for
    // plain PCM, 60 for EXTENSIBLE); +1 more when a word-alignment pad byte
    // trails the data chunk (DOLL-372).
    let after_preamble = u32::try_from(header_len - 8).unwrap_or(u32::MAX);
    let file_size = data_size
        .saturating_add(after_preamble)
        .saturating_add(u32::from(pad));
    (file_size, data_size)
}

/// Where the parts of a RIFF/WAVE file's header sit, found by walking its
/// chunks rather than assuming a fixed 44-byte layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WavLayout {
    /// Bytes per frame, from the `fmt ` chunk.
    pub block_align: u16,
    /// Offset of the `data` chunk's size field.
    pub data_size_offset: u64,
    /// Offset of the first audio byte (the `data` chunk body).
    pub data_offset: u64,
}

/// Parse the chunk layout from the start of a WAV file.
///
/// `bytes` must reach at least the `data` chunk header. Returns `None` when
/// it isn't a RIFF/WAVE file, has no `fmt ` chunk before `data`, or `bytes`
/// ends first.
pub(crate) fn parse_wav_layout(bytes: &[u8]) -> Option<WavLayout> {
    let read_u32 = |at: usize| -> Option<u32> {
        let field: [u8; 4] = bytes.get(at..at.checked_add(4)?)?.try_into().ok()?;
        Some(u32::from_le_bytes(field))
    };
    if bytes.get(0..4)? != b"RIFF" || bytes.get(8..12)? != b"WAVE" {
        return None;
    }
    let mut block_align = None;
    let mut pos = 12_usize;
    loop {
        let id = bytes.get(pos..pos.checked_add(4)?)?;
        let size = usize::try_from(read_u32(pos + 4)?).ok()?;
        if id == b"data" {
            let data_offset = u64::try_from(pos + 8).ok()?;
            return Some(WavLayout {
                block_align: block_align?,
                data_size_offset: data_offset - 4,
                data_offset,
            });
        }
        if id == b"fmt " {
            // block_align is the u16 at offset 12 of the fmt body.
            let at = pos + 8 + 12;
            let field: [u8; 2] = bytes.get(at..at + 2)?.try_into().ok()?;
            block_align = Some(u16::from_le_bytes(field)).filter(|&b| b > 0);
        }
        // Chunks are word-aligned: an odd-sized body is followed by a pad byte.
        pos = pos
            .checked_add(8)?
            .checked_add(size)?
            .checked_add(size % 2)?;
    }
}

/// The audio length, RIFF size field and data size field to write into a
/// file of `file_len` bytes with `layout`, recovering it after a crash.
///
/// The audio is everything after the data chunk header, rounded down to
/// whole frames (a crash can cut the last frame short), and capped so the
/// RIFF size (header after the preamble + data + a pad byte) still fits the
/// `u32` field. Returns `(data_bytes, riff_size, data_size)`.
pub(crate) fn recovered_sizes(file_len: u64, layout: WavLayout) -> (u64, u32, u32) {
    let align = u64::from(layout.block_align.max(1));
    let header_after_preamble = layout.data_offset - 8;
    let cap = u64::from(u32::MAX) - header_after_preamble - 1;
    let raw = file_len.saturating_sub(layout.data_offset).min(cap);
    let data_bytes = raw - raw % align;
    let pad = data_bytes % 2;
    // Both fit: data_bytes <= cap keeps the sum at or below u32::MAX.
    let riff = u32::try_from(header_after_preamble + data_bytes + pad).unwrap_or(u32::MAX);
    let data = u32::try_from(data_bytes).unwrap_or(u32::MAX);
    (data_bytes, riff, data)
}

/// How many data bytes a header rewritten after a failed flush can claim:
/// no more than reached the file (`on_disk`) or were written (`counted`),
/// rounded down to whole frames so a player never reads a torn frame.
fn salvaged_data_len(on_disk: u64, counted: u64, block_align: u16) -> u64 {
    let bytes = on_disk.min(counted);
    bytes - bytes % u64::from(block_align.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// A depth the writer can't emit is refused before any file exists: 0 and
    /// 4 would write no sample bytes, 12 and 20 aren't whole bytes, and 40
    /// would slice past the i32 in `write_sample`.
    #[test]
    fn create_rejects_unsupported_bit_depths() {
        let dir = tempdir().unwrap();
        for bits in [0_u16, 4, 12, 20, 40] {
            let path = dir.path().join(format!("bits{bits}.wav"));
            let spec = WavSpec {
                channels: 1,
                sample_rate: 48_000,
                bits_per_sample: bits,
            };
            let err = RawWavWriter::create(path.to_str().unwrap(), spec)
                .err()
                .unwrap_or_else(|| panic!("{bits}-bit spec should be rejected"));
            assert_eq!(err.kind(), io::ErrorKind::InvalidInput, "{bits}-bit spec");
            assert!(!path.exists(), "{bits}-bit spec must not create a file");
        }
    }

    /// Reads `byte_rate` (offset 28-31) and `block_align` (offset 32-33) from a WAV header.
    fn read_header_fields(path: &str) -> (u32, u16) {
        let bytes = std::fs::read(path).unwrap();
        let byte_rate = u32::from_le_bytes(bytes[28..32].try_into().unwrap());
        let block_align = u16::from_le_bytes(bytes[32..34].try_into().unwrap());
        (byte_rate, block_align)
    }

    #[test]
    fn test_header_byte_rate_normal_spec() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("normal.wav").to_str().unwrap().to_owned();
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
        let path = dir.path().join("extreme.wav").to_str().unwrap().to_owned();
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

    /// Reads the RIFF chunk size (offset 4) and the data-chunk size (offset
    /// 40 for plain PCM, 64 for EXTENSIBLE).
    fn read_size_fields(path: &str) -> (u32, u32) {
        let bytes = std::fs::read(path).unwrap();
        let riff_size = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        let at = usize::try_from(parse_wav_layout(&bytes).unwrap().data_size_offset).unwrap();
        let data_size = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        (riff_size, data_size)
    }

    /// Every header field of a plain PCM file (16-bit stereo) and an
    /// EXTENSIBLE one (24-bit mono, 16-bit with more than two channels), per
    /// Microsoft's `WAVEFORMATEXTENSIBLE` layout. Plain PCM is only valid
    /// for 8/16-bit and one or two channels.
    #[test]
    fn format_tag_follows_depth_and_channel_count() {
        let dir = tempdir().unwrap();
        let header = |name: &str, channels: u16, bits: u16| -> Vec<u8> {
            let path = dir.path().join(name).to_str().unwrap().to_owned();
            let spec = WavSpec {
                channels,
                sample_rate: 48_000,
                bits_per_sample: bits,
            };
            drop(RawWavWriter::create(&path, spec).unwrap());
            std::fs::read(&path).unwrap()
        };
        let u16_at = |b: &[u8], at: usize| u16::from_le_bytes(b[at..at + 2].try_into().unwrap());
        let u32_at = |b: &[u8], at: usize| u32::from_le_bytes(b[at..at + 4].try_into().unwrap());

        let pcm = header("pcm.wav", 2, 16);
        assert_eq!(pcm.len(), 44);
        assert_eq!(u32_at(&pcm, 16), 16, "fmt chunk size");
        assert_eq!(u16_at(&pcm, 20), 1, "WAVE_FORMAT_PCM");
        assert_eq!(&pcm[36..40], b"data");

        for (name, channels, bits, mask) in [
            ("mono24.wav", 1_u16, 24_u16, 0x4_u32),
            ("stereo32.wav", 2, 32, 0x3),
            ("quad16.wav", 4, 16, 0),
        ] {
            let ext = header(name, channels, bits);
            assert_eq!(ext.len(), 68, "{name}: header length");
            assert_eq!(u32_at(&ext, 16), 40, "{name}: fmt chunk size");
            assert_eq!(u16_at(&ext, 20), 0xFFFE, "{name}: WAVE_FORMAT_EXTENSIBLE");
            assert_eq!(u16_at(&ext, 22), channels, "{name}: channels");
            let block_align = channels * bits / 8;
            assert_eq!(u16_at(&ext, 32), block_align, "{name}: block align");
            assert_eq!(u16_at(&ext, 34), bits, "{name}: container bits");
            assert_eq!(u16_at(&ext, 36), 22, "{name}: cbSize");
            assert_eq!(u16_at(&ext, 38), bits, "{name}: valid bits");
            assert_eq!(u32_at(&ext, 40), mask, "{name}: channel mask");
            assert_eq!(ext[44..60], SUBTYPE_PCM, "{name}: PCM subformat GUID");
            assert_eq!(&ext[60..64], b"data", "{name}: data chunk");
        }
    }

    /// An EXTENSIBLE file round-trips through a WAV reader with the right
    /// spec and samples, and the header sizes count the 68-byte header.
    #[test]
    fn extensible_file_reads_back() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("ext.wav").to_str().unwrap().to_owned();
        let spec = WavSpec {
            channels: 4,
            sample_rate: 96_000,
            bits_per_sample: 24,
        };
        let mut w = RawWavWriter::create(&path, spec).unwrap();
        for i in 0..400 {
            w.write_sample(i * 1_000 - 200_000).unwrap();
        }
        w.flush().unwrap();
        assert_eq!(read_size_fields(&path), (60 + 1_200, 1_200), "flush");
        w.finalize().unwrap();
        assert_eq!(read_size_fields(&path), (60 + 1_200, 1_200), "finalize");

        let reader = hound::WavReader::open(&path).unwrap();
        assert_eq!(reader.spec().channels, 4);
        assert_eq!(reader.spec().sample_rate, 96_000);
        assert_eq!(reader.spec().bits_per_sample, 24);
        let samples: Vec<i32> = reader.into_samples::<i32>().map(Result::unwrap).collect();
        assert_eq!(samples.len(), 400);
        assert_eq!(samples[399], 399 * 1_000 - 200_000);
    }

    // DOLL-356: the prior tests only read byte_rate/block_align after create() —
    // update_header (data_size at offset 40, RIFF size at offset 4) was never
    // exercised. Write samples, finalize, and assert both size fields.
    #[test]
    fn test_header_sizes_after_writes() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("sized.wav").to_str().unwrap().to_owned();
        let spec = WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
        };
        let mut w = RawWavWriter::create(&path, spec).unwrap();
        let n = 100_u32;
        for i in 0..n {
            w.write_sample(i32::try_from(i).unwrap()).unwrap();
        }
        w.finalize().unwrap();

        let (riff_size, data_size) = read_size_fields(&path);
        let byte_width = 2; // 16-bit
        assert_eq!(data_size, n * byte_width, "data chunk size");
        assert_eq!(riff_size, n * byte_width + 36, "RIFF size = data + 36");
    }

    // DOLL-356/DOLL-204: update_header must SATURATE the u32 header fields when
    // data exceeds 4 GiB rather than wrapping. Poke the byte counter past
    // u32::MAX (no need to actually write 4 GiB) and assert both fields cap.
    #[test]
    fn test_header_caps_at_4gib_instead_of_wrapping() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("huge.wav").to_str().unwrap().to_owned();
        let spec = WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 24,
        };
        let mut w = RawWavWriter::create(&path, spec).unwrap();
        // Same module → can set the private counter directly.
        w.data_bytes_written = u64::from(u32::MAX) + 100;
        w.finalize().unwrap();

        let (riff_size, data_size) = read_size_fields(&path);
        assert_eq!(
            data_size,
            u32::MAX,
            "data size must cap at u32::MAX, not wrap"
        );
        assert_eq!(
            riff_size,
            u32::MAX,
            "RIFF size must saturate at u32::MAX, not wrap to a tiny value"
        );
    }

    // DOLL-372: an odd-length data chunk (24-bit mono, odd sample count) must
    // get a RIFF word-alignment pad byte so the file ends on an even boundary.
    // The data-chunk size stays unpadded; the parent RIFF size counts the pad.
    #[test]
    fn test_odd_length_data_chunk_gets_word_align_pad() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("odd.wav").to_str().unwrap().to_owned();
        let spec = WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 24, // 3-byte samples → 5 samples = 15 data bytes (odd)
        };
        let mut w = RawWavWriter::create(&path, spec).unwrap();
        for i in 0..5 {
            w.write_sample(i * 1000).unwrap();
        }
        w.finalize().unwrap();

        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(
            bytes.len() % 2,
            0,
            "file must end on an even (word) boundary"
        );
        // 24-bit is written as WAVE_FORMAT_EXTENSIBLE: a 68-byte header.
        assert_eq!(bytes.len(), 68 + 15 + 1, "68 header + 15 data + 1 pad");

        let (riff_size, data_size) = read_size_fields(&path);
        assert_eq!(data_size, 15, "data-chunk size is unpadded");
        assert_eq!(riff_size, 15 + 61, "RIFF size counts the pad byte");
    }

    /// After a failed flush the header may only claim whole frames that are
    /// actually in the file.
    #[test]
    fn salvaged_length_is_whole_frames_on_disk() {
        // Partial frame on disk: 6-byte stereo 24-bit frames, 20 bytes landed.
        assert_eq!(salvaged_data_len(20, 60, 6), 18);
        // More on disk than counted (e.g. a stale tail): trust the count.
        assert_eq!(salvaged_data_len(100, 60, 6), 60);
        assert_eq!(salvaged_data_len(0, 60, 6), 0);
    }

    /// `salvage_header` rewrites the placeholder sizes from what is on disk.
    #[test]
    fn salvage_header_covers_audio_on_disk() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("salvage.wav").to_str().unwrap().to_owned();
        let spec = WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 16,
        };
        let mut w = RawWavWriter::create(&path, spec).unwrap();
        for i in 0..10 {
            w.write_sample(i).unwrap();
        }
        // Push the samples to the file without touching the header, as if
        // the disk filled up right before finalize.
        w.writer.flush().unwrap();
        // One more frame stays buffered: it never reaches the file.
        w.write_sample(1).unwrap();
        w.write_sample(2).unwrap();
        assert_eq!(read_size_fields(&path), (0, 0), "placeholder header");

        w.salvage_header().unwrap();

        let (riff_size, data_size) = read_size_fields(&path);
        assert_eq!(data_size, 20, "the 5 frames on disk, not the buffered one");
        assert_eq!(riff_size, 20 + 36);
        let reader = hound::WavReader::open(&path).expect("salvaged file must be valid");
        assert_eq!(reader.len(), 10);
    }

    // An even-length data chunk must NOT get a pad byte.
    #[test]
    fn test_even_length_data_chunk_has_no_pad() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("even.wav").to_str().unwrap().to_owned();
        let spec = WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16, // 2-byte samples → always even
        };
        let mut w = RawWavWriter::create(&path, spec).unwrap();
        for i in 0_i16..5 {
            w.write_sample(i32::from(i)).unwrap();
        }
        w.finalize().unwrap();

        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes.len(), 44 + 10, "no pad: 44 header + 10 data");
        let (riff_size, data_size) = read_size_fields(&path);
        assert_eq!(data_size, 10);
        assert_eq!(riff_size, 10 + 36, "RIFF size has no pad byte");
    }
}
