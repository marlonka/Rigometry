use super::*;
use serde_json::json;

fn answer(tag: &str) -> Value {
    let download = |name: &str| format!("https://github.com/{REPO}/releases/download/{tag}/{name}");
    let version = tag.trim_start_matches('v');
    let windows = format!("Rigometry-{version}-windows-x64-20261117T073000000Z.zip");
    let mac = format!("Rigometry-{version}-macos-universal.zip");
    json!({
        "tag_name": tag,
        "draft": false,
        "prerelease": false,
        "html_url": "https://example.com/elsewhere",
        "body": "- **Duplicate files:** compare by content. [How it works](docs/usage.md)\n- Faster `scans`.\n\n**Windows:** Windows 11 on x64.\n\nSource: abc\n",
        "assets": [
            {"name": windows, "size": 10, "browser_download_url": download(&windows),
             "digest": format!("sha256:{}", "A".repeat(64))},
            {"name": format!("{windows}.sha256"), "size": 1, "browser_download_url": download(&format!("{windows}.sha256"))},
            {"name": mac, "size": 20, "browser_download_url": download(&mac)},
            {"name": format!("{mac}.sha256"), "size": 1, "browser_download_url": download(&format!("{mac}.sha256"))},
            {"name": "evil.zip", "size": 5, "browser_download_url": "https://evil.example/evil.zip"},
        ],
    })
}

#[test]
fn versions_compare_as_numbers_and_suffixes_are_no_release() {
    assert!(Version::parse("v0.10.0") > Version::parse("0.9.9"));
    assert_eq!(Version::parse("v1.2.3"), Some(Version(1, 2, 3)));
    assert_eq!(Version::parse("0.4.0").unwrap().to_string(), "0.4.0");
    for no in [
        "0.4.0-rc.1",
        "0.4.0+build",
        "0.4",
        "1.2.3.4",
        "",
        "v",
        "x.y.z",
        "1..2",
    ] {
        assert_eq!(Version::parse(no), None, "{no}");
    }
}

#[test]
fn each_platform_takes_its_own_download_with_its_checksums() {
    let windows = parse_release_for(&answer("v0.5.0"), Platform::Windows).unwrap();
    let asset = windows.asset.unwrap();
    assert!(asset.name.starts_with("Rigometry-0.5.0-windows-x64"));
    assert_eq!(asset.sha256.as_deref(), Some("a".repeat(64).as_str()));
    assert!(asset.sums_url.unwrap().ends_with(".zip.sha256"));
    let mac = parse_release_for(&answer("v0.5.0"), Platform::MacOs).unwrap();
    let asset = mac.asset.unwrap();
    assert_eq!(asset.name, "Rigometry-0.5.0-macos-universal.zip");
    assert_eq!(asset.sha256, None, "no digest: the .sha256 file is used");
    assert!(asset.sums_url.is_some());
    assert!(
        parse_release_for(&answer("v0.5.0"), Platform::Other)
            .unwrap()
            .asset
            .is_none()
    );
    assert_eq!(
        windows.url,
        format!("https://github.com/{REPO}/releases/tag/v0.5.0"),
        "the page comes from the tag, not from the answer"
    );
}

#[test]
fn files_from_elsewhere_drafts_and_odd_tags_are_never_offered() {
    let mut moved = answer("v0.5.0");
    moved["assets"][0]["browser_download_url"] = json!("https://evil.example/x.zip");
    assert!(
        parse_release_for(&moved, Platform::Windows)
            .unwrap()
            .asset
            .is_none()
    );
    let mut draft = answer("v0.5.0");
    draft["draft"] = json!(true);
    assert!(parse_release_for(&draft, Platform::Windows).is_none());
    let mut pre = answer("v0.5.0");
    pre["prerelease"] = json!(true);
    assert!(parse_release_for(&pre, Platform::Windows).is_none());
    assert!(parse_release_for(&answer("v0.5.0-rc.1"), Platform::Windows).is_none());
}

#[test]
fn the_notes_are_the_changes_as_plain_text() {
    let release = parse_release_for(&answer("v0.5.0"), Platform::Windows).unwrap();
    assert_eq!(
        release.notes,
        [
            "Duplicate files: compare by content. How it works",
            "Faster scans."
        ]
    );
    let long: String = (0..40)
        .map(|i| format!("- item {i} {}\n", "x".repeat(60)))
        .collect();
    let shown = notes(&long);
    assert!(shown.len() <= NOTE_ITEMS);
    assert!(shown.iter().map(|n| n.chars().count()).sum::<usize>() <= NOTE_CHARS);
}

