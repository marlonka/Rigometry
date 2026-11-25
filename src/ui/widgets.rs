//! Shared interface components. Pages compose these instead of styling
//! egui widgets one by one, so every page has the same rhythm and states.

use super::theme::{self, Palette, display, medium, palette, regular, semibold};
use crate::{
    i18n::{self, message as td, text as tx},
    model::{Availability, Reading, now_ms},
};
use eframe::egui::{
    self, Align, CollapsingHeader, Color32, CornerRadius, Frame, Galley, Id, Layout, Margin, Mesh,
    Pos2, Rect, Response, RichText, Sense, Shape, Stroke, StrokeKind, TextWrapMode, Ui, WidgetInfo,
    WidgetText, WidgetType, pos2, vec2,
};

/// Space between page sections.
pub const SECTION_GAP: f32 = 28.;
/// Space between cards in a row or column.
pub const CARD_GAP: f32 = 14.;

/// A data series, colored per theme.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Series {
    Cpu,
    Memory,
    Gpu,
    Temperature,
}

impl Series {
    pub fn color(self, ui: &Ui) -> Color32 {
        let p = palette(ui);
        match self {
            Self::Cpu => p.cpu,
            Self::Memory => p.memory,
            Self::Gpu => p.gpu,
            Self::Temperature => p.temperature,
        }
    }
}

/// Page title with an optional subtitle.
pub fn page_header(ui: &mut Ui, title: &str, subtitle: Option<&str>) {
    page_header_with(ui, title, subtitle, None::<fn(&mut Ui)>);
}

/// Page title with right-aligned actions; they move below the title when
/// the page is too narrow for both.
pub fn page_header_with(
    ui: &mut Ui,
    title: &str,
    subtitle: Option<&str>,
    actions: Option<impl FnOnce(&mut Ui)>,
) {
    let title = egui::Label::new(
        RichText::new(tx(ui, title))
            .font(display(28.))
            .extra_letter_spacing(-0.4),
    )
    .wrap();
    match actions {
        Some(actions) if ui.available_width() >= 560. => {
            ui.horizontal(|ui| {
                ui.add(title);
                ui.with_layout(Layout::right_to_left(Align::Center), actions);
            });
        }
        Some(actions) => {
            ui.add(title);
            ui.add_space(8.);
            ui.horizontal_wrapped(actions);
        }
        None => {
            ui.add(title);
        }
    }
    if let Some(subtitle) = subtitle {
        ui.add_space(2.);
        ui.add(
            egui::Label::new(
                RichText::new(subtitle)
                    .font(regular(14.))
                    .color(palette(ui).muted),
            )
            .wrap(),
        );
    }
    ui.add_space(24.);
}

/// A section title above cards or lists, with optional trailing content.
pub fn section_header(ui: &mut Ui, title: &str, trailing: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.set_min_height(32.);
        ui.label(RichText::new(tx(ui, title)).font(semibold(17.)));
        ui.with_layout(Layout::right_to_left(Align::Center), trailing);
    });
    ui.add_space(8.);
}

/// A title inside a card.
pub fn card_title(ui: &mut Ui, title: &str) -> Response {
    let response = ui.label(RichText::new(td(ui, title)).font(semibold(15.)));
    ui.add_space(10.);
    response
}

/// A bordered surface that fills the available width.
pub fn card<R>(ui: &mut Ui, body: impl FnOnce(&mut Ui) -> R) -> R {
    card_with(ui, Margin::same(18), body)
}

pub fn card_with<R>(ui: &mut Ui, margin: Margin, body: impl FnOnce(&mut Ui) -> R) -> R {
    let p = palette(ui);
    Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1.0_f32, p.border))
        .corner_radius(12)
        .inner_margin(margin)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            body(ui)
        })
        .inner
}

/// Lays out `count` equal columns with a fixed gap, or stacks them when each
/// would be narrower than `min_width`.
pub fn columns(ui: &mut Ui, count: usize, min_width: f32, cell: impl FnMut(&mut Ui, usize)) {
    columns_with(ui, count, min_width, false, cell);
}

/// Columns inside one surface, separated by hairlines instead of gaps.
pub fn divided_columns(
    ui: &mut Ui,
    count: usize,
    min_width: f32,
    cell: impl FnMut(&mut Ui, usize),
) {
    columns_with(ui, count, min_width, true, cell);
}

