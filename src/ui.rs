use crate::{
    i18n::{self, Language, message as td, text as tx},
    model::*,
    state::{Page, Settings, Sort, State},
};
use eframe::egui::{self, *};
use egui_extras::{Column, TableBuilder};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const CPU: Color32 = Color32::from_rgb(181, 207, 188);
const MEMORY: Color32 = Color32::from_rgb(162, 188, 216);
const GPU: Color32 = Color32::from_rgb(217, 195, 150);
const BASE_SCALE: f32 = 1.25;
const CAPTURE_WARMUP: Duration = Duration::from_secs(120);
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(180);
fn muted(ui: &Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(167, 167, 171)
    } else {
        Color32::from_rgb(96, 96, 101)
    }
}

pub struct App {
    pub state: State,
    capture_dir: Option<PathBuf>,
    capture_index: usize,
    capture_at: Instant,
    capture_pending: Option<PathBuf>,
    capture_worker: Option<std::thread::JoinHandle<Result<(), String>>>,
    capture_failed: Arc<AtomicBool>,
    map_cache: Vec<(usize, Rect)>,
    map_key: Option<(u64, Rect)>,
    cpu_tab: usize,
    window_override: Option<Vec2>,
}
impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        scan: Option<String>,
        capture_dir: Option<PathBuf>,
        capture_failed: Arc<AtomicBool>,
    ) -> Self {
        Self::with_state(cc, scan, capture_dir, capture_failed, State::new)
    }

    fn with_state(
        cc: &eframe::CreationContext<'_>,
        scan: Option<String>,
        capture_dir: Option<PathBuf>,
        capture_failed: Arc<AtomicBool>,
        create_state: impl FnOnce(Settings, Context) -> State,
    ) -> Self {
        // Completion is acknowledged only after all five image writes succeed.
        // Closing the window during any earlier phase must not report success.
        capture_failed.store(capture_dir.is_some(), Ordering::Relaxed);
        let mut settings: Settings = cc
            .storage
            .and_then(|s| eframe::get_value(s, "settings"))
            .unwrap_or_default();
        if settings.scale_revision == 0 {
            settings.scale = 1.0;
            settings.scale_revision = 1;
        }
        settings.scale = finite_scale(settings.scale);
        let args: Vec<String> = std::env::args().collect();
        if let Some(value) = args.windows(2).find(|a| a[0] == "--theme") {
            settings.theme = value[1].clone();
        }
        if let Some(value) = args
            .windows(2)
            .find(|a| a[0] == "--scale")
            .and_then(|a| a[1].parse::<f32>().ok())
        {
            settings.scale = finite_scale(value);
        }
        let mut fonts = FontDefinitions::default();
        if let Ok(font) = std::fs::read("C:/Windows/Fonts/segoeui.ttf") {
            fonts
                .font_data
                .insert("Segoe UI".into(), FontData::from_owned(font).into());
            fonts
                .families
                .get_mut(&FontFamily::Proportional)
                .unwrap()
                .insert(0, "Segoe UI".into());
        }
        // Use installed Windows glyphs for CJK file names without redistributing fonts.
        for candidate in [
            "C:/Windows/Fonts/msgothic.ttc",
            "C:/Windows/Fonts/msyh.ttc",
            "C:/Windows/Fonts/malgun.ttf",
        ] {
            if let Ok(font) = std::fs::read(candidate) {
                fonts
                    .font_data
                    .insert("System CJK".into(), FontData::from_owned(font).into());
                for family in [FontFamily::Proportional, FontFamily::Monospace] {
                    fonts
                        .families
                        .get_mut(&family)
                        .unwrap()
                        .push("System CJK".into());
                }
                break;
            }
        }
        if let Some(value) =
            crate::argument(&args, "--language").and_then(|code| Language::parse(&code))
        {
            settings.language = value;
        }
        i18n::set_context(&cc.egui_ctx, settings.language);
        cc.egui_ctx.set_fonts(fonts);
        apply_style(&cc.egui_ctx, &settings);
        let mut state = create_state(settings, cc.egui_ctx.clone());
        if let Some(path) = scan {
            state.settings.path = path;
            state.start_scan();
        }
        if capture_dir.is_some() {
            state.settings.page = Page::Overview;
        }
        Self {
            state,
            window_override: crate::requested_window_size(&args)
                .or_else(|| capture_dir.as_ref().map(|_| vec2(1440., 940.))),
            capture_dir,
            capture_index: 0,
            capture_at: Instant::now(),
            capture_pending: None,
            capture_worker: None,
            capture_failed,
            map_cache: vec![],
            map_key: None,
            cpu_tab: 0,
        }
    }
    fn sidebar(&mut self, ui: &mut Ui) {
        let compact = ui.available_height() < 580.;
        let dense = ui.available_height() < 430.;
        ui.add_space(if dense { 0. } else { 10. });
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(30., 30.), Sense::hover());
            crate::branding::paint(ui.painter(), r);
            ui.label(RichText::new("Rigometry").size(19.).strong());
        });
        ui.add_space(if dense {
            8.
        } else if compact {
            18.
        } else {
            34.
        });
        for (page, name, key, kind) in [
            (Page::Overview, "Overview", "1", 0),
            (Page::Cpu, "CPU & Memory", "2", 1),
            (Page::Gpu, "GPU", "3", 2),
            (Page::Storage, "Storage", "4", 3),
        ] {
            if navigation(ui, name, key, self.state.settings.page == page, kind) {
                self.state.settings.page = page;
            }
            ui.add_space(if dense { 2. } else { 6. });
        }
        ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
            if !dense {
                ui.add_space(8.);
                ui.label(
                    RichText::new(concat!("v", env!("CARGO_PKG_VERSION"), "  ·  Windows x64"))
                        .size(12.)
                        .color(muted(ui)),
                );
                ui.add_space(16.);
            }
            if navigation(
                ui,
                "Settings",
                "",
                self.state.settings.page == Page::Settings,
                5,
            ) {
                self.state.settings.page = Page::Settings;
            }
            ui.add_space(6.);
            if navigation(
                ui,
                "Diagnostics",
                "",
                self.state.settings.page == Page::Diagnostics,
                4,
            ) {
                self.state.settings.page = Page::Diagnostics;
            }
            if compact {
                return;
            }
            if let Some(s) = self.state.history.back() {
                if now_ms().saturating_sub(s.timestamp_ms) > 5000 {
                    ui.add_space(22.);
                    ui.label(
                        RichText::new(tx(ui, "Telemetry stale"))
                            .size(12.)
                            .color(GPU),
                    );
                }
            } else {
                ui.add_space(22.);
                ui.label(
                    RichText::new(tx(ui, "Reading hardware…"))
                        .size(12.)
                        .color(muted(ui)),
                );
            }
        });
    }
    fn overview(&mut self, ui: &mut Ui) {
        header(ui, "Overview", "Hardware and current activity");
        let Some(inv) = self.state.inventory.clone() else {
            loading(ui, &self.state);
            return;
        };
        let cpu = field_value(ui, &inv.cpu, "Model");
        let gpu = inv
            .adapters
            .iter()
            .filter(|a| a.vendor_id != 0x1414)
            .map(|a| a.name.clone())
            .collect::<Vec<_>>()
            .join(" / ");
        ui.add_space(8.);
        let mut pc = |ui: &mut Ui| {
            section(ui, "This PC", |ui| {
                summary_row(
                    ui,
                    "Processor",
                    &cpu,
                    Some((Page::Cpu, &mut self.state.settings.page)),
                );
                summary_row(
                    ui,
                    "Graphics",
                    if gpu.is_empty() {
                        "No graphics adapter reported"
                    } else {
                        &gpu
                    },
                    Some((Page::Gpu, &mut self.state.settings.page)),
                );
                summary_row(
                    ui,
                    "Memory",
                    &field_value(ui, &inv.memory, "Installed capacity"),
                    Some((Page::Cpu, &mut self.state.settings.page)),
                );
            });
        };
        let system = |ui: &mut Ui| {
            section(ui, "System", |ui| {
                summary_row(
                    ui,
                    "Motherboard",
                    &format!(
                        "{} {}",
                        field_value(ui, &inv.motherboard, "Manufacturer"),
                        field_value(ui, &inv.motherboard, "Model")
                    ),
                    None,
                );
                let os = inv
                    .os
                    .iter()
                    .filter_map(|f| f.value.as_ref())
                    .take(2)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" · ");
                summary_row(ui, "Windows", &os, None);
                summary_row(
                    ui,
                    "BIOS",
                    &field_value(ui, &inv.motherboard, "BIOS version"),
                    None,
                );
            });
        };
        if ui.available_width() < 680. {
            pc(ui);
            ui.add_space(18.);
            system(ui);
        } else {
            ui.columns(2, |cols| {
                pc(&mut cols[0]);
                system(&mut cols[1]);
            });
        }
        ui.add_space(26.);
        ui.horizontal(|ui| {
            ui.label(RichText::new(tx(ui, "Live activity")).size(18.));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new(tx(ui, "120 s window"))
                        .color(muted(ui))
                        .size(12.),
                );
            });
        });
        ui.add_space(12.);
        let cpu_points: Vec<_> = self
            .state
            .history
            .iter()
            .map(|s| (s.timestamp_ms, s.cpu_usage.value))
            .collect();
        let memory_points: Vec<_> = self
            .state
            .history
            .iter()
            .map(|s| (s.timestamp_ms, memory_percent(s)))
            .collect();
        let primary_gpu = inv.adapters.iter().find(|a| a.vendor_id != 0x1414);
        let gpu_id = primary_gpu.map(|a| a.id.as_str()).unwrap_or("");
        let gpu_points: Vec<_> = self
            .state
            .history
            .iter()
            .map(|s| {
                (
                    s.timestamp_ms,
                    gpu_reading(s, gpu_id, "Utilization").and_then(|r| r.value),
                )
            })
            .collect();
        let gpu_chart = |ui: &mut Ui| {
            let reading = self
                .state
                .history
                .back()
                .and_then(|s| gpu_reading(s, gpu_id, "Temperature"));
            MetricChart {
                label: "GPU",
                points: &gpu_points,
                unit: "%",
                max: 100.,
                color: GPU,
                height: 166.,
                secondary: Some(("Temperature", reading)),
            }
            .show(ui);
        };
        if ui.available_width() < 780. {
            metric_chart(ui, "CPU", &cpu_points, "%", 100., CPU, 166.);
            ui.add_space(12.);
            metric_chart(ui, "Memory", &memory_points, "%", 100., MEMORY, 166.);
            ui.add_space(12.);
            gpu_chart(ui);
        } else {
            ui.columns(3, |cols| {
                metric_chart(&mut cols[0], "CPU", &cpu_points, "%", 100., CPU, 166.);
                metric_chart(
                    &mut cols[1],
                    "Memory",
                    &memory_points,
                    "%",
                    100.,
                    MEMORY,
                    166.,
                );
                gpu_chart(&mut cols[2]);
            });
        }
        ui.add_space(24.);
        ui.horizontal(|ui| {
            ui.label(RichText::new(tx(ui, "Drives")).size(18.));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button(tx(ui, "Scan folder…")).clicked() {
                    self.state.settings.page = Page::Storage;
                    self.state.pick_folder();
                }
            });
        });
        ui.add_space(10.);

        let mut drives = inv.drives.clone();
        drives.sort_by(|a, b| a.mount.cmp(&b.mount));
        panel(ui, |ui| {
            let scan_width = label_width(ui, &tx(ui, "Scan"), 76.);
            if ui.available_width() < 460. {
                for (index, drive) in drives.iter().enumerate() {
                    if index > 0 {
                        ui.separator();
                    }
                    let used = drive.total_bytes.saturating_sub(drive.free_bytes);
                    ui.horizontal(|ui| {
                        let name_width = (ui.available_width() - scan_width - 10.).max(0.);
                        ui.add_sized(
                            [name_width, 31.],
                            Label::new(format!("{}  {}", drive.mount, drive.name)).truncate(),
                        )
                        .on_hover_text(&drive.name);
                        if ui
                            .add_sized([scan_width, 31.], Button::new(tx(ui, "Scan")))
                            .clicked()
                        {
                            self.state.settings.path = drive.mount.clone();
                            self.state.settings.page = Page::Storage;
                            self.state.start_scan();
                        }
                    });
                    ui.label(
                        RichText::new(td(
                            ui,
                            format!(
                                "{} · {}",
                                drive.file_system,
                                ui_bytes(ui, drive.total_bytes)
                            ),
                        ))
                        .size(12.)
                        .color(muted(ui)),
                    );
                    ui.label(
                        RichText::new(td(
                            ui,
                            format!(
                                "{} used · {} free",
                                ui_bytes(ui, used),
                                ui_bytes(ui, drive.free_bytes)
                            ),
                        ))
                        .size(12.)
                        .monospace(),
                    );
                    let ratio = if drive.total_bytes > 0 {
                        used as f32 / drive.total_bytes as f32
                    } else {
                        0.
                    };
                    ui.add(
                        ProgressBar::new(ratio)
                            .desired_width(ui.available_width())
                            .desired_height(4.)
                            .fill(Color32::from_gray(155)),
                    );
                    ui.add_space(8.);
                }
                return;
            }
            let compact = ui.available_width() < 680.;
            let drive_width = if compact { 106. } else { 170. };
            let free_width = if compact { 94. } else { 102. };
            let mut table = TableBuilder::new(ui)
                .id_salt(("drive-overview", compact))
                .vscroll(false)
                .vertical_scroll_offset(0.)
                .scroll_bar_visibility(scroll_area::ScrollBarVisibility::AlwaysHidden)
                .cell_layout(Layout::left_to_right(Align::Center))
                .column(Column::exact(drive_width))
                .column(Column::remainder().at_least(72.))
                .column(Column::exact(free_width));
            if !compact {
                table = table.column(Column::exact(102.));
            }
            table
                .column(Column::exact(scan_width))
                .header(28., |mut row| {
                    for label in if compact {
                        vec!["Drive", "Used", "Free", ""]
                    } else {
                        vec!["Drive", "Used", "Free", "Capacity", ""]
                    } {
                        row.col(|ui| {
                            ui.label(RichText::new(tx(ui, label)).size(12.).color(muted(ui)));
                        });
                    }
                })
                .body(|mut body| {
                    for drive in &drives {
                        let used = drive.total_bytes.saturating_sub(drive.free_bytes);
                        body.row(62., |mut row| {
                            row.col(|ui| {
                                ui.vertical(|ui| {
                                    ui.add(
                                        Label::new(format!("{}  {}", drive.mount, drive.name))
                                            .truncate(),
                                    )
                                    .on_hover_text(td(
                                        ui,
                                        format!(
                                            "{}\nCapacity: {}\n{}",
                                            drive.name,
                                            ui_bytes(ui, drive.total_bytes),
                                            drive.mount
                                        ),
                                    ));
                                    ui.label(
                                        RichText::new(&drive.file_system)
                                            .size(11.)
                                            .color(muted(ui)),
                                    );
                                });
                            });
                            row.col(|ui| {
                                let width = ui.available_width();
                                ui.allocate_ui_with_layout(
                                    vec2(width, 40.),
                                    Layout::top_down(Align::Min),
                                    |ui| {
                                        ui.label(
                                            RichText::new(ui_bytes(ui, used)).size(13.).monospace(),
                                        );
                                        let ratio = if drive.total_bytes > 0 {
                                            used as f32 / drive.total_bytes as f32
                                        } else {
                                            0.
                                        };
                                        ui.add(
                                            ProgressBar::new(ratio)
                                                .desired_height(4.)
                                                .desired_width(width)
                                                .fill(Color32::from_gray(155)),
                                        );
                                    },
                                );
                            });
                            row.col(|ui| {
                                numeric_value(
                                    ui,
                                    &ui_bytes(ui, drive.free_bytes),
                                    free_width - 4.,
                                    12.,
                                );
                            });
                            if !compact {
                                row.col(|ui| {
                                    numeric_value(ui, &ui_bytes(ui, drive.total_bytes), 98., 12.);
                                });
                            }
                            row.col(|ui| {
                                if ui
                                    .add_sized([scan_width - 4., 31.], Button::new(tx(ui, "Scan")))
                                    .clicked()
                                {
                                    self.state.settings.path = drive.mount.clone();
                                    self.state.settings.page = Page::Storage;
                                    self.state.start_scan();
                                }
                            });
                        });
                    }
                });
        });
        ui.add_space(8.);
        ui.label(
            RichText::new(
                "Drive space is a volume snapshot; scan totals exclude filesystem overhead.",
            )
            .size(12.)
            .color(muted(ui)),
        );
    }

    fn cpu(&mut self, ui: &mut Ui) {
        header(ui, "CPU & Memory", "");
        let Some(inv) = self.state.inventory.clone() else {
            loading(ui, &self.state);
            return;
        };
        ui.label(RichText::new(field_value(ui, &inv.cpu, "Model")).size(19.));
        ui.add_space(16.);
        let cpu_points: Vec<_> = self
            .state
            .history
            .iter()
            .map(|s| (s.timestamp_ms, s.cpu_usage.value))
            .collect();
        let mem_points: Vec<_> = self
            .state
            .history
            .iter()
            .map(|s| (s.timestamp_ms, memory_percent(s)))
            .collect();
        if ui.available_width() < 600. {
            metric_chart(ui, "CPU utilization", &cpu_points, "%", 100., CPU, 142.);
            ui.add_space(12.);
            metric_chart(
                ui,
                "Memory utilization",
                &mem_points,
                "%",
                100.,
                MEMORY,
                142.,
            );
        } else {
            ui.columns(2, |cols| {
                metric_chart(
                    &mut cols[0],
                    "CPU utilization",
                    &cpu_points,
                    "%",
                    100.,
                    CPU,
                    142.,
                );
                metric_chart(
                    &mut cols[1],
                    "Memory utilization",
                    &mem_points,
                    "%",
                    100.,
                    MEMORY,
                    142.,
                );
            });
        }
        ui.add_space(22.);
        ui.horizontal_wrapped(|ui| {
            for (id, name) in ["Processor", "Memory", "Mainboard"].iter().enumerate() {
                if ui
                    .add_sized(
                        [label_width(ui, &tx(ui, name), 104.), 34.],
                        Button::new(tx(ui, *name))
                            .selected(self.cpu_tab == id)
                            .frame(self.cpu_tab == id),
                    )
                    .clicked()
                {
                    self.cpu_tab = id;
                }
            }
        });
        ui.add_space(14.);
        match self.cpu_tab {
            0 => {
                let processor = |ui: &mut Ui| {
                    panel(ui, |ui| {
                        section(ui, "Processor", |ui| {
                            let primary: Vec<_> = inv
                                .cpu
                                .iter()
                                .filter(|f| {
                                    f.state == Availability::Valid
                                        && ![
                                            "Model",
                                            "Instruction sets (hardware)",
                                            "Temperature",
                                            "Core voltage",
                                            "Package power",
                                        ]
                                        .contains(&f.label.as_str())
                                })
                                .cloned()
                                .collect();
                            field_table(ui, "cpu-fields", &primary);
                        });
                        folding(ui, tx(ui, "Instruction sets & sensor availability")).show(
                            ui,
                            |ui| {
                                let fields: Vec<_> = inv
                                    .cpu
                                    .iter()
                                    .filter(|f| {
                                        f.state != Availability::Valid
                                            || [
                                                "Instruction sets (hardware)",
                                                "Temperature",
                                                "Core voltage",
                                                "Package power",
                                            ]
                                            .contains(&f.label.as_str())
                                    })
                                    .cloned()
                                    .collect();
                                field_table(ui, "cpu-extra", &fields);
                            },
                        );
                    });
                };
                let activity = |ui: &mut Ui| {
                    panel(ui, |ui| {
                        ui.label(RichText::new(tx(ui, "Logical processors")).size(16.));
                        ui.add_space(15.);
                        if let Some(sample) = self.state.history.back() {
                            let columns = if ui.available_width() > 390. { 4 } else { 2 };
                            let cell_width = (ui.available_width() - (columns - 1) as f32 * 14.)
                                / columns as f32;
                            Grid::new("core-grid")
                                .num_columns(columns)
                                .spacing(vec2(14., 15.))
                                .show(ui, |ui| {
                                    for (i, r) in sample.per_core_usage.iter().enumerate() {
                                        ui.allocate_ui_with_layout(
                                            vec2(cell_width, 44.),
                                            Layout::top_down(Align::Min),
                                            |ui| {
                                                ui.set_max_width(cell_width);
                                                let (rect, response) = ui.allocate_exact_size(
                                                    vec2(cell_width, 22.),
                                                    Sense::hover(),
                                                );
                                                ui.painter().text(
                                                    rect.left_center(),
                                                    Align2::LEFT_CENTER,
                                                    format!("CPU {i:02}"),
                                                    FontId::proportional(11.),
                                                    muted(ui),
                                                );
                                                let value = if r.state == Availability::Valid {
                                                    r.value.map(|v| format!("{v:.0}%"))
                                                } else {
                                                    None
                                                }
                                                .unwrap_or_else(|| {
                                                    if r.state == Availability::Stale {
                                                        tx(ui, "Stale")
                                                    } else {
                                                        "—".into()
                                                    }
                                                });
                                                ui.painter().text(
                                                    rect.right_center(),
                                                    Align2::RIGHT_CENTER,
                                                    &value,
                                                    FontId::monospace(12.),
                                                    ui.visuals().text_color(),
                                                );
                                                response.widget_info(|| {
                                                    WidgetInfo::labeled(
                                                        WidgetType::Label,
                                                        true,
                                                        format!("CPU {i:02}: {value}"),
                                                    )
                                                });
                                                if let Some(value) = r.value
                                                    && r.state == Availability::Valid
                                                {
                                                    let display =
                                                        if self.state.settings.reduce_motion {
                                                            value as f32 / 100.
                                                        } else {
                                                            ui.ctx().animate_value_with_time(
                                                                Id::new(("core", i)),
                                                                value as f32 / 100.,
                                                                0.16,
                                                            )
                                                        };
                                                    ui.add(
                                                        ProgressBar::new(display)
                                                            .desired_width(cell_width)
                                                            .desired_height(4.)
                                                            .fill(CPU),
                                                    );
                                                }
                                            },
                                        );
                                        if (i + 1) % columns == 0 {
                                            ui.end_row();
                                        }
                                    }
                                });
                            ui.add_space(18.);
                            ui.separator();
                            ui.add_space(12.);
                            reading_row(ui, "OS-reported clock", &sample.cpu_frequency);
                        } else {
                            ui.label(tx(ui, "Waiting for processor samples…"));
                        }
                    });
                };
                if ui.available_width() < 680. {
                    processor(ui);
                    ui.add_space(16.);
                    activity(ui);
                } else {
                    ui.columns(2, |cols| {
                        processor(&mut cols[0]);
                        activity(&mut cols[1]);
                    });
                }
            }
            1 => {
                panel(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(field_value(ui, &inv.memory, "Installed capacity"))
                                .size(24.),
                        );
                        ui.label(RichText::new(tx(ui, "installed")).color(muted(ui)));
                    });
                    if let Some(sample) = self.state.history.back() {
                        if sample.memory_used.state == Availability::Valid
                            && sample.memory_total.state == Availability::Valid
                            && let (Some(used), Some(total)) =
                                (sample.memory_used.value, sample.memory_total.value)
                        {
                            ui.label(
                                RichText::new(td(
                                    ui,
                                    format!(
                                        "{} in use · {} available to Windows",
                                        ui_bytes(ui, used as u64),
                                        ui_bytes(ui, total as u64)
                                    ),
                                ))
                                .color(muted(ui)),
                            );
                        } else {
                            ui.label(td(
                                ui,
                                format!(
                                    "Memory readings: {}",
                                    tx(
                                        ui,
                                        availability(
                                            if sample.memory_used.state != Availability::Valid {
                                                &sample.memory_used.state
                                            } else {
                                                &sample.memory_total.state
                                            }
                                        )
                                    )
                                ),
                            ));
                        }
                    }
                });
                ui.add_space(16.);
                let module_count = inv
                    .memory
                    .iter()
                    .filter(|f| f.label.ends_with(" capacity") && f.label.starts_with("Module "))
                    .count();
                let module = |ui: &mut Ui, n: usize| {
                    panel(ui, |ui| {
                        ui.label(RichText::new(td(ui, format!("Module {n}"))).size(16.));
                        ui.add_space(12.);
                        let prefix = format!("Module {n} ");
                        let fields: Vec<_> = inv
                            .memory
                            .iter()
                            .filter(|f| f.label.starts_with(&prefix))
                            .map(|f| {
                                let mut f = f.clone();
                                f.label = f.label[prefix.len()..].to_string();
                                if let Some(first) = f.label.get_mut(0..1) {
                                    first.make_ascii_uppercase();
                                }
                                f
                            })
                            .collect();
                        field_table(ui, &format!("dimm{n}"), &fields);
                    });
                };
                if ui.available_width() < 680. {
                    for n in 1..=module_count {
                        module(ui, n);
                        ui.add_space(16.);
                    }
                } else {
                    for pair in (0..module_count).step_by(2) {
                        ui.columns(2, |cols| {
                            for (j, col) in cols.iter_mut().enumerate() {
                                let n = pair + j + 1;
                                if n <= module_count {
                                    module(col, n);
                                }
                            }
                        });
                        ui.add_space(16.);
                    }
                }
                folding(ui, tx(ui, "Channel mode, timings & profiles")).show(ui, |ui| {
                    let fields: Vec<_> = inv
                        .memory
                        .iter()
                        .filter(|f| {
                            !f.label.starts_with("Module ") && f.label != "Installed capacity"
                        })
                        .cloned()
                        .collect();
                    field_table(ui, "memory-extra", &fields);
                });
            }
            _ => {
                panel(ui, |ui| {
                    field_table(ui, "board-fields", &inv.motherboard);
                });
            }
        }
    }
    fn gpu(&mut self, ui: &mut Ui) {
        header(ui, "GPU", "");
        let Some(inv) = self.state.inventory.clone() else {
            loading(ui, &self.state);
            return;
        };
        if inv.adapters.is_empty() {
            ui.label(tx(ui, "No graphics adapters reported by DXGI"));
            return;
        }
        self.state.adapter = self.state.adapter.min(inv.adapters.len() - 1);
        ComboBox::from_id_salt("adapter")
            .width(ui.available_width().min(440.))
            .selected_text(&inv.adapters[self.state.adapter].name)
            .show_ui(ui, |ui| {
                for (i, a) in inv.adapters.iter().enumerate() {
                    ui.selectable_value(&mut self.state.adapter, i, &a.name);
                }
            });
        let adapter = &inv.adapters[self.state.adapter];
        ui.add_space(18.);
        let utilization: Vec<_> = self
            .state
            .history
            .iter()
            .map(|s| {
                (
                    s.timestamp_ms,
                    gpu_reading(s, &adapter.id, "Utilization").and_then(|r| r.value),
                )
            })
            .collect();
        let temperature: Vec<_> = self
            .state
            .history
            .iter()
            .map(|s| {
                (
                    s.timestamp_ms,
                    gpu_reading(s, &adapter.id, "Temperature").and_then(|r| r.value),
                )
            })
            .collect();
        if ui.available_width() < 600. {
            metric_chart(ui, "Utilization", &utilization, "%", 100., GPU, 142.);
            ui.add_space(12.);
            metric_chart(ui, "Temperature", &temperature, "°C", 110., CPU, 142.);
        } else {
            ui.columns(2, |cols| {
                metric_chart(
                    &mut cols[0],
                    "Utilization",
                    &utilization,
                    "%",
                    100.,
                    GPU,
                    142.,
                );
                metric_chart(
                    &mut cols[1],
                    "Temperature",
                    &temperature,
                    "°C",
                    110.,
                    CPU,
                    142.,
                );
            });
        }
        ui.add_space(22.);
        panel(ui, |ui| {
            if ui.available_width() < 400. {
                ui.label(RichText::new(tx(ui, "Sensors")).size(17.));
                ui.label(
                    RichText::new(tx(ui, "Minimum / maximum · last 120 s"))
                        .size(12.)
                        .color(muted(ui)),
                );
            } else {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(tx(ui, "Sensors")).size(17.));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(tx(ui, "Minimum / maximum · last 120 s"))
                                .size(12.)
                                .color(muted(ui)),
                        );
                    });
                });
            }
            ui.add_space(8.);
            let sample = self
                .state
                .history
                .back()
                .and_then(|s| s.gpus.iter().find(|g| g.adapter_id == adapter.id));
            if let Some(sample) = sample {
                let main: Vec<_> = sample
                    .readings
                    .iter()
                    .filter(|(name, _)| {
                        !name.starts_with("Process ")
                            && !name.starts_with("Engine ")
                            && name != "Power (driver percentage)"
                            && name != "NVIDIA memory allocated + reserved"
                            && name != "Fan"
                    })
                    .collect();
                sensor_table(ui, "sensors", &main, &self.state.history, &adapter.id);
                folding(ui, tx(ui, "Driver counters & process memory")).show(ui, |ui| {
                    let extras: Vec<_> = sample
                        .readings
                        .iter()
                        .filter(|(name, _)| {
                            name.starts_with("Process ")
                                || name.starts_with("Engine ")
                                || name == "Power (driver percentage)"
                                || name == "NVIDIA memory allocated + reserved"
                                || name == "Fan"
                        })
                        .collect();
                    sensor_table(
                        ui,
                        "sensors-extra",
                        &extras,
                        &self.state.history,
                        &adapter.id,
                    );
                });
            } else {
                ui.label(tx(ui, "Waiting for sensor samples…"));
            }
        });
        ui.add_space(18.);
        panel(ui, |ui| {
            ui.label(RichText::new(tx(ui, "Adapter specifications")).size(17.));
            ui.add_space(12.);
            let fields: Vec<_> = adapter
                .fields
                .iter()
                .filter(|f| {
                    ![
                        "Sensor binding",
                        "LUID",
                        "Subsystem / revision",
                        "Dedicated system memory",
                    ]
                    .contains(&f.label.as_str())
                })
                .cloned()
                .collect();
            if ui.available_width() < 680. {
                field_table(ui, "gpu-stacked", &fields);
            } else {
                ui.columns(2, |cols| {
                    let midpoint = fields.len().div_ceil(2);
                    field_table(&mut cols[0], "gpu-left", &fields[..midpoint]);
                    field_table(&mut cols[1], "gpu-right", &fields[midpoint..]);
                });
            }
            folding(ui, tx(ui, "Provider binding & identifiers")).show(ui, |ui| {
                let fields: Vec<_> = adapter
                    .fields
                    .iter()
                    .filter(|f| {
                        [
                            "Sensor binding",
                            "LUID",
                            "Subsystem / revision",
                            "Dedicated system memory",
                        ]
                        .contains(&f.label.as_str())
                    })
                    .cloned()
                    .collect();
                field_table(ui, "gpu-identity", &fields);
            });
        });
    }
    fn storage(&mut self, ui: &mut Ui) {
        let compact = ui.available_width() < 520.;
        header(ui, "Storage", "Read-only folder and drive analysis");
        // Keep all scan actions together. Reserve their width even when idle,
        // so starting a scan never moves the path field or wraps Cancel alone.
        let actions_width = scan_actions_width(ui);
        let path_label_width = label_width(ui, &tx(ui, "Path"), 36.);
        let stacked = ui.available_width() < actions_width + path_label_width + 240.;
        let path_width = if stacked {
            (ui.available_width() - path_label_width - 10.).max(100.)
        } else {
            (ui.available_width() - actions_width - path_label_width - 30.).clamp(160., 360.)
        };
        if stacked {
            ui.horizontal(|ui| self.scan_path(ui, path_width));
            ui.horizontal(|ui| self.scan_actions(ui));
        } else {
            ui.horizontal(|ui| {
                self.scan_path(ui, path_width);
                self.scan_actions(ui);
            });
        }
        self.storage_results(ui, compact);
    }
    fn scan_path(&mut self, ui: &mut Ui, path_width: f32) {
        let path_label = ui.label(tx(ui, "Path"));
        let response = ui
            .add_sized(
                [path_width, 34.],
                TextEdit::singleline(&mut self.state.settings.path)
                    .id(Id::new("scan-path"))
                    .hint_text(tx(ui, "C:\\ or a folder path"))
                    .margin(vec2(10., 8.)),
            )
            .labelled_by(path_label.id);
        if response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
            self.state.start_scan();
        }
    }
    fn scan_actions(&mut self, ui: &mut Ui) {
        let compact = ui.available_width() < scan_actions_width(ui);
        let browse_label = tx(ui, "Browse…");
        let browse = ui.add_sized(
            [
                if compact {
                    36.
                } else {
                    label_width(ui, &browse_label, 82.)
                },
                34.,
            ],
            Button::new(if compact { "…" } else { &browse_label }),
        );
        browse.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, &browse_label));
        if browse.on_hover_text(&browse_label).clicked() {
            self.state.pick_folder();
        }
        if ui
            .add_sized(
                [
                    if compact {
                        label_width(ui, &tx(ui, "Scan"), 76.)
                    } else {
                        scan_primary_width(ui)
                    },
                    34.,
                ],
                Button::new(
                    RichText::new(tx(
                        ui,
                        if compact || self.state.nodes.is_empty() {
                            "Scan"
                        } else {
                            "Rescan"
                        },
                    ))
                    .color(ui.visuals().panel_fill),
                )
                .fill(ui.visuals().text_color())
                .wrap(),
            )
            .on_hover_text(tx(ui, "Start a new scan of this path (F5)"))
            .clicked()
        {
            self.state.start_scan();
        }
        if self.state.scan.is_some()
            && ui
                .add_sized(
                    [label_width(ui, &tx(ui, "Cancel"), 76.), 34.],
                    Button::new(tx(ui, "Cancel")),
                )
                .clicked()
        {
            self.state.cancel_scan();
        }
    }
    fn storage_results(&mut self, ui: &mut Ui, compact: bool) {
        if let Some(inv) = &self.state.inventory {
            let mut drive = None;
            ui.horizontal_wrapped(|ui| {
                for d in &inv.drives {
                    if ui
                        .small_button(td(
                            ui,
                            format!("{}  {} free", d.mount, ui_bytes(ui, d.free_bytes)),
                        ))
                        .clicked()
                    {
                        drive = Some(d.mount.clone());
                    }
                }
            });
            if let Some(path) = drive {
                self.state.settings.path = path;
                self.state.start_scan();
            }
        }
        ui.add_space(14.);
        if self.state.nodes.is_empty() {
            ui.add_space(38.);
            ui.label(
                RichText::new(tx(
                    ui,
                    if self.state.scan.is_some() {
                        "Reading directory metadata…"
                    } else {
                        "No scan results"
                    },
                ))
                .size(20.),
            );
            if let Some(s) = &self.state.summary {
                for note in &s.notes {
                    ui.label(RichText::new(td(ui, note)).color(GPU));
                }
            }
            return;
        }
        let scope = &self.state.nodes[self.state.scope];
        let scope_logical = scope.logical;
        let scope_allocated = scope.allocated;
        let files = scope.files;
        let partial = scope.incomplete || self.state.scan.is_some();
        let lower_bound = if partial { "≥ " } else { "" };
        let totals = [
            (
                "Logical",
                format!("{lower_bound}{}", ui_bytes(ui, scope_logical)),
            ),
            (
                "Allocated",
                format!("{lower_bound}{}", ui_bytes(ui, scope_allocated)),
            ),
            (
                "Files",
                format!("{lower_bound}{}", i18n::language(ui).integer(files)),
            ),
        ];
        if compact {
            for (label, value) in &totals {
                summary_row(ui, label, value, None);
            }
        } else {
            ui.columns(3, |columns| {
                for (column, (label, value)) in columns.iter_mut().zip(&totals) {
                    column.label(
                        RichText::new(tx(column, *label))
                            .size(12.)
                            .color(muted(column)),
                    );
                    column.add(Label::new(RichText::new(value).size(25.)).wrap());
                }
            });
        }
        ui.add_space(6.);
        ui.label(
            RichText::new(scan_status(&self.state))
                .color(if self.state.scan.is_some() {
                    GPU
                } else {
                    muted(ui)
                })
                .size(12.),
        );
        ui.add_space(10.);
        let mut crumbs = vec![self.state.scope];
        let mut parent = self.state.nodes[self.state.scope].parent;
        while let Some(p) = parent {
            crumbs.push(p);
            parent = self.state.nodes[p].parent;
        }
        crumbs.reverse();
        ui.horizontal_wrapped(|ui| {
            for (i, id) in crumbs.iter().enumerate() {
                if i > 0 {
                    ui.label(RichText::new("/").color(muted(ui)));
                }
                if ui.small_button(&self.state.nodes[*id].name).clicked() {
                    self.state.navigate_scope(*id);
                }
            }
        });
        ui.add_space(8.);
        let split = ui.available_width() >= 980.;
        // Stable control IDs while asynchronous drive discovery changes the
        // number of widgets above this toolbar.
        ui.push_id("storage-view-tabs", |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui
                    .selectable_label(
                        !self.state.largest && !self.state.map_only,
                        tx(ui, "Hierarchy"),
                    )
                    .clicked()
                {
                    self.state.largest = false;
                    self.state.map_only = false;
                    self.state.dirty = true;
                }
                if ui
                    .selectable_label(
                        self.state.largest && !self.state.map_only,
                        tx(ui, "Largest files"),
                    )
                    .clicked()
                {
                    self.state.largest = true;
                    self.state.map_only = false;
                    self.state.sort = Sort::Logical;
                    self.state.descending = true;
                    self.state.dirty = true;
                }
                if ui
                    .selectable_label(self.state.map_only, tx(ui, "Map"))
                    .clicked()
                {
                    self.state.map_only = true;
                }
            })
        });
        ui.add_space(4.);
        ui.push_id("storage-filter-export", |ui| ui.horizontal_wrapped(|ui| {
            let filter_label = ui.label(tx(ui, "Filter"));
            let filter_width = (ui.available_width() - 136.).clamp(120., 380.);
            if ui.add_sized([filter_width, 30.], TextEdit::singleline(&mut self.state.filter).id(Id::new("scan-filter")).hint_text(tx(ui, "Name or path"))).labelled_by(filter_label.id).changed() {
                self.state.dirty = true;
            }
            let can_export = self.state.scan.is_none() && self.state.task.is_none();
            ui.add_enabled_ui(can_export, |ui| {
                for format in ["CSV", "JSON"] {
                    if ui.button(format).on_hover_text(tx(ui, "Export whole scan, including entries outside the current folder or filter")).clicked() {
                        self.state.export(&format.to_lowercase());
                    }
                }
            });
        }));
        ui.add_space(16.);
        let viewport_height = ui.ctx().content_rect().height();
        // Measure content consumed above the results, independent of scrolling.
        // Leave room for the selected path, actions and scan notes below them.
        let used_height = ui.cursor().top() - ui.max_rect().top();
        let height = (viewport_height - used_height - 200.).clamp(180., 480.);
        let clicked = if self.state.map_only {
            self.storage_map(ui, height)
        } else if split {
            let width = ui.available_width();
            let map_width = (width * 0.32).clamp(280., 420.);
            ui.horizontal_top(|ui| {
                let table_clicked = ui
                    .allocate_ui_with_layout(
                        vec2(width - map_width - 18., 0.),
                        Layout::top_down(Align::Min),
                        |ui| self.storage_table(ui),
                    )
                    .inner;
                ui.add_space(8.);
                let map_clicked = ui
                    .allocate_ui_with_layout(
                        vec2(map_width, 0.),
                        Layout::top_down(Align::Min),
                        |ui| self.storage_map(ui, height),
                    )
                    .inner;
                table_clicked.or(map_clicked)
            })
            .inner
        } else {
            self.storage_table(ui)
        };
        if let Some(id) = clicked {
            self.state.navigate_scope(id);
        }
        let total = self.state.nodes[self.state.scope].logical;
        ui.add_space(10.);
        let node = &self.state.nodes[self.state.selected];
        let path = node.path.clone();
        let is_dir = node.is_dir;
        if ui.available_width() < 640. {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(td(ui, format!("Allocated: {}", node_size(ui, node, true))))
                        .monospace()
                        .size(12.),
                );
                ui.label(
                    RichText::new(td(
                        ui,
                        format!(
                            "Scope: {}%",
                            i18n::language(ui).decimal(
                                if total == 0 {
                                    0.
                                } else {
                                    node.logical as f64 / total as f64 * 100.
                                },
                                1
                            )
                        ),
                    ))
                    .monospace()
                    .size(12.),
                );
            });
        }
        copy_label(ui, &path.to_string_lossy(), 12., muted(ui));
        ui.horizontal_wrapped(|ui| {
            if ui.button(tx(ui, "Copy path")).clicked() {
                ui.ctx().copy_text(path.to_string_lossy().into());
            }
            if ui.button(tx(ui, "Reveal in Explorer")).clicked()
                && let Err(e) = reveal(&path)
            {
                self.state.notice = Some(e);
            }
            if is_dir && ui.button(tx(ui, "Open folder")).clicked() {
                self.state.navigate_scope(self.state.selected);
            }
            ui.label(
                RichText::new(td(
                    ui,
                    format!(
                        "{} entries",
                        i18n::language(ui).integer(self.state.visible.len() as u64)
                    ),
                ))
                .size(12.)
                .color(muted(ui)),
            );
        });
        if let Some(summary) = &self.state.summary {
            folding(ui, tx(ui, "Scan accounting & skipped entries")).id_salt("scan-notes").show(ui,|ui|{ui.label(td(ui, format!("{} errors · {} reparse points · {} cloud placeholders · {} hard-link aliases",summary.errors,summary.skipped_reparse,summary.skipped_cloud,summary.hard_links))); for note in &summary.notes{ui.label(td(ui, note));} });
        }
    }
    fn storage_table(&mut self, ui: &mut Ui) -> Option<usize> {
        let compact = ui.available_width() < 640.;
        let total = self.state.nodes[self.state.scope].logical;
        let mut clicked = None;
        let mut toggle = None;
        let mut new_sort = None;
        let mut table = TableBuilder::new(ui)
            .id_salt(("files", self.state.scan_generation, compact))
            // TableBody::rows virtualizes against the outer page clip rect.
            // One scrollbar therefore remains efficient for very large scans.
            .vscroll(false)
            .vertical_scroll_offset(0.)
            .scroll_bar_visibility(scroll_area::ScrollBarVisibility::AlwaysHidden)
            .striped(false)
            .resizable(false)
            .cell_layout(Layout::left_to_right(Align::Center))
            .sense(Sense::click())
            .column(Column::remainder().clip(true))
            .column(Column::exact(104.).clip(true));
        if !compact {
            table = table.column(Column::exact(104.).clip(true));
        }
        table = table.column(Column::exact(if compact { 70. } else { 84. }).clip(true));
        if !compact {
            table = table.column(Column::exact(60.).clip(true));
        }
        table
            .header(48., |mut h| {
                for (label, sort) in [
                    ("Name", Some(Sort::Name)),
                    ("Logical", Some(Sort::Logical)),
                    ("Allocated", Some(Sort::Allocated)),
                    ("Files", Some(Sort::Files)),
                    ("Scope %", None),
                ] {
                    if compact && (sort == Some(Sort::Allocated) || sort.is_none()) {
                        continue;
                    }
                    h.col(|ui| {
                        let marker = if sort == Some(self.state.sort) {
                            if self.state.descending {
                                " ↓"
                            } else {
                                " ↑"
                            }
                        } else {
                            ""
                        };
                        if ui
                            .add(
                                Button::new(
                                    RichText::new(format!("{}{marker}", tx(ui, label))).size(12.),
                                )
                                .wrap()
                                .frame(false),
                            )
                            .clicked()
                        {
                            new_sort = sort;
                        }
                    });
                }
            })
            .body(|body| {
                body.rows(30., self.state.visible.len(), |mut row| {
                    let (id, depth) = self.state.visible[row.index()];
                    let node = &self.state.nodes[id];
                    row.set_selected(self.state.selected == id);
                    row.col(|ui| {
                        let indent =
                            (depth.min(20) as f32 * 15.).min((ui.available_width() - 75.).max(0.));
                        ui.add_space(indent);
                        if node.is_dir {
                            if folder_toggle(ui, &node.name, self.state.expanded.contains(&id))
                                .clicked()
                            {
                                toggle = Some(id);
                            }
                        } else {
                            ui.add_space(38.);
                        }
                        let response =
                            ui.add(Label::new(&node.name).truncate().sense(Sense::click()));
                        if response.clicked() {
                            self.state.selected = id;
                        }
                        if response.double_clicked() && node.is_dir {
                            clicked = Some(id);
                        }
                        response.on_hover_text(td(
                            ui,
                            format!(
                                "{}\nLogical: {}\nAllocated: {}\nScope: {}%\n{}",
                                node.path.display(),
                                node_size(ui, node, false),
                                node_size(ui, node, true),
                                i18n::language(ui).decimal(
                                    if total == 0 {
                                        0.
                                    } else {
                                        node.logical as f64 / total as f64 * 100.
                                    },
                                    1
                                ),
                                td(ui, &node.note)
                            ),
                        ));
                    });
                    row.col(|ui| {
                        let value = node_size(ui, node, false);
                        numeric_value(ui, &value, ui.available_width(), 12.).on_hover_text(&value);
                    });
                    if !compact {
                        row.col(|ui| {
                            let value = node_size(ui, node, true);
                            numeric_value(ui, &value, ui.available_width(), 12.)
                                .on_hover_text(&value);
                        });
                    }
                    row.col(|ui| {
                        let count = if node.is_dir && (node.incomplete || self.state.scan.is_some())
                        {
                            format!("≥ {}", i18n::language(ui).integer(node.files))
                        } else {
                            i18n::language(ui).integer(node.files)
                        };
                        numeric_value(ui, &count, ui.available_width(), 12.).on_hover_text(&count);
                    });
                    if !compact {
                        row.col(|ui| {
                            let value = if total == 0 {
                                i18n::language(ui).decimal(0., 1)
                            } else {
                                i18n::language(ui)
                                    .decimal(node.logical as f64 / total as f64 * 100., 1)
                            };
                            numeric_value(ui, &value, ui.available_width(), 12.);
                        });
                    }
                    if row.response().clicked() {
                        self.state.selected = id;
                    }
                });
            });
        if let Some(id) = toggle {
            if !self.state.expanded.remove(&id) {
                self.state.expanded.insert(id);
            }
            self.state.dirty = true;
        }
        if let Some(sort) = new_sort {
            if self.state.sort == sort {
                self.state.descending = !self.state.descending;
            } else {
                self.state.sort = sort;
                self.state.descending = sort != Sort::Name;
            }
            self.state.dirty = true;
        }
        clicked
    }
    fn storage_map(&mut self, ui: &mut Ui, height: f32) -> Option<usize> {
        let mut clicked = None;
        ui.label(RichText::new(tx(ui, "Logical size map")).size(15.));
        ui.add_space(8.);
        let (rect, _) =
            ui.allocate_exact_size(vec2(ui.available_width(), height - 40.), Sense::hover());
        let nonzero = &self.state.map_ids;
        if nonzero.is_empty() {
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                tx(ui, "No nonzero files"),
                FontId::proportional(13.),
                muted(ui),
            );
        } else {
            if self.map_key != Some((self.state.map_revision, rect)) {
                self.map_cache = treemap_tiles(nonzero, &self.state.nodes, rect);
                self.map_key = Some((self.state.map_revision, rect));
            }
            for &(id, rect) in &self.map_cache {
                if rect.width() < 1. || rect.height() < 1. {
                    continue;
                }
                let node = &self.state.nodes[id];
                let r = rect.shrink(1.5);
                let response = ui.interact(
                    r,
                    Id::new(("tile", self.state.scan_generation, id)),
                    Sense::click(),
                );
                let accessible_label = format!("{} {}", node.name, node_size(ui, node, false));
                response.widget_info(|| {
                    WidgetInfo::labeled(WidgetType::Button, true, &accessible_label)
                });
                let base = tile_color(id, ui.visuals().dark_mode);
                ui.painter().rect_filled(
                    r,
                    4,
                    if response.hovered() {
                        base.gamma_multiply(1.18)
                    } else {
                        base
                    },
                );
                if self.state.selected == id || response.has_focus() {
                    ui.painter().rect_stroke(
                        r,
                        4,
                        Stroke::new(2.0f32, ui.visuals().text_color()),
                        StrokeKind::Inside,
                    );
                }
                if r.width() > 60. && r.height() > 40. {
                    let p = ui.painter().with_clip_rect(r.shrink(6.));
                    for (text, font, color, y) in [
                        (
                            node.name.clone(),
                            FontId::proportional(13.),
                            Color32::WHITE,
                            9.,
                        ),
                        (
                            node_size(ui, node, false),
                            FontId::monospace(11.),
                            Color32::from_gray(210),
                            29.,
                        ),
                    ] {
                        let mut job = egui::text::LayoutJob::simple_singleline(text, font, color);
                        job.wrap.max_width = r.width() - 18.;
                        job.wrap.max_rows = 1;
                        job.wrap.break_anywhere = true;
                        let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
                        p.galley(r.left_top() + vec2(9., y), galley, color);
                    }
                }
                if response.clicked() {
                    self.state.selected = id;
                }
                if response.double_clicked() && node.is_dir {
                    clicked = Some(id);
                }
                response.on_hover_text(td(
                    ui,
                    format!(
                        "{}\n{} logical · {} allocated\nDouble-click a folder to open",
                        node.path.display(),
                        node_size(ui, node, false),
                        node_size(ui, node, true)
                    ),
                ));
            }
        }
        ui.add_space(10.);
        ui.label(
            RichText::new(tx(ui, "Area = logical bytes · double-click to open"))
                .size(12.)
                .color(muted(ui)),
        );

        clicked
    }
    fn diagnostics(&mut self, ui: &mut Ui) {
        header(
            ui,
            "Diagnostics",
            "Provider sources, availability and sampling timestamps",
        );
        if ui.button(tx(ui, "Export hardware & samples…")).clicked() {
            self.state.export_hardware();
        }
        ui.add_space(16.);
        if let Some(error) = &self.state.hardware_error {
            ui.colored_label(GPU, td(ui, error));
        }
        if let Some(inv) = &self.state.inventory {
            for note in &inv.diagnostics {
                ui.label(td(ui, note));
                ui.add_space(6.);
            }
            ui.add_space(12.);
            for (name, fields) in [
                ("CPU", &inv.cpu),
                ("Memory", &inv.memory),
                ("Motherboard", &inv.motherboard),
                ("Windows", &inv.os),
            ] {
                folding(ui, tx(ui, name)).show(ui, |ui| {
                    for f in fields {
                        diagnostic_field(ui, f);
                    }
                });
            }
            for adapter in &inv.adapters {
                folding(ui, &adapter.name).show(ui, |ui| {
                    for f in &adapter.fields {
                        diagnostic_field(ui, f);
                    }
                });
            }
        }
        if let Some(sample) = self.state.history.back() {
            ui.add_space(16.);
            ui.label(td(
                ui,
                format!("Latest sample: {} ms since Unix epoch", sample.timestamp_ms),
            ));
            for (label, r) in [
                ("CPU utilization", &sample.cpu_usage),
                ("Memory used", &sample.memory_used),
                ("OS-reported frequency", &sample.cpu_frequency),
            ] {
                reading_diagnostic(ui, label, r);
            }
            for gpu in &sample.gpus {
                folding(ui, &gpu.adapter_id).show(ui, |ui| {
                    for (label, r) in &gpu.readings {
                        reading_diagnostic(ui, label, r);
                    }
                });
            }
        }
    }
    fn settings(&mut self, ui: &mut Ui) {
        header(ui, "Settings", "Appearance and keyboard controls");
        section(ui, "Appearance", |ui| {
            ui.horizontal_wrapped(|ui| {
                let label = ui.label(tx(ui, "Language"));
                egui::ComboBox::from_id_salt("language")
                    .selected_text(tx(ui, self.state.settings.language.native_name()))
                    .show_ui(ui, |ui| {
                        for language in Language::ALL {
                            if ui
                                .selectable_value(
                                    &mut self.state.settings.language,
                                    language,
                                    tx(ui, language.native_name()),
                                )
                                .changed()
                            {
                                i18n::set_context(ui.ctx(), self.state.settings.language);
                            }
                        }
                    })
                    .response
                    .labelled_by(label.id);
            });
            ui.add_space(12.);
            ui.horizontal_wrapped(|ui| {
                ui.label(tx(ui, "Theme"));
                for theme in ["Dark", "Light", "System"] {
                    if ui
                        .selectable_value(
                            &mut self.state.settings.theme,
                            theme.into(),
                            tx(ui, theme),
                        )
                        .changed()
                    {
                        apply_style(ui.ctx(), &self.state.settings);
                    }
                }
            });
            ui.add_space(12.);
            let scale_label = ui.label(tx(ui, "Interface scale"));
            ui.spacing_mut().slider_width = (ui.available_width() - 90.).clamp(80., 220.);
            if ui
                .add(
                    Slider::new(&mut self.state.settings.scale, 0.85..=1.5)
                        .custom_formatter(|value, _| format!("{:.0}%", value * 100.))
                        .custom_parser(|text| {
                            text.trim()
                                .trim_end_matches('%')
                                .replace(',', ".")
                                .parse::<f64>()
                                .ok()
                                .map(|value| value / 100.)
                        })
                        .step_by(0.05),
                )
                .labelled_by(scale_label.id)
                .changed()
            {
                apply_style(ui.ctx(), &self.state.settings);
            }
            ui.add_space(12.);
            if ui
                .checkbox(
                    &mut self.state.settings.reduce_motion,
                    tx(ui, "Reduce motion"),
                )
                .changed()
            {
                apply_style(ui.ctx(), &self.state.settings);
            }
        });
        ui.add_space(26.);
        section(ui, "Keyboard", |ui| {
            for (key, label) in [
                ("Ctrl + 1…4", "Switch primary view"),
                ("Tab / Shift + Tab", "Move focus"),
                ("Enter / Space", "Activate focused control"),
                ("F5", "Rescan current path"),
                ("Escape", "Cancel current scan"),
                ("Alt + Up", "Go to parent scan folder"),
            ] {
                summary_row(ui, key, &tx(ui, label), None);
            }
        });
        ui.add_space(24.);
        section(ui, "Storage accounting", |ui| {
            ui.label(tx(ui, "Logical bytes count every file path and data stream. Allocated bytes count each file identity once across hard links. Directory and volume metadata overhead is excluded."));
            ui.add_space(8.);
            ui.label(tx(ui, "Scans skip reparse points and cloud placeholders. Files may change during a scan; partial results and read errors remain visible."));
        });
        ui.add_space(24.);
        ui.label(
            RichText::new(i18n::language(ui).format(
                "Rigometry {0} · Application source: MIT · Rust / egui",
                &[env!("CARGO_PKG_VERSION").into()],
            ))
            .color(muted(ui)),
        );
    }

    fn fail_capture(&mut self, ctx: &Context, error: String) {
        self.capture_failed.store(true, Ordering::Relaxed);
        self.state.notice = Some(error.clone());
        eprintln!(
            "{}",
            self.state
                .settings
                .language
                .message(&format!("Screenshot export: {error}"))
        );
        ctx.send_viewport_cmd(ViewportCommand::Close);
    }
}

