//! Read-only storage scans. Node sizes in `Batch` are owned bytes, never subtree
//! totals. Consumers add each node's values to its ancestors exactly once.
//!
//! Logical bytes count every directory entry (including hard-link aliases).
//! Allocated bytes count each file identity once within the selected scope.
//! Standard scans count each file's main data stream; the MFT reader also
//! counts named streams. Filesystem bookkeeping is never included.

use crossbeam_channel::{Receiver, Sender, bounded};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs::{self, File},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const QUEUE_CAPACITY: usize = 9;
const BATCH_SIZE: usize = 256;
const MAX_DIAGNOSTICS: usize = 24;
const MAX_RETAINED_BYTES: usize = 1536 * 1024 * 1024;
const MAX_NOTE_BYTES: usize = 4096;
const NOTE_TRUNCATED: &str = "… [additional details omitted]";

/// One scanned entry. Paths are not stored: at millions of entries they would
/// be the largest part of the tree. `node_path` rebuilds them from names; the
/// root's name is its full path.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScanNode {
    pub id: usize,
    pub parent: Option<usize>,
    pub name: String,
    pub is_dir: bool,
    pub logical: u64,
    pub allocated: u64,
    pub files: u64,
    pub children: Vec<usize>,
    pub note: String,
    #[serde(default)]
    pub incomplete: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ScanSummary {
    pub elapsed_ms: u64,
    pub cancelled: bool,
    pub errors: u64,
    pub skipped_reparse: u64,
    pub skipped_cloud: u64,
    pub hard_links: u64,
    pub method: String,
    pub notes: Vec<String>,
    #[serde(default)]
    pub incomplete_nodes: Vec<usize>,
    #[serde(default)]
    pub stopped_early: bool,
    /// Errors that were Windows refusing access, typically protected system
    /// folders, which an administrator scan can read. Included in `errors`.
    #[serde(default)]
    pub denied: u64,
    /// Errors from files Windows holds open exclusively (`pagefile.sys`); their
    /// sizes remain lower bounds even for administrators. Included in `errors`.
    #[serde(default)]
    pub locked: u64,
}

/// Full path of a node, joined from its ancestors' names.
pub fn node_path(nodes: &[ScanNode], id: usize) -> PathBuf {
    let mut chain = Vec::new();
    let mut next = nodes.get(id);
    while let Some(node) = next {
        chain.push(node.name.as_str());
        next = node.parent.and_then(|p| nodes.get(p));
    }
    chain.into_iter().rev().collect()
}

#[derive(Debug)]
pub enum ScanEvent {
    Batch(Vec<ScanNode>),
    Finished(ScanSummary),
}

pub struct ScanHandle {
    pub receiver: Receiver<ScanEvent>,
    pub cancel: Arc<AtomicBool>,
}

impl Drop for ScanHandle {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// Starts a metadata-only scan. It never requests elevation, opens file contents,
/// follows reparse points, or writes to the scanned tree.
pub fn start_scan(path: PathBuf) -> ScanHandle {
    let (sender, receiver) = bounded(QUEUE_CAPACITY);
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let failure_sender = sender.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("storage-scan".into())
        .spawn(move || scan(path, sender, worker_cancel))
    {
        let _ = failure_sender.try_send(ScanEvent::Finished(ScanSummary {
            errors: 1,
            notes: vec![format!("Could not start scan worker: {error}")],
            ..Default::default()
        }));
    }
    ScanHandle { receiver, cancel }
}

#[derive(Hash, Eq, PartialEq)]
struct FileIdentity(u64, [u8; 16]);

#[derive(Default)]
struct EntryInfo {
    logical: u64,
    allocated: u64,
    identity: Option<FileIdentity>,
    links: u64,
    note: String,
    errors: u64,
    refusal: Option<Refusal>,
    skip: Option<Skip>,
}

/// Why Windows refused to open an entry, when that is the whole error.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Refusal {
    Denied,
    Locked,
}

