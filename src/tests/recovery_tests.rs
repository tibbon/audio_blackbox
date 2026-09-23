//! `recover_recordings`: repairing and renaming `.recording.wav` files a
//! crash left behind.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use tempfile::tempdir;

use crate::raw_wav_writer::{RawWavWriter, WavLayout, WavSpec, parse_wav_layout, recovered_sizes};
use crate::recover_recordings;

/// Write `samples` through the real writer and drop it without `finalize`:
/// the `BufWriter` flushes the audio, but the header keeps its placeholder
/// sizes, exactly what a crash before the first header refresh leaves.
fn crashed_recording(path: &Path, spec: WavSpec, samples: impl IntoIterator<Item = i32>) {
    let mut w = RawWavWriter::create(path.to_str().unwrap(), spec).unwrap();
    for s in samples {
        w.write_sample(s).unwrap();
    }
    drop(w);
}

fn append(path: &Path, bytes: &[u8]) {
    OpenOptions::new()
        .append(true)
        .open(path)
        .unwrap()
        .write_all(bytes)
        .unwrap();
}

const STEREO_16: WavSpec = WavSpec {
    channels: 2,
    sample_rate: 48_000,
    bits_per_sample: 16,
};

/// A crashed take gets a header covering every whole frame, loses only the
/// torn trailing frame, and is renamed to its final `.wav` name.
#[test]
fn crashed_take_is_repaired_and_renamed() {
    let dir = tempdir().unwrap();
    let tmp = dir.path().join("2026-01-02-03-04-05.recording.wav");
    crashed_recording(&tmp, STEREO_16, (0..2_002).map(|i: i32| i.rem_euclid(1000)));
    // Half a frame (one 16-bit sample of a stereo frame) torn by the crash.
    append(&tmp, &[0x11, 0x22]);

    let count = recover_recordings(dir.path()).unwrap();

    assert_eq!(count, 1);
    assert!(!tmp.exists(), "the temp name must be gone");
    let final_path = dir.path().join("2026-01-02-03-04-05.wav");
    let reader = hound::WavReader::open(&final_path).expect("recovered file must be valid");
    assert_eq!(reader.spec().channels, 2);
    assert_eq!(reader.len(), 2_002, "every whole frame is kept");
    let samples: Vec<i32> = reader.into_samples::<i32>().map(Result::unwrap).collect();
    assert_eq!(samples[1_999], 999);
    let header_len = fs::read(&final_path).unwrap().len() - 2_002 * 2;
    assert!(
        header_len == 44 || header_len == 68,
        "the torn frame must be cut off, leaving header + audio"
    );
}

/// A finished file with the same name is never replaced: the recovered take
/// gets the next free `-N` name, like a rotation collision.
#[test]
fn recovery_does_not_clobber_an_existing_file() {
    let dir = tempdir().unwrap();
    let tmp = dir.path().join("take.recording.wav");
    crashed_recording(&tmp, STEREO_16, 0..20);
    let existing = dir.path().join("take.wav");
    fs::write(&existing, b"keep me").unwrap();

    assert_eq!(recover_recordings(dir.path()).unwrap(), 1);

    assert_eq!(fs::read(&existing).unwrap(), b"keep me");
    let moved = dir.path().join("take-1.wav");
    assert_eq!(hound::WavReader::open(&moved).unwrap().len(), 20);
}

/// An odd-length data chunk (24-bit mono, odd sample count) gets its RIFF pad
/// byte, and the RIFF size counts it.
#[test]
fn odd_length_recovery_adds_pad_byte() {
    let dir = tempdir().unwrap();
    let tmp = dir.path().join("odd.recording.wav");
    let spec = WavSpec {
        channels: 1,
        sample_rate: 48_000,
        bits_per_sample: 24,
    };
    crashed_recording(&tmp, spec, 0..5);

    assert_eq!(recover_recordings(dir.path()).unwrap(), 1);

    let bytes = fs::read(dir.path().join("odd.wav")).unwrap();
    let layout = parse_wav_layout(&bytes).unwrap();
    assert_eq!(bytes.len() % 2, 0, "file must end on a word boundary");
    let data_offset = usize::try_from(layout.data_offset).unwrap();
    assert_eq!(bytes.len(), data_offset + 15 + 1);
    let riff = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    assert_eq!(u64::from(riff), layout.data_offset - 8 + 16);
}