impl eframe::App for App {
    fn raw_input_hook(&mut self, _: &Context, input: &mut RawInput) {
        if self.capture_dir.is_some() {
            // Capture the requested pages without hover tooltips or accidental input.
            // This filters only this app's input; it never moves the system pointer.
            input
                .events
                .retain(|event| matches!(event, Event::Screenshot { .. }));
            input.events.push(Event::PointerGone);
        }
    }

    fn logic(&mut self, ctx: &Context, _: &mut eframe::Frame) {
        if let Some(size) = self.window_override.take() {
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(
                size / (BASE_SCALE * self.state.settings.scale),
            ));
        }
        self.state.tick();
        if self
            .capture_worker
            .as_ref()
            .is_some_and(|worker| worker.is_finished())
        {
            let result = self
                .capture_worker
                .take()
                .unwrap()
                .join()
                .unwrap_or_else(|_| Err("Screenshot export worker failed".into()));
            if let Err(error) = result {
                self.fail_capture(ctx, error);
                return;
            }
            if self.capture_index >= 5 {
                self.capture_failed.store(false, Ordering::Relaxed);
            }
        }
        if self.capture_dir.is_some()
            && (self.capture_index < 5
                || self.capture_worker.is_some()
                || self.capture_pending.is_some())
        {
            let failure = if let Some(error) = &self.state.hardware_error {
                Some(format!("Hardware sampling failed during capture: {error}"))
            } else if self.capture_at.elapsed() > CAPTURE_TIMEOUT {
                if self.capture_pending.is_some() {
                    Some("Timed out waiting for screenshot pixels".into())
                } else if self.capture_worker.is_some() {
                    Some("Timed out writing screenshot".into())
                } else if self.state.history.len() < 3 {
                    Some("Timed out waiting for hardware samples".into())
                } else {
                    // A requested full-volume scan may legitimately take longer.
                    None
                }
            } else {
                None
            };
            if let Some(error) = failure {
                self.fail_capture(ctx, error);
                return;
            }
        }
        ctx.input(|i| {
            if i.modifiers.ctrl {
                for (key, page) in [
                    (Key::Num1, Page::Overview),
                    (Key::Num2, Page::Cpu),
                    (Key::Num3, Page::Gpu),
                    (Key::Num4, Page::Storage),
                ] {
                    if i.key_pressed(key) {
                        self.state.settings.page = page;
                    }
                }
            }
            if i.key_pressed(Key::F5) && self.state.settings.page == Page::Storage {
                self.state.start_scan();
            }
            if i.key_pressed(Key::Escape) {
                self.state.cancel_scan();
            }
            if i.modifiers.alt
                && i.key_pressed(Key::ArrowUp)
                && self.state.settings.page == Page::Storage
                && let Some(p) = self
                    .state
                    .nodes
                    .get(self.state.scope)
                    .and_then(|n| n.parent)
            {
                self.state.navigate_scope(p);
            }
        });
        if let Some(dir) = &self.capture_dir {
            let pages = [
                Page::Overview,
                Page::Cpu,
                Page::Gpu,
                Page::Storage,
                Page::Diagnostics,
            ];
            if self.capture_index < pages.len()
                && self.capture_pending.is_none()
                && self.capture_worker.is_none()
                && self.state.history.len() >= 3
                && self.state.scan.is_none()
                && self.capture_at.elapsed()
                    > if self.capture_index == 0 {
                        CAPTURE_WARMUP
                    } else {
                        Duration::from_secs(2)
                    }
            {
                self.state.settings.page = pages[self.capture_index];
                self.capture_pending = Some(dir.join(format!(
                    "{:02}-{:?}.png",
                    self.capture_index, pages[self.capture_index]
                )));
                self.capture_at = Instant::now();
                ctx.send_viewport_cmd(ViewportCommand::Screenshot(Default::default()));
            }
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let Event::Screenshot { image, .. } = event
                && let Some(path) = self.capture_pending.take()
            {
                let width = image.width() as u32;
                let height = image.height() as u32;
                let rgba: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                let worker = std::thread::Builder::new()
                    .name("screenshot-export".into())
                    .spawn(move || {
                        use image::ImageEncoder;
                        let mut png = Vec::new();
                        image::codecs::png::PngEncoder::new(&mut png)
                            .write_image(&rgba, width, height, image::ExtendedColorType::Rgba8)
                            .map_err(|error| error.to_string())?;
                        crate::storage::write_new_output(&path, &png)
                    });
                match worker {
                    Ok(worker) => self.capture_worker = Some(worker),
                    Err(error) => {
                        self.fail_capture(
                            ctx,
                            format!("Could not start screenshot worker: {error}"),
                        );
                        return;
                    }
                }
                self.capture_index += 1;
                self.capture_at = Instant::now();
                if self.capture_index >= 5 {
                    self.state.settings.page = Page::Overview;
                }
            }
        }
        if self.capture_dir.is_some()
            && self.capture_index >= 5
            && self.capture_worker.is_none()
            && self.capture_at.elapsed() > Duration::from_secs(2)
        {
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
        let live_view = matches!(
            self.state.settings.page,
            Page::Overview | Page::Cpu | Page::Gpu
        );
        ctx.request_repaint_after(if live_view && !self.state.settings.reduce_motion {
            Duration::from_millis(33)
        } else if self.state.scan.is_some() {
            Duration::from_millis(80)
        } else {
            Duration::from_secs(1)
        });
    }
    fn ui(&mut self, ui: &mut Ui, _: &mut eframe::Frame) {
        i18n::set_context(ui.ctx(), self.state.settings.language);
        egui::Panel::left("navigation")
            .resizable(false)
            .exact_size(navigation_width(ui))
            .frame(
                Frame::new()
                    .fill(ui.visuals().panel_fill)
                    .inner_margin(Margin::symmetric(18, 20)),
            )
            .show_inside(ui, |ui| self.sidebar(ui));
        if self.state.notice.is_some()
            || self.state.task.is_some()
            || (self.state.scan.is_some() && self.state.settings.page != Page::Storage)
        {
            egui::Panel::bottom("status")
                .resizable(false)
                .frame(
                    Frame::new()
                        .fill(ui.visuals().panel_fill)
                        .inner_margin(Margin::symmetric(24, 6)),
                )
                .show_inside(ui, |ui| {
                    ui.vertical(|ui| {
                        if let Some(notice) = &self.state.notice {
                            ui.label(RichText::new(td(ui, notice)).size(12.));
                        }
                        if self.state.task.is_some() {
                            ui.label(
                                RichText::new(tx(ui, "File dialog / export in progress"))
                                    .size(12.)
                                    .color(muted(ui)),
                            );
                        } else if self.state.scan.is_some()
                            && self.state.settings.page != Page::Storage
                        {
                            ui.label(RichText::new(scan_status(&self.state)).size(12.).color(GPU));
                        }
                    });
                });
        }
        CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(ui.visuals().panel_fill)
                    .inner_margin(Margin::same(26)),
            )
            .show_inside(ui, |ui| {
                if self.state.settings.page == Page::Storage {
                    ScrollArea::vertical()
                        .scroll_bar_visibility(scroll_area::ScrollBarVisibility::AlwaysVisible)
                        .id_salt("storage-outer")
                        .auto_shrink([false, false])
                        .show(ui, |ui| self.storage(ui));
                } else {
                    ScrollArea::vertical()
                        .scroll_bar_visibility(scroll_area::ScrollBarVisibility::AlwaysVisible)
                        .id_salt(("page-scroll", self.state.settings.page))
                        .auto_shrink([false, false])
                        .show(ui, |ui| match self.state.settings.page {
                            Page::Overview => self.overview(ui),
                            Page::Cpu => self.cpu(ui),
                            Page::Gpu => self.gpu(ui),
                            Page::Diagnostics => self.diagnostics(ui),
                            Page::Settings => self.settings(ui),
                            Page::Storage => {}
                        });
                }
            });
    }
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if self.capture_dir.is_none() {
            eframe::set_value(storage, "settings", &self.state.settings);
        }
    }
    fn persist_egui_memory(&self) -> bool {
        self.capture_dir.is_none()
    }
}

