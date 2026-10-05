use eframe::egui;

fn pixels() -> image::RgbaImage {
    image::load_from_memory(include_bytes!("../assets/rigometry.png"))
        .expect("bundled application icon is a valid PNG")
        .into_rgba8()
}

pub fn window_icon() -> egui::IconData {
    let image = pixels();
    egui::IconData {
        width: image.width(),
        height: image.height(),
        rgba: image.into_raw(),
    }
}

pub fn paint(painter: &egui::Painter, rect: egui::Rect) {
    use egui::{Color32, CornerRadius, Rect, Stroke, StrokeKind, pos2, vec2};
    // Keep the geometry in sync with assets/rigometry.svg. Paint directly
    // at the current UI scale: the sidebar never downsamples the window PNG.
    let scale = rect.width() / 64.;
    let point = |x: f32, y: f32| rect.min + vec2(x, y) * scale;
    painter.rect_filled(
        rect,
        CornerRadius::same((14. * scale).round() as u8),
        Color32::BLACK,
    );
    painter.rect_stroke(
        Rect::from_min_max(point(9., 12.), point(55., 44.)),
        CornerRadius::same((5. * scale).round() as u8),
        Stroke::new(4. * scale, Color32::WHITE),
        StrokeKind::Middle,
    );
    for (points, width) in [
        (
            &[
                pos2(17., 28.),
                pos2(23., 28.),
                pos2(27., 21.),
                pos2(33., 35.),
                pos2(37., 28.),
                pos2(47., 28.),
            ][..],
            3.5,
        ),
        (&[pos2(32., 44.), pos2(32., 52.)][..], 4.),
        (&[pos2(23., 52.), pos2(41., 52.)][..], 4.),
    ] {
        let stroke = Stroke::new(width * scale, Color32::WHITE);
        let points: Vec<_> = points.iter().map(|p| point(p.x, p.y)).collect();
        // Rounded joins and terminals remain legible at small sidebar sizes.
        painter.add(egui::Shape::line(points.clone(), stroke));
        for p in points {
            painter.circle_filled(p, stroke.width / 2., stroke.color);
        }
    }
}
