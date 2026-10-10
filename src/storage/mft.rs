//! Administrator fast path for NTFS volumes. Reads the master file table (MFT)
//! in large sequential blocks and takes names, folders and sizes straight from
//! its file records, so no file is ever opened. Several threads read and parse
//! blocks at once. Sizes come from the clusters each data stream actually
//! occupies, which covers compressed, sparse and CompactOS (WOF) files exactly.
//!
//! The parser treats the volume as untrusted input: every offset and length is
//! bounds-checked, and a record whose sector checksums (fixups) disagree was
//! caught mid-write and is read again. Anything this path cannot handle falls
//! back to standard enumeration before a single node is emitted.

use super::{ScanNode, Skip, Worker, append_note, native};
use std::{
    fs::File,
    io,
    os::windows::{fs::FileExt, io::FromRawHandle},
    path::Path,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use windows::{
    Win32::{
        Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_ATTRIBUTE_COMPRESSED,
            FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_SPARSE_FILE, FILE_FLAG_NO_BUFFERING,
            FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, GetFileInformationByHandle,
            OPEN_EXISTING,
        },
        System::{
            IO::DeviceIoControl,
            Ioctl::{FSCTL_GET_NTFS_VOLUME_DATA, NTFS_VOLUME_DATA_BUFFER},
        },
    },
    core::PCWSTR,
};

/// Transient index memory: one `Info` per record plus names and links.
const MAX_INDEX_BYTES: u64 = 512 * 1024 * 1024;
/// Estimated index bytes per MFT record, checked before reading anything.
const INDEX_BYTES_PER_RECORD: u64 = 80;
/// Records 0–23 hold NTFS's own metadata ($MFT, $LogFile, $Extend, …);
/// they and everything below them are excluded, as in standard scans.
const RESERVED_RECORDS: u32 = 24;
/// Bytes read and parsed as one unit of work.
const CHUNK_BYTES: usize = 4 << 20;
/// Fixups protect each 512-byte block of a record, whatever the sector size.
const FIXUP_BLOCK: usize = 512;
/// Nodes per batch: large batches keep the UI queue from throttling a scan
/// that produces millions of entries in seconds.
const MFT_BATCH: usize = 8192;
const RECORD_NUMBER: u64 = 0x0000_ffff_ffff_ffff;
const WOF_TAG: u32 = 0x8000_0017;
/// IO_REPARSE_TAG_CLOUD and CLOUD_1 … CLOUD_F (OneDrive and other sync
/// providers) differ only in bits 12–15.
const CLOUD_TAG: u32 = 0x9000_001a;
const CLOUD_TAG_MASK: u32 = 0xffff_0fff;
/// Offline files (FILE_ATTRIBUTE_OFFLINE): content lives elsewhere.
const OFFLINE: u32 = 0x1000;
/// Holds a CompactOS file's compressed bytes; Windows reports the file's
/// original length as its size and counts this stream only as allocation.
const WOF_STREAM: &str = "WofCompressedData";

const IN_USE: u8 = 1;
const DIRECTORY: u8 = 2;
const WOF: u8 = 4;
const CLOUD: u8 = 8;

