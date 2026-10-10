//! The MFT parser against a synthetic NTFS volume built byte by byte, so the
//! fast path is verified without administrator rights or a real disk.

use super::*;
use crate::storage::{ScanSummary, node_path};
use std::{
    collections::HashSet,
    sync::{Arc, atomic::AtomicBool},
    time::Instant,
};

const CLUSTER: u64 = 4096;
const SECTOR: u64 = 512;
const RECORD: usize = 1024;
const RECORDS: u64 = 43;

const ROOT_SEQ: u16 = 5;
const REPARSE: u32 = 0x400;
const SPARSE: u32 = 0x200;
const COMPRESSED: u32 = 0x800;
const EXTENDED_ATTRIBUTES: u32 = 0x4_0000;

/// A volume image that, like unbuffered I/O, only accepts sector-aligned reads.
struct Image(Vec<u8>);

impl Disk for Image {
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        assert_eq!(offset % SECTOR, 0, "unaligned offset {offset}");
        assert_eq!(
            buf.len() as u64 % SECTOR,
            0,
            "unaligned length {}",
            buf.len()
        );
        assert_eq!(
            buf.as_ptr().align_offset(SECTOR as usize),
            0,
            "unaligned buffer"
        );
        let source = self
            .0
            .get(offset as usize..offset as usize + buf.len())
            .ok_or(io::ErrorKind::UnexpectedEof)?;
        buf.copy_from_slice(source);
        Ok(())
    }
}

fn geometry() -> Geometry {
    Geometry {
        cluster: CLUSTER,
        sector: SECTOR,
        record: RECORD,
        mft_start: 4 * CLUSTER,
        valid: RECORDS * RECORD as u64,
    }
}

