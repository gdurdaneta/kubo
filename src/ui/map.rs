//! Mapa de un Service: Ingress → Service → workloads → pods, dibujado con el
//! painter. Sin librería de grafos: tres columnas y curvas Bézier.

use egui::{Color32, CornerRadius, FontId, Pos2, Rect, Stroke, StrokeKind};

use crate::k8s::mapa::MapaData;
use crate::theme;

const ALTO_NODO: f32 = 46.0;
const ANCHO_NODO: f32 = 170.0;

pub fn dibujar(ui: &mut egui::Ui, data: &MapaData) {
    if let Some(err) = &data.error {
        ui.colored_label(theme::BAD, err);
        return;
    }

    // Altura total: el mayor de los lados manda.
    let n_izq = data.ingresses.len().max(1);
    let alto_wl: f32 = data
        .workloads
        .iter()
        .map(|w| alto_workload(w.pods.len()))
        .sum::<f32>()
        + (data.workloads.len().saturating_sub(1)) as f32 * 12.0;
    let alto = (n_izq as f32 * (ALTO_NODO + 12.0))
        .max(alto_wl)
        .max(ALTO_NODO + 20.0)
        + 40.0;

    let ancho = ui.available_width().max(560.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ancho, alto), egui::Sense::hover());
    let p = ui.painter_at(rect);

    let x_izq = rect.left() + 4.0;
    let x_centro = rect.left() + (ancho - ANCHO_NODO) / 2.0;
    let x_der = rect.right() - ANCHO_NODO - 4.0;
    let cy = rect.center().y;

    // --- service en el centro --------------------------------------------
    let svc_rect = Rect::from_min_size(
        Pos2::new(x_centro, cy - ALTO_NODO / 2.0 - 8.0),
        egui::vec2(ANCHO_NODO, ALTO_NODO + 16.0),
    );
    nodo(&p, svc_rect, theme::ACENTO);
    p.text(
        svc_rect.center_top() + egui::vec2(0.0, 10.0),
        egui::Align2::CENTER_TOP,
        &data.service,
        FontId::proportional(12.5),
        theme::TEXTO,
    );
    p.text(
        svc_rect.center_top() + egui::vec2(0.0, 26.0),
        egui::Align2::CENTER_TOP,
        format!("{}  {}", data.tipo, data.cluster_ip),
        FontId::proportional(10.5),
        theme::TEXTO_TENUE,
    );
    for (i, puerto) in data.puertos.iter().take(2).enumerate() {
        p.text(
            svc_rect.center_top() + egui::vec2(0.0, 40.0 + i as f32 * 12.0),
            egui::Align2::CENTER_TOP,
            puerto,
            FontId::monospace(9.5),
            theme::TEXTO_TENUE,
        );
    }

    // --- ingresses a la izquierda ----------------------------------------
    if data.ingresses.is_empty() {
        p.text(
            Pos2::new(x_izq + ANCHO_NODO / 2.0, cy),
            egui::Align2::CENTER_CENTER,
            "sin ingress",
            FontId::proportional(10.5),
            theme::TEXTO_TENUE,
        );
    }
    for (i, ing) in data.ingresses.iter().enumerate() {
        let y = rect.top() + 20.0 + i as f32 * (ALTO_NODO + 12.0);
        let r = Rect::from_min_size(Pos2::new(x_izq, y), egui::vec2(ANCHO_NODO, ALTO_NODO));
        nodo(&p, r, theme::WARN);
        p.text(
            r.center_top() + egui::vec2(0.0, 8.0),
            egui::Align2::CENTER_TOP,
            &ing.name,
            FontId::proportional(11.5),
            theme::TEXTO,
        );
        let hosts = if ing.hosts.is_empty() {
            "*".to_string()
        } else {
            ing.hosts.join(", ")
        };
        p.text(
            r.center_top() + egui::vec2(0.0, 24.0),
            egui::Align2::CENTER_TOP,
            recortar(&hosts, 26),
            FontId::proportional(9.5),
            theme::TEXTO_TENUE,
        );
        curva(&p, r.right_center(), svc_rect.left_center(), theme::WARN);
    }

    // --- workloads con sus pods a la derecha ------------------------------
    let mut y = rect.top() + 20.0;
    if data.workloads.is_empty() {
        p.text(
            Pos2::new(x_der + ANCHO_NODO / 2.0, cy),
            egui::Align2::CENTER_CENTER,
            "sin pods que matcheen",
            FontId::proportional(10.5),
            theme::TEXTO_TENUE,
        );
    }
    for w in &data.workloads {
        let alto_w = alto_workload(w.pods.len());
        let r = Rect::from_min_size(Pos2::new(x_der, y), egui::vec2(ANCHO_NODO, alto_w));
        let listos = w.pods.iter().filter(|p| p.ready).count();
        let color = if listos == w.pods.len() && !w.pods.is_empty() {
            theme::OK
        } else if listos == 0 {
            theme::BAD
        } else {
            theme::WARN
        };
        nodo(&p, r, color);
        p.text(
            r.center_top() + egui::vec2(0.0, 7.0),
            egui::Align2::CENTER_TOP,
            format!("{} · {}/{}", w.kind, listos, w.pods.len()),
            FontId::proportional(10.0),
            theme::TEXTO_TENUE,
        );
        p.text(
            r.center_top() + egui::vec2(0.0, 20.0),
            egui::Align2::CENTER_TOP,
            recortar(&w.name, 24),
            FontId::proportional(11.5),
            theme::TEXTO,
        );

        // Puntos de pods, en filas de 8.
        let por_fila = 8usize;
        for (i, pod) in w.pods.iter().enumerate() {
            let fila = i / por_fila;
            let col = i % por_fila;
            let centro = Pos2::new(
                r.left() + 16.0 + col as f32 * 18.0,
                r.top() + 44.0 + fila as f32 * 16.0,
            );
            let c = if pod.ready {
                theme::OK
            } else if pod.estado == "Pending" {
                theme::WARN
            } else {
                theme::BAD
            };
            p.circle_filled(centro, 5.0, c);
            // Tooltip por pod: una zona interactuable invisible sobre el punto.
            let zona = Rect::from_center_size(centro, egui::vec2(14.0, 14.0));
            ui.interact(zona, egui::Id::new(("pod_dot", &w.name, i)), egui::Sense::hover())
                .on_hover_ui(|ui| {
                    ui.label(&pod.name);
                    ui.colored_label(theme::TEXTO_TENUE, &pod.estado);
                });
        }

        curva(&p, svc_rect.right_center(), r.left_center(), color);
        y += alto_w + 12.0;
    }

    // --- selector abajo ---------------------------------------------------
    if !data.selector.is_empty() {
        let sel = data
            .selector
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("  ");
        p.text(
            Pos2::new(rect.center().x, rect.bottom() - 4.0),
            egui::Align2::CENTER_BOTTOM,
            format!("selector: {sel}"),
            FontId::monospace(9.5),
            theme::TEXTO_TENUE,
        );
    }
}