fn apply_style(ctx: &Context, settings: &Settings) {
    ctx.send_viewport_cmd(ViewportCommand::MinInnerSize(
        vec2(1080., 720.) / (BASE_SCALE * settings.scale),
    ));
    ctx.set_theme(match settings.theme.as_str() {
        "Light" => ThemePreference::Light,
        "System" => ThemePreference::System,
        _ => ThemePreference::Dark,
    });
    ctx.set_zoom_factor(BASE_SCALE * settings.scale);
    for theme in [Theme::Dark, Theme::Light] {
        ctx.style_mut_of(theme, |style| {
            let dark = theme == Theme::Dark;
            style.spacing.item_spacing = vec2(10., 7.);
            style.spacing.button_padding = vec2(12., 7.);
            style.spacing.interact_size = vec2(36., 30.);
            style.spacing.icon_width = 20.;
            style.spacing.icon_width_inner = 14.;
            // Reserve a gutter: floating scrollbars cover right-aligned values.
            style.spacing.scroll = egui::style::ScrollStyle {
                bar_width: 10.,
                bar_inner_margin: 8.,
                foreground_color: true,
                ..egui::style::ScrollStyle::solid()
            };
            style.animation_time = if settings.reduce_motion { 0. } else { 0.12 };
            style
                .text_styles
                .insert(TextStyle::Body, FontId::proportional(14.));
            style
                .text_styles
                .insert(TextStyle::Button, FontId::proportional(14.));
            style
                .text_styles
                .insert(TextStyle::Small, FontId::proportional(12.));
            style
                .text_styles
                .insert(TextStyle::Heading, FontId::proportional(26.));
            let v = &mut style.visuals;
            v.panel_fill = if dark {
                Color32::from_gray(14)
            } else {
                Color32::from_gray(252)
            };
            v.window_fill = if dark {
                Color32::from_gray(30)
            } else {
                Color32::WHITE
            };
            v.faint_bg_color = if dark {
                Color32::from_gray(23)
            } else {
                Color32::from_gray(242)
            };
            v.extreme_bg_color = if dark {
                Color32::from_gray(20)
            } else {
                Color32::WHITE
            };
            v.override_text_color = Some(if dark {
                Color32::from_gray(237)
            } else {
                Color32::from_gray(18)
            });
            v.selection.bg_fill = if dark {
                Color32::from_gray(48)
            } else {
                Color32::from_gray(227)
            };
            v.selection.stroke = Stroke::new(
                1.5f32,
                if dark {
                    Color32::from_gray(180)
                } else {
                    Color32::from_gray(80)
                },
            );
            v.widgets.inactive.bg_fill = if dark {
                Color32::from_gray(32)
            } else {
                Color32::from_gray(242)
            };
            v.widgets.inactive.weak_bg_fill = v.widgets.inactive.bg_fill;
            v.widgets.inactive.bg_stroke = Stroke::new(
                1.0f32,
                if dark {
                    Color32::from_gray(70)
                } else {
                    Color32::from_gray(170)
                },
            );
            v.widgets.hovered.bg_fill = if dark {
                Color32::from_gray(45)
            } else {
                Color32::from_gray(231)
            };
            v.widgets.hovered.weak_bg_fill = v.widgets.hovered.bg_fill;
            v.widgets.active.bg_fill = if dark {
                Color32::from_gray(55)
            } else {
                Color32::from_gray(218)
            };
            v.widgets.noninteractive.bg_stroke = Stroke::new(
                1.0f32,
                if dark {
                    Color32::from_gray(46)
                } else {
                    Color32::from_gray(219)
                },
            );
            for w in [
                &mut v.widgets.inactive,
                &mut v.widgets.hovered,
                &mut v.widgets.active,
                &mut v.widgets.noninteractive,
                &mut v.widgets.open,
            ] {
                w.corner_radius = CornerRadius::same(7);
                w.expansion = 0.;
            }
        });
    }
}