#[test]
fn only_a_newer_release_with_a_download_is_offered() {
    let release = parse_release_for(&answer("v0.5.0"), Platform::Windows).unwrap();
    let record = Record {
        release: Some(release.clone()),
        ..Record::default()
    };
    assert!(record.offered(Version::parse("0.4.0")).is_some());
    assert!(record.offered(Version::parse("0.5.0")).is_none());
    assert!(record.offered(Version::parse("0.6.0")).is_none());
    assert!(record.offered(None).is_none());
    let bare = Record {
        release: Some(Release {
            asset: None,
            ..release
        }),
        ..Record::default()
    };
    assert!(bare.offered(Version::parse("0.4.0")).is_none());
}

#[test]
fn a_look_is_due_when_old_and_not_while_github_asks_to_wait() {
    let now = 10 * STALE_MS;
    assert!(Record::default().due(now));
    let mut record = Record::default();
    record.apply(&Checked::Unchanged, now - 1000);
    assert!(!record.due(now));
    assert!(record.fresh(now));
    record.apply(&Checked::Unchanged, now - STALE_MS);
    assert!(record.due(now));
    assert!(!record.fresh(now));
    record.apply(&Checked::Limited(now + 1), now);
    assert!(!record.due(now), "GitHub's limit is respected");
}

#[test]
fn headers_come_from_the_last_response_after_redirects() {
    let dump = "HTTP/2 302\r\nlocation: x\r\netag: \"old\"\r\n\r\nHTTP/2 200\r\nETag: \"new\"\r\nX-RateLimit-Remaining: 0\r\n\r\n";
    assert_eq!(last_header(dump, "etag").as_deref(), Some("\"new\""));
    assert_eq!(
        last_header(dump, "x-ratelimit-remaining").as_deref(),
        Some("0")
    );
    assert_eq!(last_header(dump, "missing"), None);
}

#[test]
fn checksum_lists_and_downloads_are_checked_exactly() {
    let a = "a".repeat(64);
    let list = format!("{a}  Rigometry-0.5.0-macos-universal.zip\n");
    assert_eq!(
        sums_line(&list, "Rigometry-0.5.0-macos-universal.zip").as_deref(),
        Some(a.as_str())
    );
    assert_eq!(sums_line(&list, "other.zip"), None);
    let folder = tempfile::tempdir().unwrap();
    let file = folder.path().join("x");
    std::fs::write(&file, b"abc").unwrap();
    let abc = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    assert!(verify(&file, 3, abc).is_ok());
    assert!(verify(&file, 4, abc).is_err(), "size must match too");
    assert!(verify(&file, 3, &a).is_err());
}

#[test]
fn only_a_packaged_copy_replaces_itself() {
    let folder = tempfile::tempdir().unwrap();
    let exe = folder.path().join("Rigometry").join("Rigometry.exe");
    assert_eq!(
        target_of(&exe, Platform::Windows),
        Target::Program(exe.clone())
    );
    std::fs::write(folder.path().join("Cargo.toml"), "").unwrap();
    assert!(matches!(
        target_of(&exe, Platform::Windows),
        Target::Fixed(_)
    ));
    let bundle = Path::new("/Applications/Rigometry.app");
    let mac_exe = bundle.join("Contents/MacOS/rigometry");
    assert_eq!(
        target_of(&mac_exe, Platform::MacOs),
        Target::Bundle(bundle.into())
    );
    let translocated = Path::new(
        "/private/var/folders/x/AppTranslocation/1/d/Rigometry.app/Contents/MacOS/rigometry",
    );
    assert!(matches!(
        target_of(translocated, Platform::MacOs),
        Target::Fixed(_)
    ));
    assert!(matches!(
        target_of(Path::new("/usr/local/bin/rigometry"), Platform::MacOs),
        Target::Fixed(_)
    ));
}

