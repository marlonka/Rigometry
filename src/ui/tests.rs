//! Semantic UI journeys run through the production eframe app. Filesystem
//! fixtures are real; hardware fixtures isolate layout from live sampling.

use super::*;
use egui::accesskit::Role;
use egui_kittest::{
    Harness,
    kittest::{By, Queryable},
};
use std::{collections::VecDeque, fs, path::Path, sync::atomic::Ordering};

#[test]
fn explorer_reveal_cannot_search_for_an_executable_in_an_untrusted_directory() {
    let fixture = tempfile::tempdir().unwrap();
    fs::write(fixture.path().join("explorer.exe"), b"untrusted executable").unwrap();
    let command = explorer_command(&fixture.path().join("a,b.txt")).unwrap();
    let program = Path::new(command.get_program());
    assert!(
        program.is_absolute(),
        "PATH or current-directory search must never select Explorer"
    );
    assert!(!program.starts_with(fixture.path()));
    assert_eq!(program.file_name().unwrap(), "explorer.exe");
    assert!(explorer_command(Path::new("C:\\data\\bad\" /root,C:\\")).is_err());
}

fn app_harness() -> Harness<'static, App> {
    let mut harness = Harness::builder()
        .with_size([1280.0, 920.0])
        .with_os(egui::os::OperatingSystem::Windows)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(2)
        .build_eframe(|cc| {
            App::with_state(
                cc,
                None,
                None,
                std::sync::Arc::default(),
                |mut settings, _| {
                    settings.language = Language::English;
                    State::fixture(settings)
                },
            )
        });
    set_window_pixels(&mut harness, vec2(1280., 920.));
    harness
}

fn set_window_pixels(harness: &mut Harness<'_, App>, pixels: Vec2) {
    // Harness::set_size consumes egui points; native window dimensions and
    // AccessKit bounds use physical pixels, including the app's zoom factor.
    harness.set_size(pixels / harness.ctx.pixels_per_point());
    harness.run_steps(3);
}

fn click(harness: &Harness<'_, App>, role: Role, label: &str) {
    // egui_kittest 0.34.3 Node::click does not undo AccessKit's root pixel
    // transform. Convert its bounds back to points for real pointer hit tests.
    let pos =
        harness.get_by_role_and_label(role, label).rect().center() / harness.ctx.pixels_per_point();
    harness.event(egui::Event::PointerMoved(pos));
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::default(),
        });
    }
}

fn navigate(harness: &mut Harness<'_, App>, label: &str) {
    click(harness, Role::Button, label);
    harness.run_steps(2);
}

fn wait_for_scan(harness: &mut Harness<'_, App>) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while harness.state().state.scan.is_some() && Instant::now() < deadline {
        harness.step();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        harness.state().state.scan.is_none(),
        "scan must finish while the UI keeps rendering"
    );
    assert!(
        harness.state().state.summary.is_some(),
        "scan must publish a completion state"
    );
    harness.run_steps(2);
}

fn type_scan_path(harness: &mut Harness<'_, App>, path: &Path) {
    click(harness, Role::TextInput, "Path");
    harness
        .get_by_role_and_label(Role::TextInput, "Path")
        .type_text(&path.to_string_lossy());
    harness.run_steps(2);
    assert_eq!(harness.state().state.settings.path, path.to_string_lossy());
}

#[test]
fn language_switch_is_immediate_preserves_data_and_survives_reload() {
    let mut harness = app_harness();
    harness.state_mut().state.settings.path = "C:\\資料\\Saved {0}".into();
    harness.state_mut().state.filter = "unchanged".into();
    navigate(&mut harness, "Settings");
    let mut previous = Language::English;
    for language in [
        Language::German,
        Language::French,
        Language::Spanish,
        Language::English,
    ] {
        click(&harness, Role::ComboBox, &previous.text("Language"));
        harness.run_steps(2);
        click(&harness, Role::Button, language.native_name());
        harness.run_steps(3);
        assert_eq!(harness.state().state.settings.language, language);
        assert!(
            harness
                .query_by_label(&language.text("Reduce motion"))
                .is_some()
        );
        assert!(
            harness
                .query_by_role_and_label(Role::Button, &language.text("Overview"))
                .is_some()
        );
        assert_eq!(harness.state().state.filter, "unchanged");
        assert_eq!(harness.state().state.settings.path, "C:\\資料\\Saved {0}");
        let saved = serde_json::to_string(&harness.state().state.settings).unwrap();
        let restored: Settings = serde_json::from_str(&saved).unwrap();
        assert_eq!(restored.language, language);
        previous = language;
    }
    let old: Settings = serde_json::from_str("{}").unwrap();
    assert_eq!(old.language, Language::System);
}