fn finite_scale(scale: f32) -> f32 {
    if scale.is_finite() {
        scale.clamp(0.85, 1.5)
    } else {
        1.0
    }
}
fn ui_bytes(ui: &Ui, value: u64) -> String {
    i18n::language(ui).bytes(value)
}
fn label_width(ui: &Ui, label: &str, minimum: f32) -> f32 {
    ui.painter()
        .layout_no_wrap(
            label.to_owned(),
            FontId::proportional(14.),
            ui.visuals().text_color(),
        )
        .size()
        .x
        .max(minimum - 24.)
        + 24.
}
fn scan_primary_width(ui: &Ui) -> f32 {
    label_width(ui, &tx(ui, "Scan"), 76.).max(label_width(ui, &tx(ui, "Rescan"), 76.))
}
fn scan_actions_width(ui: &Ui) -> f32 {
    label_width(ui, &tx(ui, "Browse…"), 82.)
        + scan_primary_width(ui)
        + label_width(ui, &tx(ui, "Cancel"), 76.)
        + 2. * ui.spacing().item_spacing.x
}
fn navigation_width(ui: &Ui) -> f32 {
    [
        "Overview",
        "CPU & Memory",
        "GPU",
        "Storage",
        "Diagnostics",
        "Settings",
    ]
    .iter()
    .map(|label| label_width(ui, &tx(ui, label), 0.) + 66.)
    .fold(204., f32::max)
}
fn field_display(ui: &Ui, field: &Field) -> String {
    let lang = i18n::language(ui);
    let Some(value) = &field.value else {
        return tx(ui, availability(&field.state));
    };
    let numeric_units = [
        "B", "KiB", "MiB", "GiB", "TiB", "MHz", "MT/s", "bits", "lanes",
    ];
    if numeric_units.contains(&field.unit.as_str())
        && let Ok(number) = value.parse::<f64>()
    {
        return format!(
            "{} {}",
            lang.decimal(
                number,
                value
                    .split_once('.')
                    .map_or(0, |(_, fraction)| fraction.len())
            ),
            tx(ui, &field.unit)
        );
    }
    if field.unit.is_empty()
        && let Some((number, suffix)) = value.split_once(' ')
        && numeric_units.contains(&suffix.split(' ').next().unwrap_or(""))
        && let Ok(number_value) = number.parse::<f64>()
    {
        return format!(
            "{} {}",
            lang.decimal(
                number_value,
                number
                    .split_once('.')
                    .map_or(0, |(_, fraction)| fraction.len())
            ),
            td(ui, suffix)
        );
    }
    let value = if field.unit.is_empty() {
        field.display()
    } else {
        format!("{} {}", value, tx(ui, &field.unit))
    };
    // Only authored sentinel values are localized; device names and identifiers are data.
    if [
        "Hardware adapter",
        "Software adapter",
        "Software renderer",
        "Other",
        "Not supplied",
        "Unavailable",
    ]
    .contains(&value.as_str())
    {
        tx(ui, value)
    } else if field.label.eq_ignore_ascii_case("type") || field.label.ends_with(" type") {
        td(ui, value)
    } else {
        value
    }
}
fn header(ui: &mut Ui, title: &str, _subtitle: &str) {
    ui.label(RichText::new(tx(ui, title)).size(27.));
    ui.add_space(22.);
}
fn panel(ui: &mut Ui, body: impl FnOnce(&mut Ui)) {
    let width = ui.available_width();
    Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .corner_radius(12)
        .inner_margin(18)
        .show(ui, |ui| {
            ui.set_width((width - 36.).max(100.));
            body(ui);
        });
}
fn section(ui: &mut Ui, title: &str, body: impl FnOnce(&mut Ui)) {
    ui.label(RichText::new(tx(ui, title)).size(17.));
    ui.add_space(9.);
    body(ui);
}
fn folding(ui: &Ui, title: impl Into<WidgetText>) -> CollapsingHeader {
    let title = title.into();
    let id = title.text().to_owned();
    let width =
        (ui.available_width() - ui.spacing().indent - ui.spacing().button_padding.x).max(1.);
    let galley = title.into_galley(ui, Some(TextWrapMode::Wrap), width, TextStyle::Button);
    CollapsingHeader::new(galley).id_salt(id)
}
fn loading(ui: &mut Ui, state: &State) {
    ui.add_space(30.);
    ui.label(td(
        ui,
        state
            .hardware_error
            .as_deref()
            .unwrap_or("Reading hardware…"),
    ));
}
fn field_value(ui: &Ui, fields: &[Field], name: &str) -> String {
    fields
        .iter()
        .find(|f| f.label == name)
        .map(|f| field_display(ui, f))
        .unwrap_or_else(|| tx(ui, "Unavailable"))
}
fn copy_label(ui: &mut Ui, value: &str, size: f32, color: Color32) {
    let r = ui.add(
        Label::new(RichText::new(value).size(size).color(color))
            .wrap()
            .sense(Sense::click()),
    );
    if r.clicked() {
        ui.ctx().copy_text(value.into());
    }
    r.on_hover_text(tx(ui, "Click to copy"));
}
fn summary_row(ui: &mut Ui, label: &str, value: &str, navigation: Option<(Page, &mut Page)>) {
    let width = ui.available_width();
    let label_width = (width * 0.32).clamp(106., 160.);
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(vec2(label_width, 26.), Layout::top_down(Align::Min), |ui| {
            ui.set_width(label_width);
            ui.add(
                Label::new(RichText::new(tx(ui, label)).size(13.).color(muted(ui)))
                    .halign(Align::Min),
            );
        });
        ui.allocate_ui_with_layout(
            vec2((width - label_width - 10.).max(80.), 26.),
            Layout::top_down(Align::Min),
            |ui| {
                ui.set_width((width - label_width - 10.).max(80.));
                let r = ui.add(Label::new(value).wrap().sense(Sense::click()));
                if let Some((page, target)) = navigation {
                    if r.clicked() {
                        *target = page;
                    }
                    r.on_hover_text(tx(ui, "Open hardware details"));
                } else {
                    if r.clicked() {
                        ui.ctx().copy_text(value.into());
                    }
                    r.on_hover_text(tx(ui, "Copy value"));
                }
            },
        );
    });
}
fn field_table(ui: &mut Ui, id: &str, fields: &[Field]) {
    ui.push_id(id,|ui|{
        let width=ui.available_width(); let label_width=(width*0.32).clamp(100.,185.);
        for f in fields {
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(vec2(label_width,0.),Layout::top_down(Align::Min),|ui|{
                    ui.set_width(label_width);ui.add(Label::new(RichText::new(td(ui, &f.label)).color(muted(ui)).size(13.)).wrap());
                });
                let available=(width-label_width-10.).max(60.);
                ui.allocate_ui_with_layout(vec2(available,0.),Layout::top_down(Align::Min),|ui|{
                    ui.set_width(available);
                    let value = if f.state == Availability::Unavailable && f.source.starts_with("SMBIOS ") { tx(ui, "Not reported by firmware") } else { field_display(ui, f) };
                    let response=ui.add(Label::new(RichText::new(&value).size(13.).color(if f.value.is_some(){ui.visuals().text_color()}else{muted(ui)})).wrap().sense(Sense::click()));
                    if response.clicked(){ui.ctx().copy_text(value);}
                    response.on_hover_text(td(ui, format!("Source: {}\nState: {}\n{}\nSample: {} ms since Unix epoch\nClick to copy",td(ui, &f.source),tx(ui, format!("{:?}", f.state)),td(ui, &f.detail),f.timestamp_ms)));
                });
            });ui.add_space(5.);
        }
    });
}
fn reading_row(ui: &mut Ui, label: &str, r: &Reading) {
    let value = reading_text(ui, r);
    let width = ui.available_width();
    if width < 280. {
        ui.label(RichText::new(td(ui, label)).color(muted(ui)).size(13.));
        numeric_value(ui, &value, width, 14.).on_hover_text(td(
            ui,
            format!(
                "{} · {}\n{}",
                tx(ui, format!("{:?}", r.state)),
                td(ui, &r.source),
                td(ui, &r.detail)
            ),
        ));
        return;
    }
    ui.horizontal(|ui| {
        let label_width = (width - 190.).max(80.);
        ui.allocate_ui_with_layout(
            vec2(label_width, 28.),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.set_width(label_width);
                ui.add(
                    Label::new(RichText::new(td(ui, label)).color(muted(ui)).size(13.)).truncate(),
                );
            },
        );
        numeric_value(ui, &value, 180., 14.).on_hover_text(td(
            ui,
            format!(
                "{}\n{} · {}\n{} ms since Unix epoch\nLast value: {}",
                td(ui, &r.source),
                tx(ui, format!("{:?}", r.state)),
                td(ui, &r.detail),
                r.timestamp_ms,
                r.value
                    .map(|v| number(ui, v, &r.unit))
                    .unwrap_or("—".into())
            ),
        ));
    });
}
fn reading_text(ui: &Ui, reading: &Reading) -> String {
    if reading.state == Availability::Valid {
        reading.value.map(|v| number(ui, v, &reading.unit))
    } else {
        None
    }
    .unwrap_or_else(|| tx(ui, availability(&reading.state)))
}
fn availability(state: &Availability) -> &'static str {
    match state {
        Availability::Unsupported => "Unsupported",
        Availability::PermissionRequired => "Permission required",
        Availability::Failed => "Provider failed",
        Availability::Stale => "Stale",
        _ => "Unavailable",
    }
}
fn numeric_value(ui: &mut Ui, text: &str, width: f32, size: f32) -> Response {
    let galley = ui.painter().layout(
        text.into(),
        FontId::monospace(size),
        ui.visuals().text_color(),
        width.max(1.),
    );
    let (rect, response) =
        ui.allocate_exact_size(vec2(width, galley.size().y.max(26.)), Sense::click());
    ui.painter().with_clip_rect(rect).galley(
        pos2(
            rect.right() - galley.size().x,
            rect.center().y - galley.size().y * 0.5,
        ),
        galley,
        ui.visuals().text_color(),
    );
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, text));
    if response.clicked() {
        ui.ctx().copy_text(text.into());
    }
    response
}

