//! The Storage page: scan controls, totals, the result table, the size map
//! and duplicate files.

use super::{
    App,
    platform::{PATH_HINT, REVEAL, elevated, relaunch_elevated, reveal},
    theme::{self, medium, palette, regular, semibold},
    widgets::*,
};
use crate::{
    i18n::{self, message as td, text as tx},
    search::Content,
    state::{DuplicateRow, Sort, State, View},
    storage::{ScanNode, node_path},
};
use eframe::egui::*;
use egui_extras::{Column, TableBuilder};

const ROW_HEIGHT: f32 = 32.;
/// Where an entry's name starts in its row: after the disclosure chevron and
/// the type glyph.
const NAME_INSET: f32 = 54.;

impl App {
    pub(super) fn storage(&mut self, ui: &mut Ui) {
        let compact = ui.available_width() < 520.;
        page_header(ui, "Storage", None);
        // Keep all scan actions together. Reserve their width even when idle,
        // so starting a scan never moves the path field or wraps Cancel alone.
        let actions_width = scan_actions_width(ui);
        let path_label_width = label_width(ui, &tx(ui, "Path"), 36.);
        let stacked = ui.available_width() < actions_width + path_label_width + 240.;
        let path_width = if stacked {
            (ui.available_width() - path_label_width - 10.).max(100.)
        } else {
            (ui.available_width() - actions_width - path_label_width - 30.).clamp(160., 380.)
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
        self.drive_chips(ui);
        self.storage_results(ui, compact);
    }

    fn scan_path(&mut self, ui: &mut Ui, path_width: f32) {
        let path_label = muted(ui, tx(ui, "Path"), 14.);
        let response = ui
            .add_sized(
                [path_width, 32.],
                TextEdit::singleline(&mut self.state.settings.path)
                    .id(Id::new("scan-path"))
                    .hint_text(tx(ui, PATH_HINT))
                    .margin(vec2(10., 7.)),
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
                32.,
            ],
            Button::new(if compact { "…" } else { &browse_label }),
        );
        browse.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, &browse_label));
        if browse.on_hover_text(&browse_label).clicked() {
            self.state.pick_folder();
        }
        let label = tx(
            ui,
            if compact || self.state.nodes.is_empty() {
                "Scan"
            } else {
                "Rescan"
            },
        );
        let width = if compact {
            label_width(ui, &tx(ui, "Scan"), 76.)
        } else {
            scan_primary_width(ui)
        };
        if ui
            .add_sized([width, 32.], primary_button(ui, label).wrap())
            .on_hover_text(tx(ui, "Start a new scan of this path (F5)"))
            .clicked()
        {
            self.state.start_scan();
        }
        if self.state.scan.is_some()
            && ui
                .add_sized(
                    [label_width(ui, &tx(ui, "Cancel"), 76.), 32.],
                    Button::new(tx(ui, "Cancel")),
                )
                .clicked()
        {
            self.state.cancel_scan();
        }
    }

    /// One chip per drive: scanning a whole drive is one click.
    fn drive_chips(&mut self, ui: &mut Ui) {
        let Some(inventory) = &self.state.inventory else {
            return;
        };
        let p = palette(ui);
        let mut chosen = None;
        ui.add_space(6.);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6., 6.);
            for drive in &inventory.drives {
                let text = td(
                    ui,
                    format!("{}  {} free", drive.mount, ui_bytes(ui, drive.free_bytes)),
                );
                let chip = Button::new(RichText::new(text).font(regular(12.5)).color(p.muted))
                    .corner_radius(14)
                    .min_size(vec2(0., 26.));
                if ui.add(chip).clicked() {
                    chosen = Some(drive.mount.clone());
                }
            }
        });
        if let Some(path) = chosen {
            self.state.settings.path = path;
            self.state.start_scan();
        }
    }

    fn storage_results(&mut self, ui: &mut Ui, compact: bool) {
        ui.add_space(18.);
        if self.state.nodes.is_empty() {
            self.empty_storage(ui);
            return;
        }
        self.scan_summary(ui, compact);
        ui.add_space(18.);
        self.toolbar(ui);
        if let Some(error) = &self.state.search_error {
            ui.add_space(6.);
            ui.label(
                RichText::new(td(ui, error))
                    .font(regular(12.5))
                    .color(ui.visuals().warn_fg_color),
            );
        } else if !self.state.filter.trim().is_empty()
            && self.state.visible.is_empty()
            && self.state.visible_filter == self.state.filter
        {
            ui.add_space(6.);
            caption(ui, tx(ui, "No matches below this folder"));
        }
        ui.add_space(12.);
        self.breadcrumbs(ui);
        ui.add_space(4.);
        ui.add_space(12.);
        let viewport_height = ui.ctx().content_rect().height();
        // Measure content consumed above the results, independent of scrolling.
        // Leave room for the selected path, actions and scan notes below them.
        let used_height = ui.cursor().top() - ui.max_rect().top();
        let height = (viewport_height - used_height - 200.).clamp(200., 520.);
        let split = ui.available_width() >= 840.;
        let clicked = if self.state.view == View::Duplicates {
            self.storage_duplicates(ui);
            None
        } else if self.state.view == View::Map {
            card_with(ui, Margin::same(14), |ui| self.storage_map(ui, height))
        } else if split {
            let width = ui.available_width();
            // Names truncate with a tooltip; the size map gets the rest of the row.
            let table_width = (width * 0.25).clamp(220., 420.) + 360.;
            let map_width = width - table_width - CARD_GAP;
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = CARD_GAP;
                let table = ui.allocate_ui_with_layout(
                    vec2(table_width, 0.),
                    Layout::top_down(Align::Min),
                    |ui| self.table_card(ui),
                );
                // Beside a short list, the map ends with the list instead of
                // pushing the actions below into empty space.
                let map_height = (table.response.rect.height() - 28.).clamp(130., height);
                let map = |ui: &mut Ui| {
                    card_with(ui, Margin::same(14), |ui| self.storage_map(ui, map_height))
                };
                let map_clicked = ui
                    .allocate_ui_with_layout(vec2(map_width, 0.), Layout::top_down(Align::Min), map)
                    .inner;
                table.inner.or(map_clicked)
            })
            .inner
        } else {
            self.table_card(ui)
        };
        if let Some(id) = clicked {
            self.state.navigate_scope(id);
        }
        ui.add_space(12.);
        self.selection_bar(ui);
        if let Some(summary) = &self.state.summary {
            ui.add_space(10.);
            folding(ui, tx(ui, "Scan accounting & skipped entries"))
                .id_salt("scan-notes")
                .show(ui, |ui| {
                    ui.label(td(
                        ui,
                        format!(
                            "{} errors · {} reparse points · {} cloud placeholders · {} hard-link aliases",
                            summary.errors,
                            summary.skipped_reparse,
                            summary.skipped_cloud,
                            summary.hard_links
                        ),
                    ));
                    if summary.denied + summary.locked > 0 {
                        ui.label(td(
                            ui,
                            format!(
                                "{} access denied · {} in use by other programs",
                                summary.denied, summary.locked
                            ),
                        ));
                    }
                    for note in &summary.notes {
                        ui.label(td(ui, note));
                    }
                });
        }
    }

    fn empty_storage(&self, ui: &mut Ui) {
        let p = palette(ui);
        card_with(ui, Margin::symmetric(24, 36), |ui| {
            ui.vertical_centered(|ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(36., 36.), Sense::hover());
                paint_icon(ui.painter(), rect, Icon::Storage, p.faint);
                ui.add_space(10.);
                let scanning = self.state.scan.is_some();
                ui.label(
                    RichText::new(tx(
                        ui,
                        if scanning {
                            "Reading directory metadata…"
                        } else {
                            "No scan results"
                        },
                    ))
                    .font(semibold(17.)),
                );
                ui.add_space(4.);
                if scanning {
                    caption(ui, scan_status(&self.state));
                } else {
                    caption(
                        ui,
                        tx(
                            ui,
                            "Choose a drive or folder to see what takes up space. Nothing is opened or changed.",
                        ),
                    );
                }
                if let Some(summary) = &self.state.summary {
                    for note in &summary.notes {
                        ui.label(RichText::new(td(ui, note)).color(p.warn));
                    }
                }
            });
        });
    }

    /// Totals of the current folder and how the scan went.
    fn scan_summary(&mut self, ui: &mut Ui, compact: bool) {
        let p = palette(ui);
        let scope = &self.state.nodes[self.state.scope];
        let (logical, allocated, files) = (scope.logical, scope.allocated, scope.files);
        let partial = scope.incomplete || self.state.scan.is_some();
        let lower_bound = if partial { "≥ " } else { "" };
        let lang = i18n::language(ui);
        let mut totals = vec![
            (
                "On disk",
                format!("{lower_bound}{}", ui_bytes(ui, allocated)),
            ),
            ("Files", format!("{lower_bound}{}", lang.integer(files))),
        ];
        // File sizes only add information where compression, sparse files or
        // cluster rounding make them differ noticeably from bytes on disk.
        if logical.abs_diff(allocated) > allocated.max(logical) / 100 {
            totals.push((
                "File size",
                format!("{lower_bound}{}", ui_bytes(ui, logical)),
            ));
        }
        let running = self.state.scan.is_some();
        let status = scan_status(&self.state);
        let denied = self.state.summary.as_ref().is_some_and(|s| s.denied > 0);
        let offer_admin = denied && !running && !elevated();
        let admin_label = tx(ui, "Scan as administrator");
        // Measured, so the status can share the totals' row when it fits.
        let status_width = 16.
            + ui.painter()
                .layout_no_wrap(status.clone(), regular(13.), p.muted)
                .size()
                .x
            + if offer_admin {
                8. + label_width(ui, &admin_label, 0.)
            } else {
                0.
            };
        let column = 180.;
        let mut elevate = false;
        let status_line = |ui: &mut Ui, elevate: &mut bool| {
            ui.spacing_mut().item_spacing.x = 8.;
            if running {
                if self.state.settings.reduce_motion {
                    dot(ui, p.accent);
                } else {
                    ui.add(Spinner::new().size(13.).color(p.accent));
                }
            } else {
                dot(ui, scan_status_color(&self.state, p));
            }
            ui.label(RichText::new(&status).font(regular(13.)).color(p.muted));
            if offer_admin
                && ui
                    .button(&admin_label)
                    .on_hover_text(tx(ui, "Windows protects some system folders from standard accounts. Rigometry restarts with administrator rights and scans this path again, still reading metadata only."))
                    .clicked()
            {
                *elevate = true;
            }
        };
        card_with(ui, Margin::symmetric(20, 16), |ui| {
            let width = ui.available_width();
            let stats_width = column * totals.len() as f32;
            let inline = !compact && stats_width + status_width + 24. <= width;
            if compact {
                for (label, value) in &totals {
                    fact_row(ui, &tx(ui, label), value, 110.);
                }
            } else {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.;
                    for (label, value) in &totals {
                        ui.allocate_ui_with_layout(
                            vec2(column, 0.),
                            Layout::top_down(Align::Min),
                            |ui| {
                                ui.set_min_width(column);
                                ui.spacing_mut().item_spacing.y = 2.;
                                muted(ui, tx(ui, label), 13.);
                                ui.add(
                                    Label::new(RichText::new(value).font(theme::display(24.)))
                                        .wrap(),
                                );
                            },
                        );
                    }
                    if inline {
                        ui.add_space(ui.available_width() - status_width);
                        ui.allocate_ui_with_layout(
                            vec2(status_width, 50.),
                            Layout::left_to_right(Align::Center),
                            |ui| status_line(ui, &mut elevate),
                        );
                    }
                });
            }
            if !inline {
                ui.add_space(10.);
                ui.horizontal_wrapped(|ui| status_line(ui, &mut elevate));
            }
        });
        if elevate {
            let root = node_path(&self.state.nodes, 0);
            match relaunch_elevated(&root) {
                Ok(()) => ui.ctx().send_viewport_cmd(ViewportCommand::Close),
                Err(error) => self.state.notice = Some(error),
            }
        }
    }

    /// The path from the scan root to the current folder; each part opens it.
    fn breadcrumbs(&mut self, ui: &mut Ui) {
        let p = palette(ui);
        let mut crumbs = vec![self.state.scope];
        let mut parent = self.state.nodes[self.state.scope].parent;
        while let Some(id) = parent {
            crumbs.push(id);
            parent = self.state.nodes[id].parent;
        }
        crumbs.reverse();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.;
            let (rect, _) = ui.allocate_exact_size(vec2(18., 26.), Sense::hover());
            paint_glyph(ui, rect, true);
            ui.add_space(2.);
            for (index, id) in crumbs.iter().enumerate() {
                if index > 0 {
                    let (rect, _) = ui.allocate_exact_size(vec2(12., 26.), Sense::hover());
                    chevron(
                        ui.painter(),
                        rect.center(),
                        8.,
                        0.,
                        Stroke::new(1.4_f32, p.faint),
                    );
                }
                // The scan root's name is its full path; show only its last
                // part, as for every other folder, and the path on hover.
                let name = &self.state.nodes[*id].name;
                let short = std::path::Path::new(name)
                    .file_name()
                    .map_or(name.clone(), |n| n.to_string_lossy().into_owned());
                let current = index + 1 == crumbs.len();
                let response = crumb(ui, &short, current);
                let response = if short == *name {
                    response
                } else {
                    response.on_hover_text(name)
                };
                if response.clicked() {
                    self.state.navigate_scope(*id);
                }
            }
        });
    }

    /// View switcher, search and export: one row where they fit.
    fn toolbar(&mut self, ui: &mut Ui) {
        let exports = self.state.view != View::Duplicates;
        let field = 260.;
        let tabs = segmented_width(ui, &["Hierarchy", "Largest files", "Map", "Duplicates"]);
        // The search block's exact width, so it can end at the page edge.
        let text = |ui: &Ui, text: &str, font| {
            ui.painter()
                .layout_no_wrap(text.to_owned(), font, Color32::WHITE)
                .size()
                .x
        };
        let gap = ui.spacing().item_spacing.x;
        let padding = 2. * ui.spacing().button_padding.x;
        let block = text(ui, &tx(ui, "Search"), regular(14.))
            + gap
            + field
            + if exports {
                ["CSV", "JSON"]
                    .iter()
                    .map(|format| gap + text(ui, format, medium(14.)) + padding)
                    .sum::<f32>()
            } else {
                0.
            };
        let views = |ui: &mut Ui, state: &mut State| {
            // Stable control IDs while asynchronous drive discovery changes the
            // number of widgets above this toolbar.
            ui.push_id("storage-view-tabs", |ui| {
                if let Some(view) = segmented(
                    ui,
                    "storage-view",
                    state.view,
                    &[
                        (View::Hierarchy, "Hierarchy"),
                        (View::Largest, "Largest files"),
                        (View::Map, "Map"),
                        (View::Duplicates, "Duplicates"),
                    ],
                ) {
                    state.view = view;
                    if view == View::Largest {
                        state.sort = Sort::Allocated;
                        state.descending = true;
                    }
                    state.refresh_now();
                }
            });
        };
        if ui.available_width() >= tabs + 24. + block {
            ui.horizontal(|ui| {
                views(ui, &mut self.state);
                ui.add_space((ui.available_width() - block - 1.).max(16.));
                ui.push_id("storage-filter-export", |ui| {
                    ui.horizontal(|ui| self.search_row(ui, field))
                });
            });
        } else {
            views(ui, &mut self.state);
            ui.add_space(10.);
            let field = (ui.available_width() - 150.).clamp(120., 420.);
            ui.push_id("storage-filter-export", |ui| {
                ui.horizontal_wrapped(|ui| self.search_row(ui, field))
            });
        }
    }

    fn search_row(&mut self, ui: &mut Ui, field: f32) {
        let filter_label = muted(ui, tx(ui, "Search"), 14.);
        let search = ui
            .add_sized(
                [field, 32.],
                TextEdit::singleline(&mut self.state.filter)
                    .id(Id::new("scan-filter"))
                    .margin(vec2(10., 7.))
                    .hint_text(tx(ui, "Name, folder or ext:pdf size:>1g")),
            )
            .labelled_by(filter_label.id);
        let search = search.on_hover_text(tx(ui, "Words match names and the folders above them. Words of five or more letters forgive one typo.\n'exact   ^prefix   suffix$   !exclude   \"two words\"\next:mp4,mkv   kind:file   kind:dir   path:text\nsize:>500m   size:1g..4g   (k m g t = KiB MiB GiB TiB; kb mb gb tb are decimal)\ntype: image, video, audio, doc, code, archive, disk, app, font"));
        if search.changed() {
            self.state.filter_changed();
        }
        // The duplicates view offers its own report instead.
        if self.state.view == View::Duplicates {
            return;
        }
        let can_export = self.state.scan.is_none() && self.state.task.is_none();
        ui.add_enabled_ui(can_export, |ui| {
            for format in ["CSV", "JSON"] {
                if ui
                    .add(Button::new(format).min_size(vec2(0., 32.)))
                    .on_hover_text(tx(
                        ui,
                        "Export whole scan, including entries outside the current folder or filter",
                    ))
                    .clicked()
                {
                    self.state.export(&format.to_lowercase());
                }
            }
        });
    }

    /// The selected entry: its path, sizes and the actions on it.
    fn selection_bar(&mut self, ui: &mut Ui) {
        let p = palette(ui);
        let total = self.state.nodes[self.state.scope].allocated;
        let node = &self.state.nodes[self.state.selected];
        let path = node_path(&self.state.nodes, self.state.selected);
        let path_text = path.to_string_lossy();
        let is_dir = node.is_dir;
        let lang = i18n::language(ui);
        let facts = [
            format!("On disk: {}", node_size(ui, node, true)),
            format!("File size: {}", node_size(ui, node, false)),
            format!("Scope: {}%", lang.decimal(share(node.allocated, total), 1)),
        ]
        .map(|text| td(ui, text))
        .join("  ·  ");
        let facts_width = ui
            .painter()
            .layout_no_wrap(facts.clone(), regular(12.5), p.faint)
            .size()
            .x;
        // The path keeps one line beside its facts; hover shows all of it.
        let width = ui.available_width();
        if width >= facts_width + 220. {
            ui.horizontal(|ui| {
                let path_width = width - facts_width - 16.;
                ui.allocate_ui_with_layout(
                    vec2(path_width, 22.),
                    Layout::left_to_right(Align::Center),
                    |ui| {
                        ui.set_width(path_width);
                        copy_label(ui, &path_text, 13.5, p.text);
                    },
                );
                caption(ui, facts);
            });
        } else {
            copy_label(ui, &path_text, 13.5, p.text);
            caption(ui, facts);
        }
        ui.add_space(6.);
        ui.horizontal_wrapped(|ui| {
            if ui.button(tx(ui, "Copy path")).clicked() {
                ui.ctx().copy_text(path_text.as_ref().into());
            }
            if ui.button(tx(ui, REVEAL)).clicked()
                && let Err(e) = reveal(&path)
            {
                self.state.notice = Some(e);
            }
            if is_dir && ui.button(tx(ui, "Open folder")).clicked() {
                self.state.navigate_scope(self.state.selected);
            }
            if self.state.view != View::Duplicates {
                ui.add_space(6.);
                caption(
                    ui,
                    td(
                        ui,
                        format!("{} entries", lang.integer(self.state.visible.len() as u64)),
                    ),
                );
            }
        });
    }

    fn table_card(&mut self, ui: &mut Ui) -> Option<usize> {
        card_with(ui, Margin::symmetric(8, 6), |ui| self.storage_table(ui))
    }

    fn storage_table(&mut self, ui: &mut Ui) -> Option<usize> {
        let p = palette(ui);
        // Row hover is a light wash, not the scrollbar's hover color.
        ui.visuals_mut().widgets.hovered.bg_fill = p.hover;
        let compact = ui.available_width() < 600.;
        // Counting files only says something about folders; the largest-files
        // list has none, so it drops the column.
        let counts = self.state.view != View::Largest;
        let total = self.state.nodes[self.state.scope].allocated;
        let mut clicked = None;
        let mut toggle = None;
        let mut new_sort = None;
        let mut action = None;
        let mut table = TableBuilder::new(ui)
            .id_salt(("files", self.state.scan_generation, compact, counts))
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
            .column(Column::exact(if compact { 86. } else { 100. }).clip(true));
        if counts {
            table = table.column(Column::exact(if compact { 52. } else { 76. }).clip(true));
        }
        if !compact {
            table = table.column(Column::exact(96.).clip(true));
        }
        table = table.column(Column::exact(34.));
        table
            .header(40., |mut header| {
                for (label, sort, right) in [
                    ("Name", Some(Sort::Name), false),
                    ("On disk", Some(Sort::Allocated), true),
                    ("Files", Some(Sort::Files), true),
                    ("Scope %", None, true),
                ] {
                    if (compact && sort.is_none()) || (!counts && sort == Some(Sort::Files)) {
                        continue;
                    }
                    header.col(|ui| {
                        let marker = if sort == Some(self.state.sort) {
                            if self.state.descending {
                                " ↓"
                            } else {
                                " ↑"
                            }
                        } else {
                            ""
                        };
                        let active = !marker.is_empty();
                        let text = RichText::new(format!("{}{marker}", tx(ui, label)))
                            .font(medium(12.5))
                            .color(if active { p.text } else { p.faint });
                        let layout = if right {
                            Layout::right_to_left(Align::Center)
                        } else {
                            Layout::left_to_right(Align::Center)
                        };
                        ui.with_layout(layout, |ui| {
                            // Headers line up with the values below them: names
                            // start after the disclosure and type glyph, numbers
                            // end at the cell's padding.
                            ui.spacing_mut().button_padding.x = 0.;
                            ui.add_space(if right { 4. } else { NAME_INSET });
                            if sort.is_none() {
                                ui.label(text);
                            } else if ui.add(Button::new(text).wrap().frame(false)).clicked() {
                                new_sort = sort;
                            }
                        });
                    });
                }
                header.col(|_| {});
            })
            .body(|body| {
                body.rows(ROW_HEIGHT, self.state.visible.len(), |mut row| {
                    let (id, depth) = self.state.visible[row.index()];
                    let nodes = &self.state.nodes;
                    let node = &nodes[id];
                    row.set_selected(self.state.selected == id);
                    // Cell widgets are keyed by entry, not row position: rows
                    // re-sort while a scan runs, and focus must stay on its entry.
                    row.col(|ui| {
                        ui.push_id(id, |ui| {
                            ui.spacing_mut().item_spacing.x = 4.;
                            let indent = (depth.min(20) as f32 * 16.)
                                .min((ui.available_width() - 90.).max(0.));
                            ui.add_space(indent);
                            if node.is_dir {
                                if folder_toggle(ui, &node.name, self.state.expanded.contains(&id))
                                    .clicked()
                                {
                                    toggle = Some(id);
                                }
                            } else {
                                ui.add_space(28.);
                            }
                            // A narrow name column keeps its room for the name.
                            if ui.available_width() > 96. {
                                entry_glyph(ui, node);
                            }
                            let response =
                                ui.add(Label::new(&node.name).truncate().sense(Sense::click()));
                            if response.clicked() || response.secondary_clicked() {
                                self.state.selected = id;
                            }
                            if response.double_clicked() && node.is_dir {
                                clicked = Some(id);
                            }
                            entry_menu(&response, nodes, id, &mut action);
                            response.on_hover_ui(|ui| {
                                ui.label(td(
                                    ui,
                                    format!(
                                        "{}\nOn disk: {}\nFile size: {}\nScope: {}%\n{}",
                                        node_path(nodes, id).display(),
                                        node_size(ui, node, true),
                                        node_size(ui, node, false),
                                        i18n::language(ui).decimal(share(node.allocated, total), 1),
                                        td(ui, &node.note)
                                    ),
                                ));
                            });
                        });
                    });
                    row.col(|ui| {
                        let value = node_size(ui, node, true);
                        numeric_value(ui, &value, ui.available_width() - 4., 13.5)
                            .on_hover_text(&value);
                    });
                    if counts {
                        row.col(|ui| {
                            if !node.is_dir {
                                return;
                            }
                            let lang = i18n::language(ui);
                            let count = if node.incomplete || self.state.scan.is_some() {
                                format!("≥ {}", lang.integer(node.files))
                            } else {
                                lang.integer(node.files)
                            };
                            numeric_value(ui, &count, ui.available_width() - 4., 13.5)
                                .on_hover_text(&count);
                        });
                    }
                    if !compact {
                        row.col(|ui| {
                            let fraction = share(node.allocated, total);
                            let value = i18n::language(ui).decimal(fraction, 1);
                            ui.add_space(10.);
                            let (rect, _) = ui.allocate_exact_size(vec2(30., 4.), Sense::hover());
                            paint_meter(ui, rect, (fraction / 100.) as f32, p.muted);
                            numeric_value(ui, &value, ui.available_width() - 4., 13.);
                        });
                    }
                    row.col(|ui| {
                        ui.push_id(id, |ui| {
                            if reveal_button(ui, &node.name).clicked() {
                                action = Some(EntryAction::Reveal(id));
                            }
                        });
                    });
                    let response = row.response();
                    if response.clicked() || response.secondary_clicked() {
                        self.state.selected = id;
                    }
                    entry_menu(&response, nodes, id, &mut action);
                });
            });
        if let Some(id) = toggle {
            if !self.state.expanded.remove(&id) {
                self.state.expanded.insert(id);
            }
            self.state.refresh_now();
        }
        if let Some(sort) = new_sort {
            if self.state.sort == sort {
                self.state.descending = !self.state.descending;
            } else {
                self.state.sort = sort;
                self.state.descending = sort != Sort::Name;
            }
            self.state.refresh_now();
        }
        self.apply_entry_action(ui, action);
        clicked
    }

    fn apply_entry_action(&mut self, ui: &Ui, action: Option<EntryAction>) {
        match action {
            Some(EntryAction::Reveal(id)) => {
                self.state.selected = id;
                if let Err(error) = reveal(&node_path(&self.state.nodes, id)) {
                    self.state.notice = Some(error);
                }
            }
            Some(EntryAction::CopyPath(id)) => {
                ui.ctx()
                    .copy_text(node_path(&self.state.nodes, id).to_string_lossy().into());
            }
            Some(EntryAction::Open(id)) => self.state.navigate_scope(id),
            None => {}
        }
    }

    /// Files with identical contents below the current folder, found on request.
    fn storage_duplicates(&mut self, ui: &mut Ui) {
        let p = palette(ui);
        let running = self.state.duplicates_running();
        ui.horizontal_wrapped(|ui| {
            let label = muted(ui, tx(ui, "Smallest file"), 14.);
            let current = self.state.settings.duplicate_min_size;
            ComboBox::from_id_salt("duplicate-min-size")
                .selected_text(minimum_label(ui, current))
                .icon(combo_icon)
                .show_ui(ui, |ui| {
                    for size in [1, 4 << 10, 64 << 10, 1 << 20, 16 << 20, 256 << 20] {
                        ui.selectable_value(
                            &mut self.state.settings.duplicate_min_size,
                            size,
                            minimum_label(ui, size),
                        );
                    }
                })
                .response
                .labelled_by(label.id);
            if running {
                if ui.button(tx(ui, "Cancel")).clicked() {
                    self.state.cancel_duplicates();
                }
            } else if ui
                .add_enabled(
                    self.state.scan.is_none(),
                    primary_button(ui, tx(ui, "Find duplicates")),
                )
                .on_hover_text(tx(ui, "Reads the files below this folder that share their size with another file and compares their contents. Nothing is changed or deleted."))
                .on_disabled_hover_text(tx(ui, "Available when the scan has finished"))
                .clicked()
            {
                self.state.find_duplicates();
            }
        });
        ui.add_space(10.);
        let Some(found) = &self.state.duplicates else {
            caption(
                ui,
                tx(
                    ui,
                    "Copies are files with the same size and the same BLAKE3 hash of their contents. Further hard links of one file are not copies; nothing is changed or deleted.",
                ),
            );
            return;
        };
        let nodes = &self.state.nodes;
        if found.root != self.state.scope {
            caption(
                ui,
                td(
                    ui,
                    format!("Results for {}", node_path(nodes, found.root).display()),
                ),
            );
        }
        let lang = i18n::language(ui);
        if running {
            if let Some(progress) = &found.progress {
                let stage = match progress.stage {
                    crate::storage::duplicates::Stage::Sampling => {
                        "Comparing the start and end of each file"
                    }
                    crate::storage::duplicates::Stage::Contents => "Comparing full contents",
                };
                let fraction = if progress.bytes_total == 0 {
                    0.
                } else {
                    progress.bytes_done as f32 / progress.bytes_total as f32
                };
                ui.label(
                    RichText::new(td(
                        ui,
                        format!(
                            "{}: {} of {} files · {} of {}",
                            tx(ui, stage),
                            lang.integer(progress.files_done),
                            lang.integer(progress.files_total),
                            ui_bytes(ui, progress.bytes_done),
                            ui_bytes(ui, progress.bytes_total)
                        ),
                    ))
                    .font(regular(13.))
                    .color(p.muted),
                );
                meter(ui, fraction, ui.available_width(), 6., p.accent);
            } else {
                ui.horizontal(|ui| {
                    ui.add(Spinner::new().size(13.).color(p.accent));
                    ui.label(RichText::new(tx(ui, "Grouping files by size…")).color(p.muted));
                });
            }
            ui.add_space(8.);
        }
        // Counts describe the groups shown, which the search may narrow.
        let shown = found.visible.iter().map(|&g| &found.groups[g]);
        let reclaimable = shown
            .clone()
            .map(|g| g.reclaimable)
            .fold(0, u64::saturating_add);
        let extra: u64 = shown.map(|g| g.copies.len() as u64 - 1).sum();
        let mut export = None;
        ui.horizontal_wrapped(|ui| {
            let text = match &found.summary {
                Some(summary) if found.groups.is_empty() => td(
                    ui,
                    format!(
                        "No copies among {} files of at least {}",
                        lang.integer(summary.files),
                        ui_bytes(ui, summary.min_size)
                    ),
                ),
                _ if found.visible.is_empty() && !found.groups.is_empty() => {
                    tx(ui, "No sets of copies match the search")
                }
                _ => td(
                    ui,
                    format!(
                        "{} sets of copies · {} extra copies · up to {} reclaimable",
                        lang.integer(found.visible.len() as u64),
                        lang.integer(extra),
                        ui_bytes(ui, reclaimable)
                    ),
                ),
            };
            ui.label(RichText::new(text).font(semibold(15.)));
            if let Some(summary) = &found.summary {
                // An incomplete search must not read as "no copies" or "all copies".
                let status = match summary.status() {
                    "cancelled_partial" => Some("Partial · cancelled"),
                    "partial" => Some("Partial"),
                    _ => None,
                };
                if let Some(status) = status {
                    badge(ui, &tx(ui, status), p.warn);
                }
                if !found.visible.is_empty() {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_enabled_ui(self.state.task.is_none(), |ui| {
                            for format in ["JSON", "CSV"] {
                                if ui
                                    .button(format)
                                    .on_hover_text(tx(ui, "Export the sets of copies shown"))
                                    .clicked()
                                {
                                    export = Some(format.to_lowercase());
                                }
                            }
                        });
                    });
                }
            }
        });
        ui.add_space(8.);
        let mut action = None;
        if !found.rows.is_empty() {
            card_with(ui, Margin::symmetric(8, 6), |ui| {
                ui.visuals_mut().widgets.hovered.bg_fill = p.hover;
                TableBuilder::new(ui)
                    .id_salt(("duplicates", self.state.scan_generation))
                    .vscroll(false)
                    .vertical_scroll_offset(0.)
                    .scroll_bar_visibility(scroll_area::ScrollBarVisibility::AlwaysHidden)
                    .striped(false)
                    .resizable(false)
                    .cell_layout(Layout::left_to_right(Align::Center))
                    .sense(Sense::click())
                    .column(Column::remainder().clip(true))
                    .column(Column::exact(100.).clip(true))
                    .column(Column::exact(34.))
                    .body(|body| {
                        body.rows(ROW_HEIGHT, found.rows.len(), |mut row| {
                            match found.rows[row.index()] {
                                DuplicateRow::Group(g) => {
                                    let group = &found.groups[g];
                                    row.col(|ui| {
                                        ui.push_id(("group", g), |ui| {
                                            ui.add(
                                                Label::new(
                                                    RichText::new(td(
                                                        ui,
                                                        format!(
                                                            "{} copies of {} · up to {} reclaimable",
                                                            group.copies.len(),
                                                            ui_bytes(ui, group.size),
                                                            ui_bytes(ui, group.reclaimable)
                                                        ),
                                                    ))
                                                    .font(semibold(13.5)),
                                                )
                                                .truncate(),
                                            );
                                        });
                                    });
                                    row.col(|_| {});
                                    row.col(|_| {});
                                }
                                DuplicateRow::Copy(g, c) => {
                                    let group = &found.groups[g];
                                    let copy = &group.copies[c];
                                    let id = copy.id;
                                    let node = &nodes[id];
                                    row.set_selected(self.state.selected == id);
                                    row.col(|ui| {
                                        ui.push_id(("copy", id), |ui| {
                                            ui.add_space(20.);
                                            entry_glyph(ui, node);
                                            let folder = relative_folder(nodes, id, found.root);
                                            let mut job = text::LayoutJob::default();
                                            job.append(
                                                &node.name,
                                                0.,
                                                TextFormat::simple(regular(14.), p.text),
                                            );
                                            job.append(
                                                &folder,
                                                12.,
                                                TextFormat::simple(regular(12.5), p.faint),
                                            );
                                            let response = ui.add(
                                                Label::new(job).truncate().sense(Sense::click()),
                                            );
                                            if response.clicked() || response.secondary_clicked() {
                                                self.state.selected = id;
                                            }
                                            entry_menu(&response, nodes, id, &mut action);
                                            response.on_hover_ui(|ui| {
                                                let mut lines =
                                                    vec![node_path(nodes, id).display().to_string()];
                                                if c == 0
                                                    && group
                                                        .copies
                                                        .iter()
                                                        .any(|other| other.modified_ms != copy.modified_ms)
                                                {
                                                    lines.push(tx(ui, "Oldest copy"));
                                                }
                                                lines.push(if copy.links > 1 {
                                                    tx(ui, "Deleting this path frees nothing: other hard links keep the file.")
                                                } else {
                                                    td(
                                                        ui,
                                                        format!(
                                                            "Deleting it frees up to {}",
                                                            ui_bytes(ui, copy.freeable)
                                                        ),
                                                    )
                                                });
                                                ui.label(lines.join("\n"));
                                            });
                                        });
                                    });
                                    row.col(|ui| {
                                        let value = node_size(ui, node, true);
                                        numeric_value(ui, &value, ui.available_width() - 4., 13.5);
                                    });
                                    row.col(|ui| {
                                        ui.push_id(("copy-reveal", id), |ui| {
                                            if reveal_button(ui, &node.name).clicked() {
                                                action = Some(EntryAction::Reveal(id));
                                            }
                                        });
                                    });
                                    if row.response().clicked() {
                                        self.state.selected = id;
                                    }
                                }
                            }
                        });
                    });
            });
        }
        if let Some(summary) = &found.summary {
            ui.add_space(8.);
            folding(ui, tx(ui, "Comparison details"))
                .id_salt("duplicate-notes")
                .show(ui, |ui| {
                    ui.label(td(
                        ui,
                        format!(
                            "{} files compared · {} read · {}",
                            lang.integer(summary.compared),
                            ui_bytes(ui, summary.bytes_read),
                            tx(ui, &summary.medium)
                        ),
                    ));
                    ui.label(td(
                        ui,
                        format!(
                            "{} smaller files not compared · {} hard-link paths · {} changed since the scan · {} skipped · {} errors",
                            lang.integer(summary.below_minimum),
                            lang.integer(summary.hard_links),
                            lang.integer(summary.changed),
                            lang.integer(summary.skipped),
                            lang.integer(summary.errors)
                        ),
                    ));
                    for note in &summary.notes {
                        ui.label(td(ui, note));
                    }
                });
        }
        if let Some(format) = export {
            self.state.export_duplicates(&format);
        }
        self.apply_entry_action(ui, action);
    }

    fn storage_map(&mut self, ui: &mut Ui, height: f32) -> Option<usize> {
        let p = palette(ui);
        let mut clicked = None;
        let mut action = None;
        let top = ui.cursor().top();
        // Beside the table the explanation moves into the title's tooltip.
        let roomy = ui.available_width() >= 420.;
        let title = ui.label(RichText::new(tx(ui, "Size map")).font(semibold(15.)));
        if !roomy {
            title.on_hover_text(tx(ui, "Area = size on disk · double-click to open"));
        }
        ui.add_space(4.);
        self.map_legend(ui);
        ui.add_space(10.);
        let footer = if roomy { 30. } else { 0. };
        let (rect, _) = ui.allocate_exact_size(
            vec2(
                ui.available_width(),
                (height - (ui.cursor().top() - top) - footer).max(64.),
            ),
            Sense::hover(),
        );
        let nonzero = &self.state.map_ids;
        let dark = ui.visuals().dark_mode;
        if nonzero.is_empty() {
            ui.painter().rect_filled(rect, 8, p.canvas);
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                tx(ui, "No nonzero files"),
                regular(13.),
                p.faint,
            );
        } else {
            // Keyed by size: scrolling moves the map without laying it out again.
            if self.map_key != Some((self.state.map_revision, rect.size())) {
                self.map_cache = treemap_tiles(nonzero, &self.state.nodes, rect)
                    .into_iter()
                    .map(|(id, tile)| (id, tile, tile_kind(&self.state.nodes[id])))
                    .collect();
                self.map_key = Some((self.state.map_revision, rect.size()));
                self.map_origin = rect.min;
            }
            let offset = rect.min - self.map_origin;
            for &(id, tile, kind) in &self.map_cache {
                let rect = tile.translate(offset);
                if rect.width() < 1. || rect.height() < 1. {
                    continue;
                }
                let node = &self.state.nodes[id];
                let r = rect.shrink(1.);
                let base = p.tiles[kind];
                // Tiles too small to point at are painted, not made interactive:
                // a folder of thousands of files must not cost a widget each.
                if rect.width() < 6. || rect.height() < 6. {
                    ui.painter().rect_filled(r, 2, base);
                    continue;
                }
                let response = ui.interact(
                    r,
                    Id::new(("tile", self.state.scan_generation, id)),
                    Sense::click(),
                );
                let accessible_label = format!("{} {}", node.name, node_size(ui, node, true));
                response.widget_info(|| {
                    WidgetInfo::labeled(WidgetType::Button, true, &accessible_label)
                });
                let fill = match (response.hovered(), dark) {
                    (true, true) => theme::mix(base, Color32::WHITE, 0.12),
                    (true, false) => theme::mix(base, Color32::BLACK, 0.07),
                    (false, _) => base,
                };
                let radius = if r.width() > 24. && r.height() > 24. {
                    6
                } else {
                    3
                };
                ui.painter().rect_filled(r, radius, fill);
                if self.state.selected == id || response.has_focus() {
                    ui.painter().rect_stroke(
                        r,
                        radius,
                        Stroke::new(2.0_f32, p.text),
                        StrokeKind::Inside,
                    );
                }
                if r.width() > 64. && r.height() > 44. {
                    let painter = ui.painter().with_clip_rect(r.shrink(6.));
                    for (text, font, alpha, y) in [
                        (node.name.clone(), medium(13.), 1., 9.),
                        (node_size(ui, node, true), regular(12.), 0.78, 28.),
                    ] {
                        let color = p.tile_text.gamma_multiply(alpha);
                        let mut job = text::LayoutJob::simple_singleline(text, font, color);
                        job.wrap.max_width = r.width() - 20.;
                        job.wrap.max_rows = 1;
                        job.wrap.break_anywhere = true;
                        let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
                        painter.galley(r.left_top() + vec2(10., y), galley, color);
                    }
                }
                if response.clicked() || response.secondary_clicked() {
                    self.state.selected = id;
                }
                if response.double_clicked() && node.is_dir {
                    clicked = Some(id);
                }
                entry_menu(&response, &self.state.nodes, id, &mut action);
                response.on_hover_ui(|ui| {
                    ui.label(td(
                        ui,
                        format!(
                            "{}\n{} on disk · {} file size\nDouble-click a folder to open",
                            node_path(&self.state.nodes, id).display(),
                            node_size(ui, node, true),
                            node_size(ui, node, false)
                        ),
                    ));
                });
            }
        }
        if roomy {
            ui.add_space(10.);
            caption(ui, tx(ui, "Area = size on disk · double-click to open"));
        }
        self.apply_entry_action(ui, action);
        clicked
    }

    /// The kinds present among the map's entries, and their colors.
    fn map_legend(&mut self, ui: &mut Ui) {
        let p = palette(ui);
        if self.map_kinds.0 != self.state.map_revision {
            let mut present = [false; 7];
            for &id in &self.state.map_ids {
                present[tile_kind(&self.state.nodes[id])] = true;
            }
            self.map_kinds = (self.state.map_revision, present);
        }
        let present = self.map_kinds.1;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6., 4.);
            for (kind, label) in TILE_KINDS.iter().enumerate() {
                if !present[kind] {
                    continue;
                }
                let (rect, _) = ui.allocate_exact_size(vec2(10., 16.), Sense::hover());
                ui.painter().rect_filled(
                    Rect::from_center_size(rect.center(), vec2(10., 10.)),
                    3,
                    p.tiles[kind],
                );
                caption(ui, tx(ui, label));
                ui.add_space(6.);
            }
        });
    }
}

