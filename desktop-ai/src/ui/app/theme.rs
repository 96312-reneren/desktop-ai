//! Material Design 3 theme for the desktop UI.
//!
//! The egui ecosystem has no complete Material 3 component package, but the
//! Visuals/Style system is expressive enough for a faithful M3 theme. The
//! palette is the M3 purple baseline (dark + light), matching the Android
//! WebView UI so both platforms share one design language.

use eframe::egui::{self, Color32, CornerRadius, Margin, Stroke};

#[derive(Clone, Copy)]
#[allow(dead_code)] // full M3 role set; only some roles are used so far
pub(crate) struct M3 {
    pub primary: Color32,
    pub on_primary: Color32,
    pub primary_container: Color32,
    pub on_primary_container: Color32,
    pub secondary_container: Color32,
    pub on_secondary_container: Color32,
    pub tertiary_container: Color32,
    pub on_tertiary_container: Color32,
    pub surface: Color32,
    pub surface_container_lowest: Color32,
    pub surface_container_low: Color32,
    pub surface_container: Color32,
    pub surface_container_high: Color32,
    pub surface_container_highest: Color32,
    pub on_surface: Color32,
    pub on_surface_variant: Color32,
    pub outline: Color32,
    pub outline_variant: Color32,
    pub error: Color32,
    pub on_error: Color32,
    pub error_container: Color32,
    pub on_error_container: Color32,
    pub success: Color32,
}

pub(crate) const DARK: M3 = M3 {
    primary: Color32::from_rgb(0xD0, 0xBC, 0xFF),
    on_primary: Color32::from_rgb(0x38, 0x1E, 0x72),
    primary_container: Color32::from_rgb(0x4F, 0x37, 0x8B),
    on_primary_container: Color32::from_rgb(0xEA, 0xDD, 0xFF),
    secondary_container: Color32::from_rgb(0x4A, 0x44, 0x58),
    on_secondary_container: Color32::from_rgb(0xE8, 0xDE, 0xF8),
    tertiary_container: Color32::from_rgb(0x63, 0x3B, 0x48),
    on_tertiary_container: Color32::from_rgb(0xFF, 0xD8, 0xE4),
    surface: Color32::from_rgb(0x14, 0x12, 0x18),
    surface_container_lowest: Color32::from_rgb(0x0F, 0x0D, 0x13),
    surface_container_low: Color32::from_rgb(0x1D, 0x1B, 0x20),
    surface_container: Color32::from_rgb(0x21, 0x1F, 0x26),
    surface_container_high: Color32::from_rgb(0x2B, 0x29, 0x30),
    surface_container_highest: Color32::from_rgb(0x36, 0x34, 0x3B),
    on_surface: Color32::from_rgb(0xE6, 0xE0, 0xE9),
    on_surface_variant: Color32::from_rgb(0xCA, 0xC4, 0xD0),
    outline: Color32::from_rgb(0x93, 0x8F, 0x99),
    outline_variant: Color32::from_rgb(0x49, 0x45, 0x4F),
    error: Color32::from_rgb(0xF2, 0xB8, 0xB5),
    on_error: Color32::from_rgb(0x60, 0x14, 0x10),
    error_container: Color32::from_rgb(0x8C, 0x1D, 0x18),
    on_error_container: Color32::from_rgb(0xF9, 0xDE, 0xDC),
    success: Color32::from_rgb(0xB6, 0xCC, 0xB2),
};

pub(crate) const LIGHT: M3 = M3 {
    primary: Color32::from_rgb(0x67, 0x50, 0xA4),
    on_primary: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    primary_container: Color32::from_rgb(0xE9, 0xDD, 0xFF),
    on_primary_container: Color32::from_rgb(0x21, 0x00, 0x5E),
    secondary_container: Color32::from_rgb(0xE8, 0xDE, 0xF8),
    on_secondary_container: Color32::from_rgb(0x1D, 0x19, 0x24),
    tertiary_container: Color32::from_rgb(0xFF, 0xD8, 0xE4),
    on_tertiary_container: Color32::from_rgb(0x31, 0x11, 0x1D),
    surface: Color32::from_rgb(0xFB, 0xF8, 0xFF),
    surface_container_lowest: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    surface_container_low: Color32::from_rgb(0xF7, 0xF2, 0xFA),
    surface_container: Color32::from_rgb(0xF3, 0xED, 0xF7),
    surface_container_high: Color32::from_rgb(0xEC, 0xE6, 0xF0),
    surface_container_highest: Color32::from_rgb(0xE6, 0xE0, 0xE9),
    on_surface: Color32::from_rgb(0x1D, 0x1B, 0x20),
    on_surface_variant: Color32::from_rgb(0x49, 0x45, 0x4F),
    outline: Color32::from_rgb(0x7A, 0x75, 0x7F),
    outline_variant: Color32::from_rgb(0xCA, 0xC4, 0xD0),
    error: Color32::from_rgb(0xB3, 0x26, 0x1E),
    on_error: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    error_container: Color32::from_rgb(0xF9, 0xDE, 0xDC),
    on_error_container: Color32::from_rgb(0x41, 0x0E, 0x0B),
    success: Color32::from_rgb(0x3B, 0x69, 0x37),
};

pub(crate) fn palette(dark: bool) -> &'static M3 {
    if dark {
        &DARK
    } else {
        &LIGHT
    }
}

