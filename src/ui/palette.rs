//! Paleta de comandos (Ctrl+K): un campo de texto que busca a la vez entre
//! los kinds del sidebar y los recursos del cluster por nombre.

use super::Accion;
use crate::app::App;
use crate::theme;

/// Una fila de la lista: saltar a una vista, o abrir un objeto concreto.
enum Fila {
    Kind { label: String, kind: String },
    Recurso { kind: String, ns: Option<String>, name: String },
}

pub fn dibujar(app: &mut App, ctx: &egui::Context, accion: &mut Accion) {
    if app.palette.is_none() {
        return;
    }

    // Filas: primero los kinds que matchean, después los recursos.
    let (filas, buscando, query_corta, parcial) = {
        let p = app.palette.as_ref().unwrap();
        let q = p.query.trim().to_lowercase();
        let mut filas: Vec<Fila> = Vec::new();

        if let Some(pane) = app.panes.iter().find(|x| x.id == p.pane) {
            if let Some(cluster) = app.cluster_de(pane) {
                for item in cluster.nav.iter().flat_map(|c| c.items.iter()) {
                    if q.is_empty() || item.label.to_lowercase().contains(&q) {
                        filas.push(Fila::Kind {
                            label: item.label.clone(),
                            kind: item.res.ar.kind.clone(),
                        });
                    }
                }
            }
        }
        filas.truncate(6);

        for h in &p.hits {
            filas.push(Fila::Recurso {
                kind: h.kind.clone(),
                ns: h.ns.clone(),
                name: h.name.clone(),
            });
        }
        (
            filas,
            p.buscando,
            p.query.trim().chars().count() < 2,
            p.hits.len() >= 60,
        )
    };

    // Teclado: flechas para moverse, Enter para abrir, Esc para cerrar.
    let (subir, bajar, enter, escape) = ctx.input_mut(|i| {
        (
            i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
            i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
            i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
            i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
        )
    });
    if let Some(p) = app.palette.as_mut() {
        if bajar && !filas.is_empty() {
            p.sel = (p.sel + 1).min(filas.len() - 1);
        }
        if subir {
            p.sel = p.sel.saturating_sub(1);
        }
        p.sel = p.sel.min(filas.len().saturating_sub(1));
    }

    let mut elegido: Option<usize> = None;
    let mut cerrar = escape;

    let pane_id = app.palette.as_ref().unwrap().pane;
    let sel = app.palette.as_ref().unwrap().sel;

    let modal = egui::Modal::new(egui::Id::new("palette")).show(ctx, |ui| {
        ui.set_width(560.0);

        let p = app.palette.as_mut().unwrap();
        let resp = ui.add(
            egui::TextEdit::singleline(&mut p.query)
                .hint_text("buscar recurso o vista…")
                .desired_width(f32::INFINITY)
                .font(egui::TextStyle::Heading),
        );
        if resp.changed() {
            p.desde_cambio = 0.0;
            p.sel = 0;
        }
        // El foco vive en el campo mientras la paleta esté abierta.
        resp.request_focus();

        ui.horizontal(|ui| {
            if buscando {
                ui.spinner();
            }
            let ayuda = if query_corta {
                "escribí al menos 2 letras para buscar recursos".to_string()
            } else if parcial {
                format!("{} coincidencias (recortado)", filas.len())
            } else {
                format!("{} coincidencias", filas.len())
            };
            ui.colored_label(theme::TEXTO_TENUE, ayuda);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.colored_label(theme::TEXTO_TENUE, "↑↓ mover · ↵ abrir · esc cerrar");
            });
        });
        ui.separator();

        egui::ScrollArea::vertical()
            .max_height(380.0)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if filas.is_empty() && !query_corta && !buscando {
                    ui.colored_label(theme::TEXTO_TENUE, "sin coincidencias");
                }
                for (i, fila) in filas.iter().enumerate() {
                    let activo = i == sel;
                    let alto = 26.0;
                    let (rect, resp) = ui
                        .allocate_exact_size(egui::vec2(ui.available_width(), alto), egui::Sense::click());
                    if activo {
                        ui.painter().rect_filled(
                            rect,
                            egui::CornerRadius::same(4),
                            theme::ACENTO.linear_multiply(0.25),
                        );
                    } else if resp.hovered() {
                        ui.painter()
                            .rect_filled(rect, egui::CornerRadius::same(4), theme::PANEL_ALT);
                    }
                    let (icono, principal, secundario, color) = match fila {
                        Fila::Kind { label, .. } => (
                            "▸",
                            label.clone(),
                            "ir a la vista".to_string(),
                            theme::TEXTO_TENUE,
                        ),
                        Fila::Recurso { kind, ns, name } => (
                            "◆",
                            name.clone(),
                            match ns {
                                Some(ns) => format!("{kind} · {ns}"),
                                None => kind.clone(),
                            },
                            theme::ACENTO,
                        ),
                    };
                    let p = ui.painter();
                    p.text(
                        rect.left_center() + egui::vec2(8.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        icono,
                        egui::FontId::proportional(11.0),
                        color,
                    );
                    p.text(
                        rect.left_center() + egui::vec2(26.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        &principal,
                        egui::FontId::proportional(13.0),
                        theme::TEXTO,
                    );
                    p.text(
                        rect.right_center() + egui::vec2(-8.0, 0.0),
                        egui::Align2::RIGHT_CENTER,
                        &secundario,
                        egui::FontId::proportional(10.5),
                        theme::TEXTO_TENUE,
                    );
                    if resp.clicked() {
                        elegido = Some(i);
                    }
                }
            });
    });

    if enter && !filas.is_empty() {
        elegido = Some(sel.min(filas.len() - 1));
    }
    if modal.should_close() {
        cerrar = true;
    }

    if let Some(i) = elegido {
        match &filas[i] {
            Fila::Kind { kind, .. } => {
                // Sin nombre: solo cambia la vista.
                *accion = Accion::IrA(pane_id, kind.clone(), None, String::new());
            }
            Fila::Recurso { kind, ns, name } => {
                *accion = Accion::IrA(pane_id, kind.clone(), ns.clone(), name.clone());
            }
        }
        cerrar = true;
    }
    if cerrar {
        app.palette = None;
    }
}
