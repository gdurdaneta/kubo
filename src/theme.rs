//! Paleta oscura densa: mucha fila visible, poco cromo.

use std::sync::Arc;

use egui::{Color32, CornerRadius, FontFamily, Stroke, Visuals};

pub const FONDO: Color32 = Color32::from_rgb(0x16, 0x18, 0x1d);
pub const PANEL: Color32 = Color32::from_rgb(0x1c, 0x1f, 0x26);
pub const PANEL_ALT: Color32 = Color32::from_rgb(0x22, 0x26, 0x2e);
pub const BORDE: Color32 = Color32::from_rgb(0x2e, 0x33, 0x3d);
pub const TEXTO: Color32 = Color32::from_rgb(0xdc, 0xe1, 0xe8);
pub const TEXTO_TENUE: Color32 = Color32::from_rgb(0x8b, 0x93, 0xa1);
pub const ACENTO: Color32 = Color32::from_rgb(0x3d, 0x90, 0xf0);
pub const OK: Color32 = Color32::from_rgb(0x4c, 0xc3, 0x8a);
pub const WARN: Color32 = Color32::from_rgb(0xe0, 0xa6, 0x3a);
pub const BAD: Color32 = Color32::from_rgb(0xe5, 0x63, 0x63);

/// Las fuentes que trae egui no cubren los símbolos geométricos (●, ▼, ×, ↻):
/// sin esto la UI se llena de cuadraditos. DejaVu sí los tiene y está en
/// cualquier distro; si falta, se sigue con las de egui.
fn cargar_fuentes(ctx: &egui::Context) {
    const PROPORCIONAL: &str = "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf";
    const MONO: &str = "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf";

    let mut fuentes = egui::FontDefinitions::default();
    let mut hubo_cambio = false;

    if let Ok(bytes) = std::fs::read(PROPORCIONAL) {
        fuentes
            .font_data
            .insert("sistema".to_owned(), Arc::new(egui::FontData::from_owned(bytes)));
        // Como fallback, no como primaria: la de egui se ve mejor para texto.
        for familia in [FontFamily::Proportional, FontFamily::Monospace] {
            fuentes.families.entry(familia).or_default().push("sistema".to_owned());
        }
        hubo_cambio = true;
    }

    if let Ok(bytes) = std::fs::read(MONO) {
        fuentes
            .font_data
            .insert("sistema_mono".to_owned(), Arc::new(egui::FontData::from_owned(bytes)));
        fuentes
            .families
            .entry(FontFamily::Monospace)
            .or_default()
            .push("sistema_mono".to_owned());
        hubo_cambio = true;
    }

    if hubo_cambio {
        ctx.set_fonts(fuentes);
    }
}

pub fn aplicar(ctx: &egui::Context) {
    cargar_fuentes(ctx);

    let mut v = Visuals::dark();
    v.panel_fill = FONDO;
    v.window_fill = PANEL;
    v.extreme_bg_color = Color32::from_rgb(0x11, 0x13, 0x17);
    v.faint_bg_color = PANEL_ALT;
    v.override_text_color = Some(TEXTO);
    v.selection.bg_fill = ACENTO.linear_multiply(0.35);
    v.selection.stroke = Stroke::new(1.0, ACENTO);
    v.hyperlink_color = ACENTO;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDE);
    v.widgets.inactive.bg_fill = PANEL_ALT;
    v.widgets.inactive.weak_bg_fill = PANEL_ALT;
    v.widgets.hovered.bg_fill = BORDE;
    v.widgets.hovered.weak_bg_fill = BORDE;
    v.widgets.active.bg_fill = ACENTO.linear_multiply(0.5);
    ctx.set_visuals(v);

    // La app fuerza tema oscuro, pero se ajustan ambos estilos para que un
    // cambio de tema del sistema no descoloque el espaciado.
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = egui::vec2(6.0, 4.0);
        s.spacing.button_padding = egui::vec2(8.0, 4.0);
        s.visuals.widgets.inactive.corner_radius = CornerRadius::same(4);
        s.visuals.widgets.hovered.corner_radius = CornerRadius::same(4);
        s.visuals.widgets.active.corner_radius = CornerRadius::same(4);
    });
}

pub fn color_tono(t: crate::columns::Tone) -> Color32 {
    match t {
        crate::columns::Tone::Normal => TEXTO,
        crate::columns::Tone::Ok => OK,
        crate::columns::Tone::Warn => WARN,
        crate::columns::Tone::Bad => BAD,
        crate::columns::Tone::Dim => TEXTO_TENUE,
    }
}