/// Legend names of the size-map colors, in palette order.
const TILE_KINDS: [&str; 7] = [
    "Folders",
    "Media",
    "Documents",
    "Code",
    "Archives & disk images",
    "Programs",
    "Other files",
];

fn tile_kind(node: &ScanNode) -> usize {
    if node.is_dir {
        return 0;
    }
    match crate::search::content(&node.name) {
        Content::Media => 1,
        Content::Document => 2,
        Content::Code => 3,
        Content::Archive => 4,
        Content::Program => 5,
        Content::Other => 6,
    }
}

/// A minimum file size for the duplicate search: 1 byte means every file.
fn minimum_label(ui: &Ui, size: u64) -> String {
    if size <= 1 {
        tx(ui, "Any size")
    } else {
        ui_bytes(ui, size)
    }
}

/// The folder holding `id`, from the searched folder `root` down: the part
/// that tells copies apart, where a full path would be cut off before it.
fn relative_folder(nodes: &[ScanNode], id: usize, root: usize) -> String {
    let mut names = Vec::new();
    let mut next = nodes.get(id).and_then(|node| node.parent);
    while let Some(folder) = next {
        let name = &nodes[folder].name;
        if folder == root {
            // The root's name is its full path; its last part is enough here.
            names.push(
                std::path::Path::new(name)
                    .file_name()
                    .map_or(name.clone(), |n| n.to_string_lossy().into_owned()),
            );
            break;
        }
        names.push(name.clone());
        next = nodes[folder].parent;
    }
    names.reverse();
    names.join(std::path::MAIN_SEPARATOR_STR)
}

