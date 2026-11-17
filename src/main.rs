#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod branding;
mod hardware;
mod i18n;
mod model;
mod search;
mod state;
mod storage;
mod ui;
mod update;

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

fn argument(args: &[String], key: &str) -> Option<String> {
    args.windows(2).find(|a| a[0] == key).map(|a| a[1].clone())
}
fn validate_arguments(args: &[String]) -> Result<(), String> {
    let mut seen = std::collections::HashSet::new();
    let mut index = 1;
    while let Some(flag) = args.get(index) {
        if !seen.insert(flag.as_str()) {
            return Err(format!("Duplicate option: {flag}"));
        }
        match flag.as_str() {
            "--headless" => index += 1,
            "--help" | "--version" => index += 1,
            "--scan" | "--export" | "--duplicates" | "--min-size" | "--report" | "--capture"
            | "--window-size" | "--theme" | "--scale" | "--language" => {
                let value = args
                    .get(index + 1)
                    .filter(|value| !value.is_empty() && !value.starts_with("--"))
                    .ok_or_else(|| format!("{flag} requires a value"))?;
                match flag.as_str() {
                    "--language" if i18n::Language::parse(value).is_none() => {
                        return Err("--language requires system, en, de, fr or es".into());
                    }
                    "--window-size" if requested_window_size(args).is_none() => {
                        return Err(
                            "--window-size requires WIDTHxHEIGHT from 1080x720 to 3840x2160".into(),
                        );
                    }
                    "--scale"
                        if !value
                            .parse::<f32>()
                            .is_ok_and(|n| (0.85..=1.5).contains(&n)) =>
                    {
                        return Err("--scale requires a number from 0.85 to 1.5".into());
                    }
                    "--min-size" if search::parse_size(value).is_err() => {
                        return Err("--min-size requires a size such as 4k, 1m or 2gb".into());
                    }
                    "--theme" if !matches!(value.as_str(), "Dark" | "Light" | "System") => {
                        return Err("--theme requires Dark, Light or System".into());
                    }
                    _ => {}
                }
                index += 2;
            }
            _ => {
                return Err(format!(
                    "Unknown or incompatible option: {flag}; use --help"
                ));
            }
        }
    }
    if seen.contains("--help") || seen.contains("--version") {
        if seen
            .iter()
            .any(|flag| !["--help", "--version", "--language"].contains(flag))
            || (seen.contains("--help") && seen.contains("--version"))
        {
            return Err("--help and --version only accept --language".into());
        }
        return Ok(());
    }
    if seen.contains("--headless") {
        if ["--capture", "--window-size", "--theme", "--scale"]
            .iter()
            .any(|flag| seen.contains(flag))
        {
            return Err("Display options cannot be combined with --headless".into());
        }
        if !seen.contains("--report") && !seen.contains("--scan") {
            return Err("--headless requires --report or --scan with --export".into());
        }
        let exports = seen.contains("--export") || seen.contains("--duplicates");
        if seen.contains("--scan") != exports {
            return Err("Headless scans require --scan with --export or --duplicates".into());
        }
        if seen.contains("--min-size") && !seen.contains("--duplicates") {
            return Err("--min-size requires --duplicates".into());
        }
        let outputs: Vec<_> = ["--report", "--export", "--duplicates"]
            .into_iter()
            .filter_map(|flag| argument(args, flag))
            .map(|path| output_key(&path))
            .collect();
        if (1..outputs.len()).any(|i| outputs[..i].contains(&outputs[i])) {
            return Err("Each output needs its own file name".into());
        }
    } else if ["--report", "--export", "--duplicates", "--min-size"]
        .iter()
        .any(|flag| seen.contains(flag))
    {
        return Err("--report, --export and --duplicates require --headless".into());
    }
    Ok(())
}