fn sensor_table(
    ui: &mut Ui,
    id: &str,
    readings: &[&(String, Reading)],
    history: &std::collections::VecDeque<Telemetry>,
    adapter: &str,
) {
    let width = ui.available_width();
    if width < 520. {
        ui.push_id(id, |ui| {
            for (index, (name, reading)) in readings.iter().enumerate() {
                if index > 0 {
                    ui.separator();
                }
                reading_row(ui, name, reading);
                let (min, max) = sensor_extrema(history, adapter, name);
                ui.horizontal_wrapped(|ui| {
                    for (label, value) in [("Min", min), ("Max", max)] {
                        ui.label(
                            RichText::new(td(
                                ui,
                                format!(
                                    "{} {}",
                                    tx(ui, label),
                                    value
                                        .map(|v| number(ui, v, &reading.unit))
                                        .unwrap_or("—".into())
                                ),
                            ))
                            .size(12.)
                            .monospace()
                            .color(muted(ui)),
                        );
                    }
                });
                ui.add_space(8.);
            }
        });
        return;
    }
    let value_width = (width * 0.19).clamp(90., 170.);
    let row_height = [
        "Unsupported",
        "Permission required",
        "Provider failed",
        "Stale",
        "Unavailable",
    ]
    .iter()
    .map(|label| {
        ui.painter()
            .layout(
                tx(ui, label),
                FontId::monospace(13.),
                ui.visuals().text_color(),
                value_width - 4.,
            )
            .size()
            .y
            + 10.
    })
    .fold(35.0f32, f32::max);
    TableBuilder::new(ui)
        .id_salt(id)
        .vscroll(false)
        .vertical_scroll_offset(0.)
        .scroll_bar_visibility(scroll_area::ScrollBarVisibility::AlwaysHidden)
        .column(Column::remainder())
        .columns(Column::exact(value_width), 3)
        .cell_layout(Layout::left_to_right(Align::Center))
        .header(30., |mut row| {
            for (index, label) in ["Sensor", "Current", "Minimum", "Maximum"]
                .iter()
                .enumerate()
            {
                row.col(|ui| {
                    if index == 0 {
                        ui.label(RichText::new(tx(ui, *label)).size(12.).color(muted(ui)));
                    } else {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.add_space(4.);
                            ui.label(RichText::new(tx(ui, *label)).size(12.).color(muted(ui)));
                        });
                    }
                });
            }
        })
        .body(|mut body| {
            for (name, r) in readings {
                body.row(row_height, |mut row| {
                    row.col(|ui| {
                        ui.add(Label::new(td(ui, name)).truncate())
                            .on_hover_text(td(
                                ui,
                                format!("{}\n{}", td(ui, &r.source), td(ui, &r.detail)),
                            ));
                    });
                    row.col(|ui| {
                        numeric_value(ui, &reading_text(ui, r), value_width - 4., 13.)
                            .on_hover_text(td(
                                ui,
                                format!(
                                    "{} · {}\n{}\nLast value: {}",
                                    tx(ui, format!("{:?}", r.state)),
                                    td(ui, &r.source),
                                    td(ui, &r.detail),
                                    r.value
                                        .map(|v| number(ui, v, &r.unit))
                                        .unwrap_or("—".into())
                                ),
                            ));
                    });
                    let (min, max) = sensor_extrema(history, adapter, name);
                    for value in [min, max] {
                        row.col(|ui| {
                            numeric_value(
                                ui,
                                &value.map(|v| number(ui, v, &r.unit)).unwrap_or("—".into()),
                                value_width - 4.,
                                13.,
                            );
                        });
                    }
                });
            }
        });
}
fn sensor_extrema(
    history: &std::collections::VecDeque<Telemetry>,
    adapter: &str,
    name: &str,
) -> (Option<f64>, Option<f64>) {
    let now = now_ms();
    let values = history
        .iter()
        .filter(|s| now.saturating_sub(s.timestamp_ms) <= 120_000)
        .filter_map(|s| gpu_reading(s, adapter, name).and_then(|r| r.value));
    (values.clone().reduce(f64::min), values.reduce(f64::max))
}
fn number(ui: &Ui, v: f64, unit: &str) -> String {
    if unit == "B" || unit == "bytes" {
        ui_bytes(ui, v.max(0.) as u64)
    } else {
        format!("{} {}", i18n::language(ui).decimal(v, 1), tx(ui, unit))
    }
}
fn memory_percent(s: &Telemetry) -> Option<f64> {
    let total = s.memory_total.value?;
    if total <= 0. {
        None
    } else {
        Some(s.memory_used.value? / total * 100.)
    }
}
fn gpu_reading<'a>(s: &'a Telemetry, id: &str, label: &str) -> Option<&'a Reading> {
    s.gpus
        .iter()
        .find(|g| g.adapter_id == id)?
        .readings
        .iter()
        .find(|(l, _)| l == label)
        .map(|(_, r)| r)
}
fn metric_chart(
    ui: &mut Ui,
    label: &str,
    points: &[(u64, Option<f64>)],
    unit: &str,
    max: f64,
    color: Color32,
    height: f32,
) {
    MetricChart {
        label,
        points,
        unit,
        max,
        color,
        height,
        secondary: None,
    }
    .show(ui);
}

