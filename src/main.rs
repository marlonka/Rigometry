#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod branding;
mod hardware;
mod i18n;
mod model;
mod search;
mod state;
mod storage;
mod ui;

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
            "--scan" | "--export" | "--report" | "--capture" | "--window-size" | "--theme"
            | "--scale" | "--language" => {
                let value = args
                    .get(index + 1)
                    .filter(|value| !value.is_empty() && !value.starts_with("--"))
                    .ok_or_else(|| format!("{flag} requires a value"))?;
                match flag.as_str() {
                    "--language" if i18n::Language::parse(value).is_none() => {
                        return Err("--language requires system, en, de, fr or es".into());
                    }
                    "--window-size" if requested_window_size(args).is_none() => {
                        return Err("--window-size requires positive finite WIDTHxHEIGHT".into());
                    }
                    "--scale" if !value.parse::<f32>().is_ok_and(|n| n.is_finite() && n > 0.) => {
                        return Err("--scale requires a positive finite number".into());
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
        if seen.contains("--scan") != seen.contains("--export") {
            return Err("Headless scans require both --scan and --export".into());
        }
    } else if seen.contains("--report") || seen.contains("--export") {
        return Err("--report and --export require --headless".into());
    }
    Ok(())
}

fn requested_window_size(args: &[String]) -> Option<eframe::egui::Vec2> {
    let value = argument(args, "--window-size")?;
    let (width, height) = value.split_once('x')?;
    let width = width.parse::<f32>().ok()?;
    let height = height.parse::<f32>().ok()?;
    (width.is_finite() && height.is_finite() && width > 0. && height > 0.)
        .then(|| eframe::egui::vec2(width.clamp(1080., 3840.), height.clamp(720., 2160.)))
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
    #[cfg(all(windows, not(debug_assertions)))]
    if std::env::args_os().len() > 1 {
        attach_parent_console();
    }
    if let Err(error) = run() {
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
            println!("  {flags:<39} {}", language.text(description));
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
    if let Some(report) = argument(args, "--report") {
        let started = std::time::Instant::now();
        let inventory = hardware::discover();
        let discovery_ms = started.elapsed().as_millis();
        let mut monitor = hardware::Monitor::new(&inventory);
        let _ = monitor.sample();
        std::thread::sleep(Duration::from_secs(1));
        let sample = monitor.sample();
        let data = serde_json::json!({"schema_version":1,"discovery_ms":discovery_ms,"inventory":inventory,"sample":sample});
        storage::write_new_output(
            std::path::Path::new(&report),
            &serde_json::to_vec_pretty(&data)?,
        )?;
    }
    if let Some(path) = argument(args, "--scan") {
        let output =
            argument(args, "--export").ok_or("--scan requires --export in headless mode")?;
        let scan = storage::start_scan(PathBuf::from(path));
        let mut nodes = vec![];
        let mut summary = loop {
            match scan.receiver.recv()? {
                storage::ScanEvent::Batch(batch) => state::append_batch(&mut nodes, batch),
                storage::ScanEvent::Finished(summary) => break summary,
            }
        };
        let target = PathBuf::from(output);
        state::finalize_scan(&mut nodes, &mut summary);
        if target
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("csv"))
        {
            storage::export_csv(&target, &nodes, Some(&summary))?;
        } else {
            storage::export_json(&target, &nodes, Some(&summary))?;
        }
    }
    Ok(())
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
            vec!["--window-size", "infx720"],
            vec!["--window-size", "-1x720"],
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