/// An output path as the filesystem tells files apart: absolute, with `.` and
/// `..` resolved, and case-insensitive on Windows and macOS.
fn output_key(path: &str) -> PathBuf {
    use std::path::Component;
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.into());
    let mut key = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::ParentDir => {
                key.pop();
            }
            Component::CurDir => {}
            other => key.push(other),
        }
    }
    if cfg!(any(windows, target_os = "macos")) {
        key.to_string_lossy().to_lowercase().into()
    } else {
        key
    }
}

fn requested_window_size(args: &[String]) -> Option<eframe::egui::Vec2> {
    let value = argument(args, "--window-size")?;
    let (width, height) = value.split_once('x')?;
    let width = width.parse::<f32>().ok()?;
    let height = height.parse::<f32>().ok()?;
    ((1080. ..=3840.).contains(&width) && (720. ..=2160.).contains(&height))
        .then(|| eframe::egui::vec2(width, height))
}
/// Release builds use the GUI subsystem, so a terminal gives them no console:
/// `--help`, `--version` and errors would print nothing. Attach to the
/// parent's console when there is one and output is not already redirected.
#[cfg(all(windows, not(debug_assertions)))]
fn attach_parent_console() {
    use windows::Win32::System::Console::{
        ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_OUTPUT_HANDLE,
    };
    let redirected = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) }
        .is_ok_and(|handle| !handle.is_invalid() && !handle.0.is_null());
    if !redirected {
        // Fails harmlessly when started from Explorer, which has no console.
        let _ = unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
    }
}