fn columns_with(
    ui: &mut Ui,
    count: usize,
    min_width: f32,
    divided: bool,
    mut cell: impl FnMut(&mut Ui, usize),
) {
    let gap = if divided { 41. } else { CARD_GAP };
    let width = ui.available_width();
    let each = ((width - gap * (count - 1) as f32) / count as f32).floor();
    if each < min_width {
        for index in 0..count {
            if index > 0 {
                if divided {
                    ui.add_space(14.);
                    divider(ui);
                    ui.add_space(14.);
                } else {
                    ui.add_space(gap);
                }
            }
            cell(ui, index);
        }
        return;
    }
    let divider_color = palette(ui).divider;
    let row = ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        let mut lines = vec![];
        for index in 0..count {
            let rect = ui
                .allocate_ui_with_layout(vec2(each, 0.), Layout::top_down(Align::Min), |ui| {
                    ui.set_width(each);
                    cell(ui, index);
                })
                .response
                .rect;
            if index + 1 < count {
                lines.push(rect.right() + gap / 2.);
            }
        }
        lines
    });
    if divided {
        let rect = row.response.rect;
        for x in row.inner {
            let x = x.round() + 0.5;
            ui.painter().line_segment(
                [pos2(x, rect.top()), pos2(x, rect.bottom())],
                Stroke::new(1.0_f32, divider_color),
            );
        }
    }
}

/// Small secondary text: hints, counts and status lines.
pub fn caption(ui: &mut Ui, text: impl Into<String>) -> Response {
    let color = palette(ui).faint;
    ui.label(RichText::new(text).font(regular(12.5)).color(color))
}

/// A field label in muted text.
pub fn muted(ui: &mut Ui, text: impl Into<String>, size: f32) -> Response {
    let color = palette(ui).muted;
    ui.label(RichText::new(text).font(regular(size)).color(color))
}

/// The filled call to action of a region.
pub fn primary_button(ui: &Ui, text: impl Into<String>) -> egui::Button<'static> {
    let p = palette(ui);
    egui::Button::new(RichText::new(text).color(p.on_primary))
        .fill(p.primary)
        .stroke(Stroke::new(1.0_f32, p.primary))
}

/// A button without a frame until hovered, for secondary actions in rows.
pub fn quiet_button(ui: &Ui, text: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text).color(palette(ui).muted)).frame_when_inactive(false)
}

/// Mutually exclusive views of one context, drawn as a segmented control.
/// Returns the option chosen this frame. Wraps into separate pills when the
/// row would not fit.
pub fn segmented<T: PartialEq + Copy>(
    ui: &mut Ui,
    id: impl std::hash::Hash,
    current: T,
    options: &[(T, &str)],
) -> Option<T> {
    let p = palette(ui);
    let font = medium(13.5);
    let height = 28.;
    let pad = 2.;
    let labels: Vec<_> = options
        .iter()
        .map(|(value, label)| {
            let text = tx(ui, label);
            let galley = ui
                .painter()
                .layout_no_wrap(text.clone(), font.clone(), p.text);
            (*value, text, galley)
        })
        .collect();
    let widths: Vec<f32> = labels
        .iter()
        .map(|(_, _, galley)| galley.size().x + 28.)
        .collect();
    let total = widths.iter().sum::<f32>() + pad * 2.;
    let id = Id::new(id);
    let mut chosen = None;
    let mut segment = |ui: &mut Ui, rect: Rect, index: usize, track: bool| {
        let (value, text, galley) = &labels[index];
        let selected = *value == current;
        let response = ui.interact(rect, id.with(index), Sense::click());
        response.widget_info(|| {
            WidgetInfo::selected(WidgetType::SelectableLabel, true, selected, text.clone())
        });
        let radius = CornerRadius::same(if track { 7 } else { 8 });
        if selected {
            let fill = if ui.visuals().dark_mode {
                p.pressed
            } else {
                p.canvas
            };
            ui.painter().rect(
                rect,
                radius,
                fill,
                Stroke::new(1.0_f32, p.control_border),
                StrokeKind::Inside,
            );
        } else if response.hovered() {
            ui.painter().rect_filled(rect, radius, p.hover);
        } else if !track {
            ui.painter().rect_stroke(
                rect,
                radius,
                Stroke::new(1.0_f32, p.border),
                StrokeKind::Inside,
            );
        }
        focus_ring(ui, &response, rect, radius);
        let color = if selected || response.hovered() {
            p.text
        } else {
            p.muted
        };
        ui.painter()
            .galley(rect.center() - galley.size() / 2., galley.clone(), color);
        if response.clicked() && !selected {
            chosen = Some(*value);
        }
    };
    if total <= ui.available_width() {
        let (track, _) = ui.allocate_exact_size(vec2(total, height + pad * 2.), Sense::hover());
        ui.painter().rect(
            track,
            10,
            if ui.visuals().dark_mode {
                p.control
            } else {
                p.hover
            },
            Stroke::new(1.0_f32, p.border),
            StrokeKind::Inside,
        );
        let mut x = track.left() + pad;
        for (index, width) in widths.iter().enumerate() {
            let rect = Rect::from_min_size(pos2(x, track.top() + pad), vec2(*width, height));
            segment(ui, rect, index, true);
            x += width;
        }
    } else {
        ui.horizontal_wrapped(|ui| {
            for (index, width) in widths.iter().enumerate() {
                let (rect, _) = ui.allocate_exact_size(vec2(*width, height), Sense::hover());
                segment(ui, rect, index, false);
            }
        });
    }
    chosen
}

