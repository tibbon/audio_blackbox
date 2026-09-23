//! Launch-time recovery of recordings a crash left under their temporary
//! `.recording.wav` names.
//!
//! The writer refreshes each open file's header every 10 s of audio, so after
//! a crash, force-quit or power cut the file holds up to 10 s more audio than
//! its header describes, and it keeps the temporary name the rename in
//! `close_files` would have replaced. [`recover_recordings`] fixes both: it
//! points the header at every whole frame in the file and renames it to the
//! final `.wav` name, never over an existing file.

use std::fs::{self, OpenOptions, TryLockError};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use log::{info, warn};

use crate::error::BlackboxError;
use crate::raw_wav_writer::{parse_wav_layout, recovered_sizes};
use crate::writer_thread::disambiguate_path;

/// Suffix of a file the writer still has open (or had open when it died).
const TMP_SUFFIX: &str = ".recording.wav";

/// Header bytes read to find the `data` chunk. Our headers are 44 or 68
/// bytes; the margin allows a foreign chunk or two before `data`.
const HEADER_PROBE_LEN: u64 = 4096;

/// Repair and rename every `*.recording.wav` file in `dir`, returning how
/// many became playable `.wav` files.
///
/// For each file: the RIFF and data sizes are rewritten from the file's
/// actual length (whole frames only, capped at what the `u32` size fields
/// can hold), a torn trailing frame is cut off, and the file is renamed to
/// the name the recorder would have given it (`<stem>.wav`, or `<stem>-1.wav`
/// and so on if that exists — an existing file is never replaced). A file
/// with no audio after its header is deleted. A file that isn't a WAV this
/// engine could have written is left alone. Per-file failures are logged and
/// skipped so one bad file doesn't strand the rest.
///
/// A file another recorder still has open is skipped: the writer holds an
/// advisory exclusive lock (`flock`) on every `.recording.wav` while it is
/// open, and a file this can't lock is left alone. A crashed process holds
/// no lock, so its files are recovered. On a file system without `flock`
/// the lock can't be checked, so there it is still up to the caller to run
/// this only while nothing records into `dir`. The CLI calls it at startup,
/// before recording starts.
///
/// # Errors
///
/// Returns [`BlackboxError::Io`] if `dir` can't be read.
pub fn recover_recordings(dir: impl AsRef<Path>) -> Result<usize, BlackboxError> {
    let dir = dir.as_ref();
    let mut recovered = 0;
    for entry in fs::read_dir(dir)? {
        let path = match entry {
            Ok(e) => e.path(),
            Err(e) => {
                warn!("Skipping an unreadable entry in {}: {e}", dir.display());
                continue;
            }
        };
        let Some(final_path) = final_path_for(&path) else {
            continue;
        };
        if !path.is_file() {
            continue;
        }
        match recover_file(&path, &final_path) {
            Ok(Some(to)) => {
                info!("Recovered {} as {}", path.display(), to.display());
                recovered += 1;
            }
            Ok(None) => {}
            Err(e) => warn!("Could not recover {}: {e}", path.display()),
        }
    }
    Ok(recovered)
}

/// `<stem>.wav` for a `<stem>.recording.wav` path; `None` for anything else.
fn final_path_for(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?.to_str()?;
    let stem = name.strip_suffix(TMP_SUFFIX).filter(|s| !s.is_empty())?;
    Some(path.with_file_name(format!("{stem}.wav")))
}

/// Repair one temp file and move it to a free name based on `final_path`.
/// Returns the name it landed under, or `None` if it was skipped or held no
/// audio.
fn recover_file(tmp: &Path, final_path: &Path) -> io::Result<Option<PathBuf>> {
    let mut file = OpenOptions::new().read(true).write(true).open(tmp)?;
    // Held until `file` is dropped, after the rename, so two recoverers
    // can't work on the same file either.
    match file.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => {
            info!(
                "{} is still being recorded by another process; leaving it",
                tmp.display()
            );
            return Ok(None);
        }
        Err(TryLockError::Error(e)) if e.kind() == io::ErrorKind::Unsupported => {
            warn!(
                "{} can't be locked on this file system; recovering it unchecked",
                tmp.display()
            );
        }
        Err(TryLockError::Error(e)) => return Err(e),
    }
    let file_len = file.metadata()?.len();
    let mut head = Vec::new();
    (&mut file).take(HEADER_PROBE_LEN).read_to_end(&mut head)?;
    let Some(layout) = parse_wav_layout(&head) else {
        warn!(
            "{} has no readable WAV header; leaving it as is",
            tmp.display()
        );
        return Ok(None);
    };

    let (data_bytes, riff_size, data_size) = recovered_sizes(file_len, layout);
    if data_bytes == 0 {
        info!("{} holds no audio; removing it", tmp.display());
        fs::remove_file(tmp)?;
        return Ok(None);
    }

    file.seek(SeekFrom::Start(4))?;
    file.write_all(&riff_size.to_le_bytes())?;
    file.seek(SeekFrom::Start(layout.data_size_offset))?;
    file.write_all(&data_size.to_le_bytes())?;

    let data_end = layout.data_offset + data_bytes;
    // Less than a frame past the audio: only a torn frame, safe to cut.
    if file_len - data_end < u64::from(layout.block_align) {
        // Drop the torn trailing frame, then add the RIFF pad byte (zeroed
        // by `set_len`) if the data chunk has odd length.
        file.set_len(data_end)?;
        if data_bytes % 2 == 1 {
            file.set_len(data_end + 1)?;
        }
    } else {
        // More audio than a WAV header can describe (only files from before
        // the 4 GiB rotation). Keep the bytes rather than destroy audio.
        warn!(
            "{} holds more than 4 GiB of audio; its header covers the first {data_bytes} bytes",
            tmp.display()
        );
    }
    file.sync_all()?;

    let moved = move_without_clobbering(tmp, final_path).map(Some);
    drop(file);
    moved
}

/// Move `from` to `to`, or to the first free `-N` variant of it. Uses a hard
/// link so an existing file is never replaced even if one appears between
/// the check and the move; falls back to a checked rename on file systems
/// without hard links.
fn move_without_clobbering(from: &Path, to: &Path) -> io::Result<PathBuf> {
    let to_str = to
        .to_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path is not UTF-8"))?;
    for _ in 0..8 {
        let candidate = PathBuf::from(disambiguate_path(to_str));
        match fs::hard_link(from, &candidate) {
            Ok(()) => {
                fs::remove_file(from)?;
                return Ok(candidate);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(_) => {
                if candidate.exists() {
                    continue;
                }
                fs::rename(from, &candidate)?;
                return Ok(candidate);
            }
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("no free name for {}", to.display()),
    ))
}