#[test]
fn the_new_program_takes_the_old_ones_place_and_the_old_one_waits_aside() {
    let folder = tempfile::tempdir().unwrap();
    let app = folder.path().join("Rigometry");
    std::fs::create_dir_all(app.join("docs")).unwrap();
    let exe = app.join("Rigometry.exe");
    std::fs::write(&exe, b"old program").unwrap();
    std::fs::write(app.join("docs/usage.md"), b"old guide").unwrap();
    std::fs::write(app.join("notes.txt"), b"the user's own file").unwrap();
    let package = folder.path().join("unpacked/Rigometry");
    std::fs::create_dir_all(package.join("docs/screenshots")).unwrap();
    std::fs::write(package.join("Rigometry.exe"), b"new program").unwrap();
    std::fs::write(package.join("docs/usage.md"), b"new guide").unwrap();
    std::fs::write(package.join("docs/screenshots/new.png"), b"png").unwrap();
    replace_program(&exe, &package).unwrap();
    assert_eq!(std::fs::read(&exe).unwrap(), b"new program");
    assert_eq!(std::fs::read(old_program(&exe)).unwrap(), b"old program");
    assert_eq!(
        std::fs::read(app.join("docs/usage.md")).unwrap(),
        b"new guide"
    );
    assert!(app.join("docs/screenshots/new.png").is_file());
    assert_eq!(
        std::fs::read(app.join("notes.txt")).unwrap(),
        b"the user's own file"
    );
}

#[test]
fn a_bundle_is_swapped_and_restored_if_the_new_one_is_missing() {
    let folder = tempfile::tempdir().unwrap();
    let bundle = folder.path().join("Rigometry.app");
    std::fs::create_dir_all(bundle.join("Contents")).unwrap();
    std::fs::write(bundle.join("Contents/Info.plist"), b"old").unwrap();
    let staging = folder.path().join(".rigometry-update-x");
    std::fs::create_dir(&staging).unwrap();
    let missing = staging.join("unpacked/Rigometry.app");
    assert!(replace_bundle(&bundle, &missing, &staging).is_err());
    assert_eq!(
        std::fs::read(bundle.join("Contents/Info.plist")).unwrap(),
        b"old",
        "a failed swap leaves the old app in place"
    );
    let new = staging.join("new/Rigometry.app");
    std::fs::create_dir_all(new.join("Contents")).unwrap();
    std::fs::write(new.join("Contents/Info.plist"), b"new").unwrap();
    replace_bundle(&bundle, &new, &staging).unwrap();
    assert_eq!(
        std::fs::read(bundle.join("Contents/Info.plist")).unwrap(),
        b"new"
    );
    assert!(staging.join("previous.app").is_dir());
}

#[test]
fn only_this_repositorys_release_pages_are_opened() {
    for bad in [
        "https://evil.example/releases/",
        "https://github.com/marlonka/Rigometry/releases/tag/v1 --flag",
        "https://github.com/other/Rigometry/releases/tag/v1",
        "file:///etc/passwd",
    ] {
        assert!(open_page(bad).is_err(), "{bad}");
    }
}

/// Downloads a published release with the system's curl, checks it against
/// GitHub's digest and unpacks it. Needs the network:
/// `cargo test -- --ignored a_published_release_downloads_verifies_and_unpacks`.
#[test]
#[ignore]
fn a_published_release_downloads_verifies_and_unpacks() {
    let Checked::Latest { release, .. } = check(None).unwrap() else {
        panic!("a first look is never unchanged");
    };
    let release = release.expect("a published release");
    // Any computer can fetch the Mac download to try the path.
    let folder = tempfile::tempdir().unwrap();
    let answer: Value = {
        let file = folder.path().join("answer.json");
        let mut command = curl_command();
        command.arg("--output").arg(&file).arg(format!(
            "https://api.github.com/repos/{REPO}/releases/tags/{}",
            release.tag
        ));
        run_curl(
            command,
            Duration::from_secs(60),
            None,
            &AtomicBool::new(false),
        )
        .unwrap();
        serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap()
    };
    let mac = parse_release_for(&answer, Platform::MacOs).unwrap();
    let asset = mac.asset.unwrap();
    let archive = folder.path().join("download.zip");
    let received = AtomicU64::new(0);
    let mut command = curl_command();
    command
        .args(["--fail", "--output"])
        .arg(&archive)
        .arg(&asset.url);
    run_curl(
        command,
        Duration::from_secs(600),
        Some((&archive, &received)),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(received.load(Ordering::Relaxed), asset.size);
    let sha256 = asset.sha256.clone().unwrap();
    verify(&archive, asset.size, &sha256).unwrap();
    let unpacked = folder.path().join("unpacked");
    std::fs::create_dir(&unpacked).unwrap();
    unpack(&archive, &unpacked).unwrap();
    let bundle = find_bundle(&unpacked).expect("Rigometry.app in the download");
    assert!(bundle.join("Contents/MacOS/rigometry").is_file());
}