/// The width [`segmented`] takes when its options fit on one row.
pub fn segmented_width(ui: &Ui, labels: &[&str]) -> f32 {
    labels
        .iter()
        .map(|label| {
            ui.painter()
                .layout_no_wrap(tx(ui, label), medium(13.5), Color32::WHITE)
                .size()
                .x
                + 28.
        })
        .sum::<f32>()
        + 4.
}

/// A switch for an on/off setting, its label to the right. The whole row
/// toggles it.
pub fn toggle(ui: &mut Ui, on: &mut bool, label: &str) -> Response {
    switch(ui, on, label, None)
}

/// A switch whose label sits in a settings label column of `column` width,
/// with the switch where the other rows have their controls.
pub fn toggle_row(ui: &mut Ui, on: &mut bool, label: &str, column: f32) -> Response {
    switch(ui, on, label, Some(column))
}

fn switch(ui: &mut Ui, on: &mut bool, label: &str, column: Option<f32>) -> Response {
    let p = palette(ui);
    let wrap = column.unwrap_or((ui.available_width() - 48.).max(40.));
    let galley = WidgetText::from(label).into_galley(
        ui,
        Some(TextWrapMode::Wrap),
        wrap,
        egui::TextStyle::Body,
    );
    let size = match column {
        Some(column) => vec2(column + 8. + 36., galley.size().y.max(32.)),
        None => vec2(36. + 12. + galley.size().x, galley.size().y.max(22.)),
    };
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    response.widget_info(|| {
        WidgetInfo::selected(WidgetType::Checkbox, ui.is_enabled(), *on, galley.text())
    });
    let t = ui.ctx().animate_bool_responsive(response.id, *on);
    let track_left = column.map_or(rect.left(), |column| rect.left() + column + 8.);
    let track = Rect::from_min_size(pos2(track_left, rect.center().y - 10.), vec2(36., 20.));
    let off = if response.hovered() {
        p.control_border
    } else {
        p.track
    };
    ui.painter()
        .rect_filled(track, 10, theme::mix(off, p.primary, t));
    // Off, the switch needs an outline that stands out from the surface.
    if t < 1. {
        ui.painter().rect_stroke(
            track,
            10,
            Stroke::new(1.0_f32, p.faint.gamma_multiply(0.75 * (1. - t))),
            StrokeKind::Inside,
        );
    }
    let knob = pos2(
        egui::lerp(track.left() + 10.0..=track.right() - 10., t),
        track.center().y,
    );
    let knob_off = if ui.visuals().dark_mode {
        p.muted
    } else {
        Color32::WHITE
    };
    ui.painter()
        .circle_filled(knob, 7., theme::mix(knob_off, p.on_primary, t));
    focus_ring(ui, &response, track.expand(2.), CornerRadius::same(12));
    let text_left = if column.is_some() {
        rect.left()
    } else {
        track.right() + 12.
    };
    ui.painter().galley(
        pos2(text_left, rect.center().y - galley.size().y / 2.),
        galley,
        p.text,
    );
    response
}

