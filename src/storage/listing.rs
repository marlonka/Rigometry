//! Standard scans read folders, not files. One directory query returns the
//! name, sizes, attributes and file ID of many entries at once, so no file is
//! opened and files Windows keeps locked still report their size. Several
//! threads list folders in parallel; the scan thread numbers the entries and
//! keeps the hard-link bookkeeping, so node IDs stay append-only.
//!
//! The listing reports each file's main data stream. Named streams have no
//! batch query and are left out; the MFT reader includes them.

use super::{
    FileIdentity, ScanNode, Worker, append_note,
    native::{OwnedHandle, io_error, query, skip_attributes, wide, win32_code},
};
use crossbeam_channel::{RecvTimeoutError, bounded, unbounded};
use std::{
    ffi::OsString,
    io,
    os::windows::ffi::OsStringExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::Duration,
};
use windows::{
    Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_COMPRESSED, FILE_ATTRIBUTE_DIRECTORY,
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_SPARSE_FILE, FILE_ATTRIBUTE_TAG_INFO,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_NO_RECALL, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_ID_INFO, FILE_INFO_BY_HANDLE_CLASS, FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FileAttributeTagInfo,
        FileFullDirectoryInfo, FileIdBothDirectoryInfo, FileIdExtdDirectoryInfo, FileIdInfo,
        GetFileInformationByHandleEx, OPEN_EXISTING,
    },
    core::PCWSTR,
};

/// More threads stop helping once the drive or the scan thread is saturated.
const MAX_THREADS: usize = 8;
const BUFFER_BYTES: usize = 64 * 1024;
/// Compact OS files carry this tag; Windows reads them as ordinary files.
const IO_REPARSE_TAG_WOF: u32 = 0x8000_0017;

/// Where one directory-information class keeps its fields. Each record
/// starts with the same 64 bytes; the file ID and name follow at different
/// offsets. Without a reparse-tag field, `EaSize` carries the tag.
#[derive(Clone, Copy)]
pub(super) struct Layout {
    pub class: FILE_INFO_BY_HANDLE_CLASS,
    pub name: usize,
    pub id: Option<(usize, usize)>,
    pub tag: usize,
}

/// Richest first. FAT and some network filesystems only offer the later ones;
/// the full layout has no file ID, so hard links there are not recognized.
pub(super) const LAYOUTS: [Layout; 3] = [
    Layout {
        class: FileIdExtdDirectoryInfo,
        name: 88,
        id: Some((72, 16)),
        tag: 68,
    },
    Layout {
        class: FileIdBothDirectoryInfo,
        name: 104,
        id: Some((96, 8)),
        tag: 64,
    },
    Layout {
        class: FileFullDirectoryInfo,
        name: 68,
        id: None,
        tag: 64,
    },
];

#[derive(Debug)]
pub(super) struct Entry {
    pub name: Vec<u16>,
    pub attributes: u32,
    pub logical: u64,
    pub allocated: u64,
    pub id: [u8; 16],
    pub tag: u32,
}

enum Failure {
    Io(io::Error),
    /// The queued folder became a junction or placeholder before it was opened.
    Changed,
}

struct Listing {
    entries: Vec<Entry>,
    volume: u64,
    /// Entries listed before a failure are kept.
    failure: Option<Failure>,
}

