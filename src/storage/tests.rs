use super::*;
use std::time::Duration;

fn collect_scan(path: &Path) -> (Vec<ScanNode>, ScanSummary) {
    let handle = start_scan(path.into());
    let mut nodes = Vec::new();
    loop {
        match handle
            .receiver
            .recv_timeout(Duration::from_secs(30))
            .expect("scan completed within 30s")
        {
            ScanEvent::Batch(batch) => {
                for node in batch {
                    assert_eq!(node.id, nodes.len(), "stable append-only IDs");
                    assert!(
                        node.parent.is_none_or(|parent| parent < node.id),
                        "parent arrives before child"
                    );
                    nodes.push(node);
                }
            }
            ScanEvent::Finished(summary) => return (nodes, summary),
        }
    }
}

#[test]
fn scan_preserves_unicode_empty_directories_and_unique_hardlink_allocation() {
    let fixture = tempfile::tempdir().unwrap();
    fs::create_dir(fixture.path().join("empty")).unwrap();
    fs::create_dir(fixture.path().join("日本語 – Grüße")).unwrap();
    let original = fixture.path().join("日本語 – Grüße").join("Daten.bin");
    fs::write(&original, vec![42u8; 16_384]).unwrap();
    fs::hard_link(&original, fixture.path().join("alias.bin")).unwrap();
    fs::write(fixture.path().join("zero.txt"), []).unwrap();
    let (nodes, summary) = collect_scan(fixture.path());
    assert_eq!(summary.errors, 0, "{:?}", summary.notes);
    assert_eq!(summary.hard_links, 1);
    assert_eq!(nodes.len(), 6);
    assert!(nodes.iter().any(|n| n.name == "日本語 – Grüße" && n.is_dir));
    assert!(
        nodes
            .iter()
            .any(|n| n.name == "empty" && n.is_dir && n.logical == 0)
    );
    assert_eq!(nodes.iter().map(|n| n.files).sum::<u64>(), 3);
    assert_eq!(nodes.iter().map(|n| n.logical).sum::<u64>(), 32_768);
    let allocated = nodes.iter().map(|n| n.allocated).sum::<u64>();
    assert!(
        allocated > 0 && allocated <= 32_768,
        "allocated={allocated}"
    );
    let aliases: Vec<_> = nodes.iter().filter(|n| n.logical == 16_384).collect();
    assert_eq!(aliases.iter().filter(|n| n.allocated == 0).count(), 1);
    assert!(aliases.iter().any(|n| n.note.contains("Hard-link alias")));
}

#[test]
fn hardlink_count_changes_never_double_count_physical_bytes() {
    for add_link_after_first_visit in [true, false] {
        let fixture = tempfile::tempdir().unwrap();
        let original = fixture.path().join("original.bin");
        let alias = fixture.path().join("alias.bin");
        let contents = vec![42u8; 16_384];
        fs::write(&original, &contents).unwrap();
        if !add_link_after_first_visit {
            fs::hard_link(&original, &alias).unwrap();
        }
        let (sender, _receiver) = bounded(QUEUE_CAPACITY);
        let mut worker = Worker {
            sender,
            cancel: Arc::new(AtomicBool::new(false)),
            summary: ScanSummary::default(),
            seen: HashSet::new(),
            batch: Vec::new(),
            next_id: 0,
            last_flush: Instant::now(),
            retained_bytes: 0,
            stopped: false,
        };
        worker.visit(original.clone(), None).unwrap();
        let allocated = worker.batch[0].allocated;
        assert!(allocated > 0, "fixture must own physical clusters");
        if add_link_after_first_visit {
            fs::hard_link(&original, &alias).unwrap();
        } else {
            fs::remove_file(&original).unwrap();
        }
        worker.visit(alias.clone(), None).unwrap();
        assert_eq!(worker.summary.errors, 0);
        assert_eq!(worker.batch[1].logical, 16_384);
        assert_eq!(
            worker.batch[1].allocated,
            0,
            "link-count transition {} must not count the same file twice",
            if add_link_after_first_visit {
                "1→2"
            } else {
                "2→1"
            }
        );
        assert_eq!(
            worker.batch.iter().map(|node| node.allocated).sum::<u64>(),
            allocated
        );
        assert_eq!(worker.summary.hard_links, 1);
        assert_eq!(fs::read(alias).unwrap(), contents);
    }
}