/// A visible keyboard-focus outline that does not change layout.
pub fn focus_ring(ui: &Ui, response: &Response, rect: Rect, radius: CornerRadius) {
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.expand(1.),
            radius,
            Stroke::new(1.5_f32, palette(ui).muted),
            StrokeKind::Outside,
        );
    }
}

/// A chevron pointing right, rotated by `angle` radians (τ/4 points down).
pub fn chevron(painter: &egui::Painter, center: Pos2, size: f32, angle: f32, stroke: Stroke) {
    let rotation = egui::emath::Rot2::from_angle(angle);
    let points: Vec<Pos2> = [vec2(-0.25, -0.5), vec2(0.25, 0.), vec2(-0.25, 0.5)]
        .into_iter()
        .map(|p| center + rotation * (p * size))
        .collect();
    painter.add(Shape::line(points.clone(), stroke));
    for point in [points[0], points[1], points[2]] {
        painter.circle_filled(point, stroke.width / 2., stroke.color);
    }
}

/// A disclosure section whose header chevron rotates as it opens.
pub fn folding(ui: &Ui, title: impl Into<WidgetText>) -> CollapsingHeader {
    let title = title.into();
    let id = title.text().to_owned();
    let width =
        (ui.available_width() - ui.spacing().indent - ui.spacing().button_padding.x).max(1.);
    let galley = title.into_galley(ui, Some(TextWrapMode::Wrap), width, egui::TextStyle::Button);
    CollapsingHeader::new(galley)
        .id_salt(id)
        .icon(|ui, openness, response| {
            let color = if response.hovered() {
                palette(ui).text
            } else {
                palette(ui).muted
            };
            chevron(
                ui.painter(),
                response.rect.center(),
                10.,
                openness * std::f32::consts::FRAC_PI_2,
                Stroke::new(1.6_f32, color),
            );
        })
}

/// The open/closed chevron of a combo box.
pub fn combo_icon(ui: &Ui, rect: Rect, _: &egui::style::WidgetVisuals, open: bool) {
    let angle = if open { -1. } else { 1. } * std::f32::consts::FRAC_PI_2;
    chevron(
        ui.painter(),
        rect.center(),
        9.,
        angle,
        Stroke::new(1.6_f32, palette(ui).muted),
    );
}

/// A horizontal meter: `fraction` of the track filled in `color`.
pub fn meter(ui: &mut Ui, fraction: f32, width: f32, height: f32, color: Color32) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    paint_meter(ui, rect, fraction, color);
    response
}

pub fn paint_meter(ui: &Ui, rect: Rect, fraction: f32, color: Color32) {
    let radius = CornerRadius::same((rect.height() / 2.).round() as u8);
    ui.painter().rect_filled(rect, radius, palette(ui).track);
    let fraction = if fraction.is_finite() {
        fraction.clamp(0., 1.)
    } else {
        0.
    };
    if fraction > 0. {
        let width = (rect.width() * fraction).max(rect.height());
        ui.painter().rect_filled(
            Rect::from_min_size(rect.min, vec2(width, rect.height())),
            radius,
            color,
        );
    }
}

/// A small status dot, for a live state beside text.
pub fn dot(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(8., 14.), Sense::hover());
    ui.painter().circle_filled(rect.center(), 3.5, color);
}

/// A pill-shaped label for a short state such as "Stale" or "Partial".
pub fn badge(ui: &mut Ui, text: &str, color: Color32) -> Response {
    let p = palette(ui);
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), medium(11.5), color);
    let size = galley.size() + vec2(14., 6.);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter()
        .rect_filled(rect, 6, theme::mix(p.surface, color, 0.14));
    ui.painter()
        .galley(rect.center() - galley.size() / 2., galley, color);
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, text));
    response
}

/// A number right-aligned in a fixed-width slot, so changing digits never
/// move neighbors. Click copies it.
pub fn numeric_value(ui: &mut Ui, text: &str, width: f32, size: f32) -> Response {
    let color = ui.visuals().text_color();
    numeric_value_in(ui, text, width, size, color)
}