/// Returns true when this path handled the scan, including cancellation. A
/// failure before the tree is emitted falls back with a note, so node IDs
/// remain clean.
pub(super) fn scan_volume(root: &Path, worker: &mut Worker) -> bool {
    let Some(letter) = drive_letter(root) else {
        return false;
    };
    let prepared = Volume::open(letter).and_then(|volume| {
        let reference = file_reference(root)?;
        Ok((volume, reference))
    });
    let (volume, reference) = match prepared {
        Ok(prepared) => prepared,
        Err(error) => {
            worker.summary.notes.push(format!(
                "MFT fast path unavailable: {error}. Standard enumeration used; no elevation requested."
            ));
            return false;
        }
    };
    let root_record = (reference & RECORD_NUMBER) as u32;
    let table = match read_table(&volume.file, volume.geometry, &worker.cancel) {
        Ok(Some(table)) => table,
        Ok(None) => return true,
        Err(error) => {
            worker.summary.notes.push(format!(
                "MFT fast path unavailable: {error}. Standard enumeration used; no elevation requested."
            ));
            return false;
        }
    };
    let root_ok = table.infos.get(root_record as usize).is_some_and(|info| {
        info.flags & (IN_USE | DIRECTORY) == IN_USE | DIRECTORY
            && info.seq == (reference >> 48) as u16
    });
    if !root_ok {
        worker.summary.notes.push("MFT fast path unavailable: the scanned folder changed while the MFT was read. Standard enumeration used; no elevation requested.".into());
        return false;
    }
    worker.summary.method = "NTFS master file table".into();
    worker.summary.notes.push("Read from the NTFS master file table: names, folders and sizes come from file records, so no file was opened. Records caught mid-change were read again. NTFS reserved metadata records are excluded; restore points in System Volume Information are counted.".into());
    let Some(visited) = worker.visit(root, None) else {
        return true;
    };
    if visited.traverse {
        walk(&table, root_record, visited.id, root, worker);
    }
    true
}

