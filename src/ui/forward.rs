//! Diálogo de port-forward y lista de los que están activos.

use super::Accion;
use crate::app::{App, EstadoPf};
use crate::k8s::portforward;
use crate::theme;

/// Modal para configurar un forward antes de levantarlo.
pub fn dialogo(app: &mut App, ctx: &egui::Context, accion: &mut Accion) {
    if app.dialogo_pf.is_none() {
        return;
    }
    let mut abrir = false;
    let mut cerrar = false;

    let modal = egui::Modal::new(egui::Id::new("dialogo_pf")).show(ctx, |ui| {
        ui.set_width(460.0);
        let Some(d) = app.dialogo_pf.as_mut() else { return };

        ui.heading("Port forward");
        ui.colored_label(
            theme::TEXTO_TENUE,
            format!("Service {} · {}", d.servicio, d.ns),
        );
        ui.add_space(8.0);

        if d.cargando {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.colored_label(theme::TEXTO_TENUE, "leyendo los puertos del Service…");
            });
            return;
        }
        if d.puertos.is_empty() {
            ui.colored_label(theme::BAD, "el Service no publica puertos TCP");
            return;
        }

        // --- puerto del servicio ---
        ui.horizontal(|ui| {
            ui.label("Puerto del servicio");
            let sel = d.sel.min(d.puertos.len() - 1);
            egui::ComboBox::from_id_salt("pf_puerto")
                .selected_text(d.puertos[sel].etiqueta())
                .show_ui(ui, |ui| {
                    for (i, p) in d.puertos.iter().enumerate() {
                        if ui.selectable_label(i == d.sel, p.etiqueta()).clicked() {
                            d.sel = i;
                            d.puerto_local =
                                portforward::puerto_local_sugerido(p.puerto).to_string();
                        }
                    }
                });
        });

        // --- puerto local ---
        ui.horizontal(|ui| {
            ui.label("Puerto local");
            ui.add(
                egui::TextEdit::singleline(&mut d.puerto_local)
                    .desired_width(90.0)
                    .hint_text("8080"),
            );
            let svc = d.puertos[d.sel.min(d.puertos.len() - 1)].puerto;
            let local: Option<u16> = d.puerto_local.trim().parse().ok();
            match local {
                Some(p) if p < 1024 => {
                    ui.colored_label(theme::BAD, "⚠ necesita privilegios");
                }
                Some(p) if p != svc => {
                    ui.colored_label(theme::WARN, format!("⚠ remapeado desde {svc}"));
                }
                Some(_) => {
                    ui.colored_label(theme::OK, "✓ igual que en el cluster");
                }
                None => {
                    ui.colored_label(theme::BAD, "número inválido");
                }
            }
        });

        ui.add_space(6.0);
        ui.separator();
        ui.add_space(6.0);

        // --- resolución por nombre ---
        let valido = crate::hosts::nombre_valido(&d.servicio);
        ui.add_enabled_ui(valido, |ui| {
            ui.checkbox(&mut d.alias, "resolver por nombre (alias en /etc/hosts)")
                .on_hover_text(
                    "Le da al servicio una IP de loopback propia y agrega la entrada \
                     en /etc/hosts. Pide la clave con el diálogo de polkit.",
                );
        });
        if !valido {
            d.alias = false;
            ui.colored_label(
                theme::TEXTO_TENUE,
                "el nombre no sirve como entrada de /etc/hosts",
            );
        }

        // --- cómo va a quedar ---
        let local = d.puerto_local.trim().parse::<u16>().unwrap_or(0);
        let (host, nota) = if d.alias {
            (
                d.servicio.clone(),
                format!("escucha en {}", portforward::ip_para(&d.servicio)),
            )
        } else {
            (
                format!("{}.localhost", d.servicio),
                "escucha en 127.0.0.1".to_string(),
            )
        };
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(theme::TEXTO_TENUE, "Vas a consumirlo en");
            ui.colored_label(
                theme::ACENTO,
                egui::RichText::new(format!("http://{host}:{local}")).monospace(),
            );
        });
        ui.colored_label(theme::TEXTO_TENUE, nota);

        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui
                .add(egui::Button::new(
                    egui::RichText::new("Levantar").color(theme::OK),
                ))
                .clicked()
            {
                abrir = true;
            }
            if ui.button("Cancelar").clicked() {
                cerrar = true;
            }
        });
    });

    if modal.should_close() || cerrar {
        app.dialogo_pf = None;
    }
    if abrir {
        *accion = Accion::AbrirForward;
    }
}

/// Los contextos de EKS son ARNs enteros; en la lista alcanza con la cola.
fn acortar_ctx(c: &str) -> &str {
    c.rsplit('/').next().unwrap_or(c)
}

/// Vista de port-forwards dentro del panel, en lugar de la tabla de recursos.
pub fn vista(app: &mut App, ui: &mut egui::Ui, _pane_id: u64, accion: &mut Accion) {
    egui::Frame::new()
        .fill(theme::PANEL)
        .inner_margin(egui::Margin::symmetric(8, 5))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Port Forwards").strong().size(14.0));
                ui.colored_label(theme::TEXTO_TENUE, app.forwards.len().to_string());
            });
        });

    if app.forwards.is_empty() {
        super::centrado(
            ui,
            "Sin port-forwards. Se abren desde el menú ⋮ de un Service.",
            theme::TEXTO_TENUE,
        );
        return;
    }

    egui::ScrollArea::vertical()
        .id_salt("lista_pf")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(6.0);
            for f in &app.forwards {
                fila(ui, f, accion);
                ui.add_space(6.0);
            }
        });
}

fn fila(ui: &mut egui::Ui, f: &crate::app::Forward, accion: &mut Accion) {
    egui::Frame::new()
        .fill(theme::PANEL_ALT)
        .stroke(egui::Stroke::new(1.0, theme::BORDE))
        .corner_radius(6)
        .inner_margin(8)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let (punto, color, ayuda) = match f.estado {
                    EstadoPf::Activo => ("●", theme::OK, "escuchando"),
                    EstadoPf::Levantando => ("●", theme::WARN, "levantando"),
                    EstadoPf::Caido => ("●", theme::BAD, "caído"),
                };
                ui.colored_label(color, punto).on_hover_text(ayuda);

                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.colored_label(
                            theme::ACENTO,
                            egui::RichText::new(f.url()).monospace().strong(),
                        );
                        if ui.small_button("copiar").clicked() {
                            ui.ctx().copy_text(f.url());
                        }
                    });
                    ui.colored_label(
                        theme::TEXTO_TENUE,
                        format!(
                            "{} · {} · {} · puerto {} del servicio{}",
                            acortar_ctx(&f.contexto),
                            f.ns,
                            f.servicio,
                            f.puerto_svc,
                            if f.conexiones > 0 {
                                format!(" · {} conexiones", f.conexiones)
                            } else {
                                String::new()
                            }
                        ),
                    );
                    if f.remapeado() {
                        ui.colored_label(
                            theme::WARN,
                            format!(
                                "⚠ puerto remapeado: en el cluster es {}, acá {}",
                                f.puerto_svc, f.puerto_local
                            ),
                        );
                    }
                    if let Some(e) = &f.error {
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                        let (color, prefijo) = if f.estado == EstadoPf::Caido {
                            (theme::BAD, "")
                        } else {
                            (theme::TEXTO_TENUE, "último fallo: ")
                        };
                        ui.colored_label(color, format!("{prefijo}{e}"));
                    }
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("×").on_hover_text("Cerrar el forward").clicked() {
                        *accion = Accion::CerrarForward(f.id);
                    }
                });
            });
        });
}
