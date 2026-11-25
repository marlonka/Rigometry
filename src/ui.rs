mod platform;
mod storage;
mod theme;
mod widgets;

use crate::{
    i18n::{self, Language, message as td, text as tx},
    model::*,
    state::{Page, Settings, State},
};
use eframe::egui::{self, *};
use egui_extras::{Column, TableBuilder};
#[cfg(test)]
use platform::*;
use platform::{COMMAND_KEY, NO_GPU, PLATFORM_LABEL, elevated};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use theme::{BASE_SCALE, medium, palette, regular, semibold};
use widgets::*;

const CAPTURE_WARMUP: Duration = Duration::from_secs(120);
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(180);
/// Space between the window edge and the content canvas.
const CANVAS_OUTSET: i8 = 8;
/// Padding inside the canvas; the right side holds the page scrollbar.
const CANVAS_PADDING: Margin = Margin {
    left: 32,
    right: 14,
    top: 28,
    bottom: 6,
};
/// The page scrollbar never comes closer than this to the window's right edge.
#[cfg(test)]
const PAGE_RIGHT_CLEARANCE: f32 = (CANVAS_OUTSET + CANVAS_PADDING.right) as f32;

pub struct App {
    pub state: State,
    capture_dir: Option<PathBuf>,
    capture_index: usize,
    capture_at: Instant,
    capture_pending: Option<PathBuf>,
    capture_worker: Option<std::thread::JoinHandle<Result<(), String>>>,
    capture_failed: Arc<AtomicBool>,
    /// Laid-out size-map tiles: entry, rectangle and color index.
    map_cache: Vec<(usize, Rect, usize)>,
    map_key: Option<(u64, Vec2)>,
    /// Where the map was when `map_cache` was laid out.
    map_origin: Pos2,
    /// Which tile kinds the map shows, for the map revision it was counted at.
    map_kinds: (u64, [bool; 7]),
    cpu_tab: CpuTab,
    window_override: Option<Vec2>,
    /// The window was closed during an update; it closes when that ends.
    close_after_update: bool,
    /// The sidebar's update notice was clicked: show the Updates section.
    reveal_updates: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CpuTab {
    Processor,
    Memory,
    Mainboard,
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        scan: Option<String>,
        capture_dir: Option<PathBuf>,
        capture_failed: Arc<AtomicBool>,
        updated_from: Option<String>,
    ) -> Self {
        let capture = capture_dir.is_some();
        let mut app = Self::with_state(cc, scan, capture_dir, capture_failed, State::new);
        // The real app looks for new versions; tests and screenshots never do.
        if !capture {
            let record = cc
                .storage
                .and_then(|storage| eframe::get_value(storage, "update"))
                .unwrap_or_default();
            // An elevated copy would start the new version elevated too.
            let elevated = cfg!(windows) && elevated();
            app.state.updates = Some(crate::state::Updates::new(record, elevated));
            if let Some(from) = updated_from {
                app.state.notice = Some(format!(
                    "Updated from {from} to {}",
                    env!("CARGO_PKG_VERSION")
                ));
            }
        }
        app
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
        if let Some(value) = crate::argument(&args, "--theme") {
            settings.theme = value;
        }
        if let Some(value) =
            crate::argument(&args, "--scale").and_then(|value| value.parse::<f32>().ok())
        {
            settings.scale = finite_scale(value);
        }
        if let Some(value) =
            crate::argument(&args, "--language").and_then(|code| Language::parse(&code))
        {
            settings.language = value;
        }
        i18n::set_context(&cc.egui_ctx, settings.language);
        theme::apply(&cc.egui_ctx, &settings);
        let mut state = create_state(settings, cc.egui_ctx.clone());
        if let Some(path) = scan {
            state.settings.path = path;
            state.settings.page = Page::Storage;
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
            map_origin: Pos2::ZERO,
            map_kinds: (u64::MAX, [false; 7]),
            cpu_tab: CpuTab::Processor,
            close_after_update: false,
            reveal_updates: false,
        }
    }

