//! Cabecera de un panel: contexto, namespace, toggle del sidebar y estado.

use super::Accion;
use crate::app::{App, Conn};
use crate::theme;

pub fn dibujar(app: &mut App, ui: &mut egui::Ui, id: u64, n_panes: usize, accion: &mut Accion) {
    let contextos = app.contextos.clone();
    let (contexto, ns_sel, nav_visible) = {
        let Some(pane) = app.panes.iter().find(|p| p.id == id) else {
            return;
        };
        (pane.contexto.clone(), pane.ns_sel.clone(), pane.nav_visible)
    };
    let mut toggle_nav = false;

    egui::Panel::top(egui::Id::new(("topbar", id)))
        .exact_size(36.0)
        .frame(
            egui::Frame::new()
                .fill(theme::PANEL)
                .inner_margin(egui::Margin::symmetric(8, 4)),
        )
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                // Toggle del sidebar.
                if ui
                    .button(if nav_visible { "◧" } else { "◨" })
                    .on_hover_text("Mostrar/ocultar recursos")
                    .clicked()
                {
                    toggle_nav = true;
                }

                // Contexto: más angosto cuando hay varios paneles.
                let ancho_ctx = if n_panes > 1 { 170.0 } else { 300.0 };
                let actual = contexto.clone().unwrap_or_else(|| "(sin contexto)".into());
                egui::ComboBox::from_id_salt(("ctx", id))
                    .selected_text(egui::RichText::new(acortar(&actual, if n_panes > 1 { 22 } else { 40 })))
                    .width(ancho_ctx)
                    .show_ui(ui, |ui| {
                        for c in &contextos {
                            let sel = contexto.as_deref() == Some(c.name.as_str());
                            if ui.selectable_label(sel, &c.name).clicked() && !sel {
                                *accion = Accion::Conectar(id, c.name.clone());
                            }
                        }
                    })
                    .response
                    .on_hover_text(actual);

                selector_namespace(app, ui, id, &ns_sel, accion);

                if ui.button("↻").on_hover_text("Recargar la vista").clicked() {
                    *accion = Accion::Refrescar(id);
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if n_panes > 1 && ui.button("×").on_hover_text("Cerrar panel").clicked() {
                        *accion = Accion::CerrarPane(id);
                    }
                    estado_cluster(app, ui, id, n_panes);
                });
            });
        });

    if toggle_nav {
        if let Some(pane) = app.panes.iter_mut().find(|p| p.id == id) {
            pane.nav_visible = !pane.nav_visible;
        }
    }
}

fn selector_namespace(
    app: &App,
    ui: &mut egui::Ui,
    id: u64,
    ns_sel: &Option<String>,
    accion: &mut Accion,
) {
    let Some(pane) = app.panes.iter().find(|p| p.id == id) else {
        return;
    };
    let Some(cluster) = app.cluster_de(pane) else { return };
    if cluster.conn != Conn::Lista {
        return;
    }
    let actual = ns_sel.clone().unwrap_or_else(|| "todos".into());
    egui::ComboBox::from_id_salt(("ns", id))
        .selected_text(egui::RichText::new(acortar(&actual, 20)))
        .width(150.0)
        .show_ui(ui, |ui| {
            if ui.selectable_label(ns_sel.is_none(), "todos los namespaces").clicked()
                && ns_sel.is_some()
            {
                *accion = Accion::CambiarNamespace(id, None);
            }
            ui.separator();
            if cluster.namespaces.is_empty() {
                ui.colored_label(theme::TEXTO_TENUE, "sin permiso para listarlos");
            }
            for ns in &cluster.namespaces {
                let sel = ns_sel.as_ref() == Some(ns);
                if ui.selectable_label(sel, ns).clicked() && !sel {
                    *accion = Accion::CambiarNamespace(id, Some(ns.clone()));
                }
            }
        });
}

fn estado_cluster(app: &App, ui: &mut egui::Ui, id: u64, n_panes: usize) {
    let Some(pane) = app.panes.iter().find(|p| p.id == id) else {
        return;
    };
    match app.cluster_de(pane) {
        Some(c) if c.conn == Conn::Lista => {
            if let Some(info) = &c.info {
                if n_panes <= 2 {
                    // Con discovery cacheado la versión llega unos ms después
                    // que el resto; hasta entonces no se muestra "v".
                    if !info.version.is_empty() {
                        ui.colored_label(theme::TEXTO_TENUE, format!("v{}", info.version));
                    }
                }
                ui.colored_label(theme::OK, "●").on_hover_text(&info.server);
            }
        }
        Some(c) if c.conn == Conn::Conectando => {
            ui.spinner();
        }
        Some(_) => {
            ui.colored_label(theme::BAD, "●");
        }
        None => {}
    }
}

/// Los nombres de contexto de EKS son ARNs enteros: se muestra la cola, que es
/// la parte que identifica el cluster.
fn acortar(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cola: String = s.chars().skip(s.chars().count() - (max - 1)).collect();
    format!("…{cola}")
}