fn node_size(ui: &Ui, node: &ScanNode, allocated: bool) -> String {
    let value = if allocated {
        node.allocated
    } else {
        node.logical
    };
    if node.incomplete {
        if node.is_dir || value > 0 {
            format!("≥ {}", ui_bytes(ui, value))
        } else {
            tx(ui, "Unavailable")
        }
    } else {
        ui_bytes(ui, value)
    }
}

fn share(part: u64, total: u64) -> f64 {
    if total == 0 {
        0.
    } else {
        part as f64 / total as f64 * 100.
    }
}

/// A value on one line that copies itself when clicked; hover shows it whole.
fn copy_label(ui: &mut Ui, value: &str, size: f32, color: Color32) {
    let response = ui.add(
        Label::new(RichText::new(value).font(regular(size)).color(color))
            .truncate()
            .show_tooltip_when_elided(false)
            .sense(Sense::click()),
    );
    if response.clicked() {
        ui.ctx().copy_text(value.into());
    }
    response.on_hover_text(format!("{value}\n{}", tx(ui, "Click to copy")));
}

#[derive(Clone, Copy)]
enum EntryAction {
    Reveal(usize),
    CopyPath(usize),
    Open(usize),
}

/// Right-click menu shared by table rows, names and map tiles. It acts on the
/// entry that was right-clicked, even when rows re-sort beneath the open menu.
fn entry_menu(
    response: &Response,
    nodes: &[ScanNode],
    id: usize,
    action: &mut Option<EntryAction>,
) {
    let target = Id::new("entry-menu-target");
    if response.secondary_clicked() || response.long_touched() {
        response.ctx.data_mut(|data| data.insert_temp(target, id));
    }
    response.context_menu(|ui| {
        let id = ui.ctx().data(|data| data.get_temp(target)).unwrap_or(id);
        let Some(node) = nodes.get(id) else {
            return;
        };
        if ui.button(tx(ui, REVEAL)).clicked() {
            *action = Some(EntryAction::Reveal(node.id));
            ui.close();
        }
        if ui.button(tx(ui, "Copy path")).clicked() {
            *action = Some(EntryAction::CopyPath(node.id));
            ui.close();
        }
        if node.is_dir && ui.button(tx(ui, "Open folder")).clicked() {
            *action = Some(EntryAction::Open(node.id));
            ui.close();
        }
    });
}

