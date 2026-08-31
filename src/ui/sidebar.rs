//! Navegación por categorías, con subgrupos anidados y filtro rápido.

use std::collections::HashSet;

use egui::Atom;

use super::Accion;
use crate::app::App;
use crate::nav::{NavCategory, VistaLocal};
use crate::theme;

const ALTO_FILA: f32 = 24.0;
const ALTO_CABECERA: f32 = 22.0;

pub fn dibujar(app: &mut App, ui: &mut egui::Ui, id: u64, accion: &mut Accion) {
    let nav = {
        let Some(pane) = app.panes.iter().find(|p| p.id == id) else {
            return;
        };
        match app.cluster_de(pane) {
            Some(c) => c.nav.clone(),
            None => return,
        }
    };
    let Some(pane) = app.panes.iter_mut().find(|p| p.id == id) else {
        return;
    };

    ui.add_space(2.0);
    ui.add(
        egui::TextEdit::singleline(&mut pane.nav_filtro)
            .hint_text("filtrar recursos")
            .desired_width(f32::INFINITY),
    );
    ui.add_space(4.0);

    let filtro = pane.nav_filtro.to_lowercase();
    // Con una vista local abierta el recurso deja de estar resaltado: lo que se
    // ve en el panel es la vista local.
    let sel_key = if pane.vista_local.is_some() {
        None
    } else {
        pane.item.as_ref().map(|i| i.res.key())
    };
    let sel_local = pane.vista_local;

    // Los clics se acumulan y se aplican al final: la nav está prestada
    // mientras se dibuja.
    let mut toggles: Vec<String> = Vec::new();
    let mut sel: Option<crate::nav::NavItem> = None;
    let mut sel_vl: Option<VistaLocal> = None;

    egui::ScrollArea::vertical()
        .id_salt(("nav_scroll", id))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for cat in &nav {
                categoria(
                    ui,
                    cat,
                    &cat.name,
                    0,
                    &filtro,
                    &pane.nav_cerradas,
                    sel_key.as_deref(),
                    sel_local,
                    &mut toggles,
                    &mut sel,
                    &mut sel_vl,
                );
            }
            ui.add_space(8.0);
        });

    // Plegar/desplegar también se aplica después de dibujar: hace falta otro
    // frame para que se vea.
    if !toggles.is_empty() {
        ui.ctx().request_repaint();
    }
    for t in toggles {
        if !pane.nav_cerradas.remove(&t) {
            pane.nav_cerradas.insert(t);
        }
    }
    if let Some(item) = sel {
        *accion = Accion::Seleccionar(id, item);
    } else if let Some(v) = sel_vl {
        *accion = Accion::VerVistaLocal(id, v);
    }
}

/// ¿Queda algo visible en esta rama con el filtro puesto?
fn hay_coincidencias(cat: &NavCategory, filtro: &str) -> bool {
    if filtro.is_empty() {
        return cat.total() > 0;
    }
    cat.items
        .iter()
        .any(|i| i.label.to_lowercase().contains(filtro))
        || cat
            .locales
            .iter()
            .any(|v| v.label().to_lowercase().contains(filtro))
        || cat.name.to_lowercase().contains(filtro)
        || cat.subgrupos.iter().any(|s| hay_coincidencias(s, filtro))
}

