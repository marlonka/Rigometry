//! Process-level contracts: argument validation, format routing and safe outputs.
use std::{
    fs,
    io::Read,
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

fn run(args: &[&std::ffi::OsStr]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rigometry"))
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start the actual application");
    // Drain both pipes while waiting: a full pipe must not stall the process.
    let drain = |mut pipe: Box<dyn Read + Send>| {
        thread::spawn(move || {
            let mut bytes = Vec::new();
            pipe.read_to_end(&mut bytes).map(|_| bytes)
        })
    };
    let stdout = drain(Box::new(child.stdout.take().unwrap()));
    let stderr = drain(Box::new(child.stderr.take().unwrap()));
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("CLI command exceeded 30 seconds");
        }
        thread::sleep(Duration::from_millis(10));
    };
    Output {
        status,
        stdout: stdout.join().unwrap().unwrap(),
        stderr: stderr.join().unwrap().unwrap(),
    }
}

#[test]
fn help_and_version_are_available_without_a_graphics_session() {
    let version = run(&["--version".as_ref()]);
    assert!(version.status.success());
    assert_eq!(
        String::from_utf8(version.stdout).unwrap().trim(),
        concat!("Rigometry ", env!("CARGO_PKG_VERSION"))
    );
    let help = run(&["--help".as_ref()]);
    assert!(help.status.success());
    let help = String::from_utf8(help.stdout).unwrap();
    for option in ["--scan", "--headless", "--export", "--report", "--capture"] {
        assert!(help.contains(option), "missing public option {option}");
    }
}

#[test]
fn invalid_command_does_not_create_an_earlier_requested_report() {
    let directory = tempfile::tempdir().unwrap();
    let report = directory.path().join("must-not-exist.json");
    let result = run(&[
        "--headless".as_ref(),
        "--report".as_ref(),
        report.as_os_str(),
        "--unknown-option".as_ref(),
    ]);
    assert!(!result.status.success());
    assert!(!report.exists());
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn selected_languages_localize_help_and_errors_without_changing_exports() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("Données_資料");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("Saved {0}.bin"), [5u8; 1536]).unwrap();
    for (language, help_text, error_text) in [
        ("de", "App öffnen", "Unbekannte oder inkompatible Option"),
        (
            "fr",
            "Ouvrir l’application",
            "Option inconnue ou incompatible",
        ),
        (
            "es",
            "Abrir la aplicación",
            "Opción desconocida o incompatible",
        ),
    ] {
        let help = run(&["--help".as_ref(), "--language".as_ref(), language.as_ref()]);
        assert!(help.status.success());
        assert!(String::from_utf8(help.stdout).unwrap().contains(help_text));
        let error = run(&[
            "--language".as_ref(),
            language.as_ref(),
            "--unknown".as_ref(),
        ]);
        assert!(!error.status.success());
        assert!(
            String::from_utf8(error.stderr)
                .unwrap()
                .contains(error_text)
        );
        let output = directory.path().join(format!("{language}.json"));
        let result = run(&[
            "--language".as_ref(),
            language.as_ref(),
            "--headless".as_ref(),
            "--scan".as_ref(),
            root.as_os_str(),
            "--export".as_ref(),
            output.as_os_str(),
        ]);
        assert!(result.status.success());
        let report: serde_json::Value = serde_json::from_slice(&fs::read(output).unwrap()).unwrap();
        assert_eq!(report["status"], "complete");
        assert_eq!(report["nodes"][0]["logical"], 1536);
        assert_eq!(report["nodes"][1]["name"], "Saved {0}.bin");
    }
}

#[test]
fn headless_scan_routes_formats_and_refuses_existing_destinations() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("資料");
    fs::create_dir(&root).unwrap();
    let source = root.join("kept.bin");
    let contents = b"preserve user data";
    fs::write(&source, contents).unwrap();
    let json = directory.path().join("scan.json");
    let csv = directory.path().join("scan.CSV");
    for output in [&json, &csv] {
        let result = run(&[
            "--headless".as_ref(),
            "--scan".as_ref(),
            root.as_os_str(),
            "--export".as_ref(),
            output.as_os_str(),
        ]);
        assert!(result.status.success(), "{:?}", result.stderr);
    }
    let json: serde_json::Value = serde_json::from_slice(&fs::read(json).unwrap()).unwrap();
    assert_eq!(json["status"], "complete");
    assert_eq!(json["nodes"][0]["logical"], contents.len() as u64);
    assert_eq!(json["nodes"][0]["files"], 1);
    let mut reader = csv::Reader::from_path(csv).unwrap();
    let headers = reader.headers().unwrap().clone();
    let logical = headers.iter().position(|h| h == "logical_bytes").unwrap();
    let records: Vec<_> = reader.records().collect::<Result<_, _>>().unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(
        records[0][logical].parse::<usize>().unwrap(),
        contents.len()
    );
    let result = run(&[
        "--headless".as_ref(),
        "--scan".as_ref(),
        root.as_os_str(),
        "--export".as_ref(),
        source.as_os_str(),
    ]);
    assert!(!result.status.success());
    assert_eq!(fs::read(&source).unwrap(), contents);
}

#[test]
fn headless_duplicate_report_lists_copies_and_failures_exit_nonzero() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("tree");
    fs::create_dir_all(root.join("a")).unwrap();
    fs::create_dir_all(root.join("b")).unwrap();
    let data: Vec<u8> = (0..100_000u32).map(|i| (i * 7 % 251) as u8).collect();
    fs::write(root.join("a/report.pdf"), &data).unwrap();
    fs::write(root.join("b/report (1).pdf"), &data).unwrap();
    fs::write(root.join("b/small.txt"), b"tiny").unwrap();
    fs::write(root.join("a/small.txt"), b"tiny").unwrap();
    let report = directory.path().join("duplicates.json");
    let result = run(&[
        "--headless".as_ref(),
        "--scan".as_ref(),
        root.as_os_str(),
        "--duplicates".as_ref(),
        report.as_os_str(),
        "--min-size".as_ref(),
        "64k".as_ref(),
    ]);
    assert!(result.status.success(), "{:?}", result.stderr);
    let json: serde_json::Value = serde_json::from_slice(&fs::read(&report).unwrap()).unwrap();
    assert_eq!(json["kind"], "duplicates");
    assert_eq!(json["status"], "complete");
    let groups = json["groups"].as_array().unwrap();
    assert_eq!(groups.len(), 1, "files below --min-size are not compared");
    assert_eq!(groups[0]["size"], 100_000);
    assert_eq!(groups[0]["copies"].as_array().unwrap().len(), 2);
    assert_eq!(json["summary"]["below_minimum"], 2);

    // A root that cannot be scanned fails instead of exporting an empty tree.
    let missing = directory.path().join("missing");
    let output = directory.path().join("missing.json");
    let result = run(&[
        "--headless".as_ref(),
        "--scan".as_ref(),
        missing.as_os_str(),
        "--export".as_ref(),
        output.as_os_str(),
    ]);
    assert!(!result.status.success());
    assert!(!output.exists());

    // An unusable output is refused before any scanning or reporting.
    let earlier = directory.path().join("hardware.json");
    let result = run(&[
        "--headless".as_ref(),
        "--report".as_ref(),
        earlier.as_os_str(),
        "--scan".as_ref(),
        root.as_os_str(),
        "--duplicates".as_ref(),
        report.as_os_str(),
    ]);
    assert!(!result.status.success());
    assert!(
        !earlier.exists(),
        "no output is written when another is refused"
    );
}