/// Only whole drives: a folder scan would still read the entire MFT, which
/// costs more than enumerating a small folder.
fn drive_letter(path: &Path) -> Option<char> {
    let name = path.to_string_lossy();
    let name = name.strip_prefix(r"\\?\").unwrap_or(&name);
    let bytes = name.as_bytes();
    (bytes.len() == 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/'))
    .then(|| bytes[0] as char)
}

/// The scanned folder's file reference: record number and sequence number.
fn file_reference(path: &Path) -> Result<u64, String> {
    let handle = native::metadata_handle(path).map_err(|e| e.to_string())?;
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(handle.0, &mut info) }.map_err(|e| e.to_string())?;
    if native::skip_attributes(info.dwFileAttributes).is_some() {
        return Err("Reparse/cloud entry excluded".into());
    }
    Ok(((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64)
}

#[derive(Clone, Copy, Debug)]
struct Geometry {
    cluster: u64,
    sector: u64,
    record: usize,
    /// Byte offset of the MFT's first cluster on the volume.
    mft_start: u64,
    /// Bytes of the MFT that hold initialized records.
    valid: u64,
}

impl Geometry {
    fn checked(self) -> Result<Self, String> {
        let pow2 = |v: u64, lo: u64, hi: u64| v.is_power_of_two() && (lo..=hi).contains(&v);
        if pow2(self.cluster, 512, 2 << 20)
            && pow2(self.sector, 512, 4096)
            && self.sector <= self.cluster
            && pow2(self.record as u64, 1024, 4096)
            && self.valid >= self.record as u64 * RESERVED_RECORDS as u64
        {
            Ok(self)
        } else {
            Err(format!("unsupported NTFS geometry {self:?}"))
        }
    }

    fn records(self) -> Result<u32, String> {
        u32::try_from(self.valid / self.record as u64).map_err(|_| "MFT is too large".into())
    }
}

struct Volume {
    file: File,
    geometry: Geometry,
}

impl Volume {
    fn open(letter: char) -> Result<Self, String> {
        let name: Vec<u16> = format!(r"\\.\{letter}:")
            .encode_utf16()
            .chain(Some(0))
            .collect();
        // Unbuffered: gigabytes of MFT would otherwise evict the file cache.
        let handle = unsafe {
            CreateFileW(
                PCWSTR(name.as_ptr()),
                0x8000_0000, // GENERIC_READ
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_NO_BUFFERING,
                None,
            )
        }
        .map_err(|e| format!("volume read handle: {e}"))?;
        let file = unsafe { File::from_raw_handle(handle.0) };
        let mut data = NTFS_VOLUME_DATA_BUFFER::default();
        let mut returned = 0;
        unsafe {
            DeviceIoControl(
                handle,
                FSCTL_GET_NTFS_VOLUME_DATA,
                None,
                0,
                Some((&mut data as *mut NTFS_VOLUME_DATA_BUFFER).cast()),
                size_of::<NTFS_VOLUME_DATA_BUFFER>() as u32,
                Some(&mut returned),
                None,
            )
        }
        .map_err(|e| format!("NTFS volume data: {e}"))?;
        let geometry = Geometry {
            cluster: data.BytesPerCluster as u64,
            sector: data.BytesPerSector as u64,
            record: data.BytesPerFileRecordSegment as usize,
            mft_start: (data.MftStartLcn.max(0) as u64).saturating_mul(data.BytesPerCluster as u64),
            valid: data.MftValidDataLength.max(0) as u64,
        }
        .checked()?;
        Ok(Self { file, geometry })
    }
}

/// Positional reads. Offsets and lengths passed by this module are always
/// sector multiples, and buffers sector-aligned, as unbuffered I/O requires.
trait Disk: Sync {
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()>;
}

impl Disk for File {
    fn read_at(&self, mut offset: u64, mut buf: &mut [u8]) -> io::Result<()> {
        while !buf.is_empty() {
            let read = self.seek_read(buf, offset)?;
            if read == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            buf = &mut buf[read..];
            offset += read as u64;
        }
        Ok(())
    }
}

/// A zeroed buffer whose start is aligned for unbuffered reads.
struct Aligned {
    raw: Vec<u8>,
    start: usize,
    len: usize,
}

impl Aligned {
    const ALIGN: usize = 4096;

    fn new(len: usize) -> Self {
        let raw = vec![0u8; len + Self::ALIGN];
        let start = raw.as_ptr().align_offset(Self::ALIGN);
        Self { raw, start, len }
    }

    fn bytes(&mut self) -> &mut [u8] {
        &mut self.raw[self.start..self.start + self.len]
    }
}

/// A contiguous run of the MFT on disk.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Extent {
    /// Offset within the MFT.
    start: u64,
    len: u64,
    /// Offset on the volume.
    physical: u64,
}

/// Reads `out.len()` bytes of the MFT starting at `offset`, across extents.
fn read_mft(
    disk: &impl Disk,
    extents: &[Extent],
    g: Geometry,
    offset: u64,
    out: &mut [u8],
) -> io::Result<()> {
    let mut done = 0;
    while done < out.len() {
        let pos = offset + done as u64;
        let index = extents.partition_point(|e| e.start + e.len <= pos);
        let extent = extents
            .get(index)
            .filter(|e| e.start <= pos)
            .ok_or(io::ErrorKind::UnexpectedEof)?;
        let piece = ((extent.start + extent.len - pos) as usize).min(out.len() - done);
        let physical = extent.physical + (pos - extent.start);
        let aligned = physical & !(g.sector - 1);
        let end = (physical + piece as u64).next_multiple_of(g.sector);
        let target = &mut out[done..done + piece];
        if aligned == physical
            && end == physical + piece as u64
            && target.as_ptr().align_offset(g.sector as usize) == 0
        {
            disk.read_at(physical, target)?;
        } else {
            let mut buffer = Aligned::new((end - aligned) as usize);
            disk.read_at(aligned, buffer.bytes())?;
            let from = (physical - aligned) as usize;
            target.copy_from_slice(&buffer.bytes()[from..from + piece]);
        }
        done += piece;
    }
    Ok(())
}

/// What a scan keeps per MFT record.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
struct Info {
    logical: u64,
    allocated: u64,
    attributes: u32,
    seq: u16,
    flags: u8,
    /// Named data streams, excluding CompactOS's internal one.
    named: u8,
}

/// One name of a file in one folder. Hard links have several.
#[derive(Clone, Copy, Debug)]
struct Link {
    record: u32,
    parent: u32,
    parent_seq: u16,
    name_len: u16,
    name: u32,
}

/// Sizes and names stored in an extension record, added to its base record.
#[derive(Clone, Copy, Debug)]
struct Extension {
    base: u32,
    base_seq: u16,
    info: Info,
}

#[derive(Default)]
struct Parsed {
    links: Vec<Link>,
    names: String,
    extensions: Vec<Extension>,
    /// Records whose fixups or structure did not check out.
    torn: Vec<u32>,
}

struct Table {
    infos: Vec<Info>,
    links: Vec<Link>,
    names: String,
    /// `children[starts[r]..starts[r + 1]]` indexes the links in folder `r`.
    starts: Vec<u32>,
    children: Vec<u32>,
    /// In-use records whose folder was replaced while the MFT was read.
    orphans: u64,
    /// Records that stayed inconsistent after being read again.
    unreadable: u64,
}

impl Table {
    fn children(&self, record: u32) -> &[u32] {
        let r = record as usize;
        match (self.starts.get(r), self.starts.get(r + 1)) {
            (Some(&a), Some(&b)) => &self.children[a as usize..b as usize],
            _ => &[],
        }
    }

    fn name(&self, link: &Link) -> &str {
        &self.names[link.name as usize..link.name as usize + link.name_len as usize]
    }
}

/// Reads and parses the whole MFT. `Ok(None)` means cancelled.
fn read_table(disk: &impl Disk, g: Geometry, cancel: &AtomicBool) -> Result<Option<Table>, String> {
    let records = g.records()?;
    if records as u64 * INDEX_BYTES_PER_RECORD > MAX_INDEX_BYTES {
        return Err("MFT index exceeded its 512 MiB memory budget".into());
    }
    let extents = mft_extents(disk, g)?;
    let mut infos = vec![Info::default(); records as usize];
    let per_chunk = CHUNK_BYTES / g.record;
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get().clamp(2, 8));
    // Threads take chunks in order; each owns its chunk's slice of `infos`.
    let work = Mutex::new(infos.chunks_mut(per_chunk).enumerate());
    let results = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| -> io::Result<Vec<(usize, Parsed)>> {
                    let mut buffer = Aligned::new(CHUNK_BYTES);
                    let mut done = Vec::new();
                    loop {
                        if cancel.load(Ordering::Relaxed) {
                            return Ok(done);
                        }
                        let Some((index, slots)) = work.lock().unwrap().next() else {
                            return Ok(done);
                        };
                        let first = index * per_chunk;
                        let bytes = &mut buffer.bytes()[..slots.len() * g.record];
                        read_mft(disk, &extents, g, (first * g.record) as u64, bytes)?;
                        let mut parsed = Parsed::default();
                        for (i, (slot, record)) in slots
                            .iter_mut()
                            .zip(bytes.chunks_exact_mut(g.record))
                            .enumerate()
                        {
                            parse_record(record, (first + i) as u32, g.cluster, slot, &mut parsed);
                        }
                        done.push((index, parsed));
                    }
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|w| w.join().expect("MFT parser thread"))
            .collect::<io::Result<Vec<_>>>()
    })
    .map_err(|e| format!("MFT read: {e}"))?;
    if cancel.load(Ordering::Relaxed) {
        return Ok(None);
    }
    let mut chunks: Vec<_> = results.into_iter().flatten().collect();
    chunks.sort_unstable_by_key(|(index, _)| *index);
    let mut merged = Parsed::default();
    for (_, parsed) in chunks {
        merge(&mut merged, parsed);
    }
    // Records caught mid-write: read each again, alone.
    let torn = std::mem::take(&mut merged.torn);
    let mut unreadable = 0;
    for number in torn {
        let mut fixed = false;
        for _ in 0..2 {
            let mut buffer = Aligned::new(g.record);
            let offset = number as u64 * g.record as u64;
            if read_mft(disk, &extents, g, offset, buffer.bytes()).is_err() {
                continue;
            }
            let mut parsed = Parsed::default();
            let mut info = Info::default();
            parse_record(buffer.bytes(), number, g.cluster, &mut info, &mut parsed);
            if parsed.torn.is_empty() {
                infos[number as usize] = info;
                merge(&mut merged, parsed);
                fixed = true;
                break;
            }
        }
        unreadable += u64::from(!fixed);
    }
    for extension in &merged.extensions {
        if let Some(base) = infos.get_mut(extension.base as usize)
            && base.flags & IN_USE != 0
            && base.seq == extension.base_seq
        {
            base.logical = base.logical.saturating_add(extension.info.logical);
            base.allocated = base.allocated.saturating_add(extension.info.allocated);
            base.named = base.named.saturating_add(extension.info.named);
            base.flags |= extension.info.flags & (WOF | CLOUD);
        }
    }
    Ok(Some(index(infos, merged, unreadable)))
}