struct MetricChart<'a> {
    label: &'a str,
    points: &'a [(u64, Option<f64>)],
    unit: &'a str,
    max: f64,
    color: Color32,
    height: f32,
    secondary: Option<(&'a str, Option<&'a Reading>)>,
}

impl MetricChart<'_> {
    fn show(self, ui: &mut Ui) {
        let Self {
            label,
            points,
            unit,
            max,
            color,
            height,
            secondary,
        } = self;
        let color = if ui.visuals().dark_mode {
            color
        } else if color == CPU {
            Color32::from_rgb(53, 109, 76)
        } else if color == MEMORY {
            Color32::from_rgb(54, 97, 151)
        } else {
            Color32::from_rgb(131, 97, 39)
        };
        Frame::new()
            .fill(ui.visuals().faint_bg_color)
            .corner_radius(12)
            .inner_margin(16)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                let stale = points
                    .last()
                    .is_some_and(|p| now_ms().saturating_sub(p.0) > 5000);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(tx(ui, label)).size(13.).color(muted(ui)));
                    if stale {
                        ui.label(RichText::new(tx(ui, "Stale")).size(12.).color(muted(ui)));
                    }
                    if let Some((label, _)) = secondary {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(RichText::new(tx(ui, label)).size(13.).color(muted(ui)));
                        });
                    }
                });
                let value = points
                    .last()
                    .and_then(|(_, v)| *v)
                    .map(|v| i18n::language(ui).decimal(v, 1))
                    .unwrap_or("—".into());
                let (rect, _) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 40.), Sense::hover());
                let mut primary_rect = rect;
                if let Some((label, reading)) = secondary {
                    let secondary_rect =
                        Rect::from_min_max(pos2(rect.right() - 100., rect.top()), rect.max);
                    primary_rect.max.x = secondary_rect.left() - 12.;
                    let current = reading
                        .filter(|r| r.state == Availability::Valid)
                        .and_then(|r| r.value);
                    let text = current
                        .map(|v| i18n::language(ui).decimal(v, 1))
                        .unwrap_or("—".into());
                    let unit = if current.is_some() {
                        reading.map(|r| r.unit.as_str()).unwrap_or("")
                    } else {
                        ""
                    };
                    paint_metric_value(ui, secondary_rect, &text, unit, 22., Align::Max);
                    let response = ui.interact(
                        secondary_rect,
                        ui.make_persistent_id(("metric-secondary", label)),
                        Sense::hover(),
                    );
                    let status = reading
                        .map(|r| reading_text(ui, r))
                        .unwrap_or_else(|| tx(ui, "Unavailable"));
                    let accessible_label = format!("{}: {status}", tx(ui, label));
                    response.widget_info(|| {
                        WidgetInfo::labeled(WidgetType::Label, true, &accessible_label)
                    });
                    response.on_hover_text(
                        reading
                            .map(|r| {
                                format!("{status}\n{}\n{}", td(ui, &r.source), td(ui, &r.detail))
                            })
                            .unwrap_or_else(|| tx(ui, "No temperature provider reading")),
                    );
                }
                paint_metric_value(ui, primary_rect, &value, unit, 28., Align::Min);
                let response = ui.interact(
                    primary_rect,
                    ui.make_persistent_id(("metric-value", label)),
                    Sense::hover(),
                );
                let accessible_label = format!(
                    "{}: {value} {unit}{}",
                    tx(ui, label),
                    if stale {
                        tx(ui, " (stale)")
                    } else {
                        String::new()
                    }
                );
                response.widget_info(|| {
                    WidgetInfo::labeled(WidgetType::Label, true, &accessible_label)
                });
                let (rect, response) = ui
                    .allocate_exact_size(vec2(ui.available_width(), height - 65.), Sense::hover());
                let rect = rect.shrink2(vec2(1., 9.));
                let p = ui.painter();
                for step in [0., 0.5, 1.] {
                    let y = rect.bottom() - rect.height() * step;
                    p.line_segment(
                        [pos2(rect.left(), y), pos2(rect.right(), y)],
                        Stroke::new(1.0f32, ui.visuals().widgets.noninteractive.bg_stroke.color),
                    );
                }
                let end = now_ms();
                let start = end.saturating_sub(120_000);
                let mut prior: Option<(u64, Pos2)> = None;
                for (time, value) in points {
                    if let Some(v) = value {
                        let x = rect.left()
                            + ((*time as f64 - start as f64) / 120_000.).clamp(0., 1.) as f32
                                * rect.width();
                        let y = rect.bottom() - (*v / max).clamp(0., 1.) as f32 * rect.height();
                        let point = pos2(x, y);
                        if let Some((last, previous)) = prior
                            && time.saturating_sub(last) <= 2500
                        {
                            p.line_segment([previous, point], Stroke::new(1.8f32, color));
                        }
                        prior = Some((*time, point));
                    } else {
                        prior = None;
                    }
                }
                if let Some((_, point)) = prior {
                    p.circle_filled(point, 2.5, color);
                }
                ui.horizontal(|ui| {
                    ui.label(RichText::new(tx(ui, "−120 s")).size(11.).color(muted(ui)));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(td(ui, format!("now · 0–{max:.0} {unit}")))
                                .size(11.)
                                .color(muted(ui)),
                        );
                    });
                });
                let values: Vec<_> = points.iter().filter_map(|(_, v)| *v).collect();
                if !values.is_empty() {
                    response.on_hover_text(td(
                        ui,
                        format!(
                            "{} samples\nMin {} {} · Max {} {}\nMissing samples create gaps",
                            i18n::language(ui).integer(values.len() as u64),
                            i18n::language(ui)
                                .decimal(values.iter().copied().fold(f64::INFINITY, f64::min), 1),
                            unit,
                            i18n::language(ui).decimal(
                                values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                                1
                            ),
                            unit
                        ),
                    ));
                } else {
                    response.on_hover_text(tx(ui, "No valid samples available from this provider"));
                }
            });
    }
}