#[test]
fn primary_views_support_accessible_navigation_keyboard_and_minimum_size() {
    let mut harness = app_harness();
    assert_eq!(
        harness.ctx.zoom_factor(),
        1.25,
        "100% uses the requested former 125% base size"
    );
    for (label, page) in [
        ("CPU & Memory", Page::Cpu),
        ("GPU", Page::Gpu),
        ("Storage", Page::Storage),
        ("Overview", Page::Overview),
    ] {
        navigate(&mut harness, label);
        assert_eq!(harness.state().state.settings.page, page);
    }
    harness.key_press_modifiers(Modifiers::CTRL, Key::Num4);
    harness.run_steps(2);
    assert_eq!(harness.state().state.settings.page, Page::Storage);
    assert!(harness.query_by_label("No scan results").is_some());

    set_window_pixels(&mut harness, vec2(1080.0, 720.0));
    let viewport = Rect::from_min_size(Pos2::ZERO, vec2(1080.0, 720.0));
    let mut previous = None;
    for label in ["Overview", "CPU & Memory", "GPU", "Storage"] {
        let rect = harness.get_by_role_and_label(Role::Button, label).rect();
        assert!(
            viewport.contains_rect(rect),
            "{label} must remain inside the minimum-size window"
        );
        if let Some(bottom) = previous {
            assert!(rect.top() >= bottom, "navigation targets cannot overlap");
        }
        previous = Some(rect.bottom());
    }
    navigate(&mut harness, "GPU");
    assert_eq!(harness.state().state.settings.page, Page::Gpu);
    harness.state_mut().state.settings.scale = 1.5;
    apply_style(&harness.ctx, &harness.state().state.settings);
    harness.run_steps(3);
    set_window_pixels(&mut harness, vec2(1080.0, 720.0));
    let mut previous_bottom = None;
    for label in [
        "Overview",
        "CPU & Memory",
        "GPU",
        "Storage",
        "Diagnostics",
        "Settings",
    ] {
        let rect = harness.get_by_role_and_label(Role::Button, label).rect();
        assert!(
            viewport.contains_rect(rect),
            "{label} must remain inside the minimum-size window at maximum scale"
        );
        if let Some(bottom) = previous_bottom {
            assert!(
                rect.top() >= bottom,
                "{label} overlaps navigation at maximum scale"
            );
        }
        previous_bottom = Some(rect.bottom());
    }
    navigate(&mut harness, "Storage");
    assert_eq!(harness.state().state.settings.page, Page::Storage);
}

