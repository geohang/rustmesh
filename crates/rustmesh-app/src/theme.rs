//! Shared visual language for the native and browser editors.
use eframe::egui::{self, Color32, FontId, Stroke, TextStyle};

pub const ACCENT: Color32 = Color32::from_rgb(0, 113, 227);

pub fn install(ctx: &egui::Context) {
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        ctx.style_mut_of(theme, |style| {
            let dark = theme == egui::Theme::Dark;
            let paper = if dark {
                Color32::from_rgb(28, 28, 30)
            } else {
                Color32::WHITE
            };
            let surface = if dark {
                Color32::from_rgb(36, 36, 38)
            } else {
                Color32::from_rgb(245, 245, 247)
            };
            let line = if dark {
                Color32::from_rgb(60, 60, 64)
            } else {
                Color32::from_rgb(222, 222, 227)
            };
            let ink = if dark {
                Color32::from_rgb(245, 245, 247)
            } else {
                Color32::from_rgb(29, 29, 31)
            };
            style
                .text_styles
                .insert(TextStyle::Heading, FontId::proportional(23.0));
            style
                .text_styles
                .insert(TextStyle::Body, FontId::proportional(14.0));
            style
                .text_styles
                .insert(TextStyle::Button, FontId::proportional(13.0));
            style
                .text_styles
                .insert(TextStyle::Small, FontId::proportional(11.5));
            style.spacing.item_spacing = egui::vec2(8.0, 10.0);
            style.spacing.button_padding = egui::vec2(12.0, 7.0);
            style.spacing.interact_size.y = 30.0;
            style.spacing.slider_width = 95.0;
            style.spacing.text_edit_width = 180.0;
            let v = &mut style.visuals;
            v.override_text_color = None;
            v.weak_text_color = Some(if dark {
                Color32::from_rgb(165, 165, 174)
            } else {
                Color32::from_rgb(110, 110, 115)
            });
            v.panel_fill = surface;
            v.window_fill = paper;
            v.extreme_bg_color = paper;
            v.text_edit_bg_color = Some(paper);
            v.faint_bg_color = surface;
            v.selection.bg_fill = ACCENT;
            v.slider_trailing_fill = true;
            v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
            v.hyperlink_color = ACCENT;
            v.window_corner_radius = 12.into();
            v.menu_corner_radius = 10.into();
            v.window_stroke = Stroke::new(1.0, line);
            v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, line);
            v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, ink);
            for w in [
                &mut v.widgets.inactive,
                &mut v.widgets.hovered,
                &mut v.widgets.active,
                &mut v.widgets.open,
            ] {
                w.corner_radius = 7.into();
                w.bg_stroke = Stroke::new(1.0, line);
                w.fg_stroke = Stroke::new(1.0, ink);
            }
            v.widgets.inactive.bg_fill = line;
            v.widgets.inactive.weak_bg_fill = paper;
            v.widgets.hovered.bg_fill = if dark {
                Color32::from_rgb(55, 55, 61)
            } else {
                Color32::from_rgb(232, 239, 249)
            };
            v.widgets.hovered.weak_bg_fill = v.widgets.hovered.bg_fill;
            v.widgets.active.bg_fill = if dark {
                Color32::from_rgb(46, 65, 89)
            } else {
                Color32::from_rgb(218, 233, 251)
            };
            v.widgets.active.weak_bg_fill = v.widgets.active.bg_fill;
        });
    }
}

pub fn card(ui: &egui::Ui) -> egui::Frame {
    egui::Frame::new()
        .fill(ui.visuals().window_fill)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(12)
        .inner_margin(14)
}