fn paint_metric_value(ui: &Ui, rect: Rect, value: &str, unit: &str, size: f32, align: Align) {
    let mut job = egui::text::LayoutJob::default();
    job.append(
        value,
        0.,
        TextFormat {
            font_id: FontId::monospace(size),
            color: ui.visuals().text_color(),
            valign: Align::BOTTOM,
            ..Default::default()
        },
    );
    if !unit.is_empty() {
        job.append(
            unit,
            4.,
            TextFormat {
                font_id: FontId::proportional(13.),
                color: muted(ui),
                valign: Align::BOTTOM,
                ..Default::default()
            },
        );
    }
    let galley = ui.painter().layout_job(job);
    let x = if align == Align::Max {
        rect.right() - galley.size().x
    } else {
        rect.left()
    };
    let y = rect.center().y - galley.size().y * 0.5;
    ui.painter()
        .with_clip_rect(rect)
        .galley(pos2(x, y), galley, ui.visuals().text_color());
}
fn diagnostic_field(ui: &mut Ui, f: &Field) {
    ui.label(
        RichText::new(td(
            ui,
            format!("{}: {}", td(ui, &f.label), field_display(ui, f)),
        ))
        .strong(),
    );
    ui.label(
        RichText::new(td(
            ui,
            format!(
                "{} · {} · {} ms",
                tx(ui, format!("{:?}", f.state)),
                td(ui, &f.source),
                f.timestamp_ms
            ),
        ))
        .size(12.)
        .color(muted(ui)),
    );
    if !f.detail.is_empty() {
        ui.label(td(ui, &f.detail));
    }
    ui.add_space(10.);
}
fn reading_diagnostic(ui: &mut Ui, label: &str, r: &Reading) {
    reading_row(ui, label, r);
    ui.label(
        RichText::new(td(
            ui,
            format!(
                "{} · {} · {} ms",
                tx(ui, format!("{:?}", r.state)),
                td(ui, &r.source),
                r.timestamp_ms
            ),
        ))
        .size(12.)
        .color(muted(ui)),
    );
    ui.add_space(10.);
}
fn node_size(ui: &Ui, node: &crate::storage::ScanNode, allocated: bool) -> String {
    let value = if allocated {
        node.allocated
    } else {
        node.logical
    };
    if node.incomplete {
        if node.is_dir {
            format!("≥ {}", ui_bytes(ui, value))
        } else {
            "Unavailable".into()
        }
    } else {
        ui_bytes(ui, value)
    }
}
fn scan_status(state: &State) -> String {
    let lang = state.settings.language.resolve();
    if let Some(s) = &state.summary {
        let status = if s.cancelled {
            "Partial · cancelled"
        } else if s.errors > 0 || s.stopped_early || !s.incomplete_nodes.is_empty() {
            "Partial"
        } else if s.skipped_cloud > 0 || s.skipped_reparse > 0 {
            "Complete with exclusions"
        } else {
            "Complete"
        };
        lang.format(
            "{0} · {1} s · {2} errors",
            &[
                lang.text(status),
                lang.decimal(s.elapsed_ms as f64 / 1000., 2),
                lang.integer(s.errors),
            ],
        )
    } else if let Some(scan) = &state.scan {
        let status = if scan.cancel.load(std::sync::atomic::Ordering::Relaxed) {
            "Cancelling"
        } else {
            "Scanning"
        };
        lang.format(
            "{0} · {1} entries · {2} s",
            &[
                lang.text(status),
                lang.integer(state.nodes.len() as u64),
                lang.decimal(state.scan_started.elapsed().as_secs_f64(), 1),
            ],
        )
    } else {
        lang.text("No scan")
    }
}