fn alto_workload(pods: usize) -> f32 {
    let filas = pods.div_ceil(8).max(1);
    58.0 + filas as f32 * 16.0
}

fn nodo(p: &egui::Painter, r: Rect, acento: Color32) {
    p.rect_filled(r, CornerRadius::same(6), theme::PANEL_ALT);
    p.rect_stroke(
        r,
        CornerRadius::same(6),
        Stroke::new(1.0, theme::BORDE),
        StrokeKind::Inside,
    );
    // Barrita de color a la izquierda, como identidad del tipo de nodo.
    p.rect_filled(
        Rect::from_min_size(r.min, egui::vec2(3.0, r.height())),
        CornerRadius::same(2),
        acento,
    );
}

fn curva(p: &egui::Painter, desde: Pos2, hasta: Pos2, color: Color32) {
    let dx = (hasta.x - desde.x) * 0.5;
    let shape = egui::epaint::CubicBezierShape::from_points_stroke(
        [
            desde,
            Pos2::new(desde.x + dx, desde.y),
            Pos2::new(hasta.x - dx, hasta.y),
            hasta,
        ],
        false,
        Color32::TRANSPARENT,
        Stroke::new(1.5, color.linear_multiply(0.6)),
    );
    p.add(shape);
}

fn recortar(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max - 1).collect::<String>())
    }
}