fn merge(into: &mut Parsed, from: Parsed) {
    let base = into.names.len() as u32;
    into.names.push_str(&from.names);
    into.links.extend(from.links.into_iter().map(|mut link| {
        link.name += base;
        link
    }));
    into.extensions.extend(from.extensions);
    into.torn.extend(from.torn);
}

/// Groups links by folder (a counting sort, so no per-folder vectors) and
/// drops links to unused records, NTFS metadata and replaced folders.
fn index(infos: Vec<Info>, parsed: Parsed, unreadable: u64) -> Table {
    let mut orphans = 0;
    let valid = |link: &Link| {
        let Some(info) = infos.get(link.record as usize) else {
            return Some(false);
        };
        if info.flags & IN_USE == 0 || link.record < RESERVED_RECORDS {
            return Some(false);
        }
        let parent = infos.get(link.parent as usize)?;
        (parent.flags & (IN_USE | DIRECTORY) == IN_USE | DIRECTORY && parent.seq == link.parent_seq)
            .then_some(true)
    };
    let mut keep = vec![false; parsed.links.len()];
    let mut counts = vec![0u32; infos.len() + 1];
    for (link, keep) in parsed.links.iter().zip(&mut keep) {
        match valid(link) {
            Some(true) => {
                *keep = true;
                counts[link.parent as usize + 1] += 1;
            }
            Some(false) => {}
            None => orphans += 1,
        }
    }
    for i in 1..counts.len() {
        counts[i] += counts[i - 1];
    }
    let starts = counts.clone();
    let mut children = vec![0u32; *counts.last().unwrap_or(&0) as usize];
    for (i, link) in parsed.links.iter().enumerate() {
        if keep[i] {
            let slot = &mut counts[link.parent as usize];
            children[*slot as usize] = i as u32;
            *slot += 1;
        }
    }
    Table {
        infos,
        links: parsed.links,
        names: parsed.names,
        starts,
        children,
        orphans,
        unreadable,
    }
}