/// [`numeric_value`] in a given color, for secondary figures.
pub fn numeric_value_in(
    ui: &mut Ui,
    text: &str,
    width: f32,
    size: f32,
    color: Color32,
) -> Response {
    let galley = ui
        .painter()
        .layout(text.into(), regular(size), color, width.max(1.));
    let (rect, response) =
        ui.allocate_exact_size(vec2(width, galley.size().y.max(26.)), Sense::click());
    ui.painter().with_clip_rect(rect).galley(
        pos2(
            rect.right() - galley.size().x,
            rect.center().y - galley.size().y * 0.5,
        ),
        galley,
        color,
    );
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, text));
    if response.clicked() {
        ui.ctx().copy_text(text.into());
    }
    response
}

/// Width of `label` in the button font, at least `minimum`, plus padding.
pub fn label_width(ui: &Ui, label: &str, minimum: f32) -> f32 {
    ui.painter()
        .layout_no_wrap(label.to_owned(), medium(14.), Color32::WHITE)
        .size()
        .x
        .max(minimum - 26.)
        + 26.
}

pub fn metric_chart(
    ui: &mut Ui,
    label: &str,
    points: &[(u64, Option<f64>)],
    unit: &str,
    max: f64,
    series: Series,
    plot_height: f32,
) {
    MetricChart {
        label,
        points,
        unit,
        max,
        series,
        plot_height,
        secondary: None,
        axis: true,
    }
    .show(ui);
}

/// A live reading over the last 120 seconds: headline value, then a line with
/// its area filled. Gaps in the samples stay gaps.
pub struct MetricChart<'a> {
    pub label: &'a str,
    pub points: &'a [(u64, Option<f64>)],
    pub unit: &'a str,
    pub max: f64,
    pub series: Series,
    pub plot_height: f32,
    /// A second current value shown at the right of the headline.
    pub secondary: Option<(&'a str, Option<&'a Reading>)>,
    /// Label the time axis and scale under the plot.
    pub axis: bool,
}

const WINDOW_MS: u64 = 120_000;

impl MetricChart<'_> {
    /// The chart on its own card.
    pub fn show(self, ui: &mut Ui) {
        card_with(ui, Margin::symmetric(18, 16), |ui| self.content(ui));
    }

    /// The chart without a card, for a column of a shared surface.
    pub fn content(self, ui: &mut Ui) {
        let Self {
            label,
            points,
            unit,
            max,
            series,
            plot_height,
            secondary,
            axis,
        } = self;
        let color = series.color(ui);
        let p = palette(ui);
        {
            let stale = points
                .last()
                .is_some_and(|point| now_ms().saturating_sub(point.0) > 5000);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.;
                dot(ui, color);
                ui.label(
                    RichText::new(tx(ui, label))
                        .font(medium(13.5))
                        .color(p.muted),
                );
                if stale {
                    badge(ui, &tx(ui, "Stale"), p.warn);
                }
                if let Some((label, _)) = secondary {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(tx(ui, label))
                                .font(medium(13.5))
                                .color(p.muted),
                        );
                    });
                }
            });
            ui.add_space(2.);
            let current = points.last().and_then(|(_, v)| *v);
            let value = current
                .map(|v| i18n::language(ui).decimal(v, 1))
                .unwrap_or("—".into());
            // A missing reading has no unit to show.
            let value_unit = if current.is_some() { unit } else { "" };
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 40.), Sense::hover());
            let mut primary_rect = rect;
            if let Some((label, reading)) = secondary {
                let secondary_rect =
                    Rect::from_min_max(pos2(rect.right() - 110., rect.top()), rect.max);
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
                paint_value(ui, secondary_rect, &text, unit, 24., Align::Max);
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
                        .map(|r| format!("{status}\n{}\n{}", td(ui, &r.source), td(ui, &r.detail)))
                        .unwrap_or_else(|| tx(ui, "No temperature provider reading")),
                );
            }
            paint_value(ui, primary_rect, &value, value_unit, 30., Align::Min);
            let response = ui.interact(
                primary_rect,
                ui.make_persistent_id(("metric-value", label)),
                Sense::hover(),
            );
            let accessible_label = format!(
                "{}: {}{}",
                tx(ui, label),
                format!("{value} {value_unit}").trim_end(),
                if stale {
                    tx(ui, " (stale)")
                } else {
                    String::new()
                }
            );
            response
                .widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, &accessible_label));
            ui.add_space(10.);
            let (rect, response) =
                ui.allocate_exact_size(vec2(ui.available_width(), plot_height), Sense::hover());
            paint_chart(ui, rect, points, max, color);
            if axis {
                ui.add_space(4.);
                ui.horizontal(|ui| {
                    let axis = |ui: &mut Ui, text: String| {
                        ui.label(RichText::new(text).font(regular(11.5)).color(p.faint))
                    };
                    axis(ui, tx(ui, "−120 s"));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        axis(ui, td(ui, format!("now · 0–{max:.0} {unit}")));
                    });
                });
            }
            let values: Vec<_> = points.iter().filter_map(|(_, v)| *v).collect();
            let lang = i18n::language(ui);
            if values.is_empty() {
                response.on_hover_text(tx(ui, "No valid samples available from this provider"));
            } else {
                response.on_hover_text(td(
                    ui,
                    format!(
                        "{} samples\nMin {} {} · Max {} {}\nMissing samples create gaps",
                        lang.integer(values.len() as u64),
                        lang.decimal(values.iter().copied().fold(f64::INFINITY, f64::min), 1),
                        unit,
                        lang.decimal(values.iter().copied().fold(f64::NEG_INFINITY, f64::max), 1),
                        unit
                    ),
                ));
            }
        }
    }
}

