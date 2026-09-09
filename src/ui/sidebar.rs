//! Navegación por categorías, con subgrupos anidados y filtro rápido.

use std::collections::HashSet;

use egui::Atom;

use super::Accion;
use crate::app::App;
use crate::nav::{NavCategory, NavItem, VistaLocal};
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
    let fijados = items_fijados(&nav, &pane.favoritos);

    // Los clics se acumulan y se aplican al final: la nav está prestada
    // mientras se dibuja.
    let mut toggles: Vec<String> = Vec::new();
    let mut pin_toggles: Vec<String> = Vec::new();
    let mut sel: Option<crate::nav::NavItem> = None;
    let mut sel_vl: Option<VistaLocal> = None;

    egui::ScrollArea::vertical()
        .id_salt(("nav_scroll", id))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if !fijados.is_empty() {
                let favoritos = NavCategory {
                    name: "Fijados".to_string(),
                    icono: "pinned".to_string(),
                    items: fijados,
                    subgrupos: Vec::new(),
                    detalle: Some("recursos fijados por vos".to_string()),
                    extension: false,
                    locales: Vec::new(),
                };
                categoria(
                    ui,
                    &favoritos,
                    &favoritos.name,
                    0,
                    &filtro,
                    &pane.nav_cerradas,
                    &pane.favoritos,
                    sel_key.as_deref(),
                    sel_local,
                    &mut toggles,
                    &mut pin_toggles,
                    &mut sel,
                    &mut sel_vl,
                );
            }
            for cat in &nav {
                categoria(
                    ui,
                    cat,
                    &cat.name,
                    0,
                    &filtro,
                    &pane.nav_cerradas,
                    &pane.favoritos,
                    sel_key.as_deref(),
                    sel_local,
                    &mut toggles,
                    &mut pin_toggles,
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
    if let Some(key) = pin_toggles.pop() {
        app.alternar_favorito(id, &key);
    }
}

fn items_fijados(nav: &[NavCategory], favoritos: &HashSet<String>) -> Vec<NavItem> {
    fn visitar(cat: &NavCategory, favoritos: &HashSet<String>, out: &mut Vec<NavItem>) {
        out.extend(
            cat.items
                .iter()
                .filter(|item| favoritos.contains(&item.res.key()))
                .cloned(),
        );
        for sub in &cat.subgrupos {
            visitar(sub, favoritos, out);
        }
    }

    let mut out = Vec::new();
    for cat in nav {
        visitar(cat, favoritos, &mut out);
    }
    out.sort_by(|a, b| a.label.cmp(&b.label));
    out
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
    favoritos: &HashSet<String>,
    sel: Option<&str>,
    sel_local: Option<VistaLocal>,
    toggles: &mut Vec<String>,
    pin_toggles: &mut Vec<String>,
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
            let (resp, pin) = fila_recurso(ui, item, activo, favoritos.contains(&key));
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
            if pin {
                pin_toggles.push(key);
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
                favoritos,
                sel,
                sel_local,
                toggles,
                pin_toggles,
                sel_out,
                sel_vl,
            );
        }
    });

    if nivel == 0 {
        ui.add_space(6.0);
    }
}

fn fila_recurso(
    ui: &mut egui::Ui,
    item: &NavItem,
    activo: bool,
    fijado: bool,
) -> (egui::Response, bool) {
    let ancho = (ui.available_width() - 24.0).max(40.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        // (texto, grow): la etiqueta queda a la izquierda y el hueco elástico
        // llena el resto — sin esto el botón la centra.
        // Reserva el ancho del glifo (12 px) más un margen visual; el texto
        // nunca debe tocar ni superponerse al icono.
        let etiqueta = format!("     {}", item.label);
        let fila = ui.add_sized(
            [ancho, ALTO_FILA],
            egui::Button::selectable(activo, (etiqueta, Atom::grow())).truncate(),
        );
        pintar_icono_recurso(
            ui,
            &item.res.ar.kind,
            egui::pos2(fila.rect.left() + 12.0, fila.rect.center().y),
            if activo {
                theme::ACENTO
            } else {
                theme::TEXTO_TENUE
            },
        );
        let pin = ui
            .add_sized(
                [20.0, ALTO_FILA],
                egui::Button::new(if fijado { "★" } else { "☆" }).frame(false),
            )
            .on_hover_text(if fijado {
                "Quitar de fijados"
            } else {
                "Fijar arriba"
            });
        (fila, pin.clicked())
    })
    .inner
}