/// Mapping pairs for `(absolute lcn or sparse, clusters)` runs.
fn runs(list: &[(Option<i64>, u64)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut previous = 0i64;
    for &(lcn, len) in list {
        let len_bytes: Vec<u8> =
            len.to_le_bytes()[..(8 - len.leading_zeros() as usize / 8).max(1)].to_vec();
        let offset_bytes = match lcn {
            None => Vec::new(),
            Some(lcn) => {
                let delta = lcn - previous;
                previous = lcn;
                let bytes = delta.to_le_bytes();
                let mut n = 8;
                // Shortest two's-complement form that keeps the sign.
                while n > 1
                    && ((bytes[n - 1] == 0 && bytes[n - 2] < 0x80)
                        || (bytes[n - 1] == 0xff && bytes[n - 2] >= 0x80))
                {
                    n -= 1;
                }
                bytes[..n].to_vec()
            }
        };
        out.push(len_bytes.len() as u8 | (offset_bytes.len() as u8) << 4);
        out.extend(len_bytes);
        out.extend(offset_bytes);
    }
    out.push(0);
    out
}

fn utf16(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

struct Rec {
    bytes: Vec<u8>,
    at: usize,
}

fn rec(seq: u16, dir: bool) -> Rec {
    rec_with(seq, dir, true, 0)
}

fn rec_with(seq: u16, dir: bool, in_use: bool, base: u64) -> Rec {
    let mut bytes = vec![0u8; RECORD];
    bytes[..4].copy_from_slice(b"FILE");
    bytes[4..6].copy_from_slice(&0x30u16.to_le_bytes());
    bytes[6..8].copy_from_slice(&3u16.to_le_bytes());
    bytes[0x10..0x12].copy_from_slice(&seq.to_le_bytes());
    bytes[0x14..0x16].copy_from_slice(&0x38u16.to_le_bytes());
    bytes[0x16] = u8::from(in_use) | u8::from(dir) << 1;
    bytes[0x20..0x28].copy_from_slice(&base.to_le_bytes());
    Rec { bytes, at: 0x38 }
}

impl Rec {
    fn attribute(
        mut self,
        kind: u32,
        non_resident: bool,
        name: &str,
        header: usize,
        body: &[u8],
        fill: impl FnOnce(&mut [u8], usize),
    ) -> Self {
        let name = utf16(name);
        let body_at = (header + name.len()).next_multiple_of(8);
        let len = (body_at + body.len()).next_multiple_of(8);
        let a = &mut self.bytes[self.at..self.at + len];
        a[..4].copy_from_slice(&kind.to_le_bytes());
        a[4..8].copy_from_slice(&(len as u32).to_le_bytes());
        a[8] = u8::from(non_resident);
        a[9] = (name.len() / 2) as u8;
        a[0x0a..0x0c].copy_from_slice(&(header as u16).to_le_bytes());
        a[header..header + name.len()].copy_from_slice(&name);
        a[body_at..body_at + body.len()].copy_from_slice(body);
        fill(a, body_at);
        self.at += len;
        self
    }

    fn resident(self, kind: u32, name: &str, value: &[u8]) -> Self {
        let len = value.len() as u32;
        self.attribute(kind, false, name, 0x18, value, |a, at| {
            a[0x10..0x14].copy_from_slice(&len.to_le_bytes());
            a[0x14..0x16].copy_from_slice(&(at as u16).to_le_bytes());
        })
    }

    fn data(self, name: &str, start_vcn: u64, size: u64, list: &[(Option<i64>, u64)]) -> Self {
        self.attribute(0x80, true, name, 0x40, &runs(list), |a, at| {
            a[0x10..0x18].copy_from_slice(&start_vcn.to_le_bytes());
            a[0x20..0x22].copy_from_slice(&(at as u16).to_le_bytes());
            a[0x30..0x38].copy_from_slice(&size.to_le_bytes());
        })
    }

    fn info(self, attributes: u32) -> Self {
        let mut value = vec![0u8; 0x48];
        value[0x20..0x24].copy_from_slice(&attributes.to_le_bytes());
        self.resident(0x10, "", &value)
    }

    fn name(self, parent: u32, parent_seq: u16, name: &str, namespace: u8) -> Self {
        let units = utf16(name);
        let mut value = vec![0u8; 0x42];
        value[..8].copy_from_slice(&(parent as u64 | (parent_seq as u64) << 48).to_le_bytes());
        value[0x40] = (units.len() / 2) as u8;
        value[0x41] = namespace;
        value.extend(units);
        self.resident(0x30, "", &value)
    }

    fn reparse(self, tag: u32) -> Self {
        self.resident(0xc0, "", &[tag.to_le_bytes(), [0; 4]].concat())
    }

    /// Ends the attribute list and protects each block with a fixup, as NTFS
    /// does on disk.
    fn finish(mut self) -> Vec<u8> {
        let end = self.at;
        self.bytes[end..end + 4].copy_from_slice(&0xffff_ffffu32.to_le_bytes());
        self.bytes[0x18..0x1c].copy_from_slice(&((end + 8) as u32).to_le_bytes());
        self.bytes[0x30..0x32].copy_from_slice(&7u16.to_le_bytes());
        for i in 1..3 {
            let tail = i * FIXUP_BLOCK - 2;
            let original = [self.bytes[tail], self.bytes[tail + 1]];
            self.bytes[0x30 + i * 2..0x32 + i * 2].copy_from_slice(&original);
            self.bytes[tail..tail + 2].copy_from_slice(&7u16.to_le_bytes());
        }
        self.bytes
    }
}

/// The MFT occupies clusters 4–6 and 10–17, so its records span two extents.
fn image(records: Vec<(u32, Vec<u8>)>) -> Image {
    let mut disk = vec![0u8; 64 * CLUSTER as usize];
    for (number, bytes) in records {
        let offset = number as u64 * RECORD as u64;
        let physical = if offset < 3 * CLUSTER {
            4 * CLUSTER + offset
        } else {
            10 * CLUSTER + offset - 3 * CLUSTER
        } as usize;
        disk[physical..physical + RECORD].copy_from_slice(&bytes);
    }
    Image(disk)
}

fn volume() -> Image {
    let mut torn = rec(1, false).name(5, ROOT_SEQ, "torn", 1).finish();
    torn[FIXUP_BLOCK - 2] ^= 0xff; // caught mid-write, on every read
    image(vec![
        (
            0,
            rec(1, false)
                .data(
                    "",
                    0,
                    RECORDS * RECORD as u64,
                    &[(Some(4), 3), (Some(10), 8)],
                )
                .finish(),
        ),
        (5, rec(ROOT_SEQ, true).name(5, ROOT_SEQ, ".", 3).finish()),
        (11, rec(11, true).name(5, ROOT_SEQ, "$Extend", 3).finish()),
        (
            24,
            rec(1, true).info(0).name(5, ROOT_SEQ, "Docs", 3).finish(),
        ),
        (
            25,
            rec(1, false)
                .info(0)
                .name(24, 1, "a.txt", 3)
                .resident(0x80, "", &[1; 100])
                .finish(),
        ),
        (
            26,
            rec(1, false)
                .info(0)
                .name(5, ROOT_SEQ, "LONGNA~1.BIN", 2)
                .name(5, ROOT_SEQ, "LongName.bin", 1)
                .data("", 0, 10_000, &[(Some(30), 3)])
                .finish(),
        ),
        (
            27,
            rec(1, false)
                .info(0)
                .name(5, ROOT_SEQ, "h1", 1)
                .name(24, 1, "h2", 1)
                .data("", 0, 5_000, &[(Some(40), 2)])
                .finish(),
        ),
        (
            28,
            rec(1, false)
                .info(REPARSE | SPARSE)
                .name(5, ROOT_SEQ, "w.dll", 3)
                .reparse(WOF_TAG)
                .data("", 0, 50_000, &[(None, 13)])
                .data(WOF_STREAM, 0, 9_000, &[(Some(45), 3)])
                .finish(),
        ),
        (
            29,
            rec(1, true)
                .info(REPARSE)
                .name(5, ROOT_SEQ, "link", 3)
                .reparse(0xa000_0003)
                .finish(),
        ),
        (30, rec(1, false).info(0).name(11, 11, "hidden", 3).finish()),
        (
            31,
            rec(1, false).info(0).name(24, 1, "frag.bin", 3).finish(),
        ),
        (
            32,
            rec_with(1, false, true, 31 | 1 << 48)
                .data("", 0, 8_000, &[(Some(50), 2)])
                .finish(),
        ),
        (
            33,
            rec_with(1, false, false, 0)
                .name(5, ROOT_SEQ, "ghost", 3)
                .finish(),
        ),
        (34, torn),
        (35, rec(1, false).info(0).name(40, 1, "orphan", 3).finish()),
        (
            36,
            rec(1, false)
                .info(0)
                .name(5, ROOT_SEQ, "zone.txt", 3)
                .resident(0x80, "", &[1; 10])
                .resident(0x80, "Zone.Identifier", &[1; 26])
                .finish(),
        ),
        (
            37,
            rec(1, false)
                .info(COMPRESSED)
                .name(5, ROOT_SEQ, "c.bin", 3)
                .data("", 0, 65_536, &[(Some(52), 4), (None, 12)])
                .finish(),
        ),
        (
            38,
            rec(1, false)
                .info(REPARSE)
                .name(5, ROOT_SEQ, "cloud.docx", 3)
                .reparse(0x9000_101a)
                .data("", 0, 4_096, &[(Some(57), 1)])
                .finish(),
        ),
        (39, rec(1, true).info(0).name(24, 2, "Stale", 3).finish()),
        // On disk, 0x40000 marks extended attributes, as on most of System32;
        // it is not the API's RECALL_ON_OPEN cloud flag.
        (
            41,
            rec(1, true)
                .info(EXTENDED_ATTRIBUTES)
                .name(5, ROOT_SEQ, "System32", 3)
                .finish(),
        ),
        (
            42,
            rec(1, false)
                .info(EXTENDED_ATTRIBUTES)
                .name(41, 1, "k.dll", 3)
                .data("", 0, 4_000, &[(Some(58), 1)])
                .finish(),
        ),
    ])
}

fn worker() -> Worker {
    let (sender, _) = crossbeam_channel::bounded(9);
    Worker {
        sender,
        cancel: Arc::new(AtomicBool::new(false)),
        summary: ScanSummary::default(),
        seen: HashSet::new(),
        batch: Vec::new(),
        next_id: 0,
        last_flush: Instant::now(),
        retained_bytes: 0,
        stopped: false,
    }
}

fn scan(disk: &Image) -> (Vec<ScanNode>, ScanSummary) {
    let table = read_table(disk, geometry(), &AtomicBool::new(false))
        .unwrap()
        .unwrap();
    let mut worker = worker();
    let root = ScanNode {
        id: 0,
        parent: None,
        name: "C:\\".into(),
        is_dir: true,
        logical: 0,
        allocated: 0,
        files: 0,
        children: Vec::new(),
        note: String::new(),
        incomplete: false,
    };
    worker.push(root, 0, Path::new("C:\\")).unwrap();
    walk(&table, 5, 0, Path::new("C:\\"), &mut worker);
    (std::mem::take(&mut worker.batch), worker.summary)
}

#[test]
fn synthetic_volume_yields_names_folders_and_exact_sizes() {
    let (nodes, summary) = scan(&volume());
    let find = |name: &str| {
        nodes
            .iter()
            .find(|n| n.name == name)
            .unwrap_or_else(|| panic!("{name} missing"))
    };
    let names = |parent: usize| {
        let mut list: Vec<&str> = nodes
            .iter()
            .filter(|n| n.parent == Some(parent))
            .map(|n| n.name.as_str())
            .collect();
        list.sort_unstable();
        list
    };
    // Reserved records, unused records, torn records and children of replaced
    // folders never appear; the 8.3 short name is not a second entry.
    assert_eq!(
        names(0),
        [
            "Docs",
            "LongName.bin",
            "System32",
            "c.bin",
            "cloud.docx",
            "h1",
            "link",
            "w.dll",
            "zone.txt"
        ]
    );
    assert_eq!(names(find("Docs").id), ["a.txt", "frag.bin", "h2"]);
    assert_eq!(names(find("System32").id), ["k.dll"]);
    assert_eq!(find("k.dll").allocated, CLUSTER);
    for node in &nodes {
        assert!(node.parent.is_none_or(|p| p < node.id), "parents first");
    }
    assert_eq!(node_path(&nodes, find("h2").id), Path::new("C:\\Docs\\h2"));

    let sizes = |name: &str| (find(name).logical, find(name).allocated);
    // Resident data lives inside the record and occupies no clusters.
    assert_eq!(sizes("a.txt"), (100, 0));
    assert_eq!(sizes("LongName.bin"), (10_000, 3 * CLUSTER));
    // CompactOS: original length as size, compressed stream as allocation.
    assert_eq!(sizes("w.dll"), (50_000, 3 * CLUSTER));
    assert!(find("w.dll").note.is_empty(), "{}", find("w.dll").note);
    // Data stored in an extension record belongs to its base record.
    assert_eq!(sizes("frag.bin"), (8_000, 2 * CLUSTER));
    assert_eq!(sizes("zone.txt"), (36, 0));
    assert!(
        find("zone.txt")
            .note
            .contains("Includes 1 named data stream(s)")
    );
    // Compressed: only clusters that hold data count.
    assert_eq!(sizes("c.bin"), (65_536, 4 * CLUSTER));
    assert!(find("c.bin").note.contains("NTFS compressed"));

    // A hard link counts its clusters once, its size on every path.
    assert_eq!(find("h1").logical, 5_000);
    assert_eq!(find("h2").logical, 5_000);
    assert_eq!(find("h1").allocated + find("h2").allocated, 2 * CLUSTER);
    assert_eq!(summary.hard_links, 1);

    assert!(find("link").incomplete && find("link").is_dir);
    assert_eq!(sizes("link"), (0, 0));
    assert_eq!(summary.skipped_reparse, 1);
    assert!(find("cloud.docx").incomplete);
    assert_eq!(sizes("cloud.docx"), (0, 0));
    assert_eq!(summary.skipped_cloud, 1);

    // One record stayed torn on every read; two had replaced folders.
    assert_eq!(summary.errors, 1, "{:?}", summary.notes);
    assert!(
        summary
            .notes
            .iter()
            .any(|n| n.contains("1 MFT records changed on every read"))
    );
    assert!(
        summary
            .notes
            .iter()
            .any(|n| n.starts_with("2 MFT records were skipped"))
    );
}

#[test]
fn a_record_fixed_on_the_second_read_is_kept() {
    struct Flaky(Image, std::sync::atomic::AtomicUsize);
    impl Disk for Flaky {
        fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
            self.0.read_at(offset, buf)?;
            // The first read of the MFT's second extent sees a torn record 26.
            if offset == 10 * CLUSTER && self.1.fetch_add(1, Ordering::Relaxed) == 0 {
                buf[(26 * RECORD) - 3 * CLUSTER as usize + FIXUP_BLOCK - 2] ^= 0xff;
            }
            Ok(())
        }
    }
    let disk = Flaky(volume(), Default::default());
    let table = read_table(&disk, geometry(), &AtomicBool::new(false))
        .unwrap()
        .unwrap();
    assert_eq!(disk.1.load(Ordering::Relaxed), 1, "the torn read happened");
    assert_eq!(table.infos[26].logical, 10_000);
    assert_eq!(table.unreadable, 1, "only the always-torn record 34");
}

#[test]
fn malformed_input_is_rejected_without_panicking() {
    // Fixups: a mismatched block check fails; the original bytes come back.
    let mut record = rec(1, false).name(5, ROOT_SEQ, "x", 3).finish();
    let mut copy = record.clone();
    assert!(apply_fixups(&mut copy));
    assert_eq!(&copy[FIXUP_BLOCK - 2..FIXUP_BLOCK], &[0, 0]);
    record[2 * FIXUP_BLOCK - 1] ^= 1;
    assert!(!apply_fixups(&mut record));

    // Runs: relative, signed offsets; sparse runs; truncation fails.
    let encoded = runs(&[
        (Some(100), 3),
        (None, 2),
        (Some(40), 1),
        (Some(70_000), 300),
    ]);
    let mut seen = Vec::new();
    decode_runs(&encoded, |lcn, len| {
        seen.push((lcn, len));
        Some(())
    })
    .unwrap();
    assert_eq!(
        seen,
        [
            (Some(100), 3),
            (None, 2),
            (Some(40), 1),
            (Some(70_000), 300)
        ]
    );
    assert!(decode_runs(&encoded[..encoded.len() - 2], |_, _| Some(())).is_none());
    assert!(decode_runs(&[0x09, 1], |_, _| Some(())).is_none());

    // Attributes that run past the record make it unreadable, not a panic.
    let mut broken = rec(1, false).name(5, ROOT_SEQ, "x", 3).finish();
    broken[0x38 + 4..0x38 + 8].copy_from_slice(&4000u32.to_le_bytes());
    let mut parsed = Parsed::default();
    parse_record(&mut broken, 40, CLUSTER, &mut Info::default(), &mut parsed);
    assert_eq!(parsed.torn, [40]);
    assert!(parsed.links.is_empty());

    // Every truncation and byte flip of a valid record is handled.
    let valid = rec(1, false)
        .info(0)
        .name(5, ROOT_SEQ, "Grüße.txt", 3)
        .data("", 0, 10, &[(Some(3), 1)])
        .finish();
    for i in 0..RECORD {
        let mut flipped = valid.clone();
        flipped[i] ^= 0xa5;
        parse_record(
            &mut flipped,
            41,
            CLUSTER,
            &mut Info::default(),
            &mut Parsed::default(),
        );
    }

    // A volume whose MFT runs do not cover its records falls back.
    let mut short = geometry();
    short.valid = 64 * RECORD as u64;
    assert!(read_table(&volume(), short, &AtomicBool::new(false)).is_err());
    assert!(
        Geometry {
            record: 1000,
            ..geometry()
        }
        .checked()
        .is_err()
    );
}

#[test]
fn only_drive_roots_use_the_mft() {
    assert_eq!(drive_letter(Path::new(r"C:\")), Some('C'));
    assert_eq!(drive_letter(Path::new(r"\\?\D:\")), Some('D'));
    assert_eq!(drive_letter(Path::new(r"C:\Users")), None);
    assert_eq!(drive_letter(Path::new(r"\\server\share\")), None);
}

#[test]
fn cancelling_stops_reading() {
    let cancel = AtomicBool::new(true);
    assert!(
        read_table(&volume(), geometry(), &cancel)
            .unwrap()
            .is_none()
    );
}