#[test]
fn storage_scan_drill_down_largest_files_and_filter_use_real_results() {
    let fixture = tempfile::tempdir().unwrap();
    fs::create_dir(fixture.path().join("Projects_日本語")).unwrap();
    fs::create_dir(fixture.path().join("Empty")).unwrap();
    fs::write(
        fixture.path().join("Projects_日本語").join("report.bin"),
        [7u8; 4096],
    )
    .unwrap();
    fs::write(fixture.path().join("small.txt"), [1u8; 128]).unwrap();

    let mut harness = app_harness();
    navigate(&mut harness, "Storage");
    type_scan_path(&mut harness, fixture.path());
    click(&harness, Role::Button, "Scan");
    harness.step();
    wait_for_scan(&mut harness);
    let state = &harness.state().state;
    assert_eq!(state.summary.as_ref().unwrap().errors, 0);
    assert_eq!(state.nodes[0].logical, 4224);
    assert_eq!(state.nodes[0].files, 2);
    let directory = state
        .nodes
        .iter()
        .find(|n| n.name == "Projects_日本語")
        .unwrap()
        .id;
    assert!(
        harness
            .query_by_role_and_label(Role::Button, "CSV")
            .is_some()
    );
    assert!(
        harness
            .query_by_role_and_label(Role::Button, "JSON")
            .is_some()
    );

    // Model a discovered directory whose enumeration did not finish. The table
    // must not turn its known zero lower bound into a claim that it is empty.
    let empty = harness
        .state()
        .state
        .nodes
        .iter()
        .find(|node| node.name == "Empty")
        .unwrap()
        .id;
    Arc::make_mut(&mut harness.state_mut().state.nodes)[empty].incomplete = true;
    harness.run_steps(2);
    assert!(
        harness
            .query_by_role_and_label(Role::Label, "≥ 0")
            .is_some(),
        "an incomplete directory's file count must be visibly a lower bound"
    );
    Arc::make_mut(&mut harness.state_mut().state.nodes)[empty].incomplete = false;

    // Short scans should not reserve hundreds of empty pixels. Resizing must
    // keep exports, columns and map tiles inside the page at every scale.
    for (pixels, scale) in [
        (vec2(1440., 940.), 1.0),
        (vec2(1920., 1080.), 1.0),
        (vec2(1080., 720.), 1.5),
    ] {
        harness.state_mut().state.settings.scale = scale;
        apply_style(&harness.ctx, &harness.state().state.settings);
        harness.run_steps(2);
        set_window_pixels(&mut harness, pixels);
        let ppp = harness.ctx.pixels_per_point();
        let page_right = pixels.x - 26. * ppp;
        assert_eq!(
            harness.query_all(By::new().role(Role::ScrollBar)).count(),
            1,
            "storage results must use the page scrollbar at {pixels:?}"
        );
        for label in ["CSV", "JSON"] {
            let rect = harness.get_by_role_and_label(Role::Button, label).rect();
            assert!(
                rect.right() <= page_right + 1.,
                "{label} escapes the page at {pixels:?}"
            );
        }
        for bar in harness.query_all(By::new().role(Role::ScrollBar)) {
            assert!(
                bar.rect().right() <= page_right + 1.,
                "scrollbar escapes the page at {pixels:?}: {:?}",
                bar.rect()
            );
        }
        let result = harness
            .get_by_role_and_label(Role::Label, "Projects_日本語")
            .rect();
        assert!(
            result.width() > 0. && result.height() > 0.,
            "result rows must have space at {pixels:?}"
        );
        assert!(result.right() <= page_right + 1.);
        if pixels.x == 1440. {
            let last_row = ["Empty", "Projects_日本語", "small.txt"]
                .iter()
                .map(|name| {
                    harness
                        .get_by_role_and_label(Role::Label, name)
                        .rect()
                        .bottom()
                })
                .reduce(f32::max)
                .unwrap();
            let actions = harness
                .get_by_role_and_label(Role::Button, "Copy path")
                .rect();
            assert!(
                actions.top() - last_row < 80. * ppp,
                "short result lists must not stretch into empty space"
            );
        }
        harness
            .get_by_role_and_label(Role::Button, "Map")
            .click_accesskit();
        harness.run_steps(2);
        assert!(
            harness.state().state.map_only,
            "Map activation failed at {pixels:?}, scale {scale}"
        );
        let tile = harness
            .get_by_role_and_label(Role::Button, "Projects_日本語 4.00 KiB")
            .rect();
        assert!(
            tile.right() <= page_right + 1.,
            "map tile escapes the page at {pixels:?}"
        );
        harness
            .get_by_role_and_label(Role::Button, "Hierarchy")
            .click_accesskit();
        harness.run_steps(2);
        assert!(
            !harness.state().state.map_only,
            "Hierarchy activation failed at {pixels:?}, scale {scale}"
        );
    }
    harness.state_mut().state.settings.scale = 1.0;
    apply_style(&harness.ctx, &harness.state().state.settings);
    harness.run_steps(2);
    set_window_pixels(&mut harness, vec2(1280., 920.));

    // The narrow map view can leave the outer page scrolled after resizing.
    harness.event(Event::PointerMoved(pos2(400., 35.)));
    harness.event(Event::MouseWheel {
        unit: MouseWheelUnit::Point,
        delta: vec2(0., 10_000.),
        phase: TouchPhase::Move,
        modifiers: Modifiers::default(),
    });
    harness.run_steps(20);

    assert!(
        !harness.state().state.expanded.contains(&directory),
        "resize and view switching must not expand a folder"
    );
    let toggle = harness
        .get_by_role_and_label(Role::Button, "Expand Projects_日本語")
        .rect();
    assert!(toggle.width() >= 28. * harness.ctx.pixels_per_point());
    assert!(toggle.height() >= 28. * harness.ctx.pixels_per_point());
    click(&harness, Role::Button, "Expand Projects_日本語");
    harness.run_steps(2);
    assert!(
        harness
            .query_by_role_and_label(Role::Label, "report.bin")
            .is_some()
    );
    click(&harness, Role::Button, "Collapse Projects_日本語");
    harness.run_steps(2);
    assert!(
        harness
            .query_by_role_and_label(Role::Label, "report.bin")
            .is_none()
    );

    click(&harness, Role::Label, "Projects_日本語");
    harness.run_steps(2);
    assert_eq!(harness.state().state.selected, directory);
    harness
        .get_by_role_and_label(Role::Button, "Open folder")
        .click_accesskit();
    harness.run_steps(2);
    assert_eq!(harness.state().state.scope, directory);
    assert!(
        harness
            .query_by_role_and_label(Role::Label, "report.bin")
            .is_some()
    );
    assert!(
        harness
            .query_by_role_and_label(Role::Label, "small.txt")
            .is_none()
    );

    harness.key_press_modifiers(Modifiers::ALT, Key::ArrowUp);
    harness.run_steps(2);
    assert_eq!(harness.state().state.scope, 0);
    click(&harness, Role::Button, "Largest files");
    harness.run_steps(2);
    let state = &harness.state().state;
    assert_eq!(state.visible.len(), 2);
    assert_eq!(state.nodes[state.visible[0].0].name, "report.bin");
    assert!(state.visible.iter().all(|(id, _)| !state.nodes[*id].is_dir));

    click(&harness, Role::TextInput, "Filter");
    harness
        .get_by_role_and_label(Role::TextInput, "Filter")
        .type_text("small.txt");
    harness.run_steps(2);
    assert_eq!(harness.state().state.visible.len(), 1);
    let id = harness.state().state.visible[0].0;
    assert_eq!(harness.state().state.nodes[id].name, "small.txt");
}