/// Glifos compactos inspirados en la iconografía de recursos Kubernetes.
/// A 16 px se simplifican deliberadamente: los SVG oficiales están pensados
/// para diagramas grandes y sus detalles se pierden en una fila de 24 px.
fn pintar_icono_recurso(ui: &egui::Ui, kind: &str, c: egui::Pos2, color: egui::Color32) {
    let p = ui.painter();
    let stroke = egui::Stroke::new(1.15, color);
    let linea = |a: (f32, f32), b: (f32, f32)| {
        p.line_segment([c + egui::vec2(a.0, a.1), c + egui::vec2(b.0, b.1)], stroke);
    };
    let punto = |x: f32, y: f32| p.circle_filled(c + egui::vec2(x, y), 1.5, color);
    let caja = |x: f32, y: f32, w: f32, h: f32| {
        p.rect_filled(
            egui::Rect::from_center_size(c + egui::vec2(x, y), egui::vec2(w, h)),
            1.0,
            color,
        );
    };
    let hexagono = || {
        let puntos = [
            (-3.0, -5.0),
            (3.0, -5.0),
            (6.0, 0.0),
            (3.0, 5.0),
            (-3.0, 5.0),
            (-6.0, 0.0),
        ];
        for i in 0..puntos.len() {
            linea(puntos[i], puntos[(i + 1) % puntos.len()]);
        }
    };

    match kind {
        "Pod" => {
            hexagono();
            punto(0.0, 0.0);
        }
        "Deployment" | "ReplicaSet" | "ReplicationController" => {
            caja(-2.5, -2.5, 6.0, 6.0);
            p.rect_stroke(
                egui::Rect::from_center_size(c + egui::vec2(2.5, 2.5), egui::vec2(6.0, 6.0)),
                1.0,
                stroke,
                egui::StrokeKind::Inside,
            );
        }
        "DaemonSet" => {
            for (x, y) in [(-3.0, -3.0), (3.0, -3.0), (-3.0, 3.0), (3.0, 3.0)] {
                punto(x, y);
            }
        }
        "StatefulSet" => {
            for y in [-4.0, 0.0, 4.0] {
                punto(-4.5, y);
                linea((-1.5, y), (5.5, y));
            }
        }
        "Job" | "CronJob" => {
            p.circle_stroke(c, 5.0, stroke);
            linea((0.0, 0.0), (0.0, -3.5));
            linea((0.0, 0.0), (3.0, 1.5));
        }
        "Service" | "Endpoints" | "EndpointSlice" => {
            for (x, y) in [(-4.5, 3.5), (0.0, -4.5), (4.5, 3.5)] {
                linea((0.0, 0.0), (x, y));
                punto(x, y);
            }
            punto(0.0, 0.0);
        }
        "Ingress" | "IngressClass" | "Gateway" => {
            linea((-6.0, 0.0), (3.5, 0.0));
            linea((0.5, -3.0), (3.5, 0.0));
            linea((0.5, 3.0), (3.5, 0.0));
            linea((5.5, -5.0), (5.5, 5.0));
        }
        "NetworkPolicy" => {
            hexagono();
            linea((-3.0, 0.0), (3.0, 0.0));
        }
        "ConfigMap" | "ResourceQuota" | "LimitRange" => {
            linea((-4.5, -5.0), (2.0, -5.0));
            linea((2.0, -5.0), (5.0, -2.0));
            linea((5.0, -2.0), (5.0, 5.0));
            linea((5.0, 5.0), (-4.5, 5.0));
            linea((-4.5, 5.0), (-4.5, -5.0));
            linea((-2.5, 0.0), (2.5, 0.0));
            linea((-2.5, 3.0), (2.5, 3.0));
        }
        "Secret" | "ExternalSecret" | "SecretStore" | "ClusterSecretStore" => {
            p.circle_stroke(c + egui::vec2(-2.5, -1.5), 3.0, stroke);
            linea((0.0, 0.0), (5.5, 4.5));
            linea((3.0, 2.5), (4.5, 1.0));
        }
        "PersistentVolume" | "PersistentVolumeClaim" | "StorageClass" => {
            p.circle_stroke(c + egui::vec2(0.0, -3.5), 4.5, stroke);
            linea((-4.5, -3.5), (-4.5, 3.5));
            linea((4.5, -3.5), (4.5, 3.5));
            p.circle_stroke(c + egui::vec2(0.0, 3.5), 4.5, stroke);
        }
        "Node" => {
            hexagono();
            for (x, y) in [(-2.5, -2.0), (2.5, -2.0), (-2.5, 2.0), (2.5, 2.0)] {
                punto(x, y);
            }
        }
        "Namespace" => {
            linea((-5.0, -5.0), (-5.0, 5.0));
            linea((-5.0, -5.0), (-2.0, -5.0));
            linea((-5.0, 5.0), (-2.0, 5.0));
            linea((5.0, -5.0), (5.0, 5.0));
            linea((2.0, -5.0), (5.0, -5.0));
            linea((2.0, 5.0), (5.0, 5.0));
        }
        "Event" => {
            linea((1.0, -6.0), (-3.0, 0.5));
            linea((-3.0, 0.5), (0.5, 0.5));
            linea((0.5, 0.5), (-1.0, 6.0));
            linea((-1.0, 6.0), (4.0, -1.5));
            linea((4.0, -1.5), (1.0, -1.5));
        }
        "ServiceAccount" | "Role" | "RoleBinding" | "ClusterRole" | "ClusterRoleBinding" => {
            hexagono();
            punto(0.0, -1.5);
            linea((-2.5, 3.0), (2.5, 3.0));
        }
        "HorizontalPodAutoscaler" | "ScaledObject" | "ScaledJob" => {
            linea((-5.5, 0.0), (5.5, 0.0));
            linea((-5.5, 0.0), (-2.5, -3.0));
            linea((-5.5, 0.0), (-2.5, 3.0));
            linea((5.5, 0.0), (2.5, -3.0));
            linea((5.5, 0.0), (2.5, 3.0));
        }
        _ => {
            linea((0.0, -5.5), (5.5, 0.0));
            linea((5.5, 0.0), (0.0, 5.5));
            linea((0.0, 5.5), (-5.5, 0.0));
            linea((-5.5, 0.0), (0.0, -5.5));
            punto(0.0, 0.0);
        }
    }
}