    fn sidebar(&mut self, ui: &mut Ui) {
        let p = palette(ui);
        let compact = ui.available_height() < 560.;
        // Below this even the update notice gives way to navigation.
        let dense = ui.available_height() < 430.;
        let page = self.state.settings.page;
        ui.horizontal(|ui| {
            ui.add_space(6.);
            let (rect, _) = ui.allocate_exact_size(vec2(28., 28.), Sense::hover());
            crate::branding::paint(ui.painter(), rect);
            ui.add_space(2.);
            ui.label(RichText::new("Rigometry").font(semibold(15.)));
        });
        ui.add_space(if compact { 12. } else { 24. });
        ui.spacing_mut().item_spacing.y = 2.;
        for (target, label, key, icon) in [
            (Page::Overview, "Overview", "1", Icon::Overview),
            (Page::Cpu, "CPU & Memory", "2", Icon::Cpu),
            (Page::Gpu, "GPU", "3", Icon::Gpu),
            (Page::Storage, "Storage", "4", Icon::Storage),
        ] {
            let shortcut = format!("{}+{key}", tx(ui, COMMAND_KEY));
            if nav_item(ui, label, Some((key, &shortcut)), page == target, icon) {
                self.state.settings.page = target;
            }
        }
        ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
            ui.spacing_mut().item_spacing.y = 2.;
            if !compact {
                ui.horizontal(|ui| {
                    ui.add_space(10.);
                    caption(ui, PLATFORM_LABEL);
                });
                if let Some((status, warning)) = self.hardware_status(ui) {
                    let color = if warning { p.warn } else { p.faint };
                    ui.horizontal(|ui| {
                        ui.add_space(10.);
                        ui.label(RichText::new(status).font(regular(12.5)).color(color));
                    });
                }
                ui.add_space(10.);
            }
            if nav_item(ui, "Settings", None, page == Page::Settings, Icon::Settings) {
                self.state.settings.page = Page::Settings;
            }
            if nav_item(
                ui,
                "Diagnostics",
                None,
                page == Page::Diagnostics,
                Icon::Diagnostics,
            ) {
                self.state.settings.page = Page::Diagnostics;
            }
            if !dense && let Some(version) = self.offered_update() {
                ui.add_space(8.);
                let text = td(ui, format!("Update {version} available"));
                let button = Button::new(RichText::new(text).font(medium(13.)).color(p.text))
                    .fill(theme::mix(p.window, p.accent, 0.16))
                    .stroke(Stroke::new(1.0_f32, theme::mix(p.window, p.accent, 0.4)))
                    .corner_radius(8)
                    .min_size(vec2(ui.available_width(), 32.));
                if ui.add(button).clicked() {
                    self.state.settings.page = Page::Settings;
                    self.reveal_updates = true;
                }
            }
        });
    }

    /// A new version, unless the user chose to skip it.
    fn offered_update(&self) -> Option<String> {
        let updates = self.state.updates.as_ref()?;
        updates
            .offered()
            .filter(|r| updates.record.skipped.as_deref() != Some(&r.version))
            .map(|r| r.version.clone())
    }

    /// Why live readings are missing or old, if they are, and whether that
    /// is a problem rather than startup.
    fn hardware_status(&self, ui: &Ui) -> Option<(String, bool)> {
        match self.state.history.back() {
            Some(sample) if now_ms().saturating_sub(sample.timestamp_ms) > 5000 => {
                Some((tx(ui, "Telemetry stale"), true))
            }
            Some(_) => None,
            None => Some((tx(ui, "Reading hardware…"), false)),
        }
    }

    fn overview(&mut self, ui: &mut Ui) {
        page_header(ui, "Overview", None);
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
            .join("\n");
        let os = inv
            .os
            .iter()
            .filter_map(|f| f.value.as_ref())
            .take(2)
            .cloned()
            .collect::<Vec<_>>()
            .join(" · ");
        let gpu = if gpu.is_empty() {
            tx(ui, "No graphics adapter reported")
        } else {
            gpu
        };
        let memory = field_value(ui, &inv.memory, "Installed capacity");
        let board = motherboard_name(ui, &inv.motherboard);
        let bios = field_value(ui, &inv.motherboard, "BIOS version");
        section_header(ui, "Live activity", |ui| {
            caption(ui, tx(ui, "120 s window"));
        });
        let cpu_points = self.series(|s| s.cpu_usage.value);
        let memory_points = self.series(memory_percent);
        let gpu_id = inv
            .adapters
            .iter()
            .find(|a| a.vendor_id != 0x1414)
            .map(|a| a.id.as_str())
            .unwrap_or("");
        let gpu_points =
            self.series(|s| gpu_reading(s, gpu_id, "Utilization").and_then(|r| r.value));
        let temperature = self
            .state
            .history
            .back()
            .and_then(|s| gpu_reading(s, gpu_id, "Temperature"));
        // One instrument strip: three readings side by side on one surface.
        card_with(ui, Margin::symmetric(20, 18), |ui| {
            // Wide enough for "100.0 %" beside the GPU temperature.
            divided_columns(ui, 3, 250., |ui, index| {
                let (label, points, series, secondary) = match index {
                    0 => ("CPU", &cpu_points, Series::Cpu, None),
                    1 => ("Memory", &memory_points, Series::Memory, None),
                    _ => (
                        "GPU",
                        &gpu_points,
                        Series::Gpu,
                        Some(("Temperature", temperature)),
                    ),
                };
                MetricChart {
                    label,
                    points,
                    unit: "%",
                    max: 100.,
                    series,
                    plot_height: 64.,
                    secondary,
                    axis: false,
                }
                .content(ui);
            });
        });
        ui.add_space(SECTION_GAP);
        let page = &mut self.state.settings.page;
        card_with(ui, Margin::symmetric(20, 18), |ui| {
            divided_columns(ui, 2, 280., |ui, index| {
                if index == 0 {
                    card_title(ui, "This PC");
                    let width = label_column(
                        ui,
                        ["Processor", "Graphics", "Memory"].map(|l| tx(ui, l)),
                        80.,
                        140.,
                    );
                    summary_row(ui, "Processor", &cpu, width, Some((Page::Cpu, &mut *page)));
                    summary_row(ui, "Graphics", &gpu, width, Some((Page::Gpu, &mut *page)));
                    summary_row(ui, "Memory", &memory, width, Some((Page::Cpu, &mut *page)));
                } else {
                    card_title(ui, "System");
                    let width = label_column(
                        ui,
                        ["Motherboard", crate::model::OS_NAME, "BIOS"].map(|l| tx(ui, l)),
                        80.,
                        140.,
                    );
                    summary_row(ui, "Motherboard", &board, width, None);
                    summary_row(ui, crate::model::OS_NAME, &os, width, None);
                    summary_row(ui, "BIOS", &bios, width, None);
                }
            });
        });
        ui.add_space(SECTION_GAP);
        section_header(ui, "Drives", |ui| {
            if ui.button(tx(ui, "Scan folder…")).clicked() {
                self.state.settings.page = Page::Storage;
                self.state.pick_folder();
            }
        });
        let mut drives = inv.drives.clone();
        drives.sort_by(|a, b| a.mount.cmp(&b.mount));
        let mut scan = None;
        card_with(ui, Margin::symmetric(18, 8), |ui| {
            if drives.is_empty() {
                ui.add_space(8.);
                caption(ui, tx(ui, "Unavailable"));
                ui.add_space(8.);
            }
            for (index, drive) in drives.iter().enumerate() {
                if index > 0 {
                    divider(ui);
                }
                if drive_row(ui, drive) {
                    scan = Some(drive.mount.clone());
                }
            }
        });
        if let Some(path) = scan {
            self.state.settings.path = path;
            self.state.settings.page = Page::Storage;
            self.state.start_scan();
        }
        ui.add_space(10.);
        caption(
            ui,
            tx(
                ui,
                "Drive space is a volume snapshot; scan totals exclude filesystem overhead.",
            ),
        );
        ui.add_space(SECTION_GAP);
    }

    /// One value per retained sample, `None` where it was missing.
    fn series(&self, value: impl Fn(&Telemetry) -> Option<f64>) -> Vec<(u64, Option<f64>)> {
        self.state
            .history
            .iter()
            .map(|s| (s.timestamp_ms, value(s)))
            .collect()
    }

    fn cpu(&mut self, ui: &mut Ui) {
        let Some(inv) = self.state.inventory.clone() else {
            page_header(ui, "CPU & Memory", None);
            loading(ui, &self.state);
            return;
        };
        let model = field_value(ui, &inv.cpu, "Model");
        page_header(ui, "CPU & Memory", Some(&model));
        let cpu_points = self.series(|s| s.cpu_usage.value);
        let memory_points = self.series(memory_percent);
        columns(ui, 2, 260., |ui, index| {
            if index == 0 {
                metric_chart(
                    ui,
                    "CPU utilization",
                    &cpu_points,
                    "%",
                    100.,
                    Series::Cpu,
                    96.,
                );
            } else {
                metric_chart(
                    ui,
                    "Memory utilization",
                    &memory_points,
                    "%",
                    100.,
                    Series::Memory,
                    96.,
                );
            }
        });
        ui.add_space(SECTION_GAP);
        if let Some(tab) = segmented(
            ui,
            "cpu-tab",
            self.cpu_tab,
            &[
                (CpuTab::Processor, "Processor"),
                (CpuTab::Memory, "Memory"),
                (CpuTab::Mainboard, "Mainboard"),
            ],
        ) {
            self.cpu_tab = tab;
        }
        ui.add_space(14.);
        match self.cpu_tab {
            CpuTab::Processor => self.processor_tab(ui, &inv),
            CpuTab::Memory => self.memory_tab(ui, &inv),
            CpuTab::Mainboard => {
                card(ui, |ui| field_table(ui, "board-fields", &inv.motherboard));
            }
        }
        ui.add_space(SECTION_GAP);
    }

    fn processor_tab(&self, ui: &mut Ui, inv: &Inventory) {
        const DETAILS: [&str; 4] = [
            "Instruction sets (hardware)",
            "Temperature",
            "Core voltage",
            "Package power",
        ];
        let p = palette(ui);
        columns(ui, 2, 320., |ui, index| {
            if index == 0 {
                card(ui, |ui| {
                    card_title(ui, "Processor");
                    let primary: Vec<_> = inv
                        .cpu
                        .iter()
                        .filter(|f| {
                            f.state == Availability::Valid
                                && f.label != "Model"
                                && !DETAILS.contains(&f.label.as_str())
                        })
                        .cloned()
                        .collect();
                    field_table(ui, "cpu-fields", &primary);
                    ui.add_space(4.);
                    folding(ui, tx(ui, "Instruction sets & sensor availability")).show(ui, |ui| {
                        let fields: Vec<_> = inv
                            .cpu
                            .iter()
                            .filter(|f| {
                                f.state != Availability::Valid
                                    || DETAILS.contains(&f.label.as_str())
                            })
                            .cloned()
                            .collect();
                        field_table(ui, "cpu-extra", &fields);
                    });
                });
                return;
            }
            card(ui, |ui| {
                card_title(ui, "Logical processors");
                let Some(sample) = self.state.history.back() else {
                    caption(ui, tx(ui, "Waiting for processor samples…"));
                    return;
                };
                // One line per logical processor: name, meter, value.
                let columns = match ui.available_width() {
                    width if width >= 720. => 4,
                    width if width >= 520. => 3,
                    _ => 2,
                };
                let gap = 20.;
                let cell_width =
                    ((ui.available_width() - (columns - 1) as f32 * gap) / columns as f32).floor();
                ui.spacing_mut().item_spacing.y = 4.;
                for (row, chunk) in sample.per_core_usage.chunks(columns).enumerate() {
                    // An exact row height: `horizontal` would pad it to a control's.
                    let width = ui.available_width();
                    ui.allocate_ui_with_layout(
                        vec2(width, 22.),
                        Layout::left_to_right(Align::Center),
                        |ui| {
                            ui.spacing_mut().item_spacing.x = gap;
                            for (offset, reading) in chunk.iter().enumerate() {
                                self.core_cell(ui, row * columns + offset, reading, cell_width, p);
                            }
                        },
                    );
                }
                ui.spacing_mut().item_spacing.y = 6.;
                ui.add_space(12.);
                divider(ui);
                ui.add_space(6.);
                reading_row(ui, "OS-reported clock", &sample.cpu_frequency);
            });
        });
    }

    fn core_cell(&self, ui: &mut Ui, i: usize, r: &Reading, width: f32, p: &theme::Palette) {
        let (rect, response) = ui.allocate_exact_size(vec2(width, 22.), Sense::hover());
        let name = format!("CPU {i:02}");
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
        response.widget_info(|| {
            WidgetInfo::labeled(WidgetType::Label, true, format!("{name}: {value}"))
        });
        let painter = ui.painter();
        let label = painter.layout_no_wrap(name, regular(12.), p.faint);
        let number = painter.layout_no_wrap(value, medium(12.5), p.text);
        let value_width = number.size().x.max(36.);
        painter.galley(
            pos2(rect.left(), rect.center().y - label.size().y / 2.),
            label.clone(),
            p.faint,
        );
        painter.galley(
            pos2(
                rect.right() - number.size().x,
                rect.center().y - number.size().y / 2.,
            ),
            number,
            p.text,
        );
        let fraction = match r.value {
            Some(value) if r.state == Availability::Valid => {
                let target = value as f32 / 100.;
                if self.state.settings.reduce_motion {
                    target
                } else {
                    ui.ctx()
                        .animate_value_with_time(Id::new(("core", i)), target, 0.16)
                }
            }
            _ => 0.,
        };
        let left = rect.left() + label.size().x + 10.;
        let right = rect.right() - value_width - 10.;
        if right > left {
            let bar = Rect::from_min_max(
                pos2(left, rect.center().y - 3.),
                pos2(right, rect.center().y + 3.),
            );
            paint_meter(ui, bar, fraction, p.cpu);
        }
    }

    fn memory_tab(&self, ui: &mut Ui, inv: &Inventory) {
        let p = palette(ui);
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(field_value(ui, &inv.memory, "Installed capacity"))
                        .font(theme::display(26.)),
                );
                muted(ui, tx(ui, "installed"), 14.);
            });
            let Some(sample) = self.state.history.back() else {
                return;
            };
            if sample.memory_used.state == Availability::Valid
                && sample.memory_total.state == Availability::Valid
                && let (Some(used), Some(total)) =
                    (sample.memory_used.value, sample.memory_total.value)
            {
                ui.add_space(4.);
                muted(
                    ui,
                    td(
                        ui,
                        format!(
                            "{} in use · {} available to the system",
                            ui_bytes(ui, used as u64),
                            ui_bytes(ui, total as u64)
                        ),
                    ),
                    13.5,
                );
                ui.add_space(10.);
                let fraction = if total > 0. {
                    (used / total) as f32
                } else {
                    0.
                };
                meter(ui, fraction, ui.available_width(), 6., p.memory);
            } else {
                let state = if sample.memory_used.state != Availability::Valid {
                    &sample.memory_used.state
                } else {
                    &sample.memory_total.state
                };
                ui.label(td(
                    ui,
                    format!("Memory readings: {}", tx(ui, availability(state))),
                ));
            }
        });
        ui.add_space(CARD_GAP);
        let module_count = inv
            .memory
            .iter()
            .filter(|f| f.label.ends_with(" capacity") && per_module(&f.label))
            .count();
        let module = |ui: &mut Ui, n: usize| {
            card(ui, |ui| {
                card_title(ui, &format!("Module {n}"));
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
        // Side by side in pairs; a lone last module keeps half the width
        // there, and stacked modules leave no gap for a missing partner.
        let side_by_side = (ui.available_width() - CARD_GAP) / 2. >= 300.;
        for pair in (0..module_count).step_by(2) {
            let count = (module_count - pair).min(2);
            if side_by_side {
                columns(ui, 2, 300., |ui, j| {
                    if j < count {
                        module(ui, pair + j + 1);
                    }
                });
            } else {
                columns(ui, count, 300., |ui, j| module(ui, pair + j + 1));
            }
            ui.add_space(CARD_GAP);
        }
        folding(ui, tx(ui, "Channel mode, timings & profiles")).show(ui, |ui| {
            let fields: Vec<_> = inv
                .memory
                .iter()
                .filter(|f| !per_module(&f.label) && f.label != "Installed capacity")
                .cloned()
                .collect();
            field_table(ui, "memory-extra", &fields);
        });
    }

    fn gpu(&mut self, ui: &mut Ui) {
        let Some(inv) = self.state.inventory.clone() else {
            page_header(ui, "GPU", None);
            loading(ui, &self.state);
            return;
        };
        if inv.adapters.is_empty() {
            page_header(ui, "GPU", None);
            empty_state(ui, Icon::Gpu, &tx(ui, NO_GPU));
            return;
        }
        self.state.adapter = self.state.adapter.min(inv.adapters.len() - 1);
        let adapters = &inv.adapters;
        let selected = &mut self.state.adapter;
        // One adapter is named under the title; several get a picker.
        let subtitle = (adapters.len() == 1).then(|| adapters[0].name.clone());
        let picker = (adapters.len() > 1).then_some(|ui: &mut Ui| {
            ComboBox::from_id_salt("adapter")
                .width(ui.available_width().min(360.))
                .selected_text(&adapters[*selected].name)
                .icon(combo_icon)
                .show_ui(ui, |ui| {
                    for (i, a) in adapters.iter().enumerate() {
                        ui.selectable_value(selected, i, &a.name);
                    }
                });
        });
        page_header_with(ui, "GPU", subtitle.as_deref(), picker);
        let adapter = &inv.adapters[self.state.adapter];
        let utilization =
            self.series(|s| gpu_reading(s, &adapter.id, "Utilization").and_then(|r| r.value));
        let temperature =
            self.series(|s| gpu_reading(s, &adapter.id, "Temperature").and_then(|r| r.value));
        columns(ui, 2, 260., |ui, index| {
            if index == 0 {
                metric_chart(ui, "Utilization", &utilization, "%", 100., Series::Gpu, 96.);
            } else {
                metric_chart(
                    ui,
                    "Temperature",
                    &temperature,
                    "°C",
                    110.,
                    Series::Temperature,
                    96.,
                );
            }
        });
        ui.add_space(CARD_GAP);
        card(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.;
            ui.label(RichText::new(tx(ui, "Sensors")).font(semibold(15.)));
            caption(ui, tx(ui, "Minimum / maximum · last 120 s"));
            ui.spacing_mut().item_spacing.y = 6.;
            ui.add_space(8.);
            let sample = self
                .state
                .history
                .back()
                .and_then(|s| s.gpus.iter().find(|g| g.adapter_id == adapter.id));
            let Some(sample) = sample else {
                caption(ui, tx(ui, "Waiting for sensor samples…"));
                return;
            };
            let detail = |name: &str| {
                name.starts_with("Process ")
                    || name.starts_with("Engine ")
                    || [
                        "Power (driver percentage)",
                        "NVIDIA memory allocated + reserved",
                        "Fan",
                    ]
                    .contains(&name)
            };
            let main: Vec<_> = sample
                .readings
                .iter()
                .filter(|(name, _)| !detail(name))
                .collect();
            sensor_table(ui, "sensors", &main, &self.state.history, &adapter.id);
            ui.add_space(4.);
            folding(ui, tx(ui, "Driver counters & process memory")).show(ui, |ui| {
                let extras: Vec<_> = sample
                    .readings
                    .iter()
                    .filter(|(name, _)| detail(name))
                    .collect();
                sensor_table(
                    ui,
                    "sensors-extra",
                    &extras,
                    &self.state.history,
                    &adapter.id,
                );
            });
        });
        ui.add_space(CARD_GAP);
        const IDENTITY: [&str; 4] = [
            "Sensor binding",
            "LUID",
            "Subsystem / revision",
            "Dedicated system memory",
        ];
        card(ui, |ui| {
            card_title(ui, "Adapter specifications");
            let fields: Vec<_> = adapter
                .fields
                .iter()
                .filter(|f| !IDENTITY.contains(&f.label.as_str()))
                .cloned()
                .collect();
            let midpoint = fields.len().div_ceil(2);
            columns(ui, 2, 280., |ui, index| {
                if index == 0 {
                    field_table(ui, "gpu-left", &fields[..midpoint]);
                } else {
                    field_table(ui, "gpu-right", &fields[midpoint..]);
                }
            });
            ui.add_space(4.);
            folding(ui, tx(ui, "Provider binding & identifiers")).show(ui, |ui| {
                let fields: Vec<_> = adapter
                    .fields
                    .iter()
                    .filter(|f| IDENTITY.contains(&f.label.as_str()))
                    .cloned()
                    .collect();
                field_table(ui, "gpu-identity", &fields);
            });
        });
        ui.add_space(SECTION_GAP);
    }

    fn diagnostics(&mut self, ui: &mut Ui) {
        let mut export = false;
        page_header_with(
            ui,
            "Diagnostics",
            None,
            Some(|ui: &mut Ui| {
                export = ui.button(tx(ui, "Export hardware & samples…")).clicked();
            }),
        );
        if export {
            self.state.export_hardware();
        }
        if let Some(error) = &self.state.hardware_error {
            ui.label(RichText::new(td(ui, error)).color(palette(ui).error));
            ui.add_space(12.);
        }
        let p = palette(ui);
        if let Some(inv) = &self.state.inventory {
            for note in &inv.diagnostics {
                ui.add(Label::new(RichText::new(td(ui, note)).color(p.muted)).wrap());
                ui.add_space(4.);
            }
            ui.add_space(16.);
            section_header(ui, "Inventory", |_| {});
            card(ui, |ui| {
                for (name, fields) in [
                    ("CPU", &inv.cpu),
                    ("Memory", &inv.memory),
                    ("Motherboard", &inv.motherboard),
                    (crate::model::OS_NAME, &inv.os),
                ] {
                    folding(ui, tx(ui, name))
                        .id_salt(("inventory", name))
                        .show(ui, |ui| {
                            for f in fields {
                                diagnostic_field(ui, f);
                            }
                        });
                }
                for adapter in &inv.adapters {
                    // Two identical GPUs share a name; their IDs differ.
                    folding(ui, &adapter.name)
                        .id_salt(("inventory-adapter", &adapter.id))
                        .show(ui, |ui| {
                            for f in &adapter.fields {
                                diagnostic_field(ui, f);
                            }
                        });
                }
            });
        }
        if let Some(sample) = self.state.history.back() {
            ui.add_space(SECTION_GAP);
            section_header(ui, "Latest sample", |ui| {
                caption(
                    ui,
                    td(ui, format!("{} ms since Unix epoch", sample.timestamp_ms)),
                );
            });
            card(ui, |ui| {
                for (label, r) in [
                    ("CPU utilization", &sample.cpu_usage),
                    ("Memory used", &sample.memory_used),
                    ("OS-reported frequency", &sample.cpu_frequency),
                ] {
                    reading_diagnostic(ui, label, r);
                }
                for gpu in &sample.gpus {
                    // Name the adapter as the inventory does; its ID is in the samples.
                    let name = self
                        .state
                        .inventory
                        .as_ref()
                        .and_then(|inv| inv.adapters.iter().find(|a| a.id == gpu.adapter_id))
                        .map_or(gpu.adapter_id.as_str(), |a| a.name.as_str());
                    folding(ui, name)
                        .id_salt(("sample-adapter", &gpu.adapter_id))
                        .show(ui, |ui| {
                            for (label, r) in &gpu.readings {
                                reading_diagnostic(ui, label, r);
                            }
                        });
                }
            });
        }
        ui.add_space(SECTION_GAP);
    }

    fn settings(&mut self, ui: &mut Ui) {
        page_header(ui, "Settings", None);
        // A form reads best at a bounded width, labels and controls together.
        ui.set_max_width(ui.available_width().min(760.));
        let width = label_column(
            ui,
            ["Language", "Theme", "Interface scale", "Reduce motion"].map(|l| tx(ui, l)),
            120.,
            200.,
        );
        section_header(ui, "Appearance", |_| {});
        card(ui, |ui| {
            setting_row(ui, "Language", width, |ui, label| {
                ComboBox::from_id_salt("language")
                    .selected_text(tx(ui, self.state.settings.language.native_name()))
                    .icon(combo_icon)
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
                    .labelled_by(label);
            });
            divider(ui);
            setting_row(ui, "Theme", width, |ui, _| {
                let current = self.state.settings.theme.clone();
                if let Some(theme) = segmented(
                    ui,
                    "theme",
                    current.as_str(),
                    &[("Dark", "Dark"), ("Light", "Light"), ("System", "System")],
                ) {
                    self.state.settings.theme = theme.into();
                    theme::apply(ui.ctx(), &self.state.settings);
                }
            });
            divider(ui);
            setting_row(ui, "Interface scale", width, |ui, _| {
                let scale = &mut self.state.settings.scale;
                let mut changed = false;
                let step = |ui: &mut Ui, text: &str, label: &str, enabled: bool| {
                    let response =
                        ui.add_enabled(enabled, Button::new(text).min_size(vec2(32., 32.)));
                    let label = tx(ui, label);
                    response
                        .widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, &label));
                    response.on_hover_text(label).clicked()
                };
                if step(ui, "−", "Smaller interface", *scale > 0.86) {
                    *scale = finite_scale(((*scale - 0.05) * 20.).round() / 20.);
                    changed = true;
                }
                ui.add_sized(
                    [56., 32.],
                    Label::new(RichText::new(format!("{:.0}%", *scale * 100.)).font(medium(14.))),
                );
                if step(ui, "+", "Larger interface", *scale < 1.49) {
                    *scale = finite_scale(((*scale + 0.05) * 20.).round() / 20.);
                    changed = true;
                }
                if (*scale - 1.).abs() > 0.001
                    && ui.add(quiet_button(ui, tx(ui, "Reset"))).clicked()
                {
                    *scale = 1.;
                    changed = true;
                }
                if changed {
                    theme::apply(ui.ctx(), &self.state.settings);
                }
            });
            divider(ui);
            ui.add_space(8.);
            let label = tx(ui, "Reduce motion");
            let column = if ui.available_width() < width + 240. {
                None
            } else {
                Some(width)
            };
            let reduce_motion = &mut self.state.settings.reduce_motion;
            let changed = match column {
                Some(column) => toggle_row(ui, reduce_motion, &label, column),
                None => toggle(ui, reduce_motion, &label),
            }
            .changed();
            if changed {
                theme::apply(ui.ctx(), &self.state.settings);
            }
            ui.add_space(8.);
        });
        ui.add_space(SECTION_GAP);
        let updates_top = ui.cursor().top();
        section_header(ui, "Updates", |_| {});
        let card_rect = card(ui, |ui| {
            self.updates_section(ui);
            ui.min_rect()
        });
        if std::mem::take(&mut self.reveal_updates) {
            let section =
                Rect::from_x_y_ranges(card_rect.x_range(), updates_top..=card_rect.bottom());
            ui.scroll_to_rect(section, Some(Align::TOP));
        }
        ui.add_space(SECTION_GAP);
        section_header(ui, "Keyboard", |_| {});
        card(ui, |ui| {
            let shortcuts = [
                (
                    if cfg!(target_os = "macos") {
                        "⌘ 1…4"
                    } else {
                        "Ctrl + 1…4"
                    },
                    "Switch primary view",
                ),
                ("Tab / Shift + Tab", "Move focus"),
                ("Enter / Space", "Activate focused control"),
                ("F5", "Rescan current path"),
                ("Escape", "Cancel current scan or comparison"),
                ("Alt + Up", "Go to parent scan folder"),
            ];
            for (index, (key, label)) in shortcuts.iter().enumerate() {
                if index > 0 {
                    ui.add_space(4.);
                }
                ui.horizontal(|ui| {
                    keycap(ui, key, width);
                    ui.add(Label::new(tx(ui, label)).wrap());
                });
            }
        });
        ui.add_space(SECTION_GAP);
        section_header(ui, "Storage accounting", |_| {});
        card(ui, |ui| {
            let p = palette(ui);
            for text in [
                "Logical bytes count every file path. Allocated bytes count each file identity once across hard links. Named data streams are included only in administrator scans of a whole drive. Directory and volume metadata overhead is excluded.",
                "Scans skip reparse points and cloud placeholders. Files may change during a scan; partial results and read errors remain visible.",
            ] {
                ui.add(Label::new(RichText::new(tx(ui, text)).color(p.muted)).wrap());
                ui.add_space(6.);
            }
        });
        ui.add_space(20.);
        caption(
            ui,
            i18n::language(ui).format(
                "Rigometry {0} · Application source: MIT · Rust / egui",
                &[env!("CARGO_PKG_VERSION").into()],
            ),
        );
        ui.add_space(SECTION_GAP);
    }

    /// The version, the automatic check, and a newer release with its notes and
    /// the one click that installs it.
    fn updates_section(&mut self, ui: &mut Ui) {
        let p = palette(ui);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(concat!("Rigometry ", env!("CARGO_PKG_VERSION"))).font(semibold(15.)),
            );
            let status = self.state.updates.as_ref().and_then(|updates| {
                if updates.checking() {
                    Some(tx(ui, "Checking for updates…"))
                } else if updates.offered().is_none()
                    && updates.installing.is_none()
                    && updates.ready.is_none()
                    && updates.record.checked_at.is_some()
                {
                    Some(tx(ui, "This is the newest version."))
                } else {
                    None
                }
            });
            if let Some(status) = status {
                caption(ui, "·");
                caption(ui, status);
            }
        });
        ui.add_space(10.);
        toggle(
            ui,
            &mut self.state.settings.check_updates,
            &tx(ui, "Check for new versions automatically"),
        );
        ui.add_space(4.);
        caption(
            ui,
            tx(
                ui,
                "Rigometry asks GitHub every six hours. Nothing about you is sent beyond the request itself. Updates install only when you click.",
            ),
        );
        let blocked = self.state.update_blocked();
        let Some(updates) = &self.state.updates else {
            return;
        };
        ui.add_space(12.);
        let mut action = None;
        if let Some(installing) = &updates.installing {
            let received = installing.received.load(Ordering::Relaxed);
            let fraction = if installing.total == 0 {
                0.
            } else {
                received as f32 / installing.total as f32
            };
            let text = if received >= installing.total {
                td(ui, format!("Installing version {}…", installing.version))
            } else {
                td(
                    ui,
                    format!(
                        "Downloading version {}: {} of {}",
                        installing.version,
                        ui_bytes(ui, received),
                        ui_bytes(ui, installing.total)
                    ),
                )
            };
            ui.label(text);
            ui.add_space(6.);
            meter(ui, fraction, ui.available_width().min(420.), 6., p.accent);
            ui.add_space(8.);
            if ui.button(tx(ui, "Cancel")).clicked() {
                action = Some(UpdateAction::Cancel);
            }
        } else if updates.ready.is_some() {
            ui.label(RichText::new(tx(ui, "The new version is installed")).font(semibold(15.)));
            ui.add_space(6.);
            if ui.add(primary_button(ui, tx(ui, "Restart now"))).clicked() {
                action = Some(UpdateAction::Restart);
            }
        } else if let Some(release) = updates.offered() {
            ui.horizontal(|ui| {
                dot(ui, p.accent);
                ui.label(
                    RichText::new(td(ui, format!("Version {} is available", release.version)))
                        .font(semibold(15.)),
                );
            });
            ui.add_space(4.);
            for note in &release.notes {
                ui.label(RichText::new(format!("•  {note}")).color(p.muted));
            }
            ui.add_space(8.);
            ui.horizontal_wrapped(|ui| {
                let install = ui
                    .add_enabled(blocked.is_none(), primary_button(ui, tx(ui, "Update and restart")))
                    .on_hover_text(tx(ui, "Downloads the new version from GitHub, checks it against GitHub's SHA-256 checksum, replaces this copy and restarts. Settings are kept."));
                if let Some(reason) = blocked {
                    install.on_disabled_hover_text(tx(ui, reason));
                } else if install.clicked() {
                    action = Some(UpdateAction::Install);
                }
                if ui.button(tx(ui, "Release page")).clicked() {
                    action = Some(UpdateAction::Page);
                }
                if updates.record.skipped.as_deref() != Some(&release.version)
                    && ui.add(quiet_button(ui, tx(ui, "Skip this version"))).clicked()
                {
                    action = Some(UpdateAction::Skip);
                }
            });
            if let Some(reason) =
                blocked.filter(|_| matches!(updates.target, crate::update::Target::Fixed(_)))
            {
                caption(ui, tx(ui, reason));
            }
        }
        if let Some(error) = &updates.error {
            ui.add_space(4.);
            ui.label(
                RichText::new(td(ui, error))
                    .font(regular(13.))
                    .color(p.warn),
            );
        }
        if updates.installing.is_none() && updates.ready.is_none() {
            ui.add_space(4.);
            if ui
                .add_enabled(!updates.checking(), Button::new(tx(ui, "Check now")))
                .clicked()
            {
                action = Some(UpdateAction::Check);
            }
        }
        match action {
            Some(UpdateAction::Install) => self.state.install_update(),
            Some(UpdateAction::Cancel) => self.state.cancel_update(),
            Some(UpdateAction::Skip) => self.state.skip_update(),
            Some(UpdateAction::Check) => self.state.check_updates(true),
            Some(UpdateAction::Restart) => {
                if let Some(updates) = &mut self.state.updates {
                    updates.error = None;
                    updates.restart = true;
                }
            }
            Some(UpdateAction::Page) => {
                let url = self
                    .state
                    .updates
                    .as_ref()
                    .and_then(|u| u.offered())
                    .map(|r| r.url.clone());
                if let Some(url) = url
                    && let Err(error) = crate::update::open_page(&url)
                {
                    self.state.notice = Some(error);
                }
            }
            None => {}
        }
    }

    /// Restarts into an installed update. While files are being replaced the
    /// window does not close: closing cancels a download, and an installation
    /// that is past it completes first.
    fn finish_update(&mut self, ctx: &Context) {
        let Some(updates) = &mut self.state.updates else {
            return;
        };
        if updates.restart
            && let Some(program) = &updates.ready
        {
            updates.restart = false;
            // Asked to close meanwhile: the new version is in place for next time.
            if self.close_after_update {
                ctx.send_viewport_cmd(ViewportCommand::Close);
                return;
            }
            match crate::update::launch(program, env!("CARGO_PKG_VERSION")) {
                Ok(()) => ctx.send_viewport_cmd(ViewportCommand::Close),
                // The new version is in place; the user can start it again or later.
                Err(error) => updates.error = Some(error),
            }
            return;
        }
        if updates.installing.is_some() {
            if ctx.input(|i| i.viewport().close_requested()) {
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);
                self.close_after_update = true;
                self.state.cancel_update();
            }
        } else if self.close_after_update {
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
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

    /// Messages and background work, shown under the page.
    fn status_bar(&mut self, ui: &mut Ui) {
        let p = palette(ui);
        let mut dismiss = false;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 8.;
            if let Some(notice) = &self.state.notice {
                dot(ui, p.accent);
                ui.label(RichText::new(td(ui, notice)).font(regular(13.)));
                let close = ui
                    .add(quiet_button(ui, "×").min_size(vec2(26., 24.)))
                    .on_hover_text(tx(ui, "Dismiss"));
                let label = tx(ui, "Dismiss");
                close.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, &label));
                dismiss = close.clicked();
            }
            if self.state.task.is_some() {
                ui.add(Spinner::new().size(12.).color(p.muted));
                caption(ui, tx(ui, "File dialog / export in progress"));
            } else if self.state.scan.is_some() && self.state.settings.page != Page::Storage {
                ui.add(Spinner::new().size(12.).color(p.accent));
                caption(ui, storage::scan_status(&self.state));
            }
        });
        if dismiss {
            self.state.notice = None;
        }
    }
}