#[test]
fn missing_root_reports_error_instead_of_empty_success() {
    let fixture = tempfile::tempdir().unwrap();
    let (nodes, summary) = collect_scan(&fixture.path().join("missing"));
    assert_eq!(summary.errors, 1);
    assert_eq!(nodes.len(), 1);
    assert!(nodes[0].note.contains("unavailable"));
    assert!(nodes[0].incomplete);
    assert_eq!(export_status(Some(&summary)), "partial");
}

#[test]
fn cancellation_cannot_block_on_full_progress_queue() {
    // Exercise the actual queue producer used by scans, without relying on disk
    // timing or creating thousands of files simply to make the UI fall behind.
    let (sender, receiver) = bounded(QUEUE_CAPACITY);
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let join = std::thread::spawn(move || {
        let mut worker = Worker {
            sender,
            cancel: worker_cancel,
            summary: ScanSummary::default(),
            seen: HashSet::new(),
            batch: Vec::new(),
            next_id: 0,
            last_flush: Instant::now(),
            retained_bytes: 0,
            stopped: false,
        };
        for index in 0..100 {
            worker.batch.push(ScanNode {
                id: index,
                parent: None,
                path: PathBuf::new(),
                name: String::new(),
                is_dir: false,
                logical: 1,
                allocated: 1,
                files: 1,
                children: vec![],
                note: String::new(),
                incomplete: false,
            });
            if !worker.flush(true) {
                break;
            }
        }
        worker.summary.cancelled = worker.cancelled();
        worker
            .sender
            .try_send(ScanEvent::Finished(worker.summary))
            .unwrap();
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    while receiver.len() < QUEUE_CAPACITY - 1 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(receiver.len(), QUEUE_CAPACITY - 1);
    cancel.store(true, Ordering::Relaxed);
    let deadline = Instant::now() + Duration::from_secs(2);
    while !join.is_finished() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        join.is_finished(),
        "cancellation must release a saturated producer"
    );
    join.join().unwrap();
    assert_eq!(receiver.len(), QUEUE_CAPACITY);
    assert!(
        receiver
            .try_iter()
            .any(|event| matches!(event, ScanEvent::Finished(s) if s.cancelled))
    );
}

#[test]
fn exports_preserve_values_and_partial_state() {
    let fixture = tempfile::tempdir().unwrap();
    fs::write(
        fixture.path().join("ä,quoted\".txt".replace('"', "'")),
        b"1234567",
    )
    .unwrap();
    let (mut nodes, mut summary) = collect_scan(fixture.path());
    summary.cancelled = true;
    nodes[1].note = "=HYPERLINK(\"https://example.invalid\")".into();
    nodes[1].incomplete = true;
    let json = fixture.path().join("result.json");
    let csv = fixture.path().join("result.csv");
    export_json(&json, &nodes, Some(&summary)).unwrap();
    export_csv(&csv, &nodes, Some(&summary)).unwrap();
    let document: serde_json::Value = serde_json::from_slice(&fs::read(json).unwrap()).unwrap();
    assert_eq!(document["status"], "cancelled_partial");
    assert_eq!(document["nodes"][1]["logical"], 7);
    assert_eq!(document["nodes"][1]["note"], nodes[1].note);
    assert_eq!(document["nodes"][1]["incomplete"], true);
    let records = csv::Reader::from_path(csv)
        .unwrap()
        .records()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(&records[1][4], "7");
    assert_eq!(&records[1][8], "cancelled_partial");
    assert_eq!(&records[1][9], "true");
    assert!(records[1][7].starts_with("'="));
    assert_eq!(&records[1][3], nodes[1].path.to_string_lossy().as_ref());
    for formula in [
        "=1+1",
        " +SUM(1,2)",
        "\u{feff}@SUM(1,2)",
        "\ttext",
        "\r=1+1",
        "\n-1+2",
    ] {
        assert_eq!(csv_text(formula), format!("'{formula}"));
    }
    assert_eq!(csv_text("ordinary text"), "ordinary text");
}

#[test]
fn exports_never_overwrite_sources_aliases_or_racing_destinations() {
    let fixture = tempfile::tempdir().unwrap();
    let source = fixture.path().join("source.bin");
    let alias = fixture.path().join("alias.bin");
    let original = b"user data\0must remain byte-for-byte unchanged\xff";
    fs::write(&source, original).unwrap();
    fs::hard_link(&source, &alias).unwrap();
    let (nodes, summary) = collect_scan(fixture.path());
    for existing in [&source, &alias] {
        assert!(export_json(existing, &nodes, Some(&summary)).is_err());
        assert!(export_csv(existing, &nodes, Some(&summary)).is_err());
        assert!(write_new_output(existing, b"replacement").is_err());
        assert_eq!(fs::read(&source).unwrap(), original);
        assert_eq!(fs::read(&alias).unwrap(), original);
    }
    let destination = fixture.path().join("racing.json");
    let result = write_new_output_with(&destination, |file| {
        file.write_all(b"complete new output")
            .map_err(|e| e.to_string())?;
        // Another writer claims the destination after the initial check.
        fs::write(&destination, b"other writer's file").map_err(|e| e.to_string())
    });
    assert!(result.is_err());
    assert_eq!(fs::read(&destination).unwrap(), b"other writer's file");
    assert_eq!(
        fs::read_dir(fixture.path()).unwrap().count(),
        3,
        "failed publication must clean up its staging file"
    );
    let incomplete = fixture.path().join("incomplete.csv");
    assert!(
        write_new_output_with(&incomplete, |file| {
            file.write_all(b"partial output")
                .map_err(|e| e.to_string())?;
            Err("fixture serialization failure".into())
        })
        .is_err()
    );
    assert!(!incomplete.exists());
    assert_eq!(fs::read_dir(fixture.path()).unwrap().count(), 3);
}

#[test]
fn metadata_budget_finishes_as_partial_without_cancel_or_retaining_more_nodes() {
    let fixture = tempfile::tempdir().unwrap();
    let file = fixture.path().join("untouched.txt");
    fs::write(&file, b"user data").unwrap();
    let (sender, receiver) = bounded(QUEUE_CAPACITY);
    let mut worker = Worker {
        sender: sender.clone(),
        cancel: Arc::new(AtomicBool::new(false)),
        summary: ScanSummary::default(),
        seen: HashSet::new(),
        batch: Vec::new(),
        next_id: 0,
        last_flush: Instant::now(),
        retained_bytes: MAX_RETAINED_BYTES,
        stopped: false,
    };
    assert!(worker.visit(file.clone(), None).is_none());
    assert!(worker.batch.is_empty());
    finish_scan(worker, Instant::now());
    let ScanEvent::Finished(summary) = receiver.recv_timeout(Duration::from_secs(1)).unwrap()
    else {
        panic!("memory limit must finish rather than retaining another batch")
    };
    assert!(!summary.cancelled);
    assert!(summary.stopped_early);
    assert_eq!(summary.errors, 1);
    assert_eq!(summary.incomplete_nodes, vec![0]);
    assert_eq!(export_status(Some(&summary)), "partial");
    assert!(
        summary
            .notes
            .iter()
            .any(|note| note.contains("memory budget"))
    );
    assert_eq!(fs::read(file).unwrap(), b"user data");
    let mut note = String::new();
    for _ in 0..1000 {
        append_note(&mut note, "日本語: named stream allocation unavailable");
    }
    assert!(note.len() <= MAX_NOTE_BYTES);
    assert!(note.starts_with("日本語"));
    assert!(note.ends_with(NOTE_TRUNCATED));
    let frozen_note = note.clone();
    append_note(&mut note, "another provider failure");
    assert_eq!(note, frozen_note, "truncated diagnostics must stay bounded");

    let file = fixture.path().join("untouched.txt");
    let alias = fixture.path().join("also-kept.txt");
    fs::hard_link(&file, &alias).unwrap();
    let mut worker = Worker {
        sender,
        cancel: Arc::new(AtomicBool::new(false)),
        summary: ScanSummary::default(),
        seen: HashSet::new(),
        batch: Vec::new(),
        next_id: 0,
        last_flush: Instant::now(),
        retained_bytes: 0,
        stopped: false,
    };
    worker.visit(file, None).unwrap();
    let first_node_bytes = worker.retained_bytes;
    worker.batch.clear();
    worker.retained_bytes = MAX_RETAINED_BYTES - first_node_bytes;
    // Identical path/name lengths would fit before metadata is read, but the
    // extra hard-link diagnostic must also fit before publishing this node.
    assert!(worker.visit(alias, None).is_none());
    assert!(worker.batch.is_empty());
    finish_scan(worker, Instant::now());
    assert!(
        matches!(receiver.recv().unwrap(), ScanEvent::Finished(summary) if summary.errors == 1 && !summary.cancelled)
    );
}

#[test]
fn changing_tree_keeps_consistent_node_links() {
    let fixture = tempfile::tempdir().unwrap();
    for n in 0..128 {
        fs::write(fixture.path().join(format!("{n}.bin")), [1u8; 1024]).unwrap();
    }
    let changing = fixture.path().to_path_buf();
    let writer = std::thread::spawn(move || {
        for n in 0..128 {
            let file = changing.join(format!("{n}.bin"));
            let _ = fs::write(&file, [2u8; 4096]);
            let _ = fs::remove_file(file);
        }
    });
    let (nodes, summary) = collect_scan(fixture.path());
    writer.join().unwrap();
    assert!(!summary.cancelled);
    assert!(!nodes.is_empty());
    for node in &nodes {
        assert!(node.parent.is_none_or(|id| id < node.id));
    }
}

#[cfg(windows)]
mod windows_tests {
    use super::*;
    use std::{
        io::Write,
        os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
    };
    use windows::Win32::{
        Foundation::HANDLE,
        System::{
            IO::DeviceIoControl,
            Ioctl::{FSCTL_SET_COMPRESSION, FSCTL_SET_SPARSE},
        },
    };

    #[test]
    fn file_and_directory_alternate_streams_are_counted() {
        let fixture = tempfile::tempdir().unwrap();
        let file = fixture.path().join("with-streams.bin");
        fs::write(&file, b"default").unwrap();
        let mut ads = file.as_os_str().to_os_string();
        ads.push(":metadata");
        fs::write(PathBuf::from(ads), vec![5u8; 8192]).unwrap();
        let mut directory_ads = fixture.path().as_os_str().to_os_string();
        directory_ads.push(":annotation");
        fs::write(PathBuf::from(directory_ads), b"folder-stream").unwrap();
        fs::hard_link(&file, fixture.path().join("stream-alias.bin")).unwrap();
        let (nodes, summary) = collect_scan(fixture.path());
        assert_eq!(summary.errors, 0, "{:?}", summary.notes);
        assert_eq!(nodes[0].logical, 13);
        assert_eq!(nodes[1].logical, 8199);
        assert!(nodes[1].allocated >= 8192);
        assert!(nodes[1].note.contains("named data stream"));
        assert_eq!(summary.hard_links, 1);
        assert_eq!(
            nodes.iter().map(|node| node.logical).sum::<u64>(),
            13 + 8199 * 2
        );
        assert_eq!(
            nodes
                .iter()
                .filter(|node| !node.is_dir && node.allocated > 0)
                .count(),
            1,
            "ADS allocation is shared by hard-link aliases"
        );
    }

    #[test]
    fn sparse_and_compressed_files_report_physical_allocation() {
        let fixture = tempfile::tempdir().unwrap();
        let sparse_path = fixture.path().join("sparse.bin");
        let sparse = File::create(&sparse_path).unwrap();
        let mut returned = 0;
        unsafe {
            DeviceIoControl(
                HANDLE(sparse.as_raw_handle()),
                FSCTL_SET_SPARSE,
                None,
                0,
                None,
                0,
                Some(&mut returned),
                None,
            )
        }
        .unwrap();
        sparse.set_len(32 * 1024 * 1024).unwrap();
        drop(sparse);
        let compressed_path = fixture.path().join("compressed.bin");
        let mut compressed = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&compressed_path)
            .unwrap();
        let data: Vec<u8> = b"Rigometry compression fixture\n"
            .iter()
            .copied()
            .cycle()
            .take(1024 * 1024)
            .collect();
        compressed.write_all(&data).unwrap();
        compressed.sync_all().unwrap();
        let compression: u16 = 1; // COMPRESSION_FORMAT_DEFAULT
        unsafe {
            DeviceIoControl(
                HANDLE(compressed.as_raw_handle()),
                FSCTL_SET_COMPRESSION,
                Some((&compression as *const u16).cast()),
                2,
                None,
                0,
                Some(&mut returned),
                None,
            )
        }
        .unwrap();
        compressed.sync_all().unwrap();
        drop(compressed);
        use std::os::windows::ffi::OsStrExt;
        use windows::{Win32::Storage::FileSystem::GetCompressedFileSizeW, core::PCWSTR};
        let name: Vec<u16> = compressed_path
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        let mut high = 0;
        let low = unsafe { GetCompressedFileSizeW(PCWSTR(name.as_ptr()), Some(&mut high)) };
        assert_ne!(
            low,
            u32::MAX,
            "independent physical-allocation query failed"
        );
        let independent_allocated = ((high as u64) << 32) | low as u64;
        let (nodes, summary) = collect_scan(fixture.path());
        assert_eq!(summary.errors, 0, "{:?}", summary.notes);
        let sparse = nodes.iter().find(|n| n.name == "sparse.bin").unwrap();
        assert_eq!(sparse.logical, 32 * 1024 * 1024);
        assert_eq!(
            sparse.allocated, 0,
            "an entirely sparse stream has no committed data clusters"
        );
        let compressed = nodes.iter().find(|n| n.name == "compressed.bin").unwrap();
        assert_eq!(compressed.logical, 1024 * 1024);
        assert_eq!(
            compressed.allocated, independent_allocated,
            "scanner must match independent GetCompressedFileSizeW"
        );
        assert!(
            compressed.allocated > 0 && compressed.allocated < compressed.logical / 4,
            "physical={}, logical={}",
            compressed.allocated,
            compressed.logical
        );
        assert!(compressed.note.contains("compressed"));
    }

    #[test]
    fn metadata_only_scan_does_not_require_data_sharing() {
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().join("locked.bin");
        fs::write(&path, [1u8; 1024]).unwrap();
        let locked = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .unwrap();
        let (nodes, summary) = collect_scan(fixture.path());
        assert_eq!(summary.errors, 0);
        assert!(nodes.iter().any(|n| n.path == path && n.logical == 1024));
        drop(locked);
    }

    #[test]
    fn denied_directory_is_reported_as_partial() {
        use std::{mem::size_of, os::windows::ffi::OsStrExt};
        use windows::{
            Win32::Security::{
                ACL, ACL_REVISION, DACL_SECURITY_INFORMATION, GetFileSecurityW, InitializeAcl,
                InitializeSecurityDescriptor, PROTECTED_DACL_SECURITY_INFORMATION,
                PSECURITY_DESCRIPTOR, SECURITY_DESCRIPTOR, SetFileSecurityW,
                SetSecurityDescriptorDacl,
            },
            core::PCWSTR,
        };
        let fixture = tempfile::tempdir().unwrap();
        let locked = fixture.path().join("denied");
        fs::create_dir(&locked).unwrap();
        fs::write(locked.join("unreadable.bin"), [1u8; 4096]).unwrap();
        let name: Vec<u16> = locked.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut needed = 0;
        unsafe {
            let _ = GetFileSecurityW(
                PCWSTR(name.as_ptr()),
                DACL_SECURITY_INFORMATION.0,
                None,
                0,
                &mut needed,
            );
        }
        assert!(needed > 0);
        let mut original = vec![0u64; (needed as usize).div_ceil(8)];
        unsafe {
            GetFileSecurityW(
                PCWSTR(name.as_ptr()),
                DACL_SECURITY_INFORMATION.0,
                Some(PSECURITY_DESCRIPTOR(original.as_mut_ptr().cast())),
                (original.len() * 8) as u32,
                &mut needed,
            )
        }
        .ok()
        .unwrap();
        struct RestoreAcl {
            name: Vec<u16>,
            original: Vec<u64>,
        }
        impl Drop for RestoreAcl {
            fn drop(&mut self) {
                unsafe {
                    let _ = SetFileSecurityW(
                        PCWSTR(self.name.as_ptr()),
                        DACL_SECURITY_INFORMATION,
                        PSECURITY_DESCRIPTOR(self.original.as_mut_ptr().cast()),
                    );
                }
            }
        }
        // Restore the fixture's original ACL even if an assertion fails. Only
        // this test-created directory receives a temporary empty DACL.
        let restore = RestoreAcl { name, original };
        let mut descriptor = SECURITY_DESCRIPTOR::default();
        let mut acl = ACL::default();
        unsafe {
            InitializeSecurityDescriptor(
                PSECURITY_DESCRIPTOR((&mut descriptor as *mut SECURITY_DESCRIPTOR).cast()),
                1,
            )
            .unwrap();
            InitializeAcl(&mut acl, size_of::<ACL>() as u32, ACL_REVISION).unwrap();
            SetSecurityDescriptorDacl(
                PSECURITY_DESCRIPTOR((&mut descriptor as *mut SECURITY_DESCRIPTOR).cast()),
                true,
                Some(&acl),
                false,
            )
            .unwrap();
            SetFileSecurityW(
                PCWSTR(restore.name.as_ptr()),
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                PSECURITY_DESCRIPTOR((&mut descriptor as *mut SECURITY_DESCRIPTOR).cast()),
            )
            .ok()
            .unwrap();
        }
        let (nodes, summary) = collect_scan(fixture.path());
        assert!(
            summary.errors > 0,
            "directory listing permission must be enforced"
        );
        assert!(!nodes.iter().any(|n| n.name == "unreadable.bin"));
        let denied_id = nodes.iter().find(|n| n.name == "denied").unwrap().id;
        assert!(summary.incomplete_nodes.contains(&denied_id));
        assert_eq!(export_status(Some(&summary)), "partial");
        drop(restore);
    }

    #[test]
    fn long_paths_use_extended_win32_metadata_paths() {
        let fixture = tempfile::tempdir().unwrap();
        let mut deep = fixture.path().to_path_buf();
        for n in 0..8 {
            deep.push(format!("segment-{n}-abcdefghijklmnopqrstuvwxyz0123456789"));
        }
        fs::create_dir_all(&deep).unwrap();
        fs::write(deep.join("long-path.bin"), b"long-path-data").unwrap();
        let (nodes, summary) = collect_scan(fixture.path());
        assert_eq!(summary.errors, 0, "{:?}", summary.notes);
        assert_eq!(
            nodes
                .iter()
                .find(|n| n.name == "long-path.bin")
                .unwrap()
                .logical,
            14
        );
    }

    #[test]
    fn junction_loop_is_excluded() {
        use std::os::windows::process::CommandExt;
        let fixture = tempfile::tempdir().unwrap();
        fs::write(fixture.path().join("kept.bin"), b"12345").unwrap();
        let junction = fixture.path().join("loop");
        let output = std::process::Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(&junction)
            .arg(fixture.path())
            .creation_flags(0x08000000)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "junction fixture: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let (nodes, summary) = collect_scan(fixture.path());
        assert_eq!(summary.skipped_reparse, 1);
        assert_eq!(nodes.len(), 3);
        assert_eq!(nodes.iter().map(|n| n.logical).sum::<u64>(), 5);
        let (escaped_nodes, escaped_summary) = collect_scan(&junction.join("kept.bin"));
        assert!(
            escaped_nodes.is_empty(),
            "a selected root beneath a junction must not be followed"
        );
        assert_eq!(escaped_summary.errors, 1);
        assert!(write_new_output(&junction.join("kept.bin"), b"replacement").is_err());
        assert!(write_new_output(&junction.join("new-output.json"), b"new output").is_err());
        assert!(prepare_output_directory(&junction).is_err());
        assert!(prepare_output_directory(&junction.join("new-output-directory")).is_err());
        assert_eq!(fs::read(fixture.path().join("kept.bin")).unwrap(), b"12345");
        assert!(!fixture.path().join("new-output.json").exists());
        assert!(!fixture.path().join("new-output-directory").exists());
        // Remove only this junction before tempfile cleans up the fixture.
        fs::remove_dir(junction).unwrap();
    }

    #[test]
    fn stream_parser_rejects_truncated_and_invalid_offsets() {
        assert!(native::parse_streams(&[0; 8]).is_err());
        let mut record = vec![0u8; 32];
        record[0..4].copy_from_slice(&8u32.to_le_bytes());
        assert!(native::parse_streams(&record).is_err());
        record[0..4].copy_from_slice(&0u32.to_le_bytes());
        record[4..8].copy_from_slice(&3u32.to_le_bytes());
        assert!(native::parse_streams(&record).is_err());
        record[4..8].copy_from_slice(&0u32.to_le_bytes());
        record[8..16].copy_from_slice(&(-1i64).to_le_bytes());
        assert!(native::parse_streams(&record).is_err());
        for name in [
            "::$DATA",
            ":metadata:$DATA",
            ":日本語:$DATA",
            ":../outside:$DATA",
            ":bad\\path:$DATA",
            ":bad\0name:$DATA",
        ] {
            let encoded: Vec<u16> = name.encode_utf16().collect();
            let mut record = vec![0; 24 + encoded.len() * 2];
            record[4..8].copy_from_slice(&((encoded.len() * 2) as u32).to_le_bytes());
            for (index, value) in encoded.into_iter().enumerate() {
                record[24 + index * 2..26 + index * 2].copy_from_slice(&value.to_le_bytes());
            }
            assert_eq!(
                native::parse_streams(&record).is_ok(),
                !name.contains(['/', '\\', '\0']),
                "{name:?}"
            );
        }
    }

    #[test]
    fn stream_size_overflow_is_reported_as_partial_instead_of_exact_maximum() {
        let mut streams: Vec<_> = (0..3)
            .map(|index| native::Stream {
                logical: i64::MAX as u64,
                allocated: i64::MAX as u64,
                name: format!(":stream-{index}:$DATA").encode_utf16().collect(),
            })
            .collect();
        let mut info = EntryInfo::default();
        native::accumulate_stream_sizes(&mut info, &streams);
        assert_eq!((info.logical, info.allocated), (u64::MAX, u64::MAX));
        assert_eq!(
            info.errors, 2,
            "overflow in each byte measure must make accounting incomplete"
        );
        assert!(info.note.contains("lower bound"));

        streams[2].logical = 1;
        streams[2].allocated = 1;
        let mut exact = EntryInfo::default();
        native::accumulate_stream_sizes(&mut exact, &streams);
        assert_eq!((exact.logical, exact.allocated), (u64::MAX, u64::MAX));
        assert_eq!(
            exact.errors, 0,
            "an exactly representable maximum is not an overflow"
        );
        let mut changed = EntryInfo {
            allocated: u64::MAX - 5,
            ..Default::default()
        };
        native::replace_stream_allocation(&mut changed, 10, i64::MAX);
        assert_eq!(changed.allocated, u64::MAX);
        assert_eq!(
            changed.errors, 1,
            "a later compression query can overflow a previously valid stream total"
        );

        let mut invalid = EntryInfo {
            allocated: 500,
            ..Default::default()
        };
        native::replace_stream_allocation(&mut invalid, 400, -1);
        assert_eq!(
            invalid.allocated, 100,
            "retain only the other streams' known allocation"
        );
        assert_eq!(
            invalid.errors, 1,
            "negative byte counts cannot become a valid zero measurement"
        );
        let mut sparse = EntryInfo {
            allocated: 500,
            ..Default::default()
        };
        native::replace_stream_allocation(&mut sparse, 400, 0);
        assert_eq!(sparse.allocated, 100);
        assert_eq!(
            sparse.errors, 0,
            "zero physical bytes remain valid for an entirely sparse stream"
        );
    }

    #[test]
    fn device_and_alternate_stream_user_paths_are_rejected_without_opening() {
        for path in [
            r"\\.\PhysicalDrive0",
            r"\\.\pipe\rigometry-test",
            r"\\?\GLOBALROOT\Device\HarddiskVolume1",
            r"C:\NUL",
            r"C:\NUL.txt",
            r"C:\COM1",
            r"C:\LPT².log",
            "C:\\valid\0truncated",
        ] {
            let (nodes, summary) = collect_scan(Path::new(path));
            assert!(nodes.is_empty(), "{path:?}");
            assert_eq!(summary.errors, 1, "{path:?}");
        }
        let fixture = tempfile::tempdir().unwrap();
        let source = fixture.path().join("kept.txt");
        fs::write(&source, b"unchanged").unwrap();
        let stream = PathBuf::from(format!("{}:injected", source.display()));
        assert!(write_new_output(&stream, b"must not create ADS").is_err());
        let (nodes, summary) = collect_scan(&stream);
        assert!(nodes.is_empty());
        assert_eq!(summary.errors, 1);
        assert_eq!(fs::read(&source).unwrap(), b"unchanged");
        assert!(fs::symlink_metadata(&stream).is_err());
        assert!(native::validate_path(&fixture.path().join("COM10-report.txt")).is_ok());
    }

    #[test]
    fn cloud_attributes_are_excluded_before_opening_contents() {
        use windows::Win32::Storage::FileSystem::{
            FILE_ATTRIBUTE_OFFLINE, FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS,
            FILE_ATTRIBUTE_RECALL_ON_OPEN, FILE_ATTRIBUTE_REPARSE_POINT,
        };
        for flag in [
            FILE_ATTRIBUTE_OFFLINE,
            FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS,
            FILE_ATTRIBUTE_RECALL_ON_OPEN,
        ] {
            assert!(matches!(native::skip_attributes(flag.0), Some(Skip::Cloud)));
        }
        assert!(matches!(
            native::skip_attributes(FILE_ATTRIBUTE_REPARSE_POINT.0),
            Some(Skip::Reparse)
        ));
        assert!(native::skip_attributes(0).is_none());
    }
}