/// Encabezado clickeable. El nivel 0 va en versalitas grises; los subgrupos
/// (Gateway API dentro de Network) se ven como un ítem con chevron.
fn cabecera(ui: &mut egui::Ui, cat: &NavCategory, abierta: bool, nivel: usize) -> egui::Response {
    let chevron = if abierta { "▼" } else { "▶" };
    let (texto, color, tamaño) = if nivel == 0 {
        (
            format!("{chevron}      {}", cat.name.to_uppercase()),
            if cat.icono == "pinned" {
                theme::WARN
            } else if cat.extension {
                theme::ACENTO
            } else {
                theme::TEXTO_TENUE
            },
            11.0,
        )
    } else {
        (
            format!("{chevron}      {}", cat.name),
            if cat.extension {
                theme::ACENTO
            } else {
                theme::TEXTO
            },
            12.5,
        )
    };

    let resp = ui.add_sized(
        [ui.available_width(), ALTO_CABECERA],
        egui::Label::new(egui::RichText::new(texto).size(tamaño).color(color))
            .truncate()
            .sense(egui::Sense::click()),
    );
    pintar_icono(
        ui,
        &cat.icono,
        egui::pos2(resp.rect.left() + 25.0, resp.rect.center().y),
        color,
    );
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

/// Iconos vectoriales: no dependen de que la fuente instalada tenga glifos
/// especiales y conservan el mismo grosor y tamaño en todas las plataformas.
fn pintar_icono(ui: &egui::Ui, tipo: &str, c: egui::Pos2, color: egui::Color32) {
    let p = ui.painter();
    let stroke = egui::Stroke::new(1.25, color);
    let linea = |a: (f32, f32), b: (f32, f32)| {
        p.line_segment([c + egui::vec2(a.0, a.1), c + egui::vec2(b.0, b.1)], stroke);
    };
    let punto = |x: f32, y: f32| p.circle_filled(c + egui::vec2(x, y), 1.8, color);

    match tipo {
        "cluster" => {
            for (x, y) in [(-4.5, -3.5), (4.5, -3.5), (-4.5, 3.5), (4.5, 3.5)] {
                linea((0.0, 0.0), (x, y));
                punto(x, y);
            }
            punto(0.0, 0.0);
        }
        "workloads" => {
            for (x, y) in [(-3.5, -3.5), (3.5, -3.5), (-3.5, 3.5), (3.5, 3.5)] {
                p.rect_filled(
                    egui::Rect::from_center_size(c + egui::vec2(x, y), egui::vec2(5.0, 5.0)),
                    1.0,
                    color,
                );
            }
        }
        "config" => {
            for (y, x) in [(-4.0, -2.0), (0.0, 3.0), (4.0, -4.0)] {
                linea((-6.0, y), (6.0, y));
                p.circle_filled(c + egui::vec2(x, y), 2.0, theme::PANEL);
                p.circle_stroke(c + egui::vec2(x, y), 2.0, stroke);
            }
        }
        "network" => {
            linea((-4.5, 4.0), (0.0, -4.5));
            linea((0.0, -4.5), (4.5, 4.0));
            linea((-4.5, 4.0), (4.5, 4.0));
            punto(0.0, -4.5);
            punto(-4.5, 4.0);
            punto(4.5, 4.0);
        }
        "storage" => {
            for y in [-4.0, 0.0, 4.0] {
                p.rect_filled(
                    egui::Rect::from_center_size(c + egui::vec2(0.0, y), egui::vec2(12.0, 2.5)),
                    1.25,
                    color,
                );
            }
        }
        "access" => {
            let puntos = [
                c + egui::vec2(0.0, -6.0),
                c + egui::vec2(5.0, -3.5),
                c + egui::vec2(4.0, 3.0),
                c + egui::vec2(0.0, 6.0),
                c + egui::vec2(-4.0, 3.0),
                c + egui::vec2(-5.0, -3.5),
            ];
            for i in 0..puntos.len() {
                p.line_segment([puntos[i], puntos[(i + 1) % puntos.len()]], stroke);
            }
            punto(0.0, 0.0);
        }
        "gateway" => {
            linea((-6.0, -3.5), (5.0, -3.5));
            linea((2.0, -6.0), (5.0, -3.5));
            linea((2.0, -1.0), (5.0, -3.5));
            linea((6.0, 3.5), (-5.0, 3.5));
            linea((-2.0, 1.0), (-5.0, 3.5));
            linea((-2.0, 6.0), (-5.0, 3.5));
        }
        "pinned" => {
            p.circle_stroke(c + egui::vec2(0.0, -2.5), 3.5, stroke);
            linea((0.0, 1.0), (0.0, 6.0));
            linea((-2.0, 5.0), (2.0, 5.0));
        }
        _ => {
            linea((0.0, -5.5), (5.5, 0.0));
            linea((5.5, 0.0), (0.0, 5.5));
            linea((0.0, 5.5), (-5.5, 0.0));
            linea((-5.5, 0.0), (0.0, -5.5));
        }
    }
}