/// A value and its smaller, muted unit on one baseline, vertically centered
/// in `rect` and aligned to its left or right edge.
fn paint_value(ui: &Ui, rect: Rect, value: &str, unit: &str, size: f32, align: Align) {
    let p = palette(ui);
    let value = ui
        .painter()
        .layout_no_wrap(value.to_owned(), display(size), p.text);
    let unit = (!unit.is_empty()).then(|| {
        ui.painter()
            .layout_no_wrap(unit.to_owned(), medium((size * 0.5).max(13.)), p.muted)
    });
    let gap = 4.;
    let width = value.size().x + unit.as_ref().map_or(0., |u| gap + u.size().x);
    let left = if align == Align::Max {
        rect.right() - width
    } else {
        rect.left()
    };
    let top = rect.center().y - value.size().y * 0.5;
    let baseline = |galley: &Galley| {
        galley
            .rows
            .first()
            .and_then(|row| row.row.glyphs.first().map(|g| row.pos.y + g.pos.y))
            .unwrap_or(galley.size().y)
    };
    let painter = ui.painter().with_clip_rect(rect);
    if let Some(unit) = unit {
        let unit_top = top + baseline(&value) - baseline(&unit);
        painter.galley(pos2(left + value.size().x + gap, unit_top), unit, p.muted);
    }
    painter.galley(pos2(left, top), value, p.text);
}

/// Grid, filled area and line of a 120-second series ending now.
fn paint_chart(ui: &Ui, rect: Rect, points: &[(u64, Option<f64>)], max: f64, color: Color32) {
    let p = palette(ui);
    let painter = ui.painter_at(rect.expand(2.));
    for step in [0., 0.5, 1.] {
        let y = (rect.bottom() - rect.height() * step).round() + 0.5;
        let stroke = Stroke::new(1.0_f32, if step == 0. { p.track } else { p.divider });
        painter.line_segment([pos2(rect.left(), y), pos2(rect.right(), y)], stroke);
    }
    let end = now_ms();
    let start = end.saturating_sub(WINDOW_MS);
    let position = |time: u64, value: f64| {
        let x = rect.left()
            + ((time as f64 - start as f64) / WINDOW_MS as f64).clamp(0., 1.) as f32 * rect.width();
        let y = rect.bottom() - (value / max).clamp(0., 1.) as f32 * rect.height();
        pos2(x, y)
    };
    // Split into runs at missing samples and at gaps longer than 2.5 s.
    let mut runs: Vec<Vec<Pos2>> = vec![];
    let mut last_time: Option<u64> = None;
    for &(time, value) in points {
        match value {
            Some(value) if value.is_finite() => {
                let point = position(time, value);
                match (runs.last_mut(), last_time) {
                    (Some(run), Some(last)) if time.saturating_sub(last) <= 2500 => run.push(point),
                    _ => runs.push(vec![point]),
                }
                last_time = Some(time);
            }
            _ => last_time = None,
        }
    }
    let fill = color.gamma_multiply(if ui.visuals().dark_mode { 0.16 } else { 0.12 });
    for run in &runs {
        if run.len() > 1 {
            let mut mesh = Mesh::default();
            for (index, point) in run.iter().enumerate() {
                mesh.colored_vertex(*point, fill);
                mesh.colored_vertex(pos2(point.x, rect.bottom()), fill);
                if index > 0 {
                    let i = (index * 2) as u32;
                    mesh.add_triangle(i - 2, i - 1, i);
                    mesh.add_triangle(i - 1, i + 1, i);
                }
            }
            painter.add(mesh);
            painter.add(Shape::line(run.clone(), Stroke::new(2.0_f32, color)));
        }
    }
    if let Some(point) = runs.last().and_then(|run| run.last()) {
        painter.circle_filled(*point, 4.5, p.surface);
        painter.circle_filled(*point, 3., color);
    }
}