impl Refusal {
    fn of_os_error(code: Option<i32>) -> Option<Self> {
        match code {
            Some(5) => Some(Self::Denied),  // ERROR_ACCESS_DENIED
            Some(32) => Some(Self::Locked), // ERROR_SHARING_VIOLATION
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
enum Skip {
    Reparse,
    Cloud,
}

fn scan(path: PathBuf, sender: Sender<ScanEvent>, cancel: Arc<AtomicBool>) {
    let started = Instant::now();
    let path = match checked_path(&path).and_then(|path| {
        check_ancestors(&path)?;
        Ok(path)
    }) {
        Ok(path) => path,
        Err(error) => {
            let _ = sender.try_send(ScanEvent::Finished(ScanSummary {
                errors: 1,
                method: "Scan path validation".into(),
                notes: vec![error],
                ..Default::default()
            }));
            return;
        }
    };
    let summary = ScanSummary {
        method: "Folder listings".into(),
        notes: vec![
            "Logical bytes count each hard-link path; allocated bytes count each file identity once, attributed to the first path listed.".into(),
            "Directory indexes, MFT records, security metadata, filesystem journals and other volume overhead are excluded. Scanned totals are not volume used space.".into(),
            "Live scan, not a filesystem snapshot: files can change or disappear during enumeration. Reparse points and cloud placeholders are excluded without reading file contents.".into(),
        ],
        ..Default::default()
    };
    let mut worker = Worker {
        sender,
        cancel,
        summary,
        seen: HashSet::new(),
        batch: Vec::with_capacity(BATCH_SIZE),
        next_id: 0,
        last_flush: Instant::now(),
        retained_bytes: 0,
        stopped: false,
    };

    #[cfg(windows)]
    if mft::scan_volume(&path, &mut worker) {
        finish_scan(worker, started);
        return;
    }

    if let Some(visited) = worker.visit(&path, None)
        && visited.traverse
    {
        #[cfg(windows)]
        listing::walk(&mut worker, visited.id, path, visited.refused);
        #[cfg(not(windows))]
        walk_portable(&mut worker, visited.id, path, visited.refused);
    }
    finish_scan(worker, started);
}

/// One entry at a time, through `visit`. Windows lists whole folders instead.
#[cfg(not(windows))]
fn walk_portable(worker: &mut Worker, root: usize, path: PathBuf, refused: bool) {
    let mut directories = vec![(root, path, refused)];
    while let Some((parent, directory, refused)) = directories.pop() {
        if worker.cancelled() {
            break;
        }
        // Re-check immediately before enumeration, including queued directories
        // that may have been replaced with a junction since discovery.
        match fs::symlink_metadata(&directory) {
            Ok(metadata) if skip_metadata(&metadata).is_some() => {
                worker.record_node_error(
                    parent,
                    &directory,
                    "Directory changed into a reparse point or cloud placeholder; not traversed",
                );
                continue;
            }
            Err(error) => {
                worker.record_io_error(parent, &directory, &error, true);
                continue;
            }
            _ => {}
        }
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) => {
                // A folder Windows already refused to open counts as one refusal.
                worker.record_io_error(parent, &directory, &error, !refused);
                continue;
            }
        };
        for entry in entries {
            if worker.cancelled() {
                break;
            }
            match entry {
                Ok(entry) => {
                    let path = entry.path();
                    if let Some(visited) = worker.visit(&path, Some(parent))
                        && visited.traverse
                    {
                        directories.push((visited.id, path, visited.refused));
                    }
                }
                Err(error) => worker.record_io_error(parent, &directory, &error, true),
            }
            if !worker.flush(false) {
                break;
            }
        }
    }
}

fn finish_scan(mut worker: Worker, started: Instant) {
    worker.flush(true);
    worker.summary.stopped_early = worker.stopped;
    worker.summary.cancelled = worker.cancel.load(Ordering::Relaxed);
    if worker.summary.cancelled {
        worker.summary.incomplete_nodes.push(0);
    }
    worker.summary.incomplete_nodes.sort_unstable();
    worker.summary.incomplete_nodes.dedup();
    worker.summary.elapsed_ms = started.elapsed().as_millis().min(u64::MAX as u128) as u64;
    if worker.summary.errors > 0 {
        worker.summary.notes.push(format!("Partial accounting: {} metadata or enumeration errors. Incomplete entries carry known lower-bound sums; unavailable allocation contributes no bytes and is not a zero measurement.", worker.summary.errors));
    }
    // Batch sends reserve the final channel slot. Cancellation therefore never
    // leaves a worker blocked behind a UI that has stopped draining results.
    let _ = worker.sender.try_send(ScanEvent::Finished(worker.summary));
}

struct Worker {
    sender: Sender<ScanEvent>,
    cancel: Arc<AtomicBool>,
    summary: ScanSummary,
    seen: HashSet<FileIdentity>,
    batch: Vec<ScanNode>,
    next_id: usize,
    last_flush: Instant,
    retained_bytes: usize,
    stopped: bool,
}

struct Visited {
    id: usize,
    traverse: bool,
    /// Windows refused to open the entry; already counted in the summary.
    refused: bool,
}

/// Memory one more entry keeps alive in the UI's tree: its slot (the vector
/// grows by doubling, so half a spare slot on average), its name and note,
/// its index in the parent's child list, allocator headers and the hard-link
/// identity the worker remembers.
fn retained_entry_bytes(name: &str, note: &str) -> usize {
    std::mem::size_of::<ScanNode>() * 3 / 2 + name.len() + note.len() + 64
}

impl Worker {
    fn cancelled(&self) -> bool {
        self.stopped || self.cancel.load(Ordering::Relaxed)
    }

