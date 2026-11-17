use super::*;
use crate::storage::{ScanEvent, start_scan};
use std::{fs, path::PathBuf};

pub(crate) fn scan(root: &Path) -> Arc<Vec<ScanNode>> {
    let handle = start_scan(root.to_path_buf());
    let mut nodes = vec![];
    let mut summary = loop {
        match handle.receiver.recv().unwrap() {
            ScanEvent::Batch(batch) => crate::state::append_batch(&mut nodes, batch),
            ScanEvent::Finished(summary) => break summary,
        }
    };
    crate::state::finalize_scan(&mut nodes, &mut summary);
    assert_eq!(summary.errors, 0, "{:?}", summary.notes);
    Arc::new(nodes)
}

fn run(nodes: &Arc<Vec<ScanNode>>, min_size: u64) -> (Vec<DuplicateGroup>, DuplicateSummary) {
    let handle = find(nodes.clone(), 0, min_size, false, Arc::new(|| {}));
    let mut groups = vec![];
    loop {
        match handle.receiver.recv().unwrap() {
            DuplicateEvent::Groups(batch) => groups.extend(batch),
            DuplicateEvent::Progress(_) => {}
            DuplicateEvent::Finished(summary) => return (groups, summary),
        }
    }
}

/// Each group's copies as sorted file names, groups sorted.
fn names(nodes: &[ScanNode], groups: &[DuplicateGroup]) -> Vec<Vec<String>> {
    let mut out: Vec<Vec<String>> = groups
        .iter()
        .map(|group| {
            let mut names: Vec<_> = group
                .copies
                .iter()
                .map(|copy| nodes[copy.id].name.clone())
                .collect();
            names.sort();
            names
        })
        .collect();
    out.sort();
    out
}

/// Deterministic bytes without repeating patterns.
pub(crate) fn bytes(seed: u64, length: usize) -> Vec<u8> {
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..length)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        })
        .collect()
}

pub(crate) fn write(path: PathBuf, data: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, data).unwrap();
}

#[test]
fn copies_are_found_by_content_not_by_name_size_or_samples() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    let large = bytes(1, 300_000);
    // Same size, same first and last 16 KiB, one byte apart in the middle:
    // only a full read can tell them apart.
    let mut middle = large.clone();
    middle[150_000] ^= 1;
    let mut ending = large.clone();
    *ending.last_mut().unwrap() ^= 1;
    write(root.join("a/original.bin"), &large);
    write(root.join("b/copy.bin"), &large);
    write(root.join("c/deep/third copy.bin"), &large);
    write(root.join("a/middle differs.bin"), &middle);
    write(root.join("b/end differs.bin"), &ending);
    // Small files are compared completely by their samples.
    write(root.join("a/small.txt"), b"same small text");
    write(root.join("b/small copy.txt"), b"same small text");
    write(root.join("b/small other.txt"), b"diff small text");
    // A 20 KiB pair whose two samples together are the whole file.
    let medium = bytes(2, 20_000);
    let mut medium_other = medium.clone();
    medium_other[17_000] ^= 1;
    write(root.join("a/medium.dat"), &medium);
    write(root.join("b/medium.dat"), &medium);
    write(root.join("c/medium other.dat"), &medium_other);
    write(root.join("a/empty"), b"");
    write(root.join("b/empty"), b"");
    write(root.join("unique.bin"), &bytes(3, 300_000));
    let nodes = scan(root);
    let (groups, summary) = run(&nodes, 1);
    assert_eq!(
        names(&nodes, &groups),
        [
            vec!["copy.bin", "original.bin", "third copy.bin"],
            vec!["medium.dat", "medium.dat"],
            vec!["small copy.txt", "small.txt"],
        ]
    );
    assert_eq!(summary.status(), "complete", "{:?}", summary.notes);
    assert_eq!((summary.groups, summary.duplicates), (3, 4));
    assert_eq!(summary.below_minimum, 2, "empty files are never copies");
    let big = groups.iter().find(|g| g.size == 300_000).unwrap();
    assert_eq!(big.blake3, blake3::hash(&large).to_hex().as_str());
    let each = big.copies[0].freeable;
    assert!(each >= 300_000, "a copy occupies at least its size on disk");
    assert_eq!(big.reclaimable, 2 * each);
    assert_eq!(
        summary.reclaimable,
        groups.iter().map(|g| g.reclaimable).sum::<u64>()
    );
}