#[test]
fn cancelling_and_restarting_keeps_queued_old_results_out_of_new_scan() {
    let old = tempfile::tempdir().unwrap();
    let new = tempfile::tempdir().unwrap();
    // Enough entries to fill the bounded producer queue while rendering is
    // paused. This creates a real backlog without timing-dependent fake events.
    for index in 0..4096 {
        fs::write(old.path().join(format!("old-{index}.txt")), b"old").unwrap();
    }
    fs::write(new.path().join("new-only.txt"), b"new scan contents").unwrap();
    let mut harness = app_harness();
    navigate(&mut harness, "Storage");
    harness.state_mut().state.settings.path = old.path().to_string_lossy().into_owned();
    harness
        .get_by_role_and_label(Role::Button, "Scan")
        .click_accesskit();
    harness.step();
    wait_for_scan(&mut harness);
    assert_eq!(harness.state().state.nodes.len(), 4097);
    assert_eq!(
        harness.query_all(By::new().role(Role::ScrollBar)).count(),
        1
    );
    assert!(
        harness
            .query_all(By::new().role(Role::Label).label_contains("old-"))
            .count()
            < 60,
        "large scans must render only the visible rows"
    );
    harness.event(Event::PointerMoved(pos2(400., 200.)));
    harness.event(Event::MouseWheel {
        unit: MouseWheelUnit::Point,
        delta: vec2(0., -1_000_000.),
        modifiers: Modifiers::default(),
        phase: TouchPhase::Move,
    });
    harness.run_steps(20);
    assert!(
        harness
            .query_by_role_and_label(Role::Label, "old-999.txt")
            .is_some(),
        "the last sorted row must be reachable through the page scrollbar"
    );
    harness.event(Event::MouseWheel {
        unit: MouseWheelUnit::Point,
        delta: vec2(0., 1_000_000.),
        modifiers: Modifiers::default(),
        phase: TouchPhase::Move,
    });
    harness.run_steps(20);
    if harness
        .query_by_role_and_label(Role::Button, "Map")
        .is_some()
    {
        navigate(&mut harness, "Map");
        assert!(harness.state().state.map_only);
    }
    // Paint the completed treemap, then start another progressive scan. Cached
    // IDs from this large tree must not be reused against a shorter first batch.
    harness.run_steps(2);
    harness
        .get_by_role_and_label(Role::Button, "Rescan")
        .click_accesskit();
    harness.step();
    let generation = harness.state().state.scan_generation;
    let cancel = harness.state().state.scan.as_ref().unwrap().cancel.clone();
    let rescan_bounds = harness.get_by_role_and_label(Role::Button, "Rescan").rect();
    let cancel_bounds = harness.get_by_role_and_label(Role::Button, "Cancel").rect();
    assert!(
        (rescan_bounds.center().y - cancel_bounds.center().y).abs() < 2.,
        "Cancel must share the scan-action row"
    );
    assert!(cancel_bounds.right() <= 1280. - 26. * harness.ctx.pixels_per_point());
    assert!(
        harness
            .get_by_role_and_label(Role::TextInput, "Path")
            .rect()
            .width()
            <= 380. * harness.ctx.pixels_per_point()
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let receiver = &harness.state().state.scan.as_ref().unwrap().receiver;
        if receiver.len() >= receiver.capacity().unwrap() - 1 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the real scan did not produce a result backlog"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    // Hold the same brief index-throttling window present immediately after a
    // rescan; exercising it must not panic or display the previous tree's IDs.
    harness.state_mut().state.last_index = Instant::now();
    harness
        .get_by_role_and_label(Role::Button, "Cancel")
        .click_accesskit();
    harness.step();
    assert!(cancel.load(Ordering::Relaxed));

    harness.state_mut().state.settings.path = new.path().to_string_lossy().into_owned();
    harness
        .get_by_role_and_label(Role::Button, "Rescan")
        .click_accesskit();
    harness.step();
    assert_eq!(harness.state().state.scan_generation, generation + 1);
    wait_for_scan(&mut harness);
    // Keep rendering after the old worker has been released by cancellation.
    for _ in 0..5 {
        std::thread::sleep(Duration::from_millis(10));
        harness.step();
    }
    let state = &harness.state().state;
    assert!(!state.summary.as_ref().unwrap().cancelled);
    assert_eq!(state.nodes.len(), 2);
    assert_eq!(state.nodes[0].logical, 17);
    assert!(state.nodes.iter().all(|n| n.path.starts_with(new.path())));
    assert!(state.nodes.iter().any(|n| n.name == "new-only.txt"));
    assert!(!state.nodes.iter().any(|n| n.name.starts_with("old-")));
}

struct SensorFixture {
    readings: Vec<(String, Reading)>,
    history: VecDeque<Telemetry>,
}

#[test]
fn diagnostic_values_stay_clear_of_the_scrollbar_while_scrolling() {
    for (window_width, scale, theme) in [
        (1440., 1.0, "Dark"),
        (1080., 1.0, "Light"),
        (1080., 1.5, "Dark"),
    ] {
        let mut reading = Reading::missing("B", "Diagnostic layout fixture", "");
        reading.state = Availability::Failed;
        let mut harness = Harness::builder().with_size([900., 320.]).build_ui(|ui| {
            ScrollArea::vertical()
                .scroll_bar_visibility(scroll_area::ScrollBarVisibility::AlwaysVisible)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.indent("adapter", |ui| {
                        for index in 0..30 {
                            reading_diagnostic(ui, &format!("Sensor {index}"), &reading);
                        }
                    });
                });
        });
        apply_style(
            &harness.ctx,
            &Settings {
                scale,
                theme: theme.into(),
                ..Settings::default()
            },
        );
        harness.run_steps(2);
        let pixels_per_point = harness.ctx.pixels_per_point();
        // Match the content width after the app's sidebar and page margins.
        harness.set_size(vec2(window_width / pixels_per_point - 204. - 52., 320.));
        harness.run_steps(3);
        let first_top = harness.get_by_label("Sensor 0").rect().top();
        let mut value_right = None;
        for scrolling in [false, true] {
            let bar = harness.get_by_role(Role::ScrollBar).rect();
            if scrolling {
                harness.event(Event::PointerMoved(
                    (bar.center() - vec2(40., 0.)) / pixels_per_point,
                ));
                harness.event(Event::MouseWheel {
                    unit: MouseWheelUnit::Point,
                    delta: vec2(0., -200.),
                    phase: TouchPhase::Move,
                    modifiers: Modifiers::default(),
                });
                harness.run_steps(10);
                assert!(harness.get_by_label("Sensor 0").rect().top() < first_top);
            }
            // Hover expands an overlay scrollbar to its full hit-test width.
            harness.event(Event::PointerMoved(bar.center() / pixels_per_point));
            harness.run_steps(10);
            let mut visible = 0;
            for value in harness.query_all_by_label("Provider failed") {
                let rect = value.rect();
                if rect.center().y >= bar.top() && rect.center().y <= bar.bottom() {
                    visible += 1;
                    assert!(
                        rect.right() + 4. * pixels_per_point <= bar.left(),
                        "{window_width}px at {scale} ({theme}): value {rect:?} overlaps scrollbar {bar:?}"
                    );
                    if let Some(right) = value_right {
                        assert_eq!(rect.right(), right, "scrolling must not shift values");
                    }
                    value_right = Some(rect.right());
                }
            }
            assert!(visible > 0, "exercise visible diagnostic values");
        }
    }
}