fn main() {
    // Only text-mode commands attach: a window attached to the terminal it was
    // started from would close with it, without saving its settings.
    #[cfg(all(windows, not(debug_assertions)))]
    if std::env::args_os().any(|arg| {
        ["--help", "--version", "--headless"].contains(&arg.to_str().unwrap_or_default())
    }) {
        attach_parent_console();
    }
    if let Err(error) = run() {
        #[cfg(all(windows, not(debug_assertions)))]
        attach_parent_console();
        let args: Vec<_> = std::env::args_os()
            .filter_map(|arg| arg.into_string().ok())
            .collect();
        let language = argument(&args, "--language")
            .and_then(|code| i18n::Language::parse(&code))
            .unwrap_or_default();
        eprintln!("{}", language.message(&error.to_string()));
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args_os()
        .map(|value| {
            value
                .into_string()
                .map_err(|_| "Command-line arguments must be valid Unicode")
        })
        .collect::<Result<Vec<_>, _>>()?;
    // Validate the entire command before starting providers or writing any output.
    validate_arguments(&args)?;
    if args.iter().any(|flag| flag == "--version") {
        println!("Rigometry {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.iter().any(|flag| flag == "--help") {
        let language = argument(&args, "--language")
            .and_then(|code| i18n::Language::parse(&code))
            .unwrap_or_default();
        println!("Rigometry {}", env!("CARGO_PKG_VERSION"));
        for (flags, description) in [
            ("--scan PATH", "Open the app and scan a folder"),
            (
                "--headless --scan PATH --export FILE",
                "Export a scan (.csv, otherwise JSON)",
            ),
            (
                "--headless --scan PATH --duplicates FILE",
                "Export duplicate files (.csv, otherwise JSON)",
            ),
            (
                "--min-size SIZE",
                "Smallest file compared for duplicates (default 1m)",
            ),
            (
                "--headless --report FILE",
                "Export hardware inventory and readings",
            ),
            (
                "--capture DIRECTORY",
                "Save five views after two minutes, then close",
            ),
            (
                "--language system|en|de|fr|es",
                "Select the interface and command-line language",
            ),
            ("--theme Dark|Light|System", "Select the display theme"),
            ("--scale NUMBER", "Set interface scale (0.85-1.5)"),
            ("--window-size WIDTHxHEIGHT", "Set window dimensions"),
            ("--version", "Print the application version"),
        ] {
            println!("  {flags:<41} {}", language.text(description));
        }
        println!(
            "{}",
            language.text("Output files must not already exist; parent directories must exist.")
        );
        return Ok(());
    }
    if args.iter().any(|s| s == "--headless") {
        return headless(&args);
    }
    let scan = argument(&args, "--scan");
    let capture = argument(&args, "--capture").map(PathBuf::from);
    if let Some(dir) = &capture {
        storage::prepare_output_directory(dir)?;
    }
    // Toolkit persistence must not write into a user-selected capture directory.
    let capture_storage = capture.as_ref().map(|_| tempfile::tempdir()).transpose()?;
    let window_size = requested_window_size(&args).unwrap_or(eframe::egui::vec2(1440., 940.));
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size(window_size)
            .with_min_inner_size([1080., 720.])
            .with_title("Rigometry")
            .with_icon(branding::window_icon()),
        renderer: eframe::Renderer::Glow,
        persist_window: capture.is_none() && argument(&args, "--window-size").is_none(),
        persistence_path: capture_storage.as_ref().map(|dir| dir.path().join("state")),
        ..Default::default()
    };
    // A version started by an update waits for the one it replaced to exit
    // and save its settings before eframe reads them.
    let updated_from = if capture.is_none() {
        update::after_update()
    } else {
        None
    };
    let capture_failed = Arc::new(AtomicBool::new(false));
    let app_capture_failed = capture_failed.clone();
    eframe::run_native(
        "Rigometry",
        options,
        Box::new(move |cc| {
            Ok(Box::new(ui::App::new(
                cc,
                scan,
                capture,
                app_capture_failed,
                updated_from,
            )))
        }),
    )?;
    if capture_failed.load(Ordering::Relaxed) {
        return Err(
            "Screenshot capture failed or was interrupted; existing files are never overwritten"
                .into(),
        );
    }
    Ok(())
}
fn headless(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let output = |flag| argument(args, flag).map(PathBuf::from);
    let (report, export, duplicates) = (
        output("--report"),
        output("--export"),
        output("--duplicates"),
    );
    // Refuse unusable outputs before minutes of scanning, not after.
    for path in [&report, &export, &duplicates].into_iter().flatten() {
        storage::check_new_output(path)?;
    }
    let scanned = match argument(args, "--scan") {
        Some(path) => Some(headless_scan(PathBuf::from(path))?),
        None => None,
    };
    if let Some(report) = report {
        let started = std::time::Instant::now();
        let inventory = hardware::discover();
        let discovery_ms = started.elapsed().as_millis();
        let mut monitor = hardware::Monitor::new(&inventory);
        let _ = monitor.sample();
        std::thread::sleep(Duration::from_secs(1));
        let sample = monitor.sample();
        let data = serde_json::json!({"schema_version":1,"discovery_ms":discovery_ms,"inventory":inventory,"sample":sample});
        storage::write_new_output(&report, &serde_json::to_vec_pretty(&data)?)?;
    }
    let Some((nodes, summary)) = scanned else {
        return Ok(());
    };
    if let Some(target) = export {
        if is_csv(&target) {
            storage::export_csv(&target, &nodes, Some(&summary))?;
        } else {
            storage::export_json(&target, &nodes, Some(&summary))?;
        }
    }
    if let Some(target) = duplicates {
        let min_size = argument(args, "--min-size")
            .and_then(|size| search::parse_size(&size).ok())
            .unwrap_or(storage::duplicates::DEFAULT_MIN_SIZE);
        let nodes = Arc::new(nodes);
        let partial = storage::subtree_partial(&nodes, &summary, 0);
        let search =
            storage::duplicates::find(nodes.clone(), 0, min_size, partial, Arc::new(|| {}));
        let mut groups = vec![];
        let summary = loop {
            match search.receiver.recv()? {
                storage::duplicates::DuplicateEvent::Groups(found) => groups.extend(found),
                storage::duplicates::DuplicateEvent::Progress(_) => {}
                storage::duplicates::DuplicateEvent::Finished(summary) => break summary,
            }
        };
        groups.sort_by_key(storage::duplicates::DuplicateGroup::order);
        if is_csv(&target) {
            storage::duplicates::export_csv(&target, &nodes, &groups, &summary)?;
        } else {
            storage::duplicates::export_json(&target, &nodes, &groups, &summary, None)?;
        }
    }
    Ok(())
}

/// Scans `path` to completion. A scan that could not read its root at all
/// fails the command instead of exporting an empty tree as success.
fn headless_scan(
    path: PathBuf,
) -> Result<(Vec<storage::ScanNode>, storage::ScanSummary), Box<dyn std::error::Error>> {
    let scan = storage::start_scan(path);
    let mut nodes = vec![];
    let mut summary = loop {
        match scan.receiver.recv()? {
            storage::ScanEvent::Batch(batch) => state::append_batch(&mut nodes, batch),
            storage::ScanEvent::Finished(summary) => break summary,
        }
    };
    if let Some(reason) = storage::root_failure(&nodes, &summary) {
        return Err(format!("Scan failed: {reason}").into());
    }
    state::finalize_scan(&mut nodes, &mut summary);
    Ok((nodes, summary))
}

fn is_csv(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("csv"))
}

