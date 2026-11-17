//! macOS scans read folders with `getattrlistbulk(2)`: one call returns the
//! name, type, flags, file ID, link count and sizes of many entries, so no
//! file is opened or `stat`ed. Several threads list folders in parallel; the
//! scan thread numbers the entries and keeps hard-link bookkeeping, as the
//! Windows listing walker does. Approach adapted from FSearch
//! (https://github.com/noahdunnagan/fsearch, MIT).
//!
//! Mount points are not crossed. Firmlinks are, which is how `/Users`,
//! `/Applications` and the rest of the data volume appear below `/` once.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

/// What one listed entry is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    File,
    Directory,
    Symlink,
    Other,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Entry {
    pub name: Vec<u8>,
    pub kind: Kind,
    pub flags: u32,
    pub file_id: u64,
    pub links: u32,
    pub logical: u64,
    pub allocated: u64,
    /// A directory that another volume is mounted on.
    pub mount: bool,
    /// Per-entry errno reported by the filesystem.
    pub error: u32,
}

pub(super) const ATTR_BIT_MAP_COUNT: u16 = 5;
pub(super) const ATTR_CMN_NAME: u32 = 0x0000_0001;
pub(super) const ATTR_CMN_OBJTYPE: u32 = 0x0000_0008;
pub(super) const ATTR_CMN_FLAGS: u32 = 0x0004_0000;
pub(super) const ATTR_CMN_FILEID: u32 = 0x0200_0000;
pub(super) const ATTR_CMN_ERROR: u32 = 0x2000_0000;
pub(super) const ATTR_CMN_RETURNED_ATTRS: u32 = 0x8000_0000;
pub(super) const ATTR_DIR_MOUNTSTATUS: u32 = 0x0000_0008;
pub(super) const ATTR_FILE_LINKCOUNT: u32 = 0x0000_0001;
pub(super) const ATTR_FILE_TOTALSIZE: u32 = 0x0000_0002;
pub(super) const ATTR_FILE_ALLOCSIZE: u32 = 0x0000_0004;
const DIR_MNTSTATUS_MNTPOINT: u32 = 0x1;
const DIR_MNTSTATUS_TRIGGER: u32 = 0x2;
/// File-provider items whose contents are not on this Mac, such as evicted iCloud files.
pub(super) const SF_DATALESS: u32 = 0x4000_0000;