/// A reading as text: its value when valid, otherwise its state.
pub fn reading_text(ui: &Ui, reading: &Reading) -> String {
    if reading.state == Availability::Valid {
        reading.value.map(|v| number(ui, v, &reading.unit))
    } else {
        None
    }
    .unwrap_or_else(|| tx(ui, availability(&reading.state)))
}

/// A reading's state as a word, for tooltips and diagnostics.
pub fn state_label(state: &Availability) -> &'static str {
    match state {
        Availability::Valid => "Valid",
        _ => availability(state),
    }
}

pub fn availability(state: &Availability) -> &'static str {
    match state {
        Availability::Unsupported => "Unsupported",
        Availability::PermissionRequired => "Permission required",
        Availability::Failed => "Provider failed",
        Availability::Stale => "Stale",
        _ => "Unavailable",
    }
}

pub fn number(ui: &Ui, v: f64, unit: &str) -> String {
    if unit == "B" || unit == "bytes" {
        ui_bytes(ui, v.max(0.) as u64)
    } else {
        format!("{} {}", i18n::language(ui).decimal(v, 1), tx(ui, unit))
    }
}

pub fn ui_bytes(ui: &Ui, value: u64) -> String {
    i18n::language(ui).bytes(value)
}

/// Navigation icons on one 24-unit grid with a shared stroke.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    Overview,
    Cpu,
    Gpu,
    Storage,
    Diagnostics,
    Settings,
}

pub fn paint_icon(painter: &egui::Painter, r: Rect, icon: Icon, color: Color32) {
    let scale = r.width() / 24.;
    let s = Stroke::new(1.7 * scale, color);
    let point = |x: f32, y: f32| r.min + vec2(x, y) * scale;
    let outline = |x: f32, y: f32, w: f32, h: f32, radius: f32| {
        painter.rect_stroke(
            Rect::from_min_max(point(x, y), point(x + w, y + h)),
            CornerRadius::same((radius * scale).round() as u8),
            s,
            StrokeKind::Middle,
        );
    };
    let line = |points: &[(f32, f32)]| {
        let points: Vec<_> = points.iter().map(|&(x, y)| point(x, y)).collect();
        painter.add(Shape::line(points.clone(), s));
        for point in points {
            painter.circle_filled(point, s.width / 2., color);
        }
    };
    match icon {
        Icon::Overview => {
            outline(3., 3., 7.5, 9., 2.);
            outline(13.5, 3., 7.5, 5., 2.);
            outline(3., 15., 7.5, 6., 2.);
            outline(13.5, 11., 7.5, 10., 2.);
        }
        Icon::Cpu => {
            outline(5., 5., 14., 14., 2.5);
            outline(9., 9., 6., 6., 1.);
            for i in [9.5, 14.5] {
                line(&[(i, 2.), (i, 5.)]);
                line(&[(i, 19.), (i, 22.)]);
                line(&[(2., i), (5., i)]);
                line(&[(19., i), (22., i)]);
            }
        }
        Icon::Gpu => {
            outline(2., 6., 20., 12., 2.5);
            painter.circle_stroke(point(15., 12.), 3.2 * scale, s);
            line(&[(6., 10.), (6., 14.)]);
            line(&[(9., 10.), (9., 14.)]);
            line(&[(6., 18.), (6., 21.)]);
            line(&[(10., 18.), (10., 21.)]);
        }
        Icon::Storage => {
            outline(3., 4., 18., 16., 3.);
            line(&[(3., 13.), (21., 13.)]);
            line(&[(7., 16.5), (10., 16.5)]);
            painter.circle_filled(point(17., 16.5), 1.1 * scale, color);
        }
        Icon::Diagnostics => {
            line(&[
                (2., 12.),
                (6., 12.),
                (9., 5.),
                (14., 19.),
                (17., 12.),
                (22., 12.),
            ]);
        }
        Icon::Settings => {
            for (x, y) in [(8., 6.), (16., 12.), (10., 18.)] {
                line(&[(3., y), (x - 2.5, y)]);
                line(&[(x + 2.5, y), (21., y)]);
                painter.circle_stroke(point(x, y), 2.5 * scale, s);
            }
        }
    }
}

