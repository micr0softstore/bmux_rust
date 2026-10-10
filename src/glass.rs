//! Portable frosted-glass styling. This paints the material, not compositor blur.
use eframe::egui::{self, Color32, Painter, Pos2, Rect, Shadow, Stroke, StrokeKind};

use crate::settings::ThemeColors;

fn opaque(color: Color32) -> Color32 {
    let [r, g, b, _] = color.to_srgba_unmultiplied();
    Color32::from_rgb(r, g, b)
}

fn mix(a: Color32, b: Color32, amount: f32) -> Color32 {
    Color32::from_rgb(
        egui::lerp(a.r() as f32..=b.r() as f32, amount) as u8,
        egui::lerp(a.g() as f32..=b.g() as f32, amount) as u8,
        egui::lerp(a.b() as f32..=b.b() as f32, amount) as u8,
    )
}

/// Read the unpremultiplied theme, so opacity never changes light/dark styling.
pub fn is_dark(background: Color32) -> bool {
    let color = opaque(background);
    (color.r() as u32 * 2126 + color.g() as u32 * 7152 + color.b() as u32 * 722) < 128 * 10000
}

pub fn style_visuals(visuals: &mut egui::Visuals, colors: &ThemeColors) {
    let background = opaque(colors.background);
    let foreground = opaque(colors.foreground);
    let dark = is_dark(background);
    let border = if dark {
        Color32::from_white_alpha(37)
    } else {
        Color32::from_black_alpha(34)
    };
    // Popups remain nearly opaque so terminal text beneath them stays quiet.
    visuals.window_fill = mix(background, foreground, 0.075).gamma_multiply(0.97);
    visuals.window_stroke = Stroke::new(1.0_f32, border);
    visuals.window_corner_radius = egui::CornerRadius::same(14);
    visuals.menu_corner_radius = egui::CornerRadius::same(12);
    visuals.window_shadow = Shadow {
        offset: [0, 10],
        blur: 30,
        spread: 1,
        color: Color32::from_black_alpha(if dark { 100 } else { 42 }),
    };
    visuals.popup_shadow = Shadow {
        offset: [0, 6],
        blur: 22,
        spread: 0,
        color: Color32::from_black_alpha(if dark { 90 } else { 38 }),
    };
    visuals.extreme_bg_color = mix(background, foreground, 0.025);
    visuals.faint_bg_color = foreground.gamma_multiply(0.035);
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, border);
    visuals.widgets.inactive.bg_fill = mix(background, foreground, 0.06);
    visuals.widgets.inactive.weak_bg_fill = foreground.gamma_multiply(0.04);
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, border.gamma_multiply(0.6));
    visuals.widgets.hovered.bg_fill = mix(background, foreground, 0.13);
    visuals.widgets.hovered.weak_bg_fill = foreground.gamma_multiply(0.10);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, border);
    visuals.widgets.active.bg_fill = mix(background, foreground, 0.18);
    visuals.widgets.active.weak_bg_fill = foreground.gamma_multiply(0.15);
    visuals.widgets.open.bg_fill = mix(background, foreground, 0.11);
    visuals.widgets.open.weak_bg_fill = foreground.gamma_multiply(0.09);
    for widget in [
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.corner_radius = egui::CornerRadius::same(7);
        widget.fg_stroke.color = foreground;
    }
}

/// Paint a fixed rectangle without allocating layout or hit-test space.
pub fn paint_surface(
    painter: &Painter,
    rect: Rect,
    colors: &ThemeColors,
    radius: u8,
    elevated: bool,
) {
    if !rect.is_positive() || !painter.is_visible() {
        return;
    }
    let background = opaque(colors.background);
    let dark = is_dark(background);
    let radius = (radius as f32)
        .min(rect.height() / 2.0)
        .min(rect.width() / 2.0);
    let rounding = egui::CornerRadius::same(radius as u8);
    if elevated {
        painter.add(
            Shadow {
                offset: [0, 3],
                blur: 12,
                spread: 0,
                color: Color32::from_black_alpha(if dark { 65 } else { 24 }),
            }
            .as_shape(rect, rounding),
        );
    }
    let alpha = (colors.background.a() as f32 / 255.0 * 0.94).max(0.80);
    let top = mix(background, Color32::WHITE, if dark { 0.105 } else { 0.60 });
    let bottom = mix(background, Color32::WHITE, if dark { 0.028 } else { 0.20 });
    let color_at = |point: Pos2| {
        let y = ((point.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
        mix(top, bottom, y).gamma_multiply(alpha)
    };
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.center(), color_at(rect.center()));
    // A rounded convex fan interpolates the tint without a texture or shader.
    let corners = [
        (
            Pos2::new(rect.right() - radius, rect.top() + radius),
            -90.0_f32,
        ),
        (
            Pos2::new(rect.right() - radius, rect.bottom() - radius),
            0.0,
        ),
        (
            Pos2::new(rect.left() + radius, rect.bottom() - radius),
            90.0,
        ),
        (Pos2::new(rect.left() + radius, rect.top() + radius), 180.0),
    ];
    for (center, start) in corners {
        for step in 0..=8 {
            let angle = (start + step as f32 * 90.0 / 8.0).to_radians();
            let point = center + egui::vec2(angle.cos(), angle.sin()) * radius;
            mesh.colored_vertex(point, color_at(point));
        }
    }
    let count = mesh.vertices.len() as u32;
    for index in 1..count {
        mesh.add_triangle(0, index, if index + 1 == count { 1 } else { index + 1 });
    }
    painter.add(mesh);
    if radius > 0.0 {
        painter.rect_stroke(
            rect.shrink(0.5),
            rounding,
            Stroke::new(
                1.0_f32,
                if dark {
                    Color32::from_white_alpha(34)
                } else {
                    Color32::from_black_alpha(25)
                },
            ),
            StrokeKind::Inside,
        );
        painter.hline(
            rect.left() + radius..=rect.right() - radius,
            rect.top() + 1.0,
            Stroke::new(
                1.0_f32,
                Color32::from_white_alpha(if dark { 38 } else { 180 }),
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transparent_light_themes_keep_light_widgets_and_readable_popups() {
        let mut colors = crate::settings::Settings::default().colors(false);
        colors.background = colors.background.gamma_multiply(0.3);
        assert!(!is_dark(colors.background));
        let mut visuals = egui::Visuals::light();
        style_visuals(&mut visuals, &colors);
        assert!(!visuals.dark_mode);
        assert!(visuals.window_fill.a() >= 240);
        assert!(!is_dark(visuals.window_fill));
    }
}
