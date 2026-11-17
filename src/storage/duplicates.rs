//! Duplicate files among the entries of a completed scan. Unlike the scan, this
//! reads file contents, and only when asked. Nothing is changed or deleted.
//!
//! The pipeline follows fclones and czkawka: files are grouped by size, extra
//! paths of the same file (hard links) are set aside, then one open per file
//! hashes its first and last 16 KiB, and only files that still have company
//! are read in full. Files of up to 32 KiB are completely covered by that
//! first read. Contents are compared by BLAKE3, a cryptographic hash, so equal
//! hashes mean equal contents for any practical purpose, even for crafted files.
//!
//! A file is compared only if it is the same file, unchanged, at every read:
//! same identity, size, write and change time, and a full-length read.
//! Results therefore mean "identical when read", not a snapshot guarantee.

use super::{
    FileIdentity, MAX_DIAGNOSTICS, Refusal, ScanNode, Wake, csv_text, node_path,
    write_new_output_with,
};
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, bounded};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::File,
    io::{self, BufWriter, Read, Seek, SeekFrom, Write},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

/// Files below this size waste little space and are many; the UI starts here.
pub const DEFAULT_MIN_SIZE: u64 = 1 << 20;
/// Bytes hashed at each end of a file before deciding to read all of it.
/// 16 KiB costs one I/O, like 4 KiB, and gets past shared headers.
const SAMPLE: u64 = 16 * 1024;
const CHUNK: usize = 1 << 20;
const QUEUE_CAPACITY: usize = 16;
const BATCH_GROUPS: usize = 256;

/// Copies of one file's contents, oldest first.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DuplicateGroup {
    /// Logical size of each copy in bytes.
    pub size: u64,
    pub blake3: String,
    pub copies: Vec<Copy>,
    /// Bytes on disk that keeping only one copy would free: the sum of what
    /// deleting each copy frees, less the largest. An upper bound where a
    /// filesystem shares blocks between copies (ReFS block cloning).
    pub reclaimable: u64,
}

