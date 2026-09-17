//! Paleta oscura densa: mucha fila visible, poco cromo.

use std::sync::Arc;

use egui::{Color32, CornerRadius, FontFamily, Stroke, Visuals};

// Paleta tipo herramienta de escritorio (JetBrains New UI / Zed dark): poco
// contraste entre paneles, acento desaturado y raro, nada de cromo azul.
pub const FONDO: Color32 = Color32::from_rgb(0x12, 0x14, 0x1a);
pub const PANEL: Color32 = Color32::from_rgb(0x18, 0x1b, 0x22);
pub const PANEL_ALT: Color32 = Color32::from_rgb(0x1e, 0x22, 0x2b);
pub const BORDE: Color32 = Color32::from_rgb(0x2a, 0x2f, 0x3a);
/// Borde del panel activo cuando hay varios: se nota sin ser azul.
pub const BORDE_FUERTE: Color32 = Color32::from_rgb(0x3d, 0x45, 0x55);
pub const TEXTO: Color32 = Color32::from_rgb(0xd6, 0xda, 0xe3);
pub const TEXTO_TENUE: Color32 = Color32::from_rgb(0x8b, 0x91, 0x9e);
pub const ACENTO: Color32 = Color32::from_rgb(0x6b, 0x9f, 0xd4);
pub const OK: Color32 = Color32::from_rgb(0x6a, 0xab, 0x73);
pub const WARN: Color32 = Color32::from_rgb(0xc9, 0xa2, 0x27);
pub const BAD: Color32 = Color32::from_rgb(0xd1, 0x6b, 0x6b);
/// Fila o pestaña seleccionada.
pub const SELECCION: Color32 = Color32::from_rgb(0x2a, 0x3a, 0x4e);
/// Hover de widgets y filas.
pub const HOVER: Color32 = Color32::from_rgb(0x22, 0x26, 0x2f);
/// Campos de texto y otros huecos "hundidos".
pub const EXTREMO: Color32 = Color32::from_rgb(0x0e, 0x10, 0x14);
/// Fondo apenas teñido, para avisar sin gritar.
pub const BAD_TENUE: egui::Color32 = egui::Color32::from_rgb(58, 30, 34);

/// Las fuentes que trae egui no cubren los símbolos geométricos (●, ▼, ×, ↻):
/// sin esto la UI se llena de cuadraditos. DejaVu sí los tiene y está en
/// cualquier distro; si falta, se sigue con las de egui.
fn cargar_fuentes(ctx: &egui::Context) {
    let mut fuentes = egui::FontDefinitions::default();
    let mut hubo_cambio = false;

    // Fuente de interfaz como primaria: la de egui es de "página"; una de
    // sistema (Inter, Fira Sans, Noto, Segoe) hace que se lea como app nativa.
    if let Some(bytes) = crate::rutas::primera_existente(crate::rutas::FUENTES_UI) {
        fuentes
            .font_data
            .insert("ui".to_owned(), Arc::new(egui::FontData::from_owned(bytes)));
        fuentes
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .insert(0, "ui".to_owned());
        hubo_cambio = true;
    }
    if let Some(bytes) = crate::rutas::primera_existente(crate::rutas::FUENTES_MONO_UI) {
        fuentes.font_data.insert(
            "ui_mono".to_owned(),
            Arc::new(egui::FontData::from_owned(bytes)),
        );
        fuentes
            .families
            .entry(FontFamily::Monospace)
            .or_default()
            .insert(0, "ui_mono".to_owned());
        hubo_cambio = true;
    }

    if let Some(bytes) = crate::rutas::primera_existente(crate::rutas::FUENTES_PROPORCIONALES) {
        fuentes.font_data.insert(
            "sistema".to_owned(),
            Arc::new(egui::FontData::from_owned(bytes)),
        );
        // Como fallback, no como primaria: la de egui se ve mejor para texto.
        for familia in [FontFamily::Proportional, FontFamily::Monospace] {
            fuentes
                .families
                .entry(familia)
                .or_default()
                .push("sistema".to_owned());
        }
        hubo_cambio = true;
    }

    if let Some(bytes) = crate::rutas::primera_existente(crate::rutas::FUENTES_MONO) {
        fuentes.font_data.insert(
            "sistema_mono".to_owned(),
            Arc::new(egui::FontData::from_owned(bytes)),
        );
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
    v.extreme_bg_color = EXTREMO;
    v.faint_bg_color = PANEL_ALT;
    v.override_text_color = Some(TEXTO);
    v.selection.bg_fill = SELECCION;
    // El color del stroke es el del texto de lo seleccionado (filas de tabla,
    // botones): con NONE las celdas sin color propio desaparecían.
    v.selection.stroke = Stroke::new(1.0, TEXTO);
    v.hyperlink_color = ACENTO;
    v.window_stroke = Stroke::new(1.0, BORDE);
    v.window_shadow = egui::epaint::Shadow::NONE;
    v.popup_shadow = egui::epaint::Shadow::NONE;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDE);
    v.widgets.noninteractive.bg_fill = PANEL;
    // Botones y combos: plano con borde fino, sin relleno tipo chip.
    v.widgets.inactive.bg_fill = PANEL;
    v.widgets.inactive.weak_bg_fill = PANEL;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDE);
    v.widgets.hovered.bg_fill = HOVER;
    v.widgets.hovered.weak_bg_fill = HOVER;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, BORDE_FUERTE);
    v.widgets.active.bg_fill = SELECCION;
    v.widgets.active.weak_bg_fill = SELECCION;
    v.widgets.active.bg_stroke = Stroke::new(1.0, BORDE_FUERTE);
    v.widgets.open.bg_fill = SELECCION;
    v.widgets.open.weak_bg_fill = SELECCION;
    v.widgets.open.bg_stroke = Stroke::new(1.0, BORDE_FUERTE);
    let radio = CornerRadius::same(2);
    v.window_corner_radius = radio;
    v.menu_corner_radius = radio;
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = radio;
        w.expansion = 0.0;
    }
    ctx.set_visuals(v);

    // La app fuerza tema oscuro, pero se ajustan ambos estilos para que un
    // cambio de tema del sistema no descoloque el espaciado.
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = egui::vec2(5.0, 3.0);
        s.spacing.button_padding = egui::vec2(6.0, 2.0);
        s.spacing.interact_size.y = 18.0;
        s.spacing.menu_margin = egui::Margin::same(4);
        s.spacing.combo_width = 100.0;
        // Tamaños de herramienta, no de página.
        use egui::{FontFamily, FontId, TextStyle};
        s.text_styles = [
            (
                TextStyle::Small,
                FontId::new(11.0, FontFamily::Proportional),
            ),
            (TextStyle::Body, FontId::new(12.5, FontFamily::Proportional)),
            (
                TextStyle::Button,
                FontId::new(12.0, FontFamily::Proportional),
            ),
            (
                TextStyle::Heading,
                FontId::new(13.5, FontFamily::Proportional),
            ),
            (
                TextStyle::Monospace,
                FontId::new(12.0, FontFamily::Monospace),
            ),
        ]
        .into();
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