#[derive(Clone, Copy)]
enum UpdateAction {
    Install,
    Cancel,
    Skip,
    Check,
    Restart,
    Page,
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
        // Escape first leaves a text field or closes a menu; only then does it
        // cancel work.
        let escape_cancels = !ctx.egui_wants_keyboard_input() && !egui::Popup::is_any_open(ctx);
        ctx.input(|i| {
            // Ctrl on Windows, Command on macOS.
            if i.modifiers.command {
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
            if escape_cancels && i.key_pressed(Key::Escape) {
                self.state.cancel_scan();
                self.state.cancel_duplicates();
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
        self.finish_update(ctx);
        let live_view = matches!(
            self.state.settings.page,
            Page::Overview | Page::Cpu | Page::Gpu
        );
        ctx.request_repaint_after(if live_view && !self.state.settings.reduce_motion {
            Duration::from_millis(33)
        } else if self.state.scan.is_some()
            || self.state.duplicates_running()
            || self
                .state
                .updates
                .as_ref()
                .is_some_and(|u| u.checking() || u.installing.is_some())
        {
            Duration::from_millis(80)
        } else {
            Duration::from_secs(1)
        });
    }

    fn ui(&mut self, ui: &mut Ui, _: &mut eframe::Frame) {
        i18n::set_context(ui.ctx(), self.state.settings.language);
        let p = palette(ui);
        egui::Panel::left("navigation")
            .resizable(false)
            .exact_size(navigation_width(ui))
            .frame(Frame::new().fill(p.window).inner_margin(Margin {
                left: 12,
                right: 10,
                top: 18,
                bottom: 14,
            }))
            .show_separator_line(false)
            .show_inside(ui, |ui| self.sidebar(ui));
        if self.state.notice.is_some()
            || self.state.task.is_some()
            || (self.state.scan.is_some() && self.state.settings.page != Page::Storage)
        {
            egui::Panel::bottom("status")
                .resizable(false)
                .show_separator_line(false)
                .frame(Frame::new().fill(p.window).inner_margin(Margin {
                    left: 6,
                    right: CANVAS_OUTSET,
                    top: 0,
                    bottom: 6,
                }))
                .show_inside(ui, |ui| self.status_bar(ui));
        }
        CentralPanel::default()
            .frame(Frame::new().fill(p.window).inner_margin(Margin {
                left: 0,
                right: CANVAS_OUTSET,
                top: CANVAS_OUTSET,
                bottom: CANVAS_OUTSET,
            }))
            .show_inside(ui, |ui| {
                Frame::new()
                    .fill(p.canvas)
                    .stroke(Stroke::new(1.0_f32, p.border))
                    .corner_radius(12)
                    .inner_margin(CANVAS_PADDING)
                    .show(ui, |ui| {
                        ui.set_min_size(ui.available_size());
                        let page = self.state.settings.page;
                        page_scroll(ui, ("page-scroll", page), |ui| match page {
                            Page::Overview => self.overview(ui),
                            Page::Cpu => self.cpu(ui),
                            Page::Gpu => self.gpu(ui),
                            Page::Storage => self.storage(ui),
                            Page::Diagnostics => self.diagnostics(ui),
                            Page::Settings => self.settings(ui),
                        });
                    });
            });
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if self.capture_dir.is_none() {
            eframe::set_value(storage, "settings", &self.state.settings);
        }
        if let Some(updates) = &self.state.updates {
            eframe::set_value(storage, "update", &updates.record);
        }
    }

    fn persist_egui_memory(&self) -> bool {
        self.capture_dir.is_none()
    }
}

/// The page's one vertical scroll area. Its bar keeps its gutter at all
/// times, so content never shifts; the handle is hidden while everything fits.
fn page_scroll(ui: &mut Ui, id: impl std::hash::Hash, add: impl FnOnce(&mut Ui)) {
    let id = Id::new(id);
    let fits_id = id.with("fits");
    let fits = ui.data(|d| d.get_temp::<bool>(fits_id)).unwrap_or(false);
    let content_style = ui.style().clone();
    ui.scope(|ui| {
        let canvas = palette(ui).canvas;
        // The page's scroll track blends into the canvas.
        ui.visuals_mut().extreme_bg_color = canvas;
        if fits {
            let widgets = &mut ui.visuals_mut().widgets;
            for w in [
                &mut widgets.inactive,
                &mut widgets.hovered,
                &mut widgets.active,
            ] {
                w.bg_fill = canvas;
            }
        }
        let output = ScrollArea::vertical()
            .id_salt(id)
            .scroll_bar_visibility(scroll_area::ScrollBarVisibility::AlwaysVisible)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_style(content_style);
                add(ui);
            });
        let fits_now = output.content_size.y <= output.inner_rect.height() + 0.5;
        if fits_now != fits {
            ui.data_mut(|d| d.insert_temp(fits_id, fits_now));
            ui.ctx().request_repaint();
        }
    });
}

fn finite_scale(scale: f32) -> f32 {
    if scale.is_finite() {
        scale.clamp(0.85, 1.5)
    } else {
        1.0
    }
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
    .map(|label| {
        ui.painter()
            .layout_no_wrap(tx(ui, label), medium(14.), Color32::WHITE)
            .size()
            .x
            + 70.
    })
    .fold(200., f32::max)
}

/// A labelled setting: the label column, then its control, or the label
/// above the control where both do not fit side by side.
fn setting_row(ui: &mut Ui, label: &str, width: f32, control: impl FnOnce(&mut Ui, Id)) {
    let text = RichText::new(tx(ui, label)).font(medium(14.));
    ui.add_space(8.);
    if ui.available_width() < width + 240. {
        let label = ui.label(text);
        ui.add_space(2.);
        ui.horizontal_wrapped(|ui| control(ui, label.id));
    } else {
        ui.horizontal(|ui| {
            ui.set_min_height(32.);
            let label = ui.allocate_ui_with_layout(
                vec2(width, 32.),
                Layout::left_to_right(Align::Center),
                |ui| {
                    ui.set_width(width);
                    ui.label(text)
                },
            );
            control(ui, label.inner.id);
        });
    }
    ui.add_space(8.);
}

/// A keyboard shortcut drawn as a key, in a column of `width`.
fn keycap(ui: &mut Ui, keys: &str, width: f32) {
    let p = palette(ui);
    ui.allocate_ui_with_layout(
        vec2(width, 28.),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.set_width(width);
            let galley = ui
                .painter()
                .layout_no_wrap(keys.to_owned(), medium(12.5), p.text);
            let (rect, response) =
                ui.allocate_exact_size(galley.size() + vec2(14., 8.), Sense::hover());
            ui.painter().rect(
                rect,
                6,
                p.control,
                Stroke::new(1.0_f32, p.control_border),
                StrokeKind::Inside,
            );
            ui.painter()
                .galley(rect.center() - galley.size() / 2., galley, p.text);
            response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, keys));
        },
    );
}