// ------------------------------------------------------------- workload

use super::Accion;
use crate::k8s::mapa::WorkloadMapaData;

/// Una cajita clickeable del mapa de workload.
struct Caja {
    color: Color32,
    /// Etiqueta visible ("ConfigMap", "PVC"…).
    tipo: &'static str,
    /// Kind real para navegar ("PersistentVolumeClaim").
    kind: &'static str,
    nombre: String,
    sub: String,
    existe: bool,
}

/// Mapa de configuración de un workload, en vertical: tráfico → workload →
/// configuración. Cada cajita navega al recurso al clickearla.
pub fn dibujar_workload(
    ui: &mut egui::Ui,
    data: &WorkloadMapaData,
    pane_id: u64,
    ns: &Option<String>,
    accion: &mut Accion,
) {
    if let Some(err) = &data.error {
        ui.colored_label(theme::BAD, err);
        return;
    }

    // ---- tráfico entrante ------------------------------------------------
    let mut trafico: Vec<Caja> = Vec::new();
    for ing in &data.ingresses {
        trafico.push(Caja {
            color: theme::WARN,
            tipo: "Ingress",
            kind: "Ingress",
            nombre: ing.name.clone(),
            sub: if ing.hosts.is_empty() {
                "*".to_string()
            } else {
                ing.hosts.join(", ")
            },
            existe: true,
        });
    }
    for svc in &data.services {
        trafico.push(Caja {
            color: theme::OK,
            tipo: "Service",
            kind: "Service",
            nombre: svc.clone(),
            sub: "selector matchea el template".into(),
            existe: true,
        });
    }

    seccion(ui, "Tráfico");
    if trafico.is_empty() {
        ui.colored_label(theme::TEXTO_TENUE, "nada apunta a este workload");
    }
    grilla(ui, &trafico, pane_id, ns, accion);

    flecha(ui);

    // ---- el workload -----------------------------------------------------
    egui::Frame::new()
        .fill(theme::PANEL_ALT)
        .stroke(egui::Stroke::new(1.0, theme::ACENTO.linear_multiply(0.6)))
        .corner_radius(6)
        .inner_margin(8)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.colored_label(theme::ACENTO, "▦");
                ui.label(egui::RichText::new(&data.name).strong());
                ui.colored_label(theme::TEXTO_TENUE, &data.kind);
            });
            for img in &data.imagenes {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(img)
                            .monospace()
                            .size(10.5)
                            .color(theme::TEXTO_TENUE),
                    )
                    .truncate(),
                )
                .on_hover_text(img);
            }
        });

    flecha(ui);

    // ---- configuración ---------------------------------------------------
    let mut config: Vec<Caja> = Vec::new();
    for r in &data.configmaps {
        config.push(Caja {
            color: theme::ACENTO,
            tipo: "ConfigMap",
            kind: "ConfigMap",
            nombre: r.name.clone(),
            sub: r.usos.join(", "),
            existe: r.existe,
        });
    }
    for r in &data.secrets {
        config.push(Caja {
            color: theme::WARN,
            tipo: "Secret",
            kind: "Secret",
            nombre: r.name.clone(),
            sub: r.usos.join(", "),
            existe: r.existe,
        });
    }
    for r in &data.pvcs {
        config.push(Caja {
            color: theme::OK,
            tipo: "PVC",
            kind: "PersistentVolumeClaim",
            nombre: r.name.clone(),
            sub: r.usos.join(", "),
            existe: r.existe,
        });
    }
    if let Some(sa) = &data.service_account {
        config.push(Caja {
            color: Color32::from_rgb(0xb0, 0x7f, 0xd8),
            tipo: "ServiceAccount",
            kind: "ServiceAccount",
            nombre: sa.name.clone(),
            sub: sa.usos.join(", "),
            existe: sa.existe,
        });
    }

    seccion(ui, "Configuración");
    if config.is_empty() {
        ui.colored_label(theme::TEXTO_TENUE, "sin configuración referenciada");
    }
    grilla(ui, &config, pane_id, ns, accion);
}