/// One part of the breadcrumb path. Its hover background extends past the
/// text without taking layout space, so the first part aligns with the page.
fn crumb(ui: &mut Ui, name: &str, current: bool) -> Response {
    let p = palette(ui);
    let font = if current { medium(14.) } else { regular(14.) };
    let galley = ui.painter().layout_no_wrap(name.to_owned(), font, p.text);
    let (rect, response) = ui.allocate_exact_size(vec2(galley.size().x, 26.), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, name));
    let background = rect.expand2(vec2(6., 0.));
    if response.hovered() {
        ui.painter().rect_filled(background, 6, p.hover);
    }
    focus_ring(ui, &response, background, CornerRadius::same(6));
    let color = if current || response.hovered() {
        p.text
    } else {
        p.muted
    };
    ui.painter().galley(
        pos2(rect.left(), rect.center().y - galley.size().y / 2.),
        galley,
        color,
    );
    response
}

/// A small folder or document outline before an entry's name.
fn entry_glyph(ui: &mut Ui, node: &ScanNode) {
    let (rect, _) = ui.allocate_exact_size(vec2(18., 18.), Sense::hover());
    paint_glyph(ui, rect, node.is_dir);
}

fn paint_glyph(ui: &Ui, rect: Rect, folder: bool) {
    let p = palette(ui);
    let r = Rect::from_center_size(rect.center(), vec2(16., 16.));
    let scale = r.width() / 24.;
    let point = |x: f32, y: f32| r.min + vec2(x, y) * scale;
    let stroke = Stroke::new(1.6 * scale * 1.2, p.faint);
    let painter = ui.painter();
    let shape = if folder {
        vec![
            point(3., 6.5),
            point(3., 19.),
            point(21., 19.),
            point(21., 8.5),
            point(11.5, 8.5),
            point(9.5, 5.),
            point(3., 5.),
            point(3., 6.5),
        ]
    } else {
        vec![
            point(6., 3.),
            point(14., 3.),
            point(19., 8.),
            point(19., 21.),
            point(6., 21.),
            point(6., 3.),
        ]
    };
    painter.add(Shape::line(shape, stroke));
    if !folder {
        painter.add(Shape::line(
            vec![point(14., 3.), point(14., 8.), point(19., 8.)],
            stroke,
        ));
    }
}