/// A centered message where a page has nothing to show.
fn empty_state(ui: &mut Ui, icon: Icon, message: &str) {
    let p = palette(ui);
    card_with(ui, Margin::symmetric(24, 40), |ui| {
        ui.vertical_centered(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(36., 36.), Sense::hover());
            paint_icon(ui.painter(), rect, icon, p.faint);
            ui.add_space(10.);
            ui.label(RichText::new(message).font(medium(15.)).color(p.muted));
        });
    });
}

/// One drive: identity, used space as a meter, and a scan button. Returns
/// whether the scan was requested.
fn drive_row(ui: &mut Ui, drive: &Drive) -> bool {
    let p = palette(ui);
    let used = drive.total_bytes.saturating_sub(drive.free_bytes);
    let fraction = if drive.total_bytes > 0 {
        used as f32 / drive.total_bytes as f32
    } else {
        0.
    };
    let color = if fraction >= 0.9 { p.warn } else { p.muted };
    let scan_label = tx(ui, "Scan");
    let scan_width = label_width(ui, &scan_label, 72.);
    let width = ui.available_width();
    let mut scan = false;
    let identity = |ui: &mut Ui, width: f32| {
        ui.allocate_ui_with_layout(vec2(width, 40.), Layout::top_down(Align::Min), |ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing.y = 1.;
            ui.add(
                Label::new(
                    RichText::new(format!("{}  {}", drive.mount, drive.name)).font(medium(14.)),
                )
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
            caption(
                ui,
                format!(
                    "{} · {}",
                    drive.file_system,
                    ui_bytes(ui, drive.total_bytes)
                ),
            );
        });
    };
    let usage = |ui: &mut Ui, width: f32| {
        ui.allocate_ui_with_layout(vec2(width, 40.), Layout::top_down(Align::Min), |ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing.y = 6.;
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(td(ui, format!("{} used", ui_bytes(ui, used))))
                        .font(regular(13.5)),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new(td(ui, format!("{} free", ui_bytes(ui, drive.free_bytes))))
                            .font(regular(13.5))
                            .color(p.muted),
                    );
                });
            });
            meter(ui, fraction, width, 6., color);
        });
    };
    ui.add_space(10.);
    if width < 520. {
        ui.horizontal(|ui| {
            identity(ui, (width - scan_width - 12.).max(60.));
            scan = ui
                .add_sized([scan_width, 30.], Button::new(&scan_label))
                .clicked();
        });
        ui.add_space(4.);
        usage(ui, width);
    } else {
        let name_width = (width * 0.3).clamp(150., 280.);
        let usage_width = (width - name_width - scan_width - 48.).max(120.);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.;
            identity(ui, name_width);
            ui.add_space(24.);
            usage(ui, usage_width);
            ui.add_space(24.);
            scan = ui
                .add_sized([scan_width, 30.], Button::new(&scan_label))
                .clicked();
        });
    }
    ui.add_space(10.);
    scan
}