/// The MFT's own location: the runs of record 0's unnamed data stream.
fn mft_extents(disk: &impl Disk, g: Geometry) -> Result<Vec<Extent>, String> {
    let span = (g.record as u64).next_multiple_of(g.cluster);
    let mut buffer = Aligned::new(span as usize);
    disk.read_at(g.mft_start, buffer.bytes())
        .map_err(|e| format!("MFT record 0: {e}"))?;
    let record = &mut buffer.bytes()[..g.record];
    if &record[..4] != b"FILE" || !apply_fixups(record) {
        return Err("MFT record 0 is not a valid file record".into());
    }
    let mut extents = Vec::new();
    for attribute in attributes(record).ok_or("MFT record 0 is malformed")? {
        let (kind, attr) = attribute;
        if kind == 0x20 {
            return Err("the MFT is split across attribute-list records".into());
        }
        if kind != 0x80 || attr[9] != 0 || attr[8] == 0 || u64_at(attr, 0x10) != Some(0) {
            continue;
        }
        let runs = u16_at(attr, 0x20).and_then(|at| attr.get(at as usize..));
        let mut vcn = 0u64;
        decode_runs(runs.ok_or("MFT runs are malformed")?, |lcn, clusters| {
            let lcn = lcn?;
            extents.push(Extent {
                start: vcn.checked_mul(g.cluster)?,
                len: clusters.checked_mul(g.cluster)?,
                physical: lcn.checked_mul(g.cluster)?,
            });
            vcn = vcn.checked_add(clusters)?;
            Some(())
        })
        .ok_or("MFT runs are malformed")?;
        break;
    }
    let covered = extents.last().map_or(0, |e| e.start + e.len);
    if covered < g.valid {
        return Err("the MFT's data runs do not cover its records".into());
    }
    Ok(extents)
}