    fn record_error(&mut self, path: &Path, message: &str) {
        self.summary.errors += 1;
        if self.summary.incomplete_nodes.first() != Some(&0) {
            self.summary.incomplete_nodes.insert(0, 0);
        }
        if self.summary.notes.len() < MAX_DIAGNOSTICS {
            self.summary
                .notes
                .push(format!("{}: {message}", path.display()));
        }
    }

    fn record_node_error(&mut self, id: usize, path: &Path, message: &str) {
        // Sorting/deduplication happens once at completion, avoiding quadratic
        // work when many directories are inaccessible.
        self.summary.incomplete_nodes.push(id);
        self.record_error(path, message);
    }

    /// Records a filesystem error on node `id`. With `count_refusal`, access
    /// denied and sharing violations also count as refusals.
    fn record_io_error(&mut self, id: usize, path: &Path, error: &io::Error, count_refusal: bool) {
        if count_refusal {
            self.count_refusal(Refusal::of_os_error(error.raw_os_error()));
        }
        self.record_node_error(id, path, &error.to_string());
    }

    fn count_refusal(&mut self, refusal: Option<Refusal>) {
        match refusal {
            Some(Refusal::Denied) => self.summary.denied += 1,
            Some(Refusal::Locked) => self.summary.locked += 1,
            None => {}
        }
    }

    /// Ends the scan at a memory budget. Reported first and even when the
    /// per-path diagnostics are full: it is why every later folder is missing.
    fn stop_at_budget(&mut self, path: &Path, message: &str) {
        self.summary.errors += 1;
        if self.summary.incomplete_nodes.first() != Some(&0) {
            self.summary.incomplete_nodes.insert(0, 0);
        }
        self.summary
            .notes
            .insert(0, format!("{}: {message}", path.display()));
        self.stopped = true;
    }

    fn visit(&mut self, path: &Path, parent: Option<usize>) -> Option<Visited> {
        if self.cancelled() {
            return None;
        }
        // The root keeps its full path; every other path is rebuilt from it.
        let name = match parent {
            None => path.as_os_str(),
            Some(_) => path.file_name().unwrap_or(path.as_os_str()),
        }
        .to_string_lossy()
        .into_owned();
        let mut node = ScanNode {
            id: self.next_id,
            parent,
            name,
            is_dir: false,
            logical: 0,
            allocated: 0,
            files: 0,
            children: Vec::new(),
            note: String::new(),
            incomplete: false,
        };
        let mut traverse = false;
        let mut refused = false;
        match fs::symlink_metadata(path) {
            Ok(metadata) => {
                node.is_dir = metadata.is_dir();
                let mut info = match skip_metadata(&metadata) {
                    Some(skip) => EntryInfo {
                        skip: Some(skip),
                        ..Default::default()
                    },
                    None => read_entry(path, &metadata),
                };
                if let Some(skip) = info.skip {
                    self.mark_skipped(&mut node, skip);
                } else {
                    traverse = node.is_dir;
                    node.logical = info.logical;
                    node.files = u64::from(!node.is_dir);
                    if let Some(identity) = info.identity.take()
                        && !self.seen.insert(identity)
                    {
                        self.summary.hard_links += 1;
                        info.allocated = 0;
                        append_note(
                            &mut info.note,
                            "Hard-link alias: allocated bytes attributed to another path in this scan",
                        );
                    }
                    node.allocated = info.allocated;
                    node.note = info.note;
                    node.incomplete = info.errors > 0;
                    self.summary.errors += info.errors;
                    refused = info.refusal.is_some();
                    self.count_refusal(info.refusal);
                }
            }
            Err(error) => {
                let refusal = Refusal::of_os_error(error.raw_os_error());
                refused = refusal.is_some();
                self.count_refusal(refusal);
                self.record_error(path, &error.to_string());
                node.note = format!("Metadata unavailable: {error}");
                node.incomplete = true;
            }
        }
        if node.note.len() > MAX_NOTE_BYTES {
            truncate_note(&mut node.note);
        }
        // Queued folders also hold their path until they are enumerated.
        let queued = if traverse { path.as_os_str().len() } else { 0 };
        let id = self.push(node, queued, path)?;
        Some(Visited {
            id,
            traverse,
            refused,
        })
    }