#[test]
fn populated_hardware_pages_fit_the_minimum_window_at_maximum_scale() {
    // Render the real app UI against a fixed hardware snapshot; no live poll
    // may replace the fixture while checking the complete scrollable content.
    struct FrozenApp(App);
    impl eframe::App for FrozenApp {
        fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
            eframe::App::ui(&mut self.0, ui, frame);
        }
    }
    let mut harness = Harness::builder()
        .with_size([1080., 720.])
        .build_eframe(|cc| {
            let mut app = App::with_state(cc, None, None, Arc::default(), |mut settings, _| {
                settings.language = Language::English;
                State::fixture(settings)
            });
            app.state.settings.scale = 1.5;
            apply_style(&cc.egui_ctx, &app.state.settings);
            app.state.inventory = Some(Inventory {
                cpu: vec![Field::valid(
                    "Model",
                    "AMD Ryzen 7 9800X3D 8-Core Processor",
                    "",
                    "Fixture",
                )],
                adapters: vec![Adapter {
                    id: "test-adapter".into(),
                    name: "NVIDIA GeForce RTX 5070 Ti".into(),
                    vendor_id: 0x10de,
                    device_id: 0,
                    fields: vec![],
                }],
                drives: vec![Drive {
                    mount: "C:\\".into(),
                    name: "A long drive name that must stay inside the window".into(),
                    file_system: "NTFS".into(),
                    total_bytes: 1_000_000_000_000,
                    free_bytes: 600_000_000_000,
                }],
                ..Inventory::default()
            });
            let mut sample = sensor_sample(50., now_ms());
            sample.gpus[0]
                .readings
                .push(("Temperature".into(), Reading::valid(51., "°C", "Fixture")));
            for index in 0..16 {
                sample.gpus[0].readings.push((
                    format!("Memory sensor {index}"),
                    Reading::valid(4096., "B", "Fixture"),
                ));
                sample.gpus[0].readings.push((
                    format!("Engine {index} clock"),
                    Reading::valid(1500., "MHz", "Fixture"),
                ));
            }
            sample.per_core_usage = vec![Reading::valid(50., "%", "Fixture"); 16];
            app.state.history.push_back(sample);
            FrozenApp(app)
        });
    harness.run_steps(2);
    let drive = harness.state().0.state.inventory.as_ref().unwrap().drives[0].clone();
    harness
        .state_mut()
        .0
        .state
        .inventory
        .as_mut()
        .unwrap()
        .drives = ["C:\\", "D:\\", "E:\\", "G:\\"]
        .into_iter()
        .map(|mount| Drive {
            mount: mount.into(),
            ..drive.clone()
        })
        .collect();
    let ppp = harness.ctx.pixels_per_point();
    harness.set_size(vec2(1080., 720.) / ppp);
    for page in [Page::Overview, Page::Cpu, Page::Gpu] {
        harness.state_mut().0.state.settings.page = page;
        harness.run_steps(3);
        let bars: Vec<_> = harness.query_all(By::new().role(Role::ScrollBar)).collect();
        assert_eq!(bars.len(), 1, "{page:?} must use only the page scrollbar");
        for bar in bars {
            assert!(
                bar.rect().right() <= 1080. - 26. * ppp + 1.,
                "{page:?} content pushes its scrollbar out of the page: {:?}",
                bar.rect()
            );
        }
        if page == Page::Gpu {
            assert!(harness.query_by_label("Min 50.0 %").is_some());
            assert!(harness.query_by_label("Max 50.0 %").is_some());
        }
        if page == Page::Overview {
            let usage = harness
                .get_by_role_and_label(Role::Label, "GPU: 50.0 %")
                .rect();
            let temperature = harness
                .get_by_role_and_label(Role::Label, "Temperature: 51.0 °C")
                .rect();
            assert!(temperature.left() > usage.right());
            assert!((temperature.center().y - usage.center().y).abs() < 1.);
        }
    }
    harness.state_mut().0.state.settings.page = Page::Overview;
    harness.state_mut().0.state.settings.scale = 1.0;
    apply_style(&harness.ctx, &harness.state().0.state.settings);
    harness.run_steps(2);
    harness.set_size(vec2(1440., 940.) / harness.ctx.pixels_per_point());
    harness.run_steps(3);
    assert_eq!(
        harness.query_all(By::new().role(Role::ScrollBar)).count(),
        1,
        "the wide drive table must grow to fit every drive without an inner scrollbar"
    );
    let usage = harness
        .get_by_role_and_label(Role::Label, "GPU: 50.0 %")
        .rect();
    let temperature = harness
        .get_by_role_and_label(Role::Label, "Temperature: 51.0 °C")
        .rect();
    assert!(temperature.left() > usage.right());
    assert!((temperature.center().y - usage.center().y).abs() < 1.);
    harness.state_mut().0.state.history.back_mut().unwrap().gpus[0].readings[0]
        .1
        .value = Some(100.);
    harness.state_mut().0.state.history.back_mut().unwrap().gpus[0].readings[1].1 =
        Reading::missing("°C", "Fixture", "Offline");
    harness.run_steps(2);
    assert_eq!(
        usage,
        harness
            .get_by_role_and_label(Role::Label, "GPU: 100.0 %")
            .rect()
    );
    assert_eq!(
        temperature,
        harness
            .get_by_role_and_label(Role::Label, "Temperature: Unavailable")
            .rect()
    );

    harness.state_mut().0.state.settings.page = Page::Gpu;
    harness.run_steps(3);
    harness
        .get_by_role_and_label(Role::Button, "Driver counters & process memory")
        .click_accesskit();
    harness.run_steps(12);
    assert_eq!(
        harness.query_all(By::new().role(Role::ScrollBar)).count(),
        1,
        "both long sensor tables must grow with the page, including expanded driver readings"
    );
    harness.event(Event::PointerMoved(pos2(450., 200.)));
    harness.event(Event::MouseWheel {
        unit: MouseWheelUnit::Point,
        delta: vec2(0., -1_000_000.),
        phase: TouchPhase::Move,
        modifiers: Modifiers::default(),
    });
    harness.run_steps(20);
    let last_sensor = harness
        .get_by_role_and_label(Role::Label, "Engine 15 clock")
        .rect();
    assert!(
        last_sensor.top() >= 0. && last_sensor.bottom() <= 940.,
        "last sensor must be visible after scrolling the page"
    );
    for language in [Language::German, Language::French, Language::Spanish] {
        harness.state_mut().0.state.settings.language = language;
        harness.state_mut().0.state.settings.scale = 1.5;
        apply_style(&harness.ctx, &harness.state().0.state.settings);
        harness.run_steps(2);
        harness.set_size(vec2(1080., 720.) / harness.ctx.pixels_per_point());
        for page in [
            Page::Overview,
            Page::Cpu,
            Page::Gpu,
            Page::Storage,
            Page::Diagnostics,
            Page::Settings,
        ] {
            harness.state_mut().0.state.settings.page = page;
            harness.run_steps(3);
            let bars: Vec<_> = harness.query_all(By::new().role(Role::ScrollBar)).collect();
            assert_eq!(
                bars.len(),
                1,
                "{language:?} {page:?}: only the page scrollbar"
            );
            assert!(
                bars[0].rect().right() <= 1080. - 26. * harness.ctx.pixels_per_point() + 1.,
                "{language:?} {page:?} overflows the window: {:?}",
                bars[0].rect()
            );
            let nav = harness
                .get_by_role_and_label(Role::Button, &language.text("CPU & Memory"))
                .rect();
            assert!(nav.left() >= 0. && nav.right() < 1080.);
        }
    }
}