fn seccion(ui: &mut egui::Ui, titulo: &str) {
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(titulo.to_uppercase())
            .size(10.5)
            .color(theme::ACENTO),
    );
    ui.add_space(2.0);
}

fn flecha(ui: &mut egui::Ui) {
    ui.vertical_centered(|ui| {
        ui.colored_label(theme::TEXTO_TENUE, "↓");
    });
}

/// Cajitas en grilla: dos por fila si el panel da el ancho, una si no.
fn grilla(ui: &mut egui::Ui, cajas: &[Caja], pane_id: u64, ns: &Option<String>, accion: &mut Accion) {
    let gap = 6.0;
    let disponible = ui.available_width();
    let por_fila = if disponible >= 420.0 { 2 } else { 1 };
    let ancho = (disponible - gap * (por_fila as f32 - 1.0)) / por_fila as f32;

    for grupo in cajas.chunks(por_fila) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for caja in grupo {
                if cajita(ui, caja, ancho) {
                    *accion = Accion::IrA(
                        pane_id,
                        caja.kind.to_string(),
                        ns.clone(),
                        caja.nombre.clone(),
                    );
                }
            }
        });
        ui.add_space(gap);
    }
}

/// Una tarjeta clickeable con barrita de color, hover y flechita "ir".
fn cajita(ui: &mut egui::Ui, caja: &Caja, ancho: f32) -> bool {
    let alto = 44.0;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ancho, alto), egui::Sense::click());
    let hover = resp.hovered();
    if hover {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let p = ui.painter_at(rect.expand(1.0));

    let acento = if caja.existe { caja.color } else { theme::BAD };
    p.rect_filled(
        rect,
        CornerRadius::same(5),
        if hover { theme::BORDE } else { theme::PANEL_ALT },
    );
    p.rect_stroke(
        rect,
        CornerRadius::same(5),
        Stroke::new(1.0, if hover { acento } else { theme::BORDE }),
        StrokeKind::Inside,
    );
    p.rect_filled(
        Rect::from_min_size(rect.min, egui::vec2(3.0, rect.height())),
        CornerRadius::same(2),
        acento,
    );

    // Tipo arriba a la izquierda, aviso/flecha a la derecha.
    p.text(
        rect.left_top() + egui::vec2(10.0, 6.0),
        egui::Align2::LEFT_TOP,
        caja.tipo,
        FontId::proportional(9.5),
        theme::TEXTO_TENUE,
    );
    p.text(
        rect.right_top() + egui::vec2(-8.0, 5.0),
        egui::Align2::RIGHT_TOP,
        if caja.existe { "→" } else { "⚠" },
        FontId::proportional(11.0),
        if caja.existe {
            if hover {
                theme::TEXTO
            } else {
                theme::TEXTO_TENUE
            }
        } else {
            theme::BAD
        },
    );

    // Nombre y uso, recortados al ancho de la caja.
    let max_nombre = ((ancho - 24.0) / 6.6) as usize;
    p.text(
        rect.left_top() + egui::vec2(10.0, 18.0),
        egui::Align2::LEFT_TOP,
        recortar(&caja.nombre, max_nombre.max(8)),
        FontId::proportional(11.5),
        if caja.existe { theme::TEXTO } else { theme::BAD },
    );
    p.text(
        rect.left_bottom() + egui::vec2(10.0, -4.0),
        egui::Align2::LEFT_BOTTOM,
        recortar(&caja.sub, max_nombre.max(8)),
        FontId::proportional(9.0),
        theme::TEXTO_TENUE,
    );

    let resp = resp.on_hover_ui(|ui| {
        ui.label(&caja.nombre);
        if !caja.sub.is_empty() {
            ui.colored_label(theme::TEXTO_TENUE, &caja.sub);
        }
        if !caja.existe {
            ui.colored_label(theme::BAD, "⚠ no existe en el namespace");
        } else {
            ui.colored_label(theme::TEXTO_TENUE, "click para ir al recurso");
        }
    });
    resp.clicked()
}