/// Parses one buffer of `count` records. Every offset is checked: the kernel
/// is trusted, but a filesystem driver may still return malformed records.
pub(super) fn parse(buffer: &[u8], count: usize, out: &mut Vec<Entry>) -> Result<(), &'static str> {
    let u32_at = |b: &[u8], at: usize| -> Result<u32, &'static str> {
        b.get(at..at + 4)
            .map(|v| u32::from_ne_bytes(v.try_into().unwrap()))
            .ok_or("Truncated directory entry")
    };
    let u64_at = |b: &[u8], at: usize| -> Result<u64, &'static str> {
        b.get(at..at + 8)
            .map(|v| u64::from_ne_bytes(v.try_into().unwrap()))
            .ok_or("Truncated directory entry")
    };
    let mut offset = 0usize;
    for _ in 0..count {
        let length = u32_at(buffer, offset)? as usize;
        if length < 24 {
            return Err("Invalid directory entry length");
        }
        let record = buffer
            .get(offset..offset + length)
            .ok_or("Truncated directory entry")?;
        offset += length;
        // attribute_set_t follows the length: common, volume, directory, file, fork.
        let common = u32_at(record, 4)?;
        let directory = u32_at(record, 12)?;
        let file = u32_at(record, 16)?;
        let mut at = 24;
        let mut entry = Entry {
            name: Vec::new(),
            kind: Kind::Other,
            flags: 0,
            file_id: 0,
            links: 1,
            logical: 0,
            allocated: 0,
            mount: false,
            error: 0,
        };
        // ATTR_CMN_ERROR is placed directly after the returned attribute set.
        if common & ATTR_CMN_ERROR != 0 {
            entry.error = u32_at(record, at)?;
            at += 4;
        }
        if common & ATTR_CMN_NAME == 0 {
            continue;
        }
        let name_offset = u32_at(record, at)? as i32;
        let name_length = u32_at(record, at + 4)? as usize;
        let start = (at as i64 + name_offset as i64)
            .try_into()
            .map_err(|_| "Invalid directory entry name")?;
        let name = record
            .get(start..start + name_length)
            .ok_or("Invalid directory entry name")?;
        // The length includes the terminating NUL.
        let name = name.strip_suffix(&[0]).unwrap_or(name);
        if name.is_empty() || name.contains(&b'/') || name.contains(&0) {
            return Err("Invalid directory entry name");
        }
        entry.name = name.to_vec();
        at += 8;
        if common & ATTR_CMN_OBJTYPE != 0 {
            entry.kind = match u32_at(record, at)? {
                1 => Kind::File,      // VREG
                2 => Kind::Directory, // VDIR
                5 => Kind::Symlink,   // VLNK
                _ => Kind::Other,
            };
            at += 4;
        }
        if common & ATTR_CMN_FLAGS != 0 {
            entry.flags = u32_at(record, at)?;
            at += 4;
        }
        if common & ATTR_CMN_FILEID != 0 {
            entry.file_id = u64_at(record, at)?;
            at += 8;
        }
        if directory & ATTR_DIR_MOUNTSTATUS != 0 {
            entry.mount =
                u32_at(record, at)? & (DIR_MNTSTATUS_MNTPOINT | DIR_MNTSTATUS_TRIGGER) != 0;
            at += 4;
        }
        if file & ATTR_FILE_LINKCOUNT != 0 {
            entry.links = u32_at(record, at)?;
            at += 4;
        }
        if file & ATTR_FILE_TOTALSIZE != 0 {
            entry.logical = u64_at(record, at)?;
            at += 8;
        }
        if file & ATTR_FILE_ALLOCSIZE != 0 {
            entry.allocated = u64_at(record, at)?;
        }
        out.push(entry);
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub(super) use native::walk;

#[cfg(target_os = "macos")]
mod native {
    use super::super::{FileIdentity, ScanNode, Skip, Worker, append_note};
    use super::*;
    use crossbeam_channel::{RecvTimeoutError, bounded, unbounded};
    use std::{
        ffi::{CString, OsStr, c_char, c_int, c_void},
        io,
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::{ffi::OsStrExt, fs::MetadataExt},
        },
        path::{Path, PathBuf},
        sync::atomic::{AtomicBool, Ordering},
        time::Duration,
    };

    /// More threads stop helping once the kernel side or the scan thread saturates.
    const MAX_THREADS: usize = 8;
    const BUFFER_BYTES: usize = 256 * 1024;
    const O_RDONLY: c_int = 0x0000;
    const O_NOFOLLOW: c_int = 0x0100;
    const O_DIRECTORY: c_int = 0x0010_0000;
    const O_CLOEXEC: c_int = 0x0100_0000;

    #[repr(C)]
    struct AttrList {
        bitmapcount: u16,
        reserved: u16,
        commonattr: u32,
        volattr: u32,
        dirattr: u32,
        fileattr: u32,
        forkattr: u32,
    }

    unsafe extern "C" {
        fn open(path: *const c_char, flags: c_int, ...) -> c_int;
        fn getattrlistbulk(
            fd: c_int,
            attrs: *mut c_void,
            buffer: *mut c_void,
            size: usize,
            options: u64,
        ) -> c_int;
    }

    struct Listing {
        entries: Vec<Entry>,
        /// The listed folder's volume; firmlinks put folders below `/` on the data volume.
        volume: u64,
        /// Entries listed before a failure are kept.
        failure: Option<io::Error>,
    }

    fn list(path: &Path, halted: &dyn Fn() -> bool) -> Listing {
        let mut listing = Listing {
            entries: Vec::new(),
            volume: 0,
            failure: None,
        };
        let Ok(name) = CString::new(path.as_os_str().as_bytes()) else {
            listing.failure = Some(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Path contains a NUL byte",
            ));
            return listing;
        };
        // O_NOFOLLOW: a folder swapped for a symbolic link after it was queued is not followed.
        // SAFETY: `name` is NUL-terminated; the flags take no mode argument.
        let fd = unsafe {
            open(
                name.as_ptr(),
                O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC,
            )
        };
        if fd < 0 {
            listing.failure = Some(io::Error::last_os_error());
            return listing;
        }
        // SAFETY: `fd` is a freshly opened descriptor that the File now owns and closes.
        let folder = unsafe { std::fs::File::from_raw_fd(fd) };
        listing.volume = match folder.metadata() {
            Ok(metadata) => metadata.dev(),
            Err(error) => {
                listing.failure = Some(error);
                return listing;
            }
        };
        let mut attributes = AttrList {
            bitmapcount: ATTR_BIT_MAP_COUNT,
            reserved: 0,
            commonattr: ATTR_CMN_RETURNED_ATTRS
                | ATTR_CMN_NAME
                | ATTR_CMN_ERROR
                | ATTR_CMN_OBJTYPE
                | ATTR_CMN_FLAGS
                | ATTR_CMN_FILEID,
            volattr: 0,
            dirattr: ATTR_DIR_MOUNTSTATUS,
            fileattr: ATTR_FILE_LINKCOUNT | ATTR_FILE_TOTALSIZE | ATTR_FILE_ALLOCSIZE,
            forkattr: 0,
        };
        let mut buffer = vec![0u64; BUFFER_BYTES / 8]; // Records are 4-byte aligned.
        while !halted() {
            // SAFETY: `attributes` and `buffer` outlive the call; the size matches.
            let count = unsafe {
                getattrlistbulk(
                    folder.as_raw_fd(),
                    (&raw mut attributes).cast(),
                    buffer.as_mut_ptr().cast(),
                    BUFFER_BYTES,
                    0,
                )
            };
            if count < 0 {
                listing.failure = Some(io::Error::last_os_error());
                break;
            }
            if count == 0 {
                break;
            }
            // SAFETY: the buffer is BUFFER_BYTES long and fully initialized.
            let bytes =
                unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), BUFFER_BYTES) };
            if let Err(message) = parse(bytes, count as usize, &mut listing.entries) {
                listing.failure = Some(io::Error::new(io::ErrorKind::InvalidData, message));
                break;
            }
        }
        listing
    }

    /// Lists `root` (node `root_id`) and everything below it into `worker`.
    pub(in super::super) fn walk(
        worker: &mut Worker,
        root_id: usize,
        root: PathBuf,
        refused: bool,
    ) {
        let threads = std::thread::available_parallelism()
            .map_or(4, |n| n.get())
            .clamp(2, MAX_THREADS);
        let (jobs, queued) = unbounded::<(usize, PathBuf)>();
        let (done, results) = bounded::<(usize, PathBuf, Listing)>(threads * 4);
        let stop = AtomicBool::new(false);
        let cancel = worker.cancel.clone();
        let mut mounts = 0u64;
        std::thread::scope(|scope| {
            for _ in 0..threads {
                let (queued, done) = (queued.clone(), done.clone());
                let (stop, cancel) = (&stop, &cancel);
                scope.spawn(move || {
                    let halted = || stop.load(Ordering::Relaxed) || cancel.load(Ordering::Relaxed);
                    for (id, path) in queued {
                        if halted() {
                            continue; // Drain quickly; the queue closes when the walk ends.
                        }
                        let listing = list(&path, &halted);
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
                let (parent, directory, listing) =
                    match results.recv_timeout(Duration::from_millis(50)) {
                        Ok(result) => result,
                        Err(RecvTimeoutError::Timeout) => {
                            worker.flush(false);
                            continue;
                        }
                        Err(RecvTimeoutError::Disconnected) => break,
                    };
                pending -= 1;
                for entry in listing.entries {
                    let Some(child) = add(
                        worker,
                        parent,
                        &directory,
                        entry,
                        listing.volume,
                        &mut mounts,
                    ) else {
                        break;
                    };
                    if let Some((id, path)) = child {
                        let _ = jobs.send((id, path));
                        pending += 1;
                    }
                    if !worker.flush(false) {
                        break;
                    }
                }
                if let Some(error) = listing.failure {
                    if error.raw_os_error() == Some(62) {
                        // ELOOP: the queued folder became a symbolic link.
                        worker.record_node_error(
                            parent,
                            &directory,
                            "Directory changed into a reparse point or cloud placeholder; not traversed",
                        );
                    } else {
                        // A root macOS already refused to open counts once.
                        let count = !(refused && parent == root_id);
                        worker.record_io_error(parent, &directory, &error, count);
                    }
                }
                if !worker.flush(false) {
                    break;
                }
            }
            stop.store(true, Ordering::Relaxed);
            drop((jobs, results));
        });
        if mounts > 0 {
            worker.summary.notes.push(format!("{mounts} mount points of other volumes were not entered, such as /System/Volumes/Data, /Volumes and /dev; scan those directly."));
        }
    }

    /// Adds one listed entry. Returns `None` when the scan must stop, and the
    /// folder to list next when the entry is one.
    fn add(
        worker: &mut Worker,
        parent: usize,
        directory: &Path,
        entry: Entry,
        volume: u64,
        mounts: &mut u64,
    ) -> Option<Option<(usize, PathBuf)>> {
        if worker.cancelled() {
            return None;
        }
        let path = directory.join(OsStr::from_bytes(&entry.name));
        let is_dir = entry.kind == Kind::Directory;
        let mut node = ScanNode {
            id: worker.next_id,
            parent: Some(parent),
            name: String::from_utf8_lossy(&entry.name).into_owned(),
            is_dir,
            logical: 0,
            allocated: 0,
            files: 0,
            children: Vec::new(),
            note: String::new(),
            incomplete: false,
        };
        let mut traverse = false;
        if entry.error != 0 {
            let error = io::Error::from_raw_os_error(entry.error as i32);
            node.incomplete = true;
            append_note(&mut node.note, &error.to_string());
            let id = worker.push(node, 0, &path)?;
            worker.record_io_error(id, &path, &error, true);
            return Some(None);
        }
        if entry.kind == Kind::Symlink {
            worker.mark_skipped(&mut node, Skip::Reparse);
        } else if entry.flags & SF_DATALESS != 0 {
            worker.mark_skipped(&mut node, Skip::Cloud);
        } else if is_dir && entry.mount {
            *mounts += 1;
            node.incomplete = true;
            node.note = "Mount point of another volume; not entered".into();
        } else if is_dir {
            traverse = true;
        } else {
            node.files = 1;
            node.logical = entry.logical;
            node.allocated = entry.allocated;
            if entry.links > 1 && entry.file_id != 0 {
                let mut identity = [0; 16];
                identity[..8].copy_from_slice(&entry.file_id.to_le_bytes());
                if !worker.seen.insert(FileIdentity(volume, identity)) {
                    worker.summary.hard_links += 1;
                    node.allocated = 0;
                    append_note(&mut node.note, crate::storage::HARD_LINK_ALIAS);
                }
            }
        }
        let queued = if traverse { path.as_os_str().len() } else { 0 };
        let id = worker.push(node, queued, &path)?;
        Some(traverse.then_some((id, path)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds one record the way the kernel lays it out.
    fn record(name: &str, kind: u32, file: Option<(u32, u64, u64)>, mount: Option<u32>) -> Vec<u8> {
        let common = ATTR_CMN_NAME | ATTR_CMN_OBJTYPE | ATTR_CMN_FLAGS | ATTR_CMN_FILEID;
        let directory = if mount.is_some() {
            ATTR_DIR_MOUNTSTATUS
        } else {
            0
        };
        let file_bits = if file.is_some() {
            ATTR_FILE_LINKCOUNT | ATTR_FILE_TOTALSIZE | ATTR_FILE_ALLOCSIZE
        } else {
            0
        };
        let mut body = Vec::new();
        for value in [common, 0, directory, file_bits, 0] {
            body.extend_from_slice(&value.to_ne_bytes());
        }
        let mut fixed = Vec::new();
        fixed.extend_from_slice(&kind.to_ne_bytes());
        fixed.extend_from_slice(&0u32.to_ne_bytes()); // flags
        fixed.extend_from_slice(&42u64.to_ne_bytes()); // file ID
        if let Some(status) = mount {
            fixed.extend_from_slice(&status.to_ne_bytes());
        }
        if let Some((links, total, allocated)) = file {
            fixed.extend_from_slice(&links.to_ne_bytes());
            fixed.extend_from_slice(&total.to_ne_bytes());
            fixed.extend_from_slice(&allocated.to_ne_bytes());
        }
        let name_offset = (8 + fixed.len()) as u32;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0u32.to_ne_bytes()); // length, patched below
        bytes.extend_from_slice(&body);
        bytes.extend_from_slice(&name_offset.to_ne_bytes());
        bytes.extend_from_slice(&(name.len() as u32 + 1).to_ne_bytes());
        bytes.extend_from_slice(&fixed);
        bytes.extend_from_slice(name.as_bytes());
        bytes.push(0);
        while bytes.len() % 4 != 0 {
            bytes.push(0);
        }
        let length = bytes.len() as u32;
        bytes[..4].copy_from_slice(&length.to_ne_bytes());
        bytes
    }

    #[test]
    fn bulk_records_yield_names_sizes_links_and_mount_points() {
        let mut buffer = record("Fotos 資料.heic", 1, Some((2, 1500, 4096)), None);
        buffer.extend(record("Data", 2, None, Some(1)));
        buffer.extend(record("Users", 2, None, Some(0)));
        let mut entries = Vec::new();
        parse(&buffer, 3, &mut entries).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].name, "Fotos 資料.heic".as_bytes());
        assert_eq!(entries[0].kind, Kind::File);
        assert_eq!(
            (entries[0].links, entries[0].logical, entries[0].allocated),
            (2, 1500, 4096)
        );
        assert_eq!(entries[0].file_id, 42);
        assert!(entries[1].mount && entries[1].kind == Kind::Directory);
        assert!(!entries[2].mount);
    }

    #[test]
    fn malformed_records_are_rejected_instead_of_read_out_of_bounds() {
        let good = record("a", 1, Some((1, 1, 1)), None);
        let mut entries = Vec::new();
        // Claims two records but holds one.
        assert!(parse(&good, 2, &mut entries).is_err());
        // A length shorter than the header.
        let mut short = good.clone();
        short[..4].copy_from_slice(&8u32.to_ne_bytes());
        assert!(parse(&short, 1, &mut Vec::new()).is_err());
        // A name that would leave the folder.
        let escape = record("a/b", 1, Some((1, 1, 1)), None);
        assert!(parse(&escape, 1, &mut Vec::new()).is_err());
        // A name offset pointing past the record.
        let mut far = good;
        far[24..28].copy_from_slice(&4096u32.to_ne_bytes());
        assert!(parse(&far, 1, &mut Vec::new()).is_err());
    }
}