fn sensor_sample(value: f64, timestamp: u64) -> Telemetry {
    let missing = || Reading::missing("%", "UI test fixture", "Unused by this component test");
    Telemetry {
        timestamp_ms: timestamp,
        cpu_usage: missing(),
        per_core_usage: vec![],
        memory_used: missing(),
        memory_total: missing(),
        cpu_frequency: missing(),
        gpus: vec![GpuSample {
            adapter_id: "test-adapter".into(),
            readings: vec![(
                "Utilization".into(),
                Reading::valid(value, "%", "UI test fixture"),
            )],
        }],
    }
}

fn sensor_value_rects(harness: &Harness<'_, SensorFixture>) -> Vec<Rect> {
    let mut values: Vec<_> = harness
        .query_all(By::new().role(Role::Label).label_contains("%"))
        .map(|node| node.rect())
        .collect();
    if let Some(missing) = harness.query_by_role_and_label(Role::Label, "Unavailable") {
        values.push(missing.rect());
    }
    values.sort_by(|left, right| left.left().total_cmp(&right.left()));
    assert_eq!(
        values.len(),
        3,
        "current, minimum and maximum must have separate accessible values"
    );
    values
}

#[test]
fn sensor_columns_stay_fixed_for_zero_full_scale_and_unavailable_values() {
    let now = now_ms();
    let fixture = SensorFixture {
        readings: vec![(
            "Utilization".into(),
            Reading::valid(0.0, "%", "UI test fixture"),
        )],
        history: VecDeque::from([
            sensor_sample(9999.0, now.saturating_sub(121_000)),
            sensor_sample(0.0, now),
            sensor_sample(99.0, now),
        ]),
    };
    let mut harness = Harness::builder().with_size([820.0, 230.0]).build_ui_state(
        |ui, fixture: &mut SensorFixture| {
            let readings: Vec<_> = fixture.readings.iter().collect();
            sensor_table(
                ui,
                "sensor-layout",
                &readings,
                &fixture.history,
                "test-adapter",
            );
        },
        fixture,
    );
    harness.run_steps(2);
    let baseline = sensor_value_rects(&harness);
    let sensor = harness
        .get_by_role_and_label(Role::Label, "Utilization")
        .rect();
    assert!(
        harness
            .query_by_role_and_label(Role::Label, "99.0 %")
            .is_some()
    );
    assert!(
        harness.query_by_label("9999.0 %").is_none(),
        "samples outside 120s must not affect extrema"
    );

    harness.state_mut().readings[0].1 = Reading::valid(100.0, "%", "UI test fixture");
    harness
        .state_mut()
        .history
        .push_back(sensor_sample(100.0, now_ms()));
    harness.run_steps(2);
    let full = sensor_value_rects(&harness);
    assert_eq!(
        baseline, full,
        "numeric digit growth must not shift or resize sensor columns"
    );
    assert_eq!(
        sensor,
        harness
            .get_by_role_and_label(Role::Label, "Utilization")
            .rect()
    );
    assert_eq!(
        harness.query_all_by_label("100.0 %").count(),
        2,
        "current and maximum reflect the new sample"
    );

    harness.state_mut().readings[0].1 =
        Reading::missing("%", "UI test fixture", "Provider temporarily unavailable");
    harness.run_steps(2);
    assert_eq!(
        baseline,
        sensor_value_rects(&harness),
        "availability changes must preserve the numeric columns"
    );
    assert!(
        harness
            .query_by_role_and_label(Role::Label, "Unavailable")
            .is_some()
    );

    let mut stale = Reading::valid(100.0, "%", "UI test fixture");
    stale.mark_stale(stale.timestamp_ms + 5_001);
    harness.state_mut().readings[0].1 = stale;
    harness.run_steps(2);
    assert!(
        harness
            .query_by_role_and_label(Role::Label, "Stale")
            .is_some(),
        "a retained reading must not masquerade as a current measurement"
    );
    assert_eq!(
        harness.query_all_by_label("100.0 %").count(),
        1,
        "retain the historical maximum while marking the current value stale"
    );
}