/// A folder with an outgoing arrow, drawn on the app's 24-unit icon grid.
fn reveal_button(ui: &mut Ui, name: &str) -> Response {
    let p = palette(ui);
    let label = format!("{} {name}", tx(ui, REVEAL));
    let response = ui.add_sized([28., 28.], Button::new("").frame_when_inactive(false));
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), &label));
    let color = if response.hovered() { p.text } else { p.faint };
    let r = Rect::from_center_size(response.rect.center(), vec2(16., 16.));
    let scale = r.width() / 24.;
    let stroke = Stroke::new(1.8 * scale * 1.1, color);
    let point = |x: f32, y: f32| r.min + vec2(x, y) * scale;
    let painter = ui.painter();
    painter.add(Shape::line(
        vec![
            point(12., 20.),
            point(3., 20.),
            point(3., 5.),
            point(9., 5.),
            point(11., 7.5),
            point(20., 7.5),
            point(20., 11.),
        ],
        stroke,
    ));
    painter.line_segment([point(14., 19.), point(21., 12.)], stroke);
    painter.add(Shape::line(
        vec![point(16., 12.), point(21., 12.), point(21., 17.)],
        stroke,
    ));
    response.on_hover_text(label)
}

/// The disclosure chevron of a folder row, with a 28-point hit target.
fn folder_toggle(ui: &mut Ui, name: &str, expanded: bool) -> Response {
    let p = palette(ui);
    let label = format!(
        "{} {name}",
        tx(ui, if expanded { "Collapse" } else { "Expand" })
    );
    let response = ui.add_sized([28., 28.], Button::new("").frame_when_inactive(false));
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), &label));
    let color = if response.hovered() { p.text } else { p.muted };
    let angle = if expanded {
        std::f32::consts::FRAC_PI_2
    } else {
        0.
    };
    chevron(
        ui.painter(),
        response.rect.center(),
        9.,
        angle,
        Stroke::new(1.7_f32, color),
    );
    response.on_hover_text(label)
}