    fn mark_skipped(&mut self, node: &mut ScanNode, skip: Skip) {
        node.incomplete = true;
        match skip {
            Skip::Reparse => {
                self.summary.skipped_reparse += 1;
                node.note = "Reparse point excluded; target not followed".into();
            }
            Skip::Cloud => {
                self.summary.skipped_cloud += 1;
                node.note = "Cloud/offline placeholder excluded; no hydration requested".into();
            }
        }
    }

    /// Queues a finished node, whose `id` must be `next_id`, within the
    /// memory budget. `queued` is memory held until a folder is enumerated;
    /// `at` names where a scan that runs out of budget stopped.
    fn push(&mut self, node: ScanNode, queued: usize, at: &Path) -> Option<usize> {
        let retained = retained_entry_bytes(&node.name, &node.note) + queued;
        if self.retained_bytes.saturating_add(retained) > MAX_RETAINED_BYTES {
            self.stop_at_budget(at, "Scan metadata reached the 1.5 GiB memory budget; remaining entries were not scanned. Scan a smaller folder to continue.");
            return None;
        }
        self.retained_bytes += retained;
        let id = node.id;
        self.next_id += 1;
        self.batch.push(node);
        Some(id)
    }

    fn flush(&mut self, force: bool) -> bool {
        if self.batch.is_empty() {
            return !self.cancelled();
        }
        if !force
            && self.batch.len() < BATCH_SIZE
            && self.last_flush.elapsed() < Duration::from_millis(50)
        {
            return !self.cancelled();
        }
        while self.sender.len() >= QUEUE_CAPACITY - 1 {
            if self.cancelled() {
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let batch = std::mem::replace(&mut self.batch, Vec::with_capacity(BATCH_SIZE));
        if self.sender.try_send(ScanEvent::Batch(batch)).is_err() {
            self.cancel.store(true, Ordering::Relaxed);
            return false;
        }
        self.last_flush = Instant::now();
        !self.cancelled()
    }
}

fn append_note(note: &mut String, addition: &str) {
    if note.ends_with(NOTE_TRUNCATED) {
        return;
    }
    if note.len() >= MAX_NOTE_BYTES - 2 {
        truncate_note(note);
        return;
    }
    if !note.is_empty() {
        note.push_str("; ");
    }
    let mut end = addition.len().min(MAX_NOTE_BYTES - note.len());
    while !addition.is_char_boundary(end) {
        end -= 1;
    }
    note.push_str(&addition[..end]);
    if end < addition.len() {
        truncate_note(note);
    }
}

fn truncate_note(note: &mut String) {
    let mut end = note.len().min(MAX_NOTE_BYTES - NOTE_TRUNCATED.len());
    while !note.is_char_boundary(end) {
        end -= 1;
    }
    note.truncate(end);
    note.push_str(NOTE_TRUNCATED);
}

#[cfg(windows)]
fn skip_metadata(metadata: &fs::Metadata) -> Option<Skip> {
    use std::os::windows::fs::MetadataExt;
    native::skip_attributes(metadata.file_attributes())
}

#[cfg(not(windows))]
fn skip_metadata(metadata: &fs::Metadata) -> Option<Skip> {
    metadata.file_type().is_symlink().then_some(Skip::Reparse)
}

#[cfg(windows)]
fn read_entry(path: &Path, metadata: &fs::Metadata) -> EntryInfo {
    native::read_entry(path, metadata)
}

#[cfg(unix)]
fn read_entry(_path: &Path, metadata: &fs::Metadata) -> EntryInfo {
    use std::os::unix::fs::MetadataExt;
    let mut identity = [0; 16];
    identity[..8].copy_from_slice(&metadata.ino().to_le_bytes());
    EntryInfo {
        logical: if metadata.is_dir() { 0 } else { metadata.len() },
        allocated: if metadata.is_dir() {
            0
        } else {
            metadata.blocks().saturating_mul(512)
        },
        identity: Some(FileIdentity(metadata.dev(), identity)),
        links: metadata.nlink(),
        note: "POSIX stat; named stream accounting unavailable on this platform".into(),
        ..Default::default()
    }
}

/// Nodes as exported: the stored fields plus each node's full path, built
/// while writing so a large scan never holds every path at once.
struct ExportNodes<'a>(&'a [ScanNode]);

impl Serialize for ExportNodes<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Node<'a> {
            id: usize,
            parent: Option<usize>,
            path: String,
            name: &'a str,
            is_dir: bool,
            logical: u64,
            allocated: u64,
            files: u64,
            children: &'a [usize],
            note: &'a str,
            incomplete: bool,
        }
        serializer.collect_seq(self.0.iter().map(|n| Node {
            id: n.id,
            parent: n.parent,
            path: node_path(self.0, n.id).to_string_lossy().into_owned(),
            name: &n.name,
            is_dir: n.is_dir,
            logical: n.logical,
            allocated: n.allocated,
            files: n.files,
            children: &n.children,
            note: &n.note,
            incomplete: n.incomplete,
        }))
    }
}