fn loading(ui: &mut Ui, state: &State) {
    let p = palette(ui);
    ui.horizontal(|ui| {
        if state.hardware_error.is_none() {
            ui.add(Spinner::new().size(14.).color(p.muted));
        }
        ui.label(
            RichText::new(td(
                ui,
                state
                    .hardware_error
                    .as_deref()
                    .unwrap_or("Reading hardware…"),
            ))
            .color(p.muted),
        );
    });
}

/// Manufacturer and model, without repeating the manufacturer and without
/// joining two "Unavailable" placeholders.
fn motherboard_name(ui: &Ui, fields: &[Field]) -> String {
    let known = |name: &str| {
        fields
            .iter()
            .find(|f| f.label == name && f.state == Availability::Valid)
            .and_then(|f| f.value.clone())
            .filter(|v| !v.trim().is_empty())
    };
    match (known("Manufacturer"), known("Model")) {
        (Some(maker), Some(model)) if model.starts_with(&maker) => model,
        (Some(maker), Some(model)) => format!("{maker} {model}"),
        (Some(value), None) | (None, Some(value)) => value,
        (None, None) => tx(ui, "Unavailable"),
    }
}

fn field_display(ui: &Ui, field: &Field) -> String {
    let lang = i18n::language(ui);
    let Some(value) = &field.value else {
        return tx(ui, availability(&field.state));
    };
    let numeric_units = [
        "B", "KiB", "MiB", "GiB", "TiB", "MHz", "MT/s", "bits", "lanes",
    ];
    let decimals = |number: &str| number.split_once('.').map_or(0, |(_, f)| f.len());
    if numeric_units.contains(&field.unit.as_str())
        && let Ok(number) = value.parse::<f64>()
    {
        return format!(
            "{} {}",
            lang.decimal(number, decimals(value)),
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
            lang.decimal(number_value, decimals(number)),
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

/// Labels like "Module 2 capacity" belong to one memory module; "Module
/// manufacturer" without a number describes them all.
fn per_module(label: &str) -> bool {
    label
        .strip_prefix("Module ")
        .and_then(|rest| rest.split_once(' '))
        .is_some_and(|(number, _)| number.parse::<usize>().is_ok())
}

fn field_value(ui: &Ui, fields: &[Field], name: &str) -> String {
    fields
        .iter()
        .find(|f| f.label == name)
        .map(|f| field_display(ui, f))
        .unwrap_or_else(|| tx(ui, "Unavailable"))
}

/// A fact in the overview: clicking opens its page, or copies the value.
fn summary_row(
    ui: &mut Ui,
    label: &str,
    value: &str,
    width: f32,
    navigation: Option<(Page, &mut Page)>,
) {
    let response = fact_row(ui, &tx(ui, label), value, width);
    if let Some((page, target)) = navigation {
        if response.clicked() {
            *target = page;
        }
        response.on_hover_text(tx(ui, "Open hardware details"));
    } else {
        if response.clicked() {
            ui.ctx().copy_text(value.into());
        }
        response.on_hover_text(tx(ui, "Copy value"));
    }
    ui.add_space(4.);
}

fn field_table(ui: &mut Ui, id: &str, fields: &[Field]) {
    let p = palette(ui);
    ui.push_id(id, |ui| {
        let width = ui.available_width();
        let label_width = (width * 0.36).clamp(100., 190.);
        for f in fields {
            let value = if f.state == Availability::Unavailable && f.source.starts_with("SMBIOS ") {
                tx(ui, "Not reported by firmware")
            } else {
                field_display(ui, f)
            };
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 12.;
                ui.allocate_ui_with_layout(
                    vec2(label_width, 0.),
                    Layout::top_down(Align::Min),
                    |ui| {
                        ui.set_width(label_width);
                        ui.add(
                            Label::new(
                                RichText::new(td(ui, &f.label))
                                    .font(regular(13.5))
                                    .color(p.muted),
                            )
                            .wrap(),
                        );
                    },
                );
                let available = (width - label_width - 12.).max(60.);
                ui.allocate_ui_with_layout(vec2(available, 0.), Layout::top_down(Align::Min), |ui| {
                    ui.set_width(available);
                    let color = if f.value.is_some() { p.text } else { p.faint };
                    let response = ui.add(
                        Label::new(RichText::new(&value).font(regular(13.5)).color(color))
                            .wrap()
                            .sense(Sense::click()),
                    );
                    if response.clicked() {
                        ui.ctx().copy_text(value.clone());
                    }
                    response.on_hover_text(td(
                        ui,
                        format!(
                            "Source: {}\nState: {}\n{}\nSample: {} ms since Unix epoch\nClick to copy",
                            td(ui, &f.source),
                            tx(ui, state_label(&f.state)),
                            td(ui, &f.detail),
                            f.timestamp_ms
                        ),
                    ));
                });
            });
            ui.add_space(6.);
        }
    });
}

fn reading_row(ui: &mut Ui, label: &str, r: &Reading) {
    let p = palette(ui);
    let value = reading_text(ui, r);
    let width = ui.available_width();
    if width < 280. {
        muted(ui, td(ui, label), 13.5);
        numeric_value(ui, &value, width, 14.).on_hover_text(td(
            ui,
            format!(
                "{} · {}\n{}",
                tx(ui, state_label(&r.state)),
                td(ui, &r.source),
                td(ui, &r.detail)
            ),
        ));
        return;
    }
    ui.horizontal(|ui| {
        let value_width = 180f32.min(width * 0.45);
        let label_width = width - value_width - ui.spacing().item_spacing.x;
        muted_line(ui, &td(ui, label), label_width, p.muted);
        numeric_value(ui, &value, value_width, 14.).on_hover_text(td(
            ui,
            format!(
                "{}\n{} · {}\n{} ms since Unix epoch\nLast value: {}",
                td(ui, &r.source),
                tx(ui, state_label(&r.state)),
                td(ui, &r.detail),
                r.timestamp_ms,
                r.value
                    .map(|v| number(ui, v, &r.unit))
                    .unwrap_or("—".into())
            ),
        ));
    });
}

/// One line of text cut to `width`, the whole text on hover when cut.
fn muted_line(ui: &mut Ui, text: &str, width: f32, color: Color32) -> Response {
    let mut job = text::LayoutJob::simple_singleline(text.to_owned(), regular(13.5), color);
    job.wrap = text::TextWrapping::truncate_at_width(width);
    let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
    let (rect, response) = ui.allocate_exact_size(vec2(width, 28.), Sense::hover());
    let cut = galley.elided;
    ui.painter().galley(
        pos2(rect.left(), rect.center().y - galley.size().y / 2.),
        galley,
        color,
    );
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, text));
    if cut {
        response.on_hover_text(text)
    } else {
        response
    }
}