impl DuplicateGroup {
    /// Largest savings first, then larger files, then scan order.
    pub fn order(&self) -> (std::cmp::Reverse<u64>, std::cmp::Reverse<u64>, usize) {
        (
            std::cmp::Reverse(self.reclaimable),
            std::cmp::Reverse(self.size),
            self.copies[0].id,
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Copy {
    pub id: usize,
    /// Last write time in milliseconds since 1970, when the filesystem reports one.
    pub modified_ms: Option<i64>,
    /// Bytes on disk deleting this path would free: zero while other hard
    /// links keep the file, and only the unshared part of an APFS clone.
    pub freeable: u64,
    pub links: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Stage {
    /// Reading the first and last 16 KiB of every same-size file.
    #[default]
    Sampling,
    /// Reading files whose samples matched another file's in full.
    Contents,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Progress {
    pub stage: Stage,
    pub files_done: u64,
    pub files_total: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DuplicateSummary {
    pub root: usize,
    pub min_size: u64,
    pub elapsed_ms: u64,
    pub cancelled: bool,
    /// Kind of drive, which sets how many files are read at once.
    pub medium: String,
    /// Files at or above the minimum size.
    pub files: u64,
    /// Files below the minimum size, not examined.
    pub below_minimum: u64,
    pub below_minimum_bytes: u64,
    /// Files whose size the scan could not determine, not examined.
    pub unknown_size: u64,
    /// Files that shared their size with another file and were read.
    pub compared: u64,
    pub bytes_read: u64,
    pub groups: u64,
    /// Copies beyond the first of each group.
    pub duplicates: u64,
    pub reclaimable: u64,
    /// Further paths of a file already compared under another path.
    pub hard_links: u64,
    /// Files that changed, disappeared or were replaced since the scan or
    /// between reads; none of them is reported.
    pub changed: u64,
    /// Links, cloud placeholders and special files found when opening.
    pub skipped: u64,
    pub errors: u64,
    pub denied: u64,
    pub locked: u64,
    /// The scan itself was partial, so files outside it were not compared.
    pub scan_incomplete: bool,
    pub notes: Vec<String>,
}

impl DuplicateSummary {
    /// Every examined file was read; groups may still miss files that are
    /// below the minimum size, which the summary reports separately.
    pub fn status(&self) -> &'static str {
        if self.cancelled {
            "cancelled_partial"
        } else if self.errors > 0 || self.changed > 0 || self.scan_incomplete {
            "partial"
        } else {
            "complete"
        }
    }
}

#[derive(Debug)]
pub enum DuplicateEvent {
    Progress(Progress),
    Groups(Vec<DuplicateGroup>),
    Finished(DuplicateSummary),
}

pub struct DuplicateHandle {
    pub receiver: Receiver<DuplicateEvent>,
    pub cancel: Arc<AtomicBool>,
}

impl Drop for DuplicateHandle {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// Starts comparing the files below `root`. `nodes` must be a finished scan,
/// `scan_partial` whether it missed anything below `root` (see
/// [`super::subtree_partial`]); `wake` is called whenever events are queued.
pub fn find(
    nodes: Arc<Vec<ScanNode>>,
    root: usize,
    min_size: u64,
    scan_partial: bool,
    wake: Wake,
) -> DuplicateHandle {
    let (sender, receiver) = bounded(QUEUE_CAPACITY);
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let failure = sender.clone();
    let failure_wake = wake.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("duplicates".into())
        .spawn(move || {
            let mut finder = Finder {
                nodes: &nodes,
                sender,
                wake,
                cancel: worker_cancel,
                summary: DuplicateSummary {
                    root,
                    min_size: min_size.max(1),
                    scan_incomplete: scan_partial,
                    ..Default::default()
                },
                pending: Vec::new(),
                last_flush: Instant::now(),
                last_progress: Instant::now(),
            };
            finder.run(root);
        })
    {
        let _ = failure.try_send(DuplicateEvent::Finished(DuplicateSummary {
            root,
            min_size,
            errors: 1,
            notes: vec![format!("Could not start duplicate search: {error}")],
            ..Default::default()
        }));
        failure_wake();
    }
    DuplicateHandle { receiver, cancel }
}

/// A file that shares its size with at least one other file.
struct Candidate {
    id: usize,
    size: u64,
    /// The scan's bytes on disk for this path; zero for a hard-link alias.
    allocated: u64,
    file: Option<Known>,
}

/// What the first read established about a candidate.
#[derive(Clone, Copy)]
struct Known {
    identity: Option<FileIdentity>,
    stamp: Stamp,
    links: u64,
    modified_ms: Option<i64>,
    /// Bytes freed by deleting it, when the filesystem can say (APFS).
    private: Option<u64>,
}

/// Changes whenever the file's contents may have.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Stamp {
    size: u64,
    written: i64,
    changed: i64,
}

/// An open file as the platform reports it.
struct Opened {
    file: File,
    known: Known,
}

enum Failure {
    /// The file is no longer the scanned one, changed or vanished.
    Changed,
    /// A link, cloud placeholder or special file: not compared, not an error.
    Skipped,
    Io(io::Error),
    Cancelled,
}

impl From<io::Error> for Failure {
    fn from(error: io::Error) -> Self {
        match error.kind() {
            io::ErrorKind::NotFound | io::ErrorKind::UnexpectedEof => Self::Changed,
            _ if platform::is_placeholder(&error) => Self::Skipped,
            _ if platform::is_link(&error) => Self::Changed,
            _ => Self::Io(error),
        }
    }
}

/// Jobs for one stage of reads: candidate index and, for full reads, the group.
type Job = (usize, usize);

struct Finder<'a> {
    nodes: &'a [ScanNode],
    sender: Sender<DuplicateEvent>,
    wake: Wake,
    cancel: Arc<AtomicBool>,
    summary: DuplicateSummary,
    pending: Vec<DuplicateGroup>,
    last_flush: Instant,
    last_progress: Instant,
}

impl Finder<'_> {
    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    fn run(&mut self, root: usize) {
        let started = Instant::now();
        let nodes = self.nodes;
        let root_path = node_path(nodes, root);
        let (medium, sharing) = platform::volume(&root_path);
        self.summary.medium = medium.label().into();
        if self.summary.scan_incomplete {
            self.summary.notes.push(
                "The scan of this folder is partial; files it did not reach were not compared."
                    .into(),
            );
        }
        if let Some(note) = sharing {
            self.summary.notes.push(note.into());
        }
        let (sample_threads, content_threads) = medium.threads();
        let mut candidates = self.collect(root);
        let sizes = size_groups(&candidates);
        let mut jobs: Vec<Job> = sizes
            .iter()
            .flat_map(|r| r.clone())
            .map(|i| (i, 0))
            .collect();
        if medium == Medium::Rotational {
            // Scan order approximates disk order, which a single head favors.
            jobs.sort_unstable_by_key(|&(i, _)| candidates[i].id);
        }
        let mut samples = vec![None; candidates.len()];
        let mut known = vec![None; candidates.len()];
        self.stage(
            Stage::Sampling,
            &jobs,
            &candidates,
            sample_threads,
            |candidate, buffer, bytes, cancel| {
                sample(
                    &node_path(nodes, candidate.id),
                    candidate,
                    buffer,
                    bytes,
                    cancel,
                )
            },
            |finder, (index, _), outcome| {
                if let Some((file, hash)) = finder.accept(&candidates[index], outcome) {
                    samples[index] = Some(hash);
                    known[index] = Some(file);
                }
            },
        );
        for (candidate, file) in candidates.iter_mut().zip(known) {
            candidate.file = file;
        }
        if self.cancelled() {
            return self.finish(started);
        }
        // Split each size by sample; files up to 32 KiB are now fully compared.
        let mut full: Vec<Vec<usize>> = Vec::new();
        for range in sizes {
            let members = self.distinct_files(&candidates, range);
            for group in split(&members, |&i| samples[i]) {
                if candidates[group[0]].size <= 2 * SAMPLE {
                    let hash = samples[group[0]].expect("sampled");
                    self.emit(&candidates, &group, hash);
                } else {
                    full.push(group);
                }
            }
        }
        drop(samples);
        let jobs: Vec<Job> = full
            .iter()
            .enumerate()
            .flat_map(|(g, members)| members.iter().map(move |&i| (i, g)))
            .collect();
        let mut hashes: Vec<Option<[u8; 32]>> = vec![None; candidates.len()];
        let mut remaining: Vec<usize> = full.iter().map(Vec::len).collect();
        self.stage(
            Stage::Contents,
            &jobs,
            &candidates,
            content_threads,
            |candidate, buffer, bytes, cancel| {
                contents(
                    &node_path(nodes, candidate.id),
                    candidate,
                    buffer,
                    bytes,
                    cancel,
                )
            },
            |finder, (index, group), outcome| {
                if let Some((_, hash)) = finder.accept(&candidates[index], outcome) {
                    hashes[index] = Some(hash);
                }
                remaining[group] -= 1;
                // Largest sizes come first: each group appears once all its
                // members are read, so the biggest savings stream in early.
                if remaining[group] == 0 {
                    for same in split(&full[group], |&i| hashes[i]) {
                        let hash = hashes[same[0]].expect("hashed");
                        finder.emit(&candidates, &same, hash);
                    }
                }
            },
        );
        self.finish(started);
    }

    /// Files below `root` at or above the minimum size whose size is shared,
    /// sorted by descending size so the largest groups are read first.
    fn collect(&mut self, root: usize) -> Vec<Candidate> {
        let mut files = Vec::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            let Some(node) = self.nodes.get(id) else {
                continue;
            };
            if node.is_dir {
                stack.extend(node.children.iter().copied());
            } else if node.incomplete {
                self.summary.unknown_size += 1;
            } else if node.logical < self.summary.min_size {
                self.summary.below_minimum += 1;
                self.summary.below_minimum_bytes = self
                    .summary
                    .below_minimum_bytes
                    .saturating_add(node.logical);
            } else {
                files.push(Candidate {
                    id,
                    size: node.logical,
                    allocated: node.allocated,
                    file: None,
                });
            }
        }
        self.summary.files = files.len() as u64;
        files.sort_unstable_by_key(|c| (std::cmp::Reverse(c.size), c.id));
        files
    }

    /// Records a read's outcome; returns what it established when it succeeded.
    fn accept(
        &mut self,
        candidate: &Candidate,
        outcome: Result<(Known, [u8; 32]), Failure>,
    ) -> Option<(Known, [u8; 32])> {
        match outcome {
            Ok(result) => return Some(result),
            Err(Failure::Changed) => self.summary.changed += 1,
            Err(Failure::Skipped) => self.summary.skipped += 1,
            Err(Failure::Cancelled) => {}
            Err(Failure::Io(error)) => {
                self.summary.errors += 1;
                match Refusal::of_os_error(error.raw_os_error()) {
                    Some(Refusal::Denied) => self.summary.denied += 1,
                    Some(Refusal::Locked) => self.summary.locked += 1,
                    None => {}
                }
                if self.summary.notes.len() < MAX_DIAGNOSTICS {
                    let path = node_path(self.nodes, candidate.id);
                    self.summary
                        .notes
                        .push(format!("{}: {error}", path.display()));
                }
            }
        }
        None
    }

    /// The sampled members of one size, one path per file: further paths of
    /// the same file are hard links, not copies, and free nothing.
    fn distinct_files(
        &mut self,
        candidates: &[Candidate],
        range: std::ops::Range<usize>,
    ) -> Vec<usize> {
        let mut seen = HashMap::new();
        let mut members = Vec::new();
        for index in range {
            let Some(known) = &candidates[index].file else {
                continue;
            };
            if let Some(identity) = known.identity
                && seen.insert(identity, index).is_some()
            {
                self.summary.hard_links += 1;
                continue;
            }
            members.push(index);
        }
        members
    }

    fn emit(&mut self, candidates: &[Candidate], members: &[usize], hash: [u8; 32]) {
        let mut copies: Vec<Copy> = members
            .iter()
            .map(|&i| {
                let candidate = &candidates[i];
                let known = candidate.file.expect("read before grouping");
                Copy {
                    id: candidate.id,
                    modified_ms: known.modified_ms,
                    freeable: if known.links > 1 {
                        0
                    } else {
                        known.private.unwrap_or(candidate.allocated)
                    },
                    links: known.links,
                }
            })
            .collect();
        // The oldest copy is most likely the original.
        copies.sort_by_key(|c| (c.modified_ms.is_none(), c.modified_ms, c.id));
        let total: u64 = copies.iter().map(|c| c.freeable).sum();
        let largest = copies.iter().map(|c| c.freeable).max().unwrap_or(0);
        let group = DuplicateGroup {
            size: candidates[members[0]].size,
            blake3: hex(&hash),
            reclaimable: total - largest,
            copies,
        };
        self.summary.groups += 1;
        self.summary.duplicates += group.copies.len() as u64 - 1;
        self.summary.reclaimable = self.summary.reclaimable.saturating_add(group.reclaimable);
        self.pending.push(group);
        if self.pending.len() >= BATCH_GROUPS
            || self.last_flush.elapsed() > Duration::from_millis(100)
        {
            self.flush();
        }
    }

    /// Sends finished groups, waiting while the UI catches up. One queue slot
    /// stays free for the summary, so finishing never blocks.
    fn flush(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        while self.sender.len() >= QUEUE_CAPACITY - 1 {
            if self.cancelled() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let groups = std::mem::take(&mut self.pending);
        if self
            .sender
            .try_send(DuplicateEvent::Groups(groups))
            .is_err()
        {
            self.cancel.store(true, Ordering::Relaxed);
        }
        (self.wake)();
        self.last_flush = Instant::now();
    }

    fn progress(&mut self, progress: Progress) {
        if self.last_progress.elapsed() < Duration::from_millis(100) {
            return;
        }
        self.last_progress = Instant::now();
        // Progress is advisory: drop it rather than wait or take the last slot.
        if self.sender.len() < QUEUE_CAPACITY - 2
            && self
                .sender
                .try_send(DuplicateEvent::Progress(progress))
                .is_ok()
        {
            (self.wake)();
        }
    }

    fn finish(&mut self, started: Instant) {
        self.flush();
        self.summary.cancelled = self.cancelled();
        self.summary.elapsed_ms = started.elapsed().as_millis().min(u64::MAX as u128) as u64;
        if self.summary.errors > 0 {
            self.summary.notes.push(format!(
                "{} files could not be read; copies among them are not reported.",
                self.summary.errors
            ));
        }
        let _ = self
            .sender
            .try_send(DuplicateEvent::Finished(std::mem::take(&mut self.summary)));
        (self.wake)();
    }

    /// Reads `jobs` on `threads` threads and hands each outcome to `each` on
    /// this thread, in completion order, reporting progress meanwhile.
    fn stage<W, E>(
        &mut self,
        stage: Stage,
        jobs: &[Job],
        candidates: &[Candidate],
        threads: usize,
        work: W,
        mut each: E,
    ) where
        W: Fn(&Candidate, &mut [u8], &AtomicU64, &AtomicBool) -> Result<(Known, [u8; 32]), Failure>
            + Sync,
        E: FnMut(&mut Self, Job, Result<(Known, [u8; 32]), Failure>),
    {
        if jobs.is_empty() {
            return;
        }
        let bytes_total = jobs
            .iter()
            .map(|&(i, _)| match stage {
                Stage::Sampling => candidates[i].size.min(2 * SAMPLE),
                Stage::Contents => candidates[i].size,
            })
            .fold(0u64, u64::saturating_add);
        let bytes = AtomicU64::new(0);
        let next = AtomicUsize::new(0);
        let cancel = self.cancel.clone();
        let (results, received) = bounded(threads.max(1) * 4);
        let mut files_done = 0;
        std::thread::scope(|scope| {
            let reader = |results: Sender<_>| {
                platform::prepare_reader_thread();
                let mut buffer = vec![0; CHUNK];
                while !cancel.load(Ordering::Relaxed) {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(&job) = jobs.get(index) else {
                        break;
                    };
                    let outcome = work(&candidates[job.0], &mut buffer, &bytes, &cancel);
                    if results.send((job, outcome)).is_err() {
                        break;
                    }
                }
            };
            let spawned = (0..threads.clamp(1, jobs.len()))
                .filter(|_| {
                    let results = results.clone();
                    std::thread::Builder::new()
                        .name("duplicate-reader".into())
                        .spawn_scoped(scope, move || reader(results))
                        .is_ok()
                })
                .count();
            // Only the readers hold senders now: the loop below ends with them.
            drop(results);
            if spawned == 0 {
                // No thread could start: read on this one instead.
                let mut buffer = vec![0; CHUNK];
                for &job in jobs {
                    if self.cancelled() {
                        break;
                    }
                    let outcome = work(&candidates[job.0], &mut buffer, &bytes, &cancel);
                    each(self, job, outcome);
                }
                return;
            }
            loop {
                match received.recv_timeout(Duration::from_millis(100)) {
                    Ok((job, outcome)) => {
                        files_done += 1;
                        each(self, job, outcome);
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
                self.progress(Progress {
                    stage,
                    files_done,
                    files_total: jobs.len() as u64,
                    bytes_done: bytes.load(Ordering::Relaxed),
                    bytes_total,
                });
            }
        });
        self.summary.bytes_read = self
            .summary
            .bytes_read
            .saturating_add(bytes.load(Ordering::Relaxed));
        if stage == Stage::Sampling {
            self.summary.compared = files_done;
        }
    }
}

/// Ranges of candidates sharing one size, for sizes held by two or more.
fn size_groups(candidates: &[Candidate]) -> Vec<std::ops::Range<usize>> {
    let mut groups = Vec::new();
    let mut start = 0;
    for end in 1..=candidates.len() {
        if end == candidates.len() || candidates[end].size != candidates[start].size {
            if end - start >= 2 {
                groups.push(start..end);
            }
            start = end;
        }
    }
    groups
}

/// Members with equal keys, in groups of two or more; unread members drop out.
fn split(members: &[usize], key: impl Fn(&usize) -> Option<[u8; 32]>) -> Vec<Vec<usize>> {
    let mut keyed: Vec<_> = members
        .iter()
        .filter_map(|m| key(m).map(|k| (k, *m)))
        .collect();
    keyed.sort_unstable();
    keyed
        .chunk_by(|a, b| a.0 == b.0)
        .filter(|run| run.len() >= 2)
        .map(|run| run.iter().map(|&(_, m)| m).collect())
        .collect()
}

/// Opens a candidate and checks it is still the file the scan listed, or,
/// once read before, the very file read then.
fn open(candidate: &Candidate, path: &Path, sequential: bool) -> Result<Opened, Failure> {
    let opened = platform::open(path, sequential)?;
    let known = &opened.known;
    if known.stamp.size != candidate.size {
        return Err(Failure::Changed);
    }
    if let Some(before) = &candidate.file
        && (before.stamp != known.stamp || before.identity != known.identity)
    {
        return Err(Failure::Changed);
    }
    Ok(opened)
}

/// Fails as changed if the file was modified while it was read.
fn unchanged(opened: &Opened) -> Result<(), Failure> {
    match platform::stamp(&opened.file)? {
        stamp if stamp == opened.known.stamp => Ok(()),
        _ => Err(Failure::Changed),
    }
}

/// BLAKE3 of the first and last 16 KiB; of the whole file up to 32 KiB.
fn sample(
    path: &Path,
    candidate: &Candidate,
    buffer: &mut [u8],
    bytes: &AtomicU64,
    cancel: &AtomicBool,
) -> Result<(Known, [u8; 32]), Failure> {
    let mut opened = open(candidate, path, false)?;
    let mut hasher = blake3::Hasher::new();
    let size = candidate.size;
    let head = size.min(SAMPLE);
    read_into(
        &mut opened.file,
        &mut hasher,
        0,
        head,
        buffer,
        bytes,
        cancel,
    )?;
    if size > SAMPLE {
        let start = SAMPLE.max(size - SAMPLE);
        read_into(
            &mut opened.file,
            &mut hasher,
            start,
            size - start,
            buffer,
            bytes,
            cancel,
        )?;
    }
    unchanged(&opened)?;
    Ok((opened.known, *hasher.finalize().as_bytes()))
}

/// BLAKE3 of the whole file.
fn contents(
    path: &Path,
    candidate: &Candidate,
    buffer: &mut [u8],
    bytes: &AtomicU64,
    cancel: &AtomicBool,
) -> Result<(Known, [u8; 32]), Failure> {
    let mut opened = open(candidate, path, true)?;
    let mut hasher = blake3::Hasher::new();
    read_into(
        &mut opened.file,
        &mut hasher,
        0,
        candidate.size,
        buffer,
        bytes,
        cancel,
    )?;
    // A file that grew while read has more than its size behind the last byte.
    if opened.file.read(&mut buffer[..1])? != 0 {
        return Err(Failure::Changed);
    }
    unchanged(&opened)?;
    Ok((opened.known, *hasher.finalize().as_bytes()))
}

/// Hashes exactly `length` bytes from `offset`; a shorter file has changed.
fn read_into(
    file: &mut File,
    hasher: &mut blake3::Hasher,
    offset: u64,
    length: u64,
    buffer: &mut [u8],
    bytes: &AtomicU64,
    cancel: &AtomicBool,
) -> Result<(), Failure> {
    file.seek(SeekFrom::Start(offset))?;
    let mut left = length;
    while left > 0 {
        if cancel.load(Ordering::Relaxed) {
            return Err(Failure::Cancelled);
        }
        let want = buffer
            .len()
            .min(usize::try_from(left).unwrap_or(usize::MAX));
        let read = match file.read(&mut buffer[..want]) {
            Ok(0) => return Err(Failure::Changed),
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        };
        hasher.update(&buffer[..read]);
        bytes.fetch_add(read as u64, Ordering::Relaxed);
        left -= read as u64;
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// How many files to read at once, by the kind of drive the folder is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
pub(super) enum Medium {
    SolidState,
    Rotational,
    Network,
    Unknown,
}

impl Medium {
    fn label(self) -> &'static str {
        match self {
            Self::SolidState => "solid-state drive",
            Self::Rotational => "hard disk",
            Self::Network => "network drive",
            Self::Unknown => "unknown drive type",
        }
    }

    /// Threads for the sampling and for the full reads. Seeking between files
    /// costs a hard disk most of its throughput, so it reads one at a time.
    fn threads(self) -> (usize, usize) {
        let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
        match self {
            Self::SolidState => (cores.clamp(2, 8), cores.clamp(1, 4)),
            Self::Network => (4, 2),
            Self::Rotational => (1, 1),
            Self::Unknown => (2, 1),
        }
    }
}

/// Whether a `plist` from `diskutil info -plist` describes a solid-state drive.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn solid_state(plist: &str) -> Option<bool> {
    let rest = &plist[plist.find("<key>SolidState</key>")? + "<key>SolidState</key>".len()..];
    let value = rest.trim_start();
    if value.starts_with("<true/>") {
        Some(true)
    } else if value.starts_with("<false/>") {
        Some(false)
    } else {
        None
    }
}

#[cfg(windows)]
mod platform {
    use super::{super::native, Failure, Known, Medium, Opened, Stamp};
    use std::{fs::File, io, os::windows::io::AsRawHandle, path::Path};
    use windows::{
        Win32::{
            Foundation::HANDLE,
            Storage::FileSystem::{
                FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_SHARE_READ,
                FILE_SHARE_WRITE, GetDriveTypeW, GetVolumeInformationW,
                GetVolumeNameForVolumeMountPointW, GetVolumePathNameW, OPEN_EXISTING,
            },
            System::{
                IO::DeviceIoControl,
                Ioctl::{
                    DEVICE_SEEK_PENALTY_DESCRIPTOR, IOCTL_STORAGE_QUERY_PROPERTY,
                    PropertyStandardQuery, STORAGE_PROPERTY_QUERY,
                    StorageDeviceSeekPenaltyProperty,
                },
                WindowsProgramming::DRIVE_REMOTE,
            },
        },
        core::PCWSTR,
    };

    const IO_REPARSE_TAG_DEDUP: u32 = 0x8000_0013;
    /// 100 ns intervals between 1601 and 1970.
    const UNIX_EPOCH: i64 = 116_444_736_000_000_000;

    pub(super) fn prepare_reader_thread() {}

    pub(super) fn is_placeholder(_: &io::Error) -> bool {
        false
    }

    pub(super) fn is_link(_: &io::Error) -> bool {
        false
    }

    fn facts(file: &File) -> io::Result<native::Facts> {
        native::facts(HANDLE(file.as_raw_handle())).map_err(native::io_error)
    }

    fn stamp_of(facts: &native::Facts) -> Stamp {
        Stamp {
            size: facts.size,
            written: facts.written,
            changed: facts.changed,
        }
    }

    pub(super) fn stamp(file: &File) -> io::Result<Stamp> {
        facts(file).map(|facts| stamp_of(&facts))
    }

    pub(super) fn open(path: &Path, sequential: bool) -> Result<Opened, Failure> {
        let mut file = native::open_contents(path, true, sequential)?;
        let facts = facts(&file)?;
        if facts.attributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0 {
            return Err(Failure::Changed);
        }
        // CompactOS and deduplicated files are reparse points to the volume
        // but ordinary files to Windows: their filter driver supplies the
        // contents only through a handle that follows the reparse point.
        let filtered = facts.tag == native::IO_REPARSE_TAG_WOF || facts.tag == IO_REPARSE_TAG_DEDUP;
        let attributes = if filtered {
            facts.attributes & !FILE_ATTRIBUTE_REPARSE_POINT.0
        } else {
            facts.attributes
        };
        if native::skip_attributes(attributes).is_some() {
            return Err(Failure::Skipped);
        }
        if filtered {
            let followed = native::open_contents(path, false, sequential)?;
            // The followed handle must reach the very file just checked.
            if facts.identity.is_none() || self::facts(&followed)?.identity != facts.identity {
                return Err(Failure::Changed);
            }
            file = followed;
        }
        let modified_ms = (facts.written > 0).then(|| (facts.written - UNIX_EPOCH) / 10_000);
        Ok(Opened {
            file,
            known: Known {
                identity: facts.identity,
                stamp: stamp_of(&facts),
                links: facts.links,
                modified_ms,
                private: None,
            },
        })
    }

    /// A NUL-terminated copy of the string an API wrote into `buffer`.
    fn terminated(buffer: &[u16]) -> Vec<u16> {
        let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
        buffer[..end].iter().copied().chain([0]).collect()
    }

    /// The kind of drive behind `path`, and a note when its filesystem can
    /// share blocks between files.
    pub(super) fn volume(path: &Path) -> (Medium, Option<&'static str>) {
        use std::os::windows::ffi::OsStrExt;
        // Plain DOS and UNC syntax: the volume functions expect roots like
        // `C:\` and `\\server\share\`, not their `\\?\` forms.
        let name: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        let mut root = [0u16; 1024];
        if unsafe { GetVolumePathNameW(PCWSTR(name.as_ptr()), &mut root) }.is_err() {
            return (Medium::Unknown, None);
        }
        let root = terminated(&root);
        let mut file_system = [0u16; 64];
        let refs = unsafe {
            GetVolumeInformationW(
                PCWSTR(root.as_ptr()),
                None,
                None,
                None,
                None,
                Some(&mut file_system),
            )
        }
        .is_ok()
            && terminated(&file_system).starts_with(&"ReFS\0".encode_utf16().collect::<Vec<_>>());
        let sharing = refs.then_some(
            "ReFS can share blocks between copies (block cloning); deleting a copy can free less than shown.",
        );
        if unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) } == DRIVE_REMOTE {
            return (
                Medium::Network,
                Some(
                    "Network drives may not report file identities, so hard links there can appear as copies.",
                ),
            );
        }
        (seek_penalty(&root).unwrap_or(Medium::Unknown), sharing)
    }

    /// Whether the volume's disk must seek, as Windows' drive optimizer asks.
    fn seek_penalty(root: &[u16]) -> Option<Medium> {
        let mut volume = [0u16; 64];
        unsafe { GetVolumeNameForVolumeMountPointW(PCWSTR(root.as_ptr()), &mut volume) }.ok()?;
        // `\\?\Volume{…}\` names the root folder; without the backslash, the volume.
        let mut device = terminated(&volume);
        device.pop();
        if device.last() == Some(&u16::from(b'\\')) {
            device.pop();
        }
        device.push(0);
        // No access rights are needed to query storage properties.
        let handle = unsafe {
            windows::Win32::Storage::FileSystem::CreateFileW(
                PCWSTR(device.as_ptr()),
                0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                Default::default(),
                None,
            )
        }
        .ok()
        .map(native::OwnedHandle)?;
        let query = STORAGE_PROPERTY_QUERY {
            PropertyId: StorageDeviceSeekPenaltyProperty,
            QueryType: PropertyStandardQuery,
            ..Default::default()
        };
        let mut descriptor = DEVICE_SEEK_PENALTY_DESCRIPTOR::default();
        let mut returned = 0u32;
        unsafe {
            DeviceIoControl(
                handle.0,
                IOCTL_STORAGE_QUERY_PROPERTY,
                Some((&query as *const STORAGE_PROPERTY_QUERY).cast()),
                size_of::<STORAGE_PROPERTY_QUERY>() as u32,
                Some((&mut descriptor as *mut DEVICE_SEEK_PENALTY_DESCRIPTOR).cast()),
                size_of::<DEVICE_SEEK_PENALTY_DESCRIPTOR>() as u32,
                Some(&mut returned),
                None,
            )
        }
        .ok()?;
        let answered = std::mem::offset_of!(DEVICE_SEEK_PENALTY_DESCRIPTOR, IncursSeekPenalty) + 1;
        (returned as usize >= answered).then_some(if descriptor.IncursSeekPenalty {
            Medium::Rotational
        } else {
            Medium::SolidState
        })
    }
}

#[cfg(unix)]
mod platform {
    use super::{super::FileIdentity, Failure, Known, Medium, Opened, Stamp};
    use std::{
        fs::{File, Metadata, OpenOptions},
        io,
        os::unix::fs::{MetadataExt, OpenOptionsExt},
        path::Path,
    };

    /// macOS 11 refuses a link anywhere in the path, not only at its end. The
    /// scan resolved the system's own links in the root's folders already.
    #[cfg(target_os = "macos")]
    const NO_LINKS: i32 = libc::O_NOFOLLOW_ANY;
    #[cfg(not(target_os = "macos"))]
    const NO_LINKS: i32 = libc::O_NOFOLLOW;

    #[cfg(target_os = "macos")]
    unsafe extern "C" {
        fn setiopolicy_np(iotype: i32, scope: i32, policy: i32) -> i32;
    }

    /// Reads on this thread neither download evicted iCloud files nor update
    /// access times. Both are per-thread I/O policies of macOS 10.15 and later.
    pub(super) fn prepare_reader_thread() {
        #[cfg(target_os = "macos")]
        {
            const IOPOL_TYPE_VFS_ATIME_UPDATES: i32 = 2;
            const IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES: i32 = 3;
            const IOPOL_SCOPE_THREAD: i32 = 1;
            const IOPOL_ATIME_UPDATES_OFF: i32 = 1;
            const IOPOL_MATERIALIZE_DATALESS_FILES_OFF: i32 = 1;
            // SAFETY: plain integer arguments; failure leaves the default policy.
            unsafe {
                setiopolicy_np(
                    IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES,
                    IOPOL_SCOPE_THREAD,
                    IOPOL_MATERIALIZE_DATALESS_FILES_OFF,
                );
                setiopolicy_np(
                    IOPOL_TYPE_VFS_ATIME_UPDATES,
                    IOPOL_SCOPE_THREAD,
                    IOPOL_ATIME_UPDATES_OFF,
                );
            }
        }
    }

    /// A read of an evicted file that the I/O policy refused to download.
    pub(super) fn is_placeholder(error: &io::Error) -> bool {
        cfg!(target_os = "macos") && error.raw_os_error() == Some(libc::EDEADLK)
    }

    /// The path now leads through a link, so it is no longer the scanned file.
    pub(super) fn is_link(error: &io::Error) -> bool {
        error.raw_os_error() == Some(libc::ELOOP)
    }

    fn nanoseconds(seconds: i64, nanoseconds: i64) -> i64 {
        seconds
            .saturating_mul(1_000_000_000)
            .saturating_add(nanoseconds)
    }

    fn stamp_of(metadata: &Metadata) -> Stamp {
        Stamp {
            size: metadata.len(),
            written: nanoseconds(metadata.mtime(), metadata.mtime_nsec()),
            changed: nanoseconds(metadata.ctime(), metadata.ctime_nsec()),
        }
    }

    pub(super) fn stamp(file: &File) -> io::Result<Stamp> {
        file.metadata().map(|metadata| stamp_of(&metadata))
    }

    pub(super) fn open(path: &Path, sequential: bool) -> Result<Opened, Failure> {
        // O_NONBLOCK: a file replaced by a FIFO must not block the open.
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(NO_LINKS | libc::O_NONBLOCK)
            .open(path)?;
        let metadata = file.metadata()?;
        if !metadata.file_type().is_file() {
            return Err(Failure::Skipped);
        }
        #[cfg(target_os = "macos")]
        {
            use std::os::{fd::AsRawFd, macos::fs::MetadataExt};
            if metadata.st_flags() & super::super::bulk::SF_DATALESS != 0 {
                return Err(Failure::Skipped);
            }
            // Sampling reads 32 KiB; read-ahead beyond it would be wasted.
            // SAFETY: a valid descriptor and an integer argument.
            unsafe { libc::fcntl(file.as_raw_fd(), libc::F_RDAHEAD, i32::from(sequential)) };
        }
        #[cfg(not(target_os = "macos"))]
        let _ = sequential;
        let mut identity = [0; 16];
        identity[..8].copy_from_slice(&metadata.ino().to_le_bytes());
        Ok(Opened {
            known: Known {
                identity: Some(FileIdentity(metadata.dev(), identity)),
                stamp: stamp_of(&metadata),
                links: metadata.nlink(),
                modified_ms: Some(
                    metadata
                        .mtime()
                        .saturating_mul(1000)
                        .saturating_add(metadata.mtime_nsec() / 1_000_000),
                ),
                private: private_size(&file),
            },
            file,
        })
    }

    /// Bytes deleting the file would free: APFS excludes blocks it shares
    /// with clones. Other filesystems do not report it.
    #[cfg(target_os = "macos")]
    fn private_size(file: &File) -> Option<u64> {
        use std::os::fd::AsRawFd;
        let mut request = libc::attrlist {
            bitmapcount: libc::ATTR_BIT_MAP_COUNT,
            reserved: 0,
            commonattr: libc::ATTR_CMN_RETURNED_ATTRS,
            volattr: 0,
            dirattr: 0,
            fileattr: 0,
            forkattr: libc::ATTR_CMNEXT_PRIVATESIZE,
        };
        // u32 length, five returned attribute groups, then the off_t.
        let mut buffer = [0u8; 4 + 20 + 8];
        // SAFETY: `request` and `buffer` are valid for the call and sized as passed.
        let status = unsafe {
            libc::fgetattrlist(
                file.as_raw_fd(),
                (&mut request as *mut libc::attrlist).cast(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                libc::FSOPT_ATTR_CMN_EXTENDED,
            )
        };
        if status != 0 {
            return None;
        }
        let returned_forks = u32::from_ne_bytes(buffer[20..24].try_into().ok()?);
        (returned_forks & libc::ATTR_CMNEXT_PRIVATESIZE != 0)
            .then(|| i64::from_ne_bytes(buffer[24..32].try_into().unwrap()))
            .and_then(|size| u64::try_from(size).ok())
    }

    #[cfg(not(target_os = "macos"))]
    fn private_size(_: &File) -> Option<u64> {
        None
    }

    #[cfg(target_os = "macos")]
    pub(super) fn volume(path: &Path) -> (Medium, Option<&'static str>) {
        use std::{ffi::CStr, os::unix::ffi::OsStrExt};
        let Ok(name) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
            return (Medium::Unknown, None);
        };
        // SAFETY: zeroed statfs is a valid output buffer for statfs(2).
        let mut info: libc::statfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statfs(name.as_ptr(), &mut info) } != 0 {
            return (Medium::Unknown, None);
        }
        if info.f_flags & libc::MNT_LOCAL as u32 == 0 {
            return (
                Medium::Network,
                Some(
                    "Network drives may not report file identities, so hard links there can appear as copies.",
                ),
            );
        }
        // SAFETY: the kernel NUL-terminates the device name.
        let device = unsafe { CStr::from_ptr(info.f_mntfromname.as_ptr()) };
        let medium = std::process::Command::new("/usr/sbin/diskutil")
            .args(["info", "-plist"])
            .arg(std::ffi::OsStr::from_bytes(device.to_bytes()))
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| super::solid_state(&String::from_utf8_lossy(&output.stdout)));
        let medium = match medium {
            Some(true) => Medium::SolidState,
            Some(false) => Medium::Rotational,
            None => Medium::Unknown,
        };
        (medium, None)
    }

    #[cfg(not(target_os = "macos"))]
    pub(super) fn volume(_: &Path) -> (Medium, Option<&'static str>) {
        (Medium::Unknown, None)
    }
}

/// The JSON report: every group with its copies' paths, oldest first.
pub fn export_json(
    path: &Path,
    nodes: &[ScanNode],
    groups: &[DuplicateGroup],
    summary: &DuplicateSummary,
) -> Result<(), String> {
    #[derive(Serialize)]
    struct Export<'a> {
        schema_version: u32,
        kind: &'static str,
        comparison: &'static str,
        status: &'static str,
        summary: &'a DuplicateSummary,
        groups: Groups<'a>,
    }
    struct Groups<'a>(&'a [ScanNode], &'a [DuplicateGroup]);
    impl Serialize for Groups<'_> {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            #[derive(Serialize)]
            struct Group<'a> {
                size: u64,
                blake3: &'a str,
                reclaimable: u64,
                copies: Vec<CopyOut>,
            }
            #[derive(Serialize)]
            struct CopyOut {
                id: usize,
                path: String,
                freeable: u64,
                links: u64,
                modified_ms: Option<i64>,
            }
            let nodes = self.0;
            serializer.collect_seq(self.1.iter().map(|group| {
                Group {
                    size: group.size,
                    blake3: &group.blake3,
                    reclaimable: group.reclaimable,
                    copies: group
                        .copies
                        .iter()
                        .map(|copy| CopyOut {
                            id: copy.id,
                            path: node_path(nodes, copy.id).to_string_lossy().into_owned(),
                            freeable: copy.freeable,
                            links: copy.links,
                            modified_ms: copy.modified_ms,
                        })
                        .collect(),
                }
            }))
        }
    }
    write_new_output_with(path, |file| {
        let mut writer = BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, &Export {
            schema_version: 1,
            kind: "duplicates",
            comparison: "Copies have equal size and BLAKE3 hash of their main data stream when read; each file was unchanged during its reads. Extra hard-link paths of one file are not copies. freeable: bytes on disk deleting that path frees (0 while other hard links remain; APFS excludes blocks shared with clones). reclaimable: freeable sum less the largest copy.",
            status: summary.status(),
            summary,
            groups: Groups(nodes, groups),
        })
        .map_err(|e| e.to_string())?;
        writer.flush().map_err(|e| e.to_string())
    })
}