/// Restores the bytes NTFS swapped out for per-block checksums. False when a
/// block's checksum differs: the record was being written while it was read.
fn apply_fixups(record: &mut [u8]) -> bool {
    let (Some(at), Some(count)) = (u16_at(record, 4), u16_at(record, 6)) else {
        return false;
    };
    let (at, count) = (at as usize, count as usize);
    if count != record.len() / FIXUP_BLOCK + 1 || at + count * 2 > record.len() {
        return false;
    }
    let check = [record[at], record[at + 1]];
    for i in 1..count {
        let end = i * FIXUP_BLOCK;
        if record[end - 2..end] != check {
            return false;
        }
        record[end - 2] = record[at + i * 2];
        record[end - 1] = record[at + i * 2 + 1];
    }
    true
}

/// Parses one record into `info` (base records) or `out` (names, extension
/// records, inconsistent records).
fn parse_record(record: &mut [u8], number: u32, cluster: u64, info: &mut Info, out: &mut Parsed) {
    *info = Info::default();
    let in_use = record.get(0x16).is_some_and(|flags| flags & 1 != 0);
    if &record[..4] != b"FILE" || !in_use {
        return;
    }
    if !apply_fixups(record) {
        out.torn.push(number);
        return;
    }
    let (Some(seq), Some(flags), Some(base)) = (
        u16_at(record, 0x10),
        u16_at(record, 0x16),
        u64_at(record, 0x20),
    ) else {
        out.torn.push(number);
        return;
    };
    let owner = if base == 0 {
        number
    } else {
        (base & RECORD_NUMBER) as u32
    };
    let names_before = (out.links.len(), out.names.len());
    let mut own = Info {
        seq,
        flags: IN_USE | if flags & 2 != 0 { DIRECTORY } else { 0 },
        ..Info::default()
    };
    let parsed = attributes(record).and_then(|list| {
        for (kind, attr) in list {
            add_attribute(kind, attr, owner, cluster, &mut own, out)?;
        }
        Some(())
    });
    if parsed.is_none() {
        // Keep nothing from a record that does not parse completely.
        out.links.truncate(names_before.0);
        out.names.truncate(names_before.1);
        out.torn.push(number);
        return;
    }
    if base == 0 {
        *info = own;
    } else {
        out.extensions.push(Extension {
            base: owner,
            base_seq: (base >> 48) as u16,
            info: own,
        });
    }
}

/// The attributes of a record with fixups applied, as (type, bytes). `None`
/// when an attribute runs past the record.
fn attributes(record: &[u8]) -> Option<Vec<(u32, &[u8])>> {
    let mut at = u16_at(record, 0x14)? as usize;
    let used = (u32_at(record, 0x18)? as usize).min(record.len());
    let mut list = Vec::new();
    loop {
        let kind = u32_at(record.get(..used)?, at)?;
        if kind == 0xffff_ffff {
            return Some(list);
        }
        let len = u32_at(record, at + 4)? as usize;
        if len < 0x18 || at + len > used {
            return None;
        }
        list.push((kind, &record[at..at + len]));
        at += len;
    }
}

fn add_attribute(
    kind: u32,
    attr: &[u8],
    owner: u32,
    cluster: u64,
    info: &mut Info,
    out: &mut Parsed,
) -> Option<()> {
    let non_resident = attr[8] != 0;
    let name_units = attr[9] as usize;
    let name_at = u16_at(attr, 0x0a)? as usize;
    let stream_name = attr.get(name_at..name_at + name_units * 2)?;
    let value = || {
        let len = u32_at(attr, 0x10)? as usize;
        let at = u16_at(attr, 0x14)? as usize;
        attr.get(at..at.checked_add(len)?)
    };
    match kind {
        // $STANDARD_INFORMATION: the attributes Windows reports.
        0x10 if !non_resident => info.attributes = u32_at(value()?, 0x20).unwrap_or(0),
        // $FILE_NAME: one name in one folder.
        0x30 if !non_resident => add_name(value()?, owner, out)?,
        // $DATA: a data stream.
        0x80 => {
            let wof_stream = name_units > 0 && utf16_eq(stream_name, WOF_STREAM);
            if non_resident {
                let runs = attr.get(u16_at(attr, 0x20)? as usize..)?;
                let mut clusters = 0u64;
                decode_runs(runs, |lcn, len| {
                    if lcn.is_some() {
                        clusters = clusters.checked_add(len)?;
                    }
                    Some(())
                })?;
                info.allocated = info
                    .allocated
                    .saturating_add(clusters.saturating_mul(cluster));
                // Only the first extent of a stream carries its size.
                if u64_at(attr, 0x10)? == 0 && !wof_stream {
                    info.logical = info.logical.saturating_add(u64_at(attr, 0x30)?);
                    info.named = info.named.saturating_add(u8::from(name_units > 0));
                }
            } else if !wof_stream {
                // Resident data lives inside the record: no clusters.
                info.logical = info.logical.saturating_add(value()?.len() as u64);
                info.named = info.named.saturating_add(u8::from(name_units > 0));
            }
        }
        // $REPARSE_POINT: CompactOS files are ordinary files to Windows.
        0xc0 if !non_resident => match u32_at(value()?, 0) {
            Some(WOF_TAG) => info.flags |= WOF,
            Some(tag) if tag & CLOUD_TAG_MASK == CLOUD_TAG => info.flags |= CLOUD,
            _ => {}
        },
        _ => {}
    }
    Some(())
}