fn sensor_table(
    ui: &mut Ui,
    id: &str,
    readings: &[&(String, Reading)],
    history: &std::collections::VecDeque<Telemetry>,
    adapter: &str,
) {
    let p = palette(ui);
    let width = ui.available_width();
    if width < 520. {
        ui.push_id(id, |ui| {
            for (index, (name, reading)) in readings.iter().enumerate() {
                if index > 0 {
                    divider(ui);
                    ui.add_space(4.);
                }
                reading_row(ui, name, reading);
                let (min, max) = sensor_extrema(history, adapter, name);
                ui.horizontal_wrapped(|ui| {
                    for (label, value) in [("Min", min), ("Max", max)] {
                        caption(
                            ui,
                            td(
                                ui,
                                format!(
                                    "{} {}",
                                    tx(ui, label),
                                    value
                                        .map(|v| number(ui, v, &reading.unit))
                                        .unwrap_or("—".into())
                                ),
                            ),
                        );
                    }
                });
                ui.add_space(6.);
            }
        });
        return;
    }
    let value_width = (width * 0.17).clamp(96., 140.);
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
            .layout(tx(ui, label), regular(13.5), p.text, value_width - 4.)
            .size()
            .y
            + 10.
    })
    .fold(32.0f32, f32::max);
    ui.push_id(id, |ui| {
        ui.visuals_mut().widgets.hovered.bg_fill = p.hover;
        ui.spacing_mut().item_spacing = vec2(0., 0.);
        // Row dividers span the table, painted from the first cell within the
        // page's clip, since each cell clips its own painting.
        let lines = Painter::new(ui.ctx().clone(), ui.layer_id(), ui.clip_rect());
        let table_x = ui.max_rect().x_range();
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
                        let text = RichText::new(tx(ui, label))
                            .font(medium(12.5))
                            .color(p.faint);
                        if index == 0 {
                            ui.label(text);
                        } else {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.add_space(4.);
                                ui.label(text);
                            });
                        }
                    });
                }
            })
            .body(|mut body| {
                for (index, (name, r)) in readings.iter().enumerate() {
                    body.row(row_height, |mut row| {
                        row.col(|ui| {
                            if index > 0 {
                                lines.hline(
                                    table_x,
                                    ui.max_rect().top(),
                                    Stroke::new(1.0_f32, p.divider),
                                );
                            }
                            ui.add(Label::new(td(ui, name)).truncate())
                                .on_hover_text(td(
                                    ui,
                                    format!("{}\n{}", td(ui, &r.source), td(ui, &r.detail)),
                                ));
                        });
                        row.col(|ui| {
                            numeric_value(ui, &reading_text(ui, r), value_width - 4., 13.5)
                                .on_hover_text(td(
                                    ui,
                                    format!(
                                        "{} · {}\n{}\nLast value: {}",
                                        tx(ui, state_label(&r.state)),
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
                                numeric_value_in(
                                    ui,
                                    &value.map(|v| number(ui, v, &r.unit)).unwrap_or("—".into()),
                                    value_width - 4.,
                                    13.5,
                                    p.muted,
                                );
                            });
                        }
                    });
                }
            });
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

fn diagnostic_field(ui: &mut Ui, f: &Field) {
    ui.label(
        RichText::new(td(
            ui,
            format!("{}: {}", td(ui, &f.label), field_display(ui, f)),
        ))
        .font(medium(14.)),
    );
    caption(
        ui,
        td(
            ui,
            format!(
                "{} · {} · {} ms",
                tx(ui, state_label(&f.state)),
                td(ui, &f.source),
                f.timestamp_ms
            ),
        ),
    );
    if !f.detail.is_empty() {
        ui.add(Label::new(RichText::new(td(ui, &f.detail)).color(palette(ui).muted)).wrap());
    }
    ui.add_space(10.);
}

fn reading_diagnostic(ui: &mut Ui, label: &str, r: &Reading) {
    reading_row(ui, label, r);
    caption(
        ui,
        td(
            ui,
            format!(
                "{} · {} · {} ms",
                tx(ui, state_label(&r.state)),
                td(ui, &r.source),
                r.timestamp_ms
            ),
        ),
    );
    ui.add_space(10.);
}

#[cfg(test)]
mod tests;
