//! Process-level contracts: argument validation, format routing and safe outputs.
use std::{
    fs,
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
    let deadline = Instant::now() + Duration::from_secs(30);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("CLI command exceeded 30 seconds");
        }
        thread::sleep(Duration::from_millis(10));
    }
    child.wait_with_output().unwrap()
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