fn scan_status_color(state: &State, p: &theme::Palette) -> Color32 {
    match &state.summary {
        Some(s) if s.cancelled || s.stopped_early => p.warn,
        Some(s) if s.errors.saturating_sub(s.denied.saturating_add(s.locked)) > 0 => p.warn,
        Some(_) => p.memory,
        None => p.faint,
    }
}

pub(super) fn scan_status(state: &State) -> String {
    let lang = state.settings.language.resolve();
    if let Some(s) = &state.summary {
        // Refused and locked entries are expected on system drives: protected
        // folders need an administrator, files Windows holds open keep lower
        // bounds. Only other errors make a scan partial.
        let other = s.errors.saturating_sub(s.denied.saturating_add(s.locked));
        let status = if s.cancelled {
            "Partial · cancelled"
        } else if s.stopped_early {
            "Partial · stopped early"
        } else if other > 0 {
            "Partial"
        } else if s.denied > 0 || s.skipped_cloud > 0 || s.skipped_reparse > 0 {
            "Complete with exclusions"
        } else {
            "Complete"
        };
        let mut text = lang.format(
            "{0} · {1} s",
            &[
                lang.text(status),
                lang.decimal(s.elapsed_ms as f64 / 1000., 2),
            ],
        );
        if s.denied > 0 {
            text += " · ";
            text += &lang.format("{0} need administrator rights", &[lang.integer(s.denied)]);
        }
        if other > 0 {
            text += " · ";
            text += &lang.format("{0} errors", &[lang.integer(other)]);
        }
        text
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

fn scan_primary_width(ui: &Ui) -> f32 {
    label_width(ui, &tx(ui, "Scan"), 76.).max(label_width(ui, &tx(ui, "Rescan"), 76.))
}

fn scan_actions_width(ui: &Ui) -> f32 {
    label_width(ui, &tx(ui, "Browse…"), 82.)
        + scan_primary_width(ui)
        + label_width(ui, &tx(ui, "Cancel"), 76.)
        + 2. * ui.spacing().item_spacing.x
}

// Balanced binary subdivision: O(n log n), no zero-area hit targets or fictitious weights.
pub fn treemap_tiles(ids: &[usize], nodes: &[ScanNode], rect: Rect) -> Vec<(usize, Rect)> {
    fn split(ids: &[usize], nodes: &[ScanNode], rect: Rect, out: &mut Vec<(usize, Rect)>) {
        if ids.is_empty() {
            return;
        }
        if ids.len() == 1 {
            out.push((ids[0], rect));
            return;
        }
        let total: f64 = ids.iter().map(|id| nodes[*id].allocated as f64).sum();
        if total <= 0. {
            return;
        }
        let mut sum = 0.;
        let mut cut = 1;
        for (i, id) in ids.iter().enumerate().take(ids.len() - 1) {
            sum += nodes[*id].allocated as f64;
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