fn add_name(value: &[u8], owner: u32, out: &mut Parsed) -> Option<()> {
    let parent = u64_at(value, 0)?;
    let units = *value.get(0x40)? as usize;
    let namespace = *value.get(0x41)?;
    // Namespace 2 is the extra 8.3 short name of a file that has a long one.
    if namespace == 2 {
        return Some(());
    }
    let raw = value.get(0x42..0x42 + units * 2)?;
    let units: Vec<u16> = raw
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| u16::from_le_bytes(*b))
        .collect();
    // Never let a name step outside its folder.
    if units.is_empty()
        || units.iter().any(|c| matches!(*c, 0 | 47 | 58 | 92))
        || units == [46]
        || units == [46, 46]
    {
        return Some(());
    }
    let start = out.names.len();
    out.names.extend(
        char::decode_utf16(units.iter().copied()).map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER)),
    );
    out.links.push(Link {
        record: owner,
        parent: (parent & RECORD_NUMBER).try_into().ok()?,
        parent_seq: (parent >> 48) as u16,
        name_len: (out.names.len() - start).try_into().ok()?,
        name: start.try_into().ok()?,
    });
    Some(())
}

/// Calls `run(lcn, clusters)` for each run of an NTFS mapping-pairs list;
/// `lcn` is `None` for sparse runs. `None` when the list is malformed.
fn decode_runs(runs: &[u8], mut run: impl FnMut(Option<u64>, u64) -> Option<()>) -> Option<()> {
    let mut at = 0;
    let mut lcn = 0i64;
    loop {
        let header = *runs.get(at)?;
        if header == 0 {
            return Some(());
        }
        let (len_bytes, offset_bytes) = ((header & 0x0f) as usize, (header >> 4) as usize);
        if len_bytes == 0 || len_bytes > 8 || offset_bytes > 8 {
            return None;
        }
        let len = runs.get(at + 1..at + 1 + len_bytes)?;
        let offset = runs.get(at + 1 + len_bytes..at + 1 + len_bytes + offset_bytes)?;
        let clusters = len.iter().rev().fold(0u64, |v, b| v << 8 | *b as u64);
        if offset_bytes == 0 {
            run(None, clusters)?;
        } else {
            // Signed, relative to the previous run.
            let mut delta = offset.iter().rev().fold(0i64, |v, b| v << 8 | *b as i64);
            let unused = 64 - offset_bytes * 8;
            if unused > 0 {
                delta = delta << unused >> unused;
            }
            lcn = lcn.checked_add(delta)?;
            run(Some(u64::try_from(lcn).ok()?), clusters)?;
        }
        at += 1 + len_bytes + offset_bytes;
    }
}