/// Lists `root` (node `root_id`) and everything below it into `worker`.
/// `refused` says Windows already refused the root and that was counted.
pub(super) fn walk(worker: &mut Worker, root_id: usize, root: PathBuf, refused: bool) {
    worker.summary.notes.push("Sizes come from folder listings, so no file is opened. Named data streams, such as download markers, are not included; an administrator scan of a whole drive includes them.".into());
    let threads = std::thread::available_parallelism()
        .map_or(4, |n| n.get())
        .clamp(2, MAX_THREADS);
    let (jobs, queued) = unbounded::<(usize, PathBuf)>();
    let (done, results) = bounded::<(usize, PathBuf, Listing)>(threads * 4);
    let layout = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let cancel = worker.cancel.clone();
    std::thread::scope(|scope| {
        for _ in 0..threads {
            let (queued, done) = (queued.clone(), done.clone());
            let (layout, stop, cancel) = (&layout, &stop, &cancel);
            scope.spawn(move || {
                let halted = || stop.load(Ordering::Relaxed) || cancel.load(Ordering::Relaxed);
                for (id, path) in queued {
                    if halted() {
                        continue; // Drain quickly; the queue closes when the walk ends.
                    }
                    let listing = list(&path, layout, &halted);
                    if done.send((id, path, listing)).is_err() {
                        break;
                    }
                }
            });
        }
        drop((queued, done));
        let _ = jobs.send((root_id, root));
        let mut pending = 1usize;
        while pending > 0 && !worker.cancelled() {
            let (parent, directory, listing) = match results.recv_timeout(Duration::from_millis(50))
            {
                Ok(result) => result,
                Err(RecvTimeoutError::Timeout) => {
                    worker.flush(false);
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => break,
            };
            pending -= 1;
            for entry in listing.entries {
                let Some(child) = add(worker, parent, &directory, entry, listing.volume) else {
                    break;
                };
                if let Some((id, path)) = child {
                    let _ = jobs.send((id, path));
                    pending += 1;
                }
                if !worker.flush(false) {
                    break; // Batches stay small however large the folder.
                }
            }
            match listing.failure {
                Some(Failure::Io(error)) => {
                    // A root Windows already refused to open counts once.
                    let count = !(refused && parent == root_id);
                    worker.record_io_error(parent, &directory, &error, count);
                }
                Some(Failure::Changed) => worker.record_node_error(
                    parent,
                    &directory,
                    "Directory changed into a reparse point or cloud placeholder; not traversed",
                ),
                None => {}
            }
            if !worker.flush(false) {
                break;
            }
        }
        stop.store(true, Ordering::Relaxed);
        drop((jobs, results));
    });
}

/// Adds one listed entry. Returns `None` when the scan must stop, and the
/// folder to list next when the entry is one.
fn add(
    worker: &mut Worker,
    parent: usize,
    directory: &Path,
    entry: Entry,
    volume: u64,
) -> Option<Option<(usize, PathBuf)>> {
    if worker.cancelled() {
        return None;
    }
    let is_dir = entry.attributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0;
    let mut node = ScanNode {
        id: worker.next_id,
        parent: Some(parent),
        name: String::from_utf16_lossy(&entry.name),
        is_dir,
        logical: 0,
        allocated: 0,
        files: 0,
        children: Vec::new(),
        note: String::new(),
        incomplete: false,
    };
    let compact = entry.attributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0
        && entry.tag == IO_REPARSE_TAG_WOF
        && !is_dir;
    let attributes = if compact {
        entry.attributes & !FILE_ATTRIBUTE_REPARSE_POINT.0
    } else {
        entry.attributes
    };
    let mut traverse = false;
    if let Some(skip) = skip_attributes(attributes) {
        worker.mark_skipped(&mut node, skip);
    } else if is_dir {
        traverse = true;
    } else {
        node.files = 1;
        node.logical = entry.logical;
        node.allocated = entry.allocated;
        // Zero means the filesystem has no stable ID; never merge those.
        if entry.id != [0; 16] && !worker.seen.insert(FileIdentity(volume, entry.id)) {
            worker.summary.hard_links += 1;
            node.allocated = 0;
            append_note(
                &mut node.note,
                "Hard-link alias: allocated bytes attributed to another path in this scan",
            );
        }
        if attributes & FILE_ATTRIBUTE_SPARSE_FILE.0 != 0 {
            append_note(&mut node.note, "Sparse");
        }
        if attributes & FILE_ATTRIBUTE_COMPRESSED.0 != 0 {
            append_note(&mut node.note, "NTFS compressed");
        }
    }
    let path = directory.join(OsString::from_wide(&entry.name));
    let queued = if traverse { path.as_os_str().len() } else { 0 };
    let id = worker.push(node, queued, &path)?;
    Some(traverse.then_some((id, path)))
}

fn list(path: &Path, layout: &AtomicUsize, halted: &dyn Fn() -> bool) -> Listing {
    let mut listing = Listing {
        entries: Vec::new(),
        volume: 0,
        failure: None,
    };
    let handle = match open(path) {
        Ok(handle) => handle,
        Err(error) => {
            listing.failure = Some(Failure::Io(io_error(error)));
            return listing;
        }
    };
    // Checked on the handle itself, so a folder swapped for a junction after
    // it was queued is never followed.
    match query::<FILE_ATTRIBUTE_TAG_INFO>(handle.0, FileAttributeTagInfo) {
        Ok(info) if skip_attributes(info.FileAttributes).is_some() => {
            listing.failure = Some(Failure::Changed);
            return listing;
        }
        Ok(_) => {}
        Err(error) => {
            listing.failure = Some(Failure::Io(io_error(error)));
            return listing;
        }
    }
    listing.volume =
        query::<FILE_ID_INFO>(handle.0, FileIdInfo).map_or(0, |id| id.VolumeSerialNumber);
    let mut buffer = vec![0u64; BUFFER_BYTES / 8]; // Records are 8-byte aligned.
    let mut started = false;
    while !halted() {
        let index = layout.load(Ordering::Relaxed);
        let current = LAYOUTS[index];
        let result = unsafe {
            GetFileInformationByHandleEx(
                handle.0,
                current.class,
                buffer.as_mut_ptr().cast(),
                BUFFER_BYTES as u32,
            )
        };
        if let Err(error) = result {
            match win32_code(&error) {
                Some(18) => {} // ERROR_NO_MORE_FILES
                // Unsupported class: fall back for this and every later folder.
                Some(1 | 50 | 87 | 124) if !started && index + 1 < LAYOUTS.len() => {
                    let _ = layout.compare_exchange(
                        index,
                        index + 1,
                        Ordering::Relaxed,
                        Ordering::Relaxed,
                    );
                    continue;
                }
                _ => listing.failure = Some(Failure::Io(io_error(error))),
            }
            break;
        }
        started = true;
        let bytes =
            unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), BUFFER_BYTES) };
        if let Err(message) = parse(bytes, current, &mut listing.entries) {
            listing.failure = Some(Failure::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                message,
            )));
            break;
        }
    }
    listing
}