#[allow(clippy::too_many_arguments)]
fn categoria(
    ui: &mut egui::Ui,
    cat: &NavCategory,
    ruta: &str,
    nivel: usize,
    filtro: &str,
    cerradas: &HashSet<String>,
    sel: Option<&str>,
    sel_local: Option<VistaLocal>,
    toggles: &mut Vec<String>,
    sel_out: &mut Option<crate::nav::NavItem>,
    sel_vl: &mut Option<VistaLocal>,
) {
    if !hay_coincidencias(cat, filtro) {
        return;
    }

    // Con filtro activo se despliega todo: buscar y tener que abrir la
    // categoría a mano sería absurdo.
    let abierta = !cerradas.contains(ruta) || !filtro.is_empty();
    let resp = cabecera(ui, cat, abierta, nivel);
    if resp.clicked() {
        toggles.push(ruta.to_string());
    }
    if let Some(d) = &cat.detalle {
        resp.on_hover_text(format!("detectado por: {d}"));
    }

    if !abierta {
        return;
    }

    ui.indent(ruta, |ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        for item in &cat.items {
            if !filtro.is_empty()
                && !item.label.to_lowercase().contains(filtro)
                && !cat.name.to_lowercase().contains(filtro)
            {
                continue;
            }
            let key = item.res.key();
            let activo = sel == Some(key.as_str());
            // (texto, grow): el texto queda a la izquierda y el hueco
            // elástico llena el resto — sin esto el botón centra la etiqueta.
            let resp = ui.add_sized(
                [ui.available_width(), ALTO_FILA],
                egui::Button::selectable(activo, (item.label.as_str(), Atom::grow()))
                    .truncate(),
            );
            if activo {
                // Barra de acento a la izquierda, como en Lens.
                let r = resp.rect;
                ui.painter().rect_filled(
                    egui::Rect::from_min_size(r.min, egui::vec2(3.0, r.height())),
                    egui::CornerRadius::same(2),
                    theme::ACENTO,
                );
            }
            if resp.clicked() && !activo {
                *sel_out = Some(item.clone());
            }
            resp.on_hover_text(format!(
                "{}\n{}",
                item.res.ar.api_version,
                if item.res.namespaced {
                    "namespaced"
                } else {
                    "cluster-scoped"
                }
            ));
        }

        for v in &cat.locales {
            let label = v.label();
            if !filtro.is_empty()
                && !label.to_lowercase().contains(filtro)
                && !cat.name.to_lowercase().contains(filtro)
            {
                continue;
            }
            let activo = sel_local == Some(*v);
            let resp = ui.add_sized(
                [ui.available_width(), ALTO_FILA],
                egui::Button::selectable(activo, (label, Atom::grow())).truncate(),
            );
            if activo {
                let r = resp.rect;
                ui.painter().rect_filled(
                    egui::Rect::from_min_size(r.min, egui::vec2(3.0, r.height())),
                    egui::CornerRadius::same(2),
                    theme::ACENTO,
                );
            }
            if resp.clicked() && !activo {
                *sel_vl = Some(*v);
            }
            resp.on_hover_text("túneles locales a Services (estado de kubo, no del cluster)");
        }

        for sub in &cat.subgrupos {
            let ruta_sub = format!("{ruta}/{}", sub.name);
            categoria(
                ui,
                sub,
                &ruta_sub,
                nivel + 1,
                filtro,
                cerradas,
                sel,
                sel_local,
                toggles,
                sel_out,
                sel_vl,
            );
        }
    });

    if nivel == 0 {
        ui.add_space(6.0);
    }
}

/// Encabezado clickeable. El nivel 0 va en versalitas grises; los subgrupos
/// (Gateway API dentro de Network) se ven como un ítem con chevron.
fn cabecera(
    ui: &mut egui::Ui,
    cat: &NavCategory,
    abierta: bool,
    nivel: usize,
) -> egui::Response {
    let chevron = if abierta { "▼" } else { "▶" };
    let (texto, color, tamaño) = if nivel == 0 {
        (
            format!("{chevron} {} {}", cat.icono, cat.name.to_uppercase()),
            if cat.extension {
                theme::ACENTO
            } else {
                theme::TEXTO_TENUE
            },
            11.0,
        )
    } else {
        (
            format!("{chevron} {} {}", cat.icono, cat.name),
            theme::TEXTO,
            12.5,
        )
    };

    let resp = ui.add_sized(
        [ui.available_width(), ALTO_CABECERA],
        egui::Label::new(egui::RichText::new(texto).size(tamaño).color(color))
            .truncate()
            .sense(egui::Sense::click()),
    );
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}