/// A temp file with no audio after its header is deleted, not turned into
/// an empty take; files that aren't temp recordings are not touched.
#[test]
fn empty_and_unrelated_files() {
    let dir = tempdir().unwrap();
    let empty = dir.path().join("empty.recording.wav");
    crashed_recording(&empty, STEREO_16, std::iter::empty());
    let garbage = dir.path().join("garbage.recording.wav");
    fs::write(&garbage, b"not a wav file at all").unwrap();
    let finished = dir.path().join("done.wav");
    fs::write(&finished, b"finished").unwrap();
    let other = dir.path().join("notes.txt");
    fs::write(&other, b"notes").unwrap();

    assert_eq!(recover_recordings(dir.path()).unwrap(), 0);

    assert!(!empty.exists(), "a header-only temp file is removed");
    assert!(!dir.path().join("empty.wav").exists());
    assert_eq!(
        fs::read(&garbage).unwrap(),
        b"not a wav file at all",
        "an unreadable file is left as is"
    );
    assert_eq!(fs::read(&finished).unwrap(), b"finished");
    assert_eq!(fs::read(&other).unwrap(), b"notes");
}

/// A missing directory is an error, not "nothing recovered".
#[test]
fn missing_directory_is_an_error() {
    let dir = tempdir().unwrap();
    assert!(recover_recordings(dir.path().join("nope")).is_err());
}

/// The layout parser walks chunks instead of assuming 44 bytes: a foreign
/// chunk before `data` moves the data offset.
#[test]
fn layout_parser_walks_chunks() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF\0\0\0\0WAVE");
    bytes.extend_from_slice(b"fmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&2_u16.to_le_bytes()); // channels
    bytes.extend_from_slice(&48_000_u32.to_le_bytes());
    bytes.extend_from_slice(&192_000_u32.to_le_bytes());
    bytes.extend_from_slice(&4_u16.to_le_bytes()); // block_align
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"LIST");
    bytes.extend_from_slice(&3_u32.to_le_bytes());
    bytes.extend_from_slice(b"abc\0"); // 3 bytes + pad
    bytes.extend_from_slice(b"data\0\0\0\0");

    let layout = parse_wav_layout(&bytes).unwrap();
    assert_eq!(
        layout,
        WavLayout {
            block_align: 4,
            data_size_offset: 52,
            data_offset: 56,
        }
    );
    assert_eq!(parse_wav_layout(b"RIFX\0\0\0\0WAVE"), None);
    assert_eq!(parse_wav_layout(&bytes[..40]), None, "truncated header");
}

/// A file longer than the `u32` size fields can describe gets a header
/// capped to whole frames that still fit, never a wrapped value.
#[test]
fn recovered_sizes_respect_the_4gib_limit() {
    let layout = WavLayout {
        block_align: 6,
        data_size_offset: 64,
        data_offset: 68,
    };
    let (data_bytes, riff, data) = recovered_sizes(5 << 30, layout);
    assert_eq!(data_bytes % 6, 0, "whole frames only");
    assert_eq!(u64::from(data), data_bytes);
    assert_eq!(u64::from(riff), 60 + data_bytes + data_bytes % 2);
    assert!(u64::from(u32::MAX) - data_bytes < 60 + 1 + 6);

    // Ordinary case: whole frames of what follows the header.
    assert_eq!(recovered_sizes(68 + 6 * 10 + 4, layout), (60, 120, 60));
}

/// A `.recording.wav` another recorder still has open is left alone: the
/// writer holds an exclusive lock on it, and recovery skips files it can't
/// lock. Once the lock is released (as when a recorder crashes), the file
/// is recovered. Recovery used to finalize and rename a live file when two
/// recorders shared a folder.
#[test]
fn a_locked_temp_file_is_skipped() {
    let dir = tempdir().unwrap();
    let tmp = dir.path().join("live.recording.wav");
    crashed_recording(&tmp, STEREO_16, 0..20);
    let holder = fs::File::open(&tmp).unwrap();
    holder.lock().unwrap();

    assert_eq!(recover_recordings(dir.path()).unwrap(), 0);
    assert!(tmp.exists(), "a locked file keeps its temp name");
    assert!(!dir.path().join("live.wav").exists());

    drop(holder);
    assert_eq!(recover_recordings(dir.path()).unwrap(), 1);
    assert!(dir.path().join("live.wav").exists());
}

/// The writer holds that lock for as long as the file is open.
#[test]
fn the_writer_locks_its_file_while_open() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("open.recording.wav");
    let writer = RawWavWriter::create(path.to_str().unwrap(), STEREO_16).unwrap();
    let other = fs::File::open(&path).unwrap();
    assert!(
        matches!(other.try_lock(), Err(fs::TryLockError::WouldBlock)),
        "an open recording must be locked"
    );
    writer.finalize().unwrap();
    other
        .try_lock()
        .expect("closing the writer releases the lock");
}