/// Apply the M3 theme: palette, shapes, elevation and spacing.
pub(crate) fn apply(ctx: &egui::Context, dark: bool) {
    let m = palette(dark);
    let mut v = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };

    v.panel_fill = m.surface;
    v.window_fill = m.surface_container;
    v.window_stroke = Stroke::new(1.0, m.outline_variant);
    v.window_corner_radius = CornerRadius::same(16);
    v.menu_corner_radius = CornerRadius::same(12);
    v.extreme_bg_color = m.surface_container_lowest;
    v.faint_bg_color = m.surface_container_low;
    v.selection.bg_fill = m.secondary_container;
    v.selection.stroke = Stroke::new(1.0, m.on_secondary_container);
    v.hyperlink_color = m.primary;
    v.error_fg_color = m.error;
    v.warn_fg_color = m.tertiary_container;
    v.override_text_color = None;

    {
        let w = &mut v.widgets;

        w.noninteractive.bg_fill = m.surface;
        w.noninteractive.weak_bg_fill = m.surface;
        w.noninteractive.bg_stroke = Stroke::new(1.0, m.outline_variant);
        w.noninteractive.fg_stroke = Stroke::new(1.0, m.on_surface);
        w.noninteractive.corner_radius = CornerRadius::same(12);
        w.noninteractive.expansion = 0.0;

        w.inactive.bg_fill = m.surface_container_highest;
        w.inactive.weak_bg_fill = m.surface_container_highest;
        w.inactive.bg_stroke = Stroke::NONE;
        w.inactive.fg_stroke = Stroke::new(1.0, m.on_surface);
        w.inactive.corner_radius = CornerRadius::same(20);
        w.inactive.expansion = 0.0;

        w.hovered.bg_fill = m
            .surface_container_highest
            .lerp_to_gamma(m.on_surface, 0.10);
        w.hovered.weak_bg_fill = w.hovered.bg_fill;
        w.hovered.bg_stroke = Stroke::NONE;
        w.hovered.fg_stroke = Stroke::new(1.0, m.on_surface);
        w.hovered.corner_radius = CornerRadius::same(20);
        w.hovered.expansion = 0.0;

        w.active.bg_fill = m
            .surface_container_highest
            .lerp_to_gamma(m.on_surface, 0.16);
        w.active.weak_bg_fill = w.active.bg_fill;
        w.active.bg_stroke = Stroke::NONE;
        w.active.fg_stroke = Stroke::new(1.0, m.on_surface);
        w.active.corner_radius = CornerRadius::same(20);
        w.active.expansion = 0.0;

        w.open.bg_fill = m.surface_container_highest;
        w.open.weak_bg_fill = m.surface_container_highest;
        w.open.bg_stroke = Stroke::new(1.0, m.outline_variant);
        w.open.fg_stroke = Stroke::new(1.0, m.on_surface);
        w.open.corner_radius = CornerRadius::same(12);
        w.open.expansion = 0.0;
    }

    ctx.set_visuals(v);

    let mut s = (*ctx.style()).clone();
    s.spacing.item_spacing = egui::vec2(8.0, 8.0);
    s.spacing.button_padding = egui::vec2(14.0, 7.0);
    s.spacing.interact_size.y = 30.0;
    s.spacing.menu_margin = Margin::same(8);
    s.spacing.window_margin = Margin::same(12);
    s.spacing.scroll.bar_width = 6.0;
    s.spacing.scroll.bar_inner_margin = 2.0;
    ctx.set_style(s);
}

/// User chat bubble: primary filled, asymmetric M3 radii (sharp tail top-right).
pub(crate) fn user_bubble(m: &M3) -> egui::Frame {
    egui::Frame::new()
        .fill(m.primary)
        .corner_radius(CornerRadius {
            nw: 18,
            ne: 6,
            sw: 18,
            se: 18,
        })
        .inner_margin(Margin {
            left: 14,
            right: 14,
            top: 8,
            bottom: 8,
        })
}

/// Assistant chat bubble: surface-container-high, tail top-left.
pub(crate) fn bot_bubble(m: &M3) -> egui::Frame {
    egui::Frame::new()
        .fill(m.surface_container_high)
        .corner_radius(CornerRadius {
            nw: 6,
            ne: 18,
            sw: 18,
            se: 18,
        })
        .inner_margin(Margin {
            left: 14,
            right: 14,
            top: 8,
            bottom: 8,
        })
}

/// M3 filled primary button (pill).
pub(crate) fn primary_button(text: egui::RichText, m: &M3) -> egui::Button<'static> {
    egui::Button::new(text.color(m.on_primary))
        .fill(m.primary)
        .corner_radius(CornerRadius::same(20))
        .stroke(Stroke::NONE)
}

/// M3 filled tonal button (secondary container).
pub(crate) fn tonal_button(text: egui::RichText, m: &M3) -> egui::Button<'static> {
    egui::Button::new(text.color(m.on_secondary_container))
        .fill(m.secondary_container)
        .corner_radius(CornerRadius::same(20))
        .stroke(Stroke::NONE)
}

/// M3 error button (stop generation).
pub(crate) fn error_button(text: egui::RichText, m: &M3) -> egui::Button<'static> {
    egui::Button::new(text.color(m.on_error_container))
        .fill(m.error_container)
        .corner_radius(CornerRadius::same(20))
        .stroke(Stroke::NONE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palettes_have_contrast() {
        // Sanity: text vs surface must differ substantially in luminance for
        // both themes (guards against accidental color swaps).
        for m in [&DARK, &LIGHT] {
            let text = m.on_surface.to_array();
            let surf = m.surface.to_array();
            let diff: i32 = text
                .iter()
                .zip(surf.iter())
                .map(|(a, b)| (*a as i32 - *b as i32).abs())
                .sum();
            assert!(diff > 200, "insufficient contrast: {:?}", diff);
        }
    }
}