fn reveal(path: &std::path::Path) -> Result<(), String> {
    explorer_command(path)?
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}
fn explorer_command(path: &std::path::Path) -> Result<std::process::Command, String> {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use windows::Win32::System::SystemInformation::GetWindowsDirectoryW;
    let path = std::path::absolute(path).map_err(|error| error.to_string())?;
    if path
        .as_os_str()
        .encode_wide()
        .any(|unit| unit == 0 || unit == b'"' as u16)
    {
        return Err("Path cannot be passed to Explorer".into());
    }
    let mut directory = vec![0u16; 32768];
    // Obtain the trusted OS location, independent of PATH and the working directory.
    let length = unsafe { GetWindowsDirectoryW(Some(&mut directory)) } as usize;
    if length == 0 || length >= directory.len() {
        return Err("Windows directory is unavailable".into());
    }
    let executable =
        PathBuf::from(std::ffi::OsString::from_wide(&directory[..length])).join("explorer.exe");
    let mut argument = std::ffi::OsString::from("/select,\"");
    argument.push(path.as_os_str());
    argument.push("\"");
    let mut command = std::process::Command::new(executable);
    command.raw_arg(argument);
    Ok(command)
}
#[cfg(windows)]
use std::os::windows::process::CommandExt;

fn navigation(ui: &mut Ui, label: &str, key: &str, selected: bool, kind: usize) -> bool {
    let translated = tx(ui, label);
    let label = translated.as_str();
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 40.), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::SelectableLabel, true, label));
    if selected || response.hovered() {
        ui.painter().rect_filled(
            rect,
            7,
            if selected {
                ui.visuals().selection.bg_fill
            } else {
                ui.visuals().widgets.hovered.bg_fill
            },
        );
    }
    if response.has_focus() {
        ui.painter()
            .rect_stroke(rect, 7, ui.visuals().selection.stroke, StrokeKind::Inside);
    }
    if selected {
        ui.painter().rect_filled(
            Rect::from_center_size(pos2(rect.left() + 2., rect.center().y), vec2(2., 18.)),
            1,
            ui.visuals().text_color(),
        );
    }
    let color = if selected {
        ui.visuals().text_color()
    } else {
        muted(ui)
    };
    icon(
        ui.painter(),
        Rect::from_min_size(rect.left_center() + vec2(10., -11.), vec2(22., 22.)),
        kind,
        color,
    );
    ui.painter().text(
        rect.left_center() + vec2(42., 0.),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(14.),
        ui.visuals().text_color(),
    );
    let label_width = ui
        .painter()
        .layout_no_wrap(
            label.into(),
            FontId::proportional(14.),
            ui.visuals().text_color(),
        )
        .size()
        .x;
    // Long translations keep their full name; shortcuts remain in the tooltip.
    if !key.is_empty() && label_width + 76. <= rect.width() {
        ui.painter().text(
            rect.right_center() - vec2(9., 0.),
            Align2::RIGHT_CENTER,
            key,
            FontId::monospace(10.),
            muted(ui),
        );
    }
    response
        .on_hover_text(if key.is_empty() {
            label.to_owned()
        } else {
            format!("{label} · {}+{key}", tx(ui, "Ctrl"))
        })
        .clicked()
}
fn folder_toggle(ui: &mut Ui, name: &str, expanded: bool) -> Response {
    let label = format!(
        "{} {name}",
        tx(ui, if expanded { "Collapse" } else { "Expand" })
    );
    let response = ui.add_sized([28., 28.], Button::new("").frame(false));
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), &label));
    let c = response.rect.center();
    let points = if expanded {
        vec![c + vec2(-5., -2.5), c + vec2(0., 2.5), c + vec2(5., -2.5)]
    } else {
        vec![c + vec2(-2.5, -5.), c + vec2(2.5, 0.), c + vec2(-2.5, 5.)]
    };
    ui.painter().add(Shape::line(
        points,
        Stroke::new(2.0_f32, ui.visuals().text_color()),
    ));
    response.on_hover_text(tx(ui, label))
}
fn icon(p: &Painter, r: Rect, kind: usize, color: Color32) {
    // One 24-unit grid, optical size and rounded stroke treatment throughout.
    let scale = r.width() / 24.;
    let s = Stroke::new(1.8 * scale, color);
    let point = |x: f32, y: f32| r.min + vec2(x, y) * scale;
    let outline = |x: f32, y: f32, w: f32, h: f32, radius: f32| {
        p.rect_stroke(
            Rect::from_min_max(point(x, y), point(x + w, y + h)),
            CornerRadius::same((radius * scale).round() as u8),
            s,
            StrokeKind::Middle,
        );
    };
    let line = |points: &[(f32, f32)]| {
        let points: Vec<_> = points.iter().map(|&(x, y)| point(x, y)).collect();
        p.add(Shape::line(points.clone(), s));
        for point in points {
            p.circle_filled(point, s.width / 2., color);
        }
    };
    match kind {
        0 => {
            outline(2., 4., 20., 16., 2.);
            line(&[(9., 4.), (9., 20.)]);
            line(&[(9., 12.), (22., 12.)]);
        }
        1 => {
            outline(6., 6., 12., 12., 2.);
            outline(9., 9., 6., 6., 1.);
            for i in [9., 15.] {
                line(&[(i, 2.), (i, 6.)]);
                line(&[(i, 18.), (i, 22.)]);
                line(&[(2., i), (6., i)]);
                line(&[(18., i), (22., i)]);
            }
        }
        2 => {
            outline(4., 6., 18., 11., 2.);
            line(&[(2., 3.), (2., 20.), (5., 20.)]);
            for x in [8., 11.] {
                line(&[(x, 17.), (x, 20.)]);
            }
            line(&[(7., 9.5), (7., 13.5)]);
            p.circle_stroke(point(16., 11.5), 3. * scale, s);
            p.circle_filled(point(16., 11.5), 0.8 * scale, color);
        }
        3 => {
            outline(4., 3., 16., 18., 3.);
            line(&[(4., 14.), (20., 14.)]);
            line(&[(7., 17.5), (10., 17.5)]);
            p.circle_filled(point(16., 17.5), 0.9 * scale, color);
        }
        4 => {
            line(&[
                (2., 13.),
                (7., 13.),
                (10., 6.),
                (14., 19.),
                (17., 13.),
                (22., 13.),
            ]);
        }
        _ => {
            for (x, y) in [(8., 5.), (16., 12.), (10., 19.)] {
                line(&[(2., y), (x - 2.5, y)]);
                line(&[(x + 2.5, y), (22., y)]);
                p.circle_stroke(point(x, y), 2.5 * scale, s);
            }
        }
    }
}
fn tile_color(id: usize, dark: bool) -> Color32 {
    let colors = if dark {
        [
            (60, 79, 73),
            (60, 71, 85),
            (87, 77, 58),
            (76, 69, 86),
            (65, 79, 83),
            (87, 66, 65),
        ]
    } else {
        [
            (69, 97, 86),
            (70, 92, 119),
            (115, 97, 63),
            (96, 79, 112),
            (70, 99, 109),
            (116, 79, 74),
        ]
    };
    let (r, g, b) = colors[id % colors.len()];
    Color32::from_rgb(r, g, b)
}

// Balanced binary subdivision: O(n log n), no zero-area hit targets or fictitious weights.
pub fn treemap_tiles(
    ids: &[usize],
    nodes: &[crate::storage::ScanNode],
    rect: Rect,
) -> Vec<(usize, Rect)> {
    fn split(
        ids: &[usize],
        nodes: &[crate::storage::ScanNode],
        rect: Rect,
        out: &mut Vec<(usize, Rect)>,
    ) {
        if ids.is_empty() {
            return;
        }
        if ids.len() == 1 {
            out.push((ids[0], rect));
            return;
        }
        let total: f64 = ids.iter().map(|id| nodes[*id].logical as f64).sum();
        if total <= 0. {
            return;
        }
        let mut sum = 0.;
        let mut cut = 1;
        for (i, id) in ids.iter().enumerate().take(ids.len() - 1) {
            sum += nodes[*id].logical as f64;
            cut = i + 1;
            if sum >= total / 2. {
                break;
            }
        }
        let fraction = (sum / total) as f32;
        let (a, b) = if rect.width() >= rect.height() {
            let x = rect.left() + rect.width() * fraction;
            (
                Rect::from_min_max(rect.min, pos2(x, rect.bottom())),
                Rect::from_min_max(pos2(x, rect.top()), rect.max),
            )
        } else {
            let y = rect.top() + rect.height() * fraction;
            (
                Rect::from_min_max(rect.min, pos2(rect.right(), y)),
                Rect::from_min_max(pos2(rect.left(), y), rect.max),
            )
        };
        split(&ids[..cut], nodes, a, out);
        split(&ids[cut..], nodes, b, out);
    }
    let mut out = Vec::new();
    split(ids, nodes, rect, &mut out);
    out
}

#[cfg(test)]
mod tests;