pub fn export_json(
    path: &Path,
    nodes: &[ScanNode],
    summary: Option<&ScanSummary>,
) -> Result<(), String> {
    #[derive(Serialize)]
    struct Export<'a> {
        schema_version: u32,
        sizes: &'static str,
        status: &'static str,
        summary: Option<&'a ScanSummary>,
        nodes: ExportNodes<'a>,
    }
    write_new_output_with(path, |file| {
        let mut writer = BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, &Export {
        schema_version: 1,
        sizes: "Bytes. Directory values are subtree totals as displayed. Incomplete entries contain known lower-bound sums; missing allocation contributes no bytes, not a zero measurement. Logical counts hard-link paths; allocated counts each file identity once. Named data streams are included only when summary.method is the NTFS master file table; filesystem overhead excluded.",
        status: export_status(summary), summary, nodes: ExportNodes(nodes),
        }).map_err(|e| e.to_string())?;
        writer.flush().map_err(|e| e.to_string())
    })
}

pub fn export_csv(
    path: &Path,
    nodes: &[ScanNode],
    summary: Option<&ScanSummary>,
) -> Result<(), String> {
    write_new_output_with(path, |file| {
        let mut writer = csv::Writer::from_writer(file);
        writer
            .write_record([
                "id",
                "parent_id",
                "kind",
                "path",
                "logical_bytes",
                "allocated_bytes",
                "file_count",
                "note",
                "scan_status",
                "incomplete",
            ])
            .map_err(|e| e.to_string())?;
        for node in nodes {
            writer
                .write_record([
                    node.id.to_string(),
                    node.parent.map(|v| v.to_string()).unwrap_or_default(),
                    if node.is_dir { "directory" } else { "file" }.into(),
                    csv_text(&node_path(nodes, node.id).to_string_lossy()),
                    node.logical.to_string(),
                    node.allocated.to_string(),
                    node.files.to_string(),
                    csv_text(&node.note),
                    export_status(summary).into(),
                    node.incomplete.to_string(),
                ])
                .map_err(|e| e.to_string())?;
        }
        writer.flush().map_err(|e| e.to_string())
    })
}

/// Publish a complete new output without replacing existing files, aliases,
/// symlinks or directories. Failed writes leave no partial destination.
pub(crate) fn write_new_output(path: &Path, data: &[u8]) -> Result<(), String> {
    write_new_output_with(path, |file| file.write_all(data).map_err(|e| e.to_string()))
}

/// Prepare a single output directory only after validating its existing parent
/// chain. Do not recursively create paths through unverified ancestors.
pub(crate) fn prepare_output_directory(path: &Path) -> Result<(), String> {
    let path = checked_path(path)?;
    check_ancestors(&path)?;
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_dir() && skip_metadata(&metadata).is_none() => Ok(()),
        Ok(_) => Err("Capture destination must be an ordinary directory".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(path).map_err(|error| error.to_string())
        }
        Err(error) => Err(error.to_string()),
    }
}