#[cfg(test)]
mod cli_tests {
    use super::*;

    #[test]
    fn malformed_commands_fail_before_execution() {
        for flags in [
            vec!["--headless", "--bogus"],
            vec!["--headless"],
            vec!["--headless", "--report", "report.json", "--scan", "folder"],
            vec!["--headless", "--scan", "--export", "out.json"],
            vec!["--headless", "--export", "out.json"],
            vec!["--headless", "--report", "out.json", "--capture", "shots"],
            vec!["--report", "out.json"],
            vec!["--capture", "shots", "--capture", "other"],
            vec!["--scale", "NaN"],
            vec!["--scale", "4"],
            vec!["--window-size", "infx720"],
            vec!["--window-size", "-1x720"],
            vec!["--window-size", "10x10"],
            vec!["--headless", "--scan", "folder", "--min-size", "1m"],
            vec![
                "--headless",
                "--scan",
                "f",
                "--duplicates",
                "d.json",
                "--min-size",
                "lots",
            ],
            vec!["--duplicates", "d.json"],
            vec![
                "--headless",
                "--report",
                "same.json",
                "--scan",
                "f",
                "--export",
                "same.json",
            ],
            vec![
                "--headless",
                "--report",
                "out/../a.json",
                "--scan",
                "f",
                "--export",
                "a.json",
            ],
            vec!["--theme", "typo"],
            vec!["--version", "--scan", "folder"],
        ] {
            let args: Vec<_> = std::iter::once("rigometry")
                .chain(flags)
                .map(str::to_owned)
                .collect();
            assert!(validate_arguments(&args).is_err(), "Accepted {args:?}");
        }
        for flags in [
            vec![],
            vec!["--version"],
            vec!["--help"],
            vec![
                "--headless",
                "--scan",
                "C:/測定 Folder",
                "--export",
                "scan.CSV",
            ],
            vec![
                "--headless",
                "--report",
                "hardware.json",
                "--scan",
                "folder",
                "--export",
                "scan.json",
            ],
            vec![
                "--headless",
                "--scan",
                "folder",
                "--duplicates",
                "copies.csv",
                "--min-size",
                "4k",
            ],
            vec![
                "--capture",
                "shots",
                "--theme",
                "Light",
                "--scale",
                "1.5",
                "--window-size",
                "1080x720",
            ],
        ] {
            let args: Vec<_> = std::iter::once("rigometry")
                .chain(flags)
                .map(str::to_owned)
                .collect();
            assert!(validate_arguments(&args).is_ok(), "Rejected {args:?}");
        }
    }
}