fn open(path: &Path) -> windows::core::Result<OwnedHandle> {
    let name = wide(path);
    unsafe {
        CreateFileW(
            PCWSTR(name.as_ptr()),
            FILE_LIST_DIRECTORY.0 | FILE_READ_ATTRIBUTES.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_OPEN_NO_RECALL,
            None,
        )
        .map(OwnedHandle)
    }
}

/// Parses untrusted driver output, as `parse_streams` does: every offset is
/// checked, and names that could leave the folder are rejected.
pub(super) fn parse(
    bytes: &[u8],
    layout: Layout,
    out: &mut Vec<Entry>,
) -> Result<(), &'static str> {
    let u32_at =
        |record: &[u8], at: usize| u32::from_le_bytes(record[at..at + 4].try_into().unwrap());
    let i64_at =
        |record: &[u8], at: usize| i64::from_le_bytes(record[at..at + 8].try_into().unwrap());
    let mut offset = 0usize;
    loop {
        let record = bytes.get(offset..).unwrap_or_default();
        if record.len() < layout.name {
            return Err("Truncated directory entry");
        }
        let next = u32_at(record, 0) as usize;
        let name_length = u32_at(record, 60) as usize;
        let end = layout
            .name
            .checked_add(name_length)
            .ok_or("Invalid directory entry name")?;
        if name_length == 0 || !name_length.is_multiple_of(2) || end > record.len() {
            return Err("Invalid directory entry name");
        }
        if next != 0 && (next < end || !next.is_multiple_of(8) || next >= record.len()) {
            return Err("Invalid directory entry offset");
        }
        let name: Vec<u16> = record[layout.name..end]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        let dots = name == [46] || name == [46, 46];
        if !dots {
            if name.iter().any(|c| matches!(*c, 0 | 47 | 58 | 92)) {
                return Err("Invalid directory entry name");
            }
            let (logical, allocated) = (i64_at(record, 40), i64_at(record, 48));
            if logical < 0 || allocated < 0 {
                return Err("Invalid directory entry size");
            }
            let mut id = [0; 16];
            if let Some((at, length)) = layout.id {
                id[..length].copy_from_slice(&record[at..at + length]);
            }
            out.push(Entry {
                name,
                attributes: u32_at(record, 56),
                logical: logical as u64,
                allocated: allocated as u64,
                id,
                tag: u32_at(record, layout.tag),
            });
        }
        if next == 0 {
            return Ok(());
        }
        offset += next;
    }
}