/// Emits the tree below `root` depth-first, parents before children.
fn walk(table: &Table, root: u32, root_id: usize, at: &Path, worker: &mut Worker) {
    let records = table.infos.len();
    let mut counted = vec![false; records];
    let mut entered = vec![false; records];
    entered[root as usize] = true;
    let mut stack = vec![(root, root_id)];
    while let Some((folder, id)) = stack.pop() {
        if worker.cancelled() {
            return;
        }
        for &index in table.children(folder) {
            let link = &table.links[index as usize];
            let info = &table.infos[link.record as usize];
            let is_dir = info.flags & DIRECTORY != 0;
            let mut node = ScanNode {
                id: worker.next_id,
                parent: Some(id),
                name: table.name(link).to_owned(),
                is_dir,
                logical: 0,
                allocated: 0,
                files: 0,
                children: Vec::new(),
                note: String::new(),
                incomplete: false,
            };
            let mut attributes = info.attributes;
            if info.flags & WOF != 0 {
                attributes &= !(FILE_ATTRIBUTE_REPARSE_POINT.0 | FILE_ATTRIBUTE_SPARSE_FILE.0);
            }
            let mut traverse = false;
            match exclusion(info, attributes) {
                Some(Skip::Reparse) => {
                    worker.summary.skipped_reparse += 1;
                    node.incomplete = true;
                    node.note = "Reparse point excluded; target not followed".into();
                }
                Some(Skip::Cloud) => {
                    worker.summary.skipped_cloud += 1;
                    node.incomplete = true;
                    node.note = "Cloud/offline placeholder excluded; no hydration requested".into();
                }
                None => {
                    node.files = u64::from(!is_dir);
                    node.logical = info.logical;
                    let first = !std::mem::replace(&mut counted[link.record as usize], true);
                    if first {
                        node.allocated = info.allocated;
                    } else {
                        worker.summary.hard_links += 1;
                        append_note(
                            &mut node.note,
                            "Hard-link alias: allocated bytes attributed to another path in this scan",
                        );
                    }
                    if info.named > 0 {
                        append_note(
                            &mut node.note,
                            &format!("Includes {} named data stream(s)", info.named),
                        );
                    }
                    if attributes & FILE_ATTRIBUTE_SPARSE_FILE.0 != 0 {
                        append_note(&mut node.note, "Sparse");
                    }
                    if attributes & FILE_ATTRIBUTE_COMPRESSED.0 != 0 {
                        append_note(&mut node.note, "NTFS compressed");
                    }
                    if is_dir {
                        // A folder reachable twice would be scanned twice.
                        traverse = !std::mem::replace(&mut entered[link.record as usize], true);
                        if !traverse {
                            node.incomplete = true;
                            worker.record_error(
                                at,
                                "Cycle in MFT directory references; branch excluded",
                            );
                        }
                    }
                }
            }
            let Some(child) = worker.push(node, 0, at) else {
                return;
            };
            if traverse {
                stack.push((link.record, child));
            }
            if worker.batch.len() >= MFT_BATCH && !worker.flush(true) {
                return;
            }
        }
    }
    if table.orphans > 0 {
        worker.summary.notes.push(format!(
            "{} MFT records were skipped because their folder changed during the read.",
            table.orphans
        ));
    }
    if table.unreadable > 0 {
        worker.record_error(
            at,
            &format!(
                "{} MFT records changed on every read; their entries are missing",
                table.unreadable
            ),
        );
    }
}

/// Like `native::skip_attributes`, from what the MFT stores. On disk, bit
/// 0x40000 (RECALL_ON_OPEN in the API) marks extended attributes, which most
/// of System32 has, so cloud placeholders are recognized by their reparse tag.
fn exclusion(info: &Info, attributes: u32) -> Option<Skip> {
    if info.flags & CLOUD != 0 || attributes & OFFLINE != 0 {
        Some(Skip::Cloud)
    } else if attributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
        Some(Skip::Reparse)
    } else {
        None
    }
}

fn utf16_eq(raw: &[u8], text: &str) -> bool {
    raw.as_chunks::<2>()
        .0
        .iter()
        .map(|b| u16::from_le_bytes(*b))
        .eq(text.encode_utf16())
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn u64_at(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes.get(at..at + 8)?.try_into().ok()?))
}

#[cfg(test)]
mod tests;