#[test]
fn minimum_size_limits_what_is_read() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    write(root.join("a.txt"), b"tiny");
    write(root.join("b.txt"), b"tiny");
    let data = bytes(4, 70_000);
    write(root.join("x/big"), &data);
    write(root.join("y/big"), &data);
    let nodes = scan(root);
    let (groups, summary) = run(&nodes, 64 * 1024);
    assert_eq!(names(&nodes, &groups), [vec!["big", "big"]]);
    assert_eq!((summary.below_minimum, summary.below_minimum_bytes), (2, 8));
    assert_eq!(summary.compared, 2);
}

#[cfg(unix)]
#[test]
fn hard_links_are_one_file_and_free_nothing() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    let data = bytes(5, 50_000);
    write(root.join("a/file"), &data);
    fs::create_dir_all(root.join("b")).unwrap();
    fs::hard_link(root.join("a/file"), root.join("b/link")).unwrap();
    let nodes = scan(root);
    let (groups, summary) = run(&nodes, 1);
    assert!(groups.is_empty(), "two names of one file are not copies");
    assert_eq!(summary.hard_links, 1);
    // A real copy beside them: the linked file frees nothing when deleted.
    write(root.join("c/copy"), &data);
    let nodes = scan(root);
    let (groups, summary) = run(&nodes, 1);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].copies.len(), 2);
    let linked = groups[0].copies.iter().find(|c| c.links == 2).unwrap();
    assert_eq!(linked.freeable, 0);
    let copy = groups[0].copies.iter().find(|c| c.links == 1).unwrap();
    assert!(copy.freeable > 0);
    assert_eq!(
        groups[0].reclaimable, copy.freeable,
        "keeping the linked file and deleting the copy frees the copy"
    );
    assert_eq!(summary.hard_links, 1);
}

#[test]
fn files_changed_since_the_scan_are_not_reported() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    let data = bytes(6, 40_000);
    write(root.join("a"), &data);
    write(root.join("b"), &data);
    write(root.join("c"), &data);
    let nodes = scan(root);
    fs::write(root.join("b"), bytes(6, 40_001)).unwrap();
    fs::remove_file(root.join("c")).unwrap();
    let (groups, summary) = run(&nodes, 1);
    assert!(groups.is_empty());
    assert_eq!(summary.changed, 2);
    assert_eq!(summary.status(), "partial");
}

#[cfg(unix)]
#[test]
fn a_file_replaced_by_a_link_is_not_followed() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    let data = bytes(7, 40_000);
    write(root.join("a"), &data);
    write(root.join("b"), &data);
    write(root.join("c"), &data);
    let nodes = scan(root);
    fs::remove_file(root.join("b")).unwrap();
    std::os::unix::fs::symlink(root.join("a"), root.join("b")).unwrap();
    let (groups, summary) = run(&nodes, 1);
    assert_eq!(names(&nodes, &groups), [vec!["a", "c"]]);
    assert_eq!(summary.changed, 1);
}

#[test]
fn cancellation_ends_with_a_partial_summary() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    let data = bytes(8, 4 << 20);
    for index in 0..8 {
        write(root.join(format!("{index}")), &data);
    }
    let nodes = scan(root);
    let handle = find(nodes, 0, 1, false, Arc::new(|| {}));
    handle.cancel.store(true, Ordering::Relaxed);
    let summary = loop {
        if let DuplicateEvent::Finished(summary) = handle
            .receiver
            .recv_timeout(Duration::from_secs(30))
            .unwrap()
        {
            break summary;
        }
    };
    assert!(summary.cancelled);
    assert_eq!(summary.status(), "cancelled_partial");
}