fn capture_harness(directory: &Path, failed: Arc<AtomicBool>) -> Harness<'static, App> {
    let directory = directory.to_owned();
    Harness::builder()
        .with_size([1280., 920.])
        .with_max_steps(2)
        .build_eframe(move |cc| {
            App::with_state(cc, None, Some(directory), failed, |mut settings, _| {
                settings.language = Language::English;
                State::fixture(settings)
            })
        })
}

#[test]
fn interrupted_capture_is_not_reported_as_success() {
    let directory = tempfile::tempdir().unwrap();
    let failed = Arc::new(AtomicBool::new(false));
    let harness = capture_harness(directory.path(), failed.clone());
    drop(harness);
    assert!(
        failed.load(Ordering::Relaxed),
        "closing before PNG completion must fail the capture command"
    );
    let ordinary = app_harness();
    assert!(!ordinary.state().capture_failed.load(Ordering::Relaxed));
}

#[test]
fn capture_succeeds_only_after_five_pngs_are_written() {
    let directory = tempfile::tempdir().unwrap();
    let failed = Arc::new(AtomicBool::new(false));
    let mut harness = capture_harness(directory.path(), failed.clone());
    harness.state_mut().state.inventory = Some(Inventory::default());
    harness.state_mut().state.history = VecDeque::from([
        sensor_sample(0., now_ms()),
        sensor_sample(1., now_ms()),
        sensor_sample(2., now_ms()),
    ]);
    harness.state_mut().capture_at = Instant::now() - Duration::from_secs(3);
    harness.step();
    assert!(
        harness.state().capture_pending.is_none(),
        "screenshots must wait for the live history warmup"
    );
    for index in 0..5 {
        harness.state_mut().capture_at = Instant::now()
            - if index == 0 {
                CAPTURE_WARMUP + Duration::from_secs(1)
            } else {
                Duration::from_secs(3)
            };
        harness.step();
        let path = harness
            .state()
            .capture_pending
            .clone()
            .expect("capture requested screenshot pixels");
        assert!(
            failed.load(Ordering::Relaxed),
            "an incomplete capture must remain unsuccessful"
        );
        harness.event(Event::Screenshot {
            viewport_id: ViewportId::ROOT,
            user_data: Default::default(),
            image: Arc::new(ColorImage::filled([2, 2], Color32::WHITE)),
        });
        harness.step();
        let deadline = Instant::now() + Duration::from_secs(10);
        while harness.state().capture_worker.is_some() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
            harness.step();
        }
        assert!(
            harness.state().capture_worker.is_none(),
            "PNG worker did not finish"
        );
        let png = image::open(&path).expect("capture must publish a decodable PNG");
        assert_eq!((png.width(), png.height()), (2, 2));
        assert_eq!(failed.load(Ordering::Relaxed), index != 4);
    }
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 5);
    harness.state_mut().state.hardware_error =
        Some("Provider stopped after capture completed".into());
    harness.step();
    assert!(
        !failed.load(Ordering::Relaxed),
        "a later provider failure cannot invalidate five successfully written images"
    );
    drop(harness);
    assert!(!failed.load(Ordering::Relaxed));
}