/// The CSV report: one row per copy, with its group number.
pub fn export_csv(
    path: &Path,
    nodes: &[ScanNode],
    groups: &[DuplicateGroup],
    summary: &DuplicateSummary,
) -> Result<(), String> {
    write_new_output_with(path, |file| {
        let mut writer = csv::Writer::from_writer(file);
        let error = |e: csv::Error| e.to_string();
        writer
            .write_record([
                "group",
                "size_bytes",
                "blake3",
                "group_reclaimable_bytes",
                "path",
                "freeable_bytes",
                "hard_links",
                "modified_unix_ms",
                "status",
            ])
            .map_err(error)?;
        for (number, group) in groups.iter().enumerate() {
            for copy in &group.copies {
                writer
                    .write_record([
                        (number + 1).to_string(),
                        group.size.to_string(),
                        group.blake3.clone(),
                        group.reclaimable.to_string(),
                        csv_text(&node_path(nodes, copy.id).to_string_lossy()),
                        copy.freeable.to_string(),
                        copy.links.to_string(),
                        copy.modified_ms
                            .map(|ms| ms.to_string())
                            .unwrap_or_default(),
                        summary.status().into(),
                    ])
                    .map_err(error)?;
            }
        }
        writer.flush().map_err(|e| e.to_string())
    })
}

#[cfg(test)]
pub(crate) mod tests;