#[test]
fn exports_list_every_copy_and_never_overwrite() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("tree");
    let data = bytes(9, 40_000);
    write(root.join("x/one"), &data);
    write(root.join("y/two"), &data);
    let nodes = scan(&root);
    let (groups, summary) = run(&nodes, 1);
    let json = fixture.path().join("duplicates.json");
    export_json(&json, &nodes, &groups, &summary, None).unwrap();
    let report: serde_json::Value = serde_json::from_slice(&fs::read(&json).unwrap()).unwrap();
    assert_eq!(report["kind"], "duplicates");
    assert_eq!(report["status"], "complete");
    let copies = report["groups"][0]["copies"].as_array().unwrap();
    assert_eq!(copies.len(), 2);
    assert!(
        copies
            .iter()
            .any(|c| c["path"].as_str().unwrap().ends_with("one"))
    );
    assert_eq!(report["groups"][0]["blake3"].as_str().unwrap().len(), 64);
    let csv = fixture.path().join("duplicates.csv");
    export_csv(&csv, &nodes, &groups, &summary).unwrap();
    let text = fs::read_to_string(&csv).unwrap();
    assert_eq!(text.lines().count(), 3);
    assert!(text.lines().nth(1).unwrap().starts_with("1,40000,"));
    assert!(export_csv(&csv, &nodes, &groups, &summary).is_err());
    assert_eq!(fs::read_to_string(&csv).unwrap(), text);
}

#[test]
fn diskutil_solid_state_key_is_read() {
    let plist = "<dict>\n\t<key>SMARTStatus</key>\n\t<string>Verified</string>\n\t<key>SolidState</key>\n\t<true/>\n</dict>";
    assert_eq!(solid_state(plist), Some(true));
    assert_eq!(solid_state("<key>SolidState</key>  <false/>"), Some(false));
    assert_eq!(solid_state("<key>Internal</key><true/>"), None);
}

#[cfg(unix)]
#[test]
fn excluded_links_do_not_make_a_complete_comparison_partial() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    let data = bytes(10, 40_000);
    write(root.join("a/one"), &data);
    write(root.join("b/two"), &data);
    std::os::unix::fs::symlink(root.join("a/one"), root.join("b/link")).unwrap();
    let nodes = scan(root);
    assert!(nodes[0].incomplete, "the scan marks the excluded link");
    let (groups, summary) = run(&nodes, 1);
    assert_eq!(names(&nodes, &groups), [vec!["one", "two"]]);
    assert_eq!(summary.status(), "complete", "{:?}", summary.notes);
}

#[test]
fn only_errors_inside_a_folder_make_its_scan_partial() {
    let node = |id: usize, parent: Option<usize>| ScanNode {
        id,
        parent,
        name: id.to_string(),
        is_dir: true,
        logical: 0,
        allocated: 0,
        files: 0,
        children: vec![],
        note: String::new(),
        incomplete: false,
    };
    // 0 ─┬─ 1 ── 3
    //    └─ 2
    let nodes = vec![
        node(0, None),
        node(1, Some(0)),
        node(2, Some(0)),
        node(3, Some(1)),
    ];
    let summary = crate::storage::ScanSummary {
        incomplete_nodes: vec![0, 3],
        ..Default::default()
    };
    let partial = |root| crate::storage::subtree_partial(&nodes, &summary, root);
    assert!(partial(0) && partial(1) && partial(3));
    assert!(
        !partial(2),
        "an error elsewhere says nothing about this folder"
    );
    let stopped = crate::storage::ScanSummary {
        stopped_early: true,
        ..Default::default()
    };
    assert!(crate::storage::subtree_partial(&nodes, &stopped, 2));
}

#[test]
fn copies_are_compared_by_main_stream_when_the_scan_counted_named_streams() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    let data = bytes(14, 50_000);
    write(root.join("a/setup.exe"), &data);
    write(root.join("b/setup.exe"), &data);
    let mut nodes = Arc::unwrap_or_clone(scan(root));
    // An administrator scan on Windows adds each file's named streams, such
    // as a Zone.Identifier of its own length, to the size it reports.
    for (extra, name) in [(26, "a"), (87, "b")] {
        let folder = nodes.iter().position(|n| n.name == name).unwrap();
        let file = nodes[folder].children[0];
        nodes[file].logical += extra;
        nodes[file].note = format!("Includes 1 {}", crate::storage::NAMED_STREAMS);
    }
    let (groups, summary) = run(&Arc::new(nodes), 1);
    assert_eq!(groups.len(), 1, "{:?}", summary.notes);
    assert_eq!(groups[0].size, 50_000);
    assert_eq!(summary.changed, 0);
}