/// A sidebar destination: icon and label in a full-width row. `shortcut`
/// is the key shown at the right edge and the full combination for the tooltip.
pub fn nav_item(
    ui: &mut Ui,
    label: &str,
    shortcut: Option<(&str, &str)>,
    selected: bool,
    icon: Icon,
) -> bool {
    let p: &Palette = palette(ui);
    let label = tx(ui, label);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 36.), Sense::click());
    response
        .widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, true, selected, &label));
    if selected {
        ui.painter().rect_filled(rect, 8, p.selected);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, 8, p.hover);
    }
    focus_ring(ui, &response, rect, CornerRadius::same(8));
    let color = if selected || response.hovered() {
        p.text
    } else {
        p.muted
    };
    paint_icon(
        ui.painter(),
        Rect::from_center_size(rect.left_center() + vec2(20., 0.), vec2(18., 18.)),
        icon,
        color,
    );
    // One weight in every state, so selecting never shifts the label.
    let galley = ui.painter().layout(
        label.clone(),
        regular(14.),
        color,
        (rect.width() - 46.).max(10.),
    );
    let label_right = rect.left() + 38. + galley.size().x;
    ui.painter().galley(
        pos2(rect.left() + 38., rect.center().y - galley.size().y / 2.),
        galley,
        color,
    );
    if let Some((key, _)) = shortcut {
        let hint = ui
            .painter()
            .layout_no_wrap(key.to_owned(), medium(11.5), p.faint);
        // Long translations keep their full name; the tooltip keeps the key.
        if label_right + 12. + hint.size().x <= rect.right() - 10. {
            ui.painter().galley(
                pos2(
                    rect.right() - 10. - hint.size().x,
                    rect.center().y - hint.size().y / 2.,
                ),
                hint,
                p.faint,
            );
        }
    }
    response
        .on_hover_text(match shortcut {
            Some((_, keys)) => format!("{label} · {keys}"),
            None => label,
        })
        .clicked()
}

/// One labelled fact: label column, then a value that wraps.
pub fn fact_row(ui: &mut Ui, label: &str, value: &str, label_width: f32) -> Response {
    let p = palette(ui);
    let width = ui.available_width();
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 12.;
        ui.allocate_ui_with_layout(vec2(label_width, 0.), Layout::top_down(Align::Min), |ui| {
            ui.set_width(label_width);
            ui.add(
                egui::Label::new(RichText::new(label).font(regular(13.5)).color(p.muted)).wrap(),
            );
        });
        let available = (width - label_width - 12.).max(60.);
        ui.allocate_ui_with_layout(vec2(available, 0.), Layout::top_down(Align::Min), |ui| {
            ui.set_width(available);
            ui.add(
                egui::Label::new(RichText::new(value).font(regular(13.5)))
                    .wrap()
                    .sense(Sense::click()),
            )
        })
        .inner
    })
    .inner
}

/// A thin horizontal rule in the divider color.
pub fn divider(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.), Sense::hover());
    ui.painter().rect_filled(rect, 0, palette(ui).divider);
}

/// Width of the column of a label list: the widest label, within bounds.
pub fn label_column(ui: &Ui, labels: impl IntoIterator<Item = String>, min: f32, max: f32) -> f32 {
    labels
        .into_iter()
        .map(|label| {
            ui.painter()
                .layout_no_wrap(label, regular(13.5), Color32::WHITE)
                .size()
                .x
        })
        .fold(min, f32::max)
        .min(max)
}