#[test]
fn capture_reports_missing_hardware_and_missing_screenshot_events() {
    let directory = tempfile::tempdir().unwrap();
    let failed = Arc::new(AtomicBool::new(false));
    let mut harness = capture_harness(directory.path(), failed.clone());
    harness.state_mut().capture_at = Instant::now() - CAPTURE_TIMEOUT - Duration::from_secs(1);
    harness.step();
    assert!(
        harness
            .state()
            .state
            .notice
            .as_deref()
            .is_some_and(|text| text.contains("hardware samples"))
    );
    assert!(failed.load(Ordering::Relaxed));

    harness.state_mut().capture_pending = Some(directory.path().join("never-returned.png"));
    harness.state_mut().capture_at = Instant::now() - CAPTURE_TIMEOUT - Duration::from_secs(1);
    harness.step();
    assert!(
        harness
            .state()
            .state
            .notice
            .as_deref()
            .is_some_and(|text| text.contains("screenshot pixels"))
    );
    assert!(!directory.path().join("never-returned.png").exists());
}

#[test]
fn stale_chart_status_is_exposed_to_assistive_technology() {
    let points = vec![(now_ms().saturating_sub(6_000), Some(42.0))];
    let mut harness = Harness::builder()
        .with_size([420., 250.])
        .build_ui(move |ui| {
            metric_chart(ui, "CPU", &points, "%", 100., CPU, 166.);
        });
    harness.run_steps(2);
    assert!(
        harness
            .query_by_role_and_label(Role::Label, "CPU: 42.0 % (stale)")
            .is_some()
    );
}