fn write_new_output_with(
    path: &Path,
    write: impl FnOnce(&mut File) -> Result<(), String>,
) -> Result<(), String> {
    let path = checked_path(path)?;
    if path.file_name().is_none() {
        return Err("Choose a new file name for the output".into());
    }
    check_ancestors(&path)?;
    match fs::symlink_metadata(&path) {
        Ok(_) => {
            return Err(
                "File already exists; choose a new file name. Existing files are never replaced."
                    .into(),
            );
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }
    let parent = path.parent().ok_or("Output has no parent directory")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    write(temporary.as_file_mut())?;
    temporary.as_file().sync_all().map_err(|e| e.to_string())?;
    // No existence-check/rename race: persist_noclobber atomically refuses a
    // destination created while serialization was in progress.
    temporary.persist_noclobber(&path).map_err(|e| {
        if e.error.kind() == std::io::ErrorKind::AlreadyExists {
            "File already exists; choose a new file name. Existing files are never replaced.".into()
        } else {
            e.error.to_string()
        }
    })?;
    Ok(())
}

/// A typed or pasted scan path without surrounding whitespace or the double
/// quotes Explorer's "Copy as path" adds.
pub(crate) fn scan_input_path(input: &str) -> &str {
    let trimmed = input.trim();
    trimmed
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .map_or(trimmed, str::trim)
}

fn checked_path(path: &Path) -> Result<PathBuf, String> {
    #[cfg(windows)]
    native::validate_path(path)?;
    #[cfg(windows)]
    let path = &drive_root_or_absolute(path)?;
    let path = std::path::absolute(path).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    native::validate_path(&path)?;
    Ok(path)
}

/// Windows resolves `C:` and `C:folder` against a hidden per-drive current
/// directory, not the drive root. A bare drive letter means its root; a
/// drive-relative path is rejected rather than scanning an unrelated folder.
#[cfg(windows)]
fn drive_root_or_absolute(path: &Path) -> Result<PathBuf, String> {
    use std::path::{Component, Prefix};
    let Some(Component::Prefix(prefix)) = path.components().next() else {
        return Ok(path.into());
    };
    if !matches!(prefix.kind(), Prefix::Disk(_)) {
        return Ok(path.into());
    }
    // Inspect the raw text: `components()` drops a `.` after the prefix.
    let drive = prefix.as_os_str().to_string_lossy();
    let text = path.to_string_lossy();
    let rest = &text[drive.len()..];
    if rest.is_empty() {
        Ok(PathBuf::from(format!("{drive}\\")))
    } else if rest.starts_with(['\\', '/']) {
        Ok(path.into())
    } else {
        Err(format!(
            "{} is drive-relative; include a backslash after the drive letter, for example {drive}\\",
            path.display()
        ))
    }
}

/// Check from the root down so a selected path beneath an existing junction or
/// cloud placeholder is rejected before opening anything through that ancestor.
/// This is deliberately not a snapshot guarantee against concurrent renames.
fn check_ancestors(path: &Path) -> Result<(), String> {
    let mut ancestors: Vec<_> = path
        .parent()
        .into_iter()
        .flat_map(Path::ancestors)
        .collect();
    ancestors.reverse();
    for ancestor in ancestors {
        let metadata =
            fs::symlink_metadata(ancestor).map_err(|e| format!("{}: {e}", ancestor.display()))?;
        if skip_metadata(&metadata).is_some() {
            return Err(format!(
                "{}: reparse/symlink or cloud ancestor excluded",
                ancestor.display()
            ));
        }
        if !metadata.is_dir() {
            return Err(format!(
                "{}: ancestor is not a directory",
                ancestor.display()
            ));
        }
    }
    Ok(())
}

fn export_status(summary: Option<&ScanSummary>) -> &'static str {
    match summary {
        None => "scanning_partial",
        Some(s) if s.cancelled => "cancelled_partial",
        Some(s) if s.errors > 0 || s.stopped_early || !s.incomplete_nodes.is_empty() => "partial",
        Some(s) if s.skipped_cloud > 0 || s.skipped_reparse > 0 => "complete_with_exclusions",
        Some(_) => "complete",
    }
}

// Prevent spreadsheet formulas when an exported filename/diagnostic begins with
// a formula introducer. JSON preserves exact strings without this CSV escape.
fn csv_text(text: &str) -> String {
    let trimmed = text.trim_start_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
    if text.starts_with(['\t', '\r', '\n']) || trimmed.starts_with(['=', '+', '-', '@']) {
        format!("'{text}")
    } else {
        text.into()
    }
}

#[cfg(windows)]
mod listing;
#[cfg(windows)]
mod native;

#[cfg(windows)]
mod mft;

#[cfg(test)]
mod tests;
