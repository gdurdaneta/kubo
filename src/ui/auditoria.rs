//! Vista del registro local de acciones.

use super::Accion;
use crate::app::App;
use crate::theme;

pub fn vista(_app: &mut App, ui: &mut egui::Ui, _pane_id: u64, _accion: &mut Accion) {
    let entradas = crate::auditoria::ultimas(500);

    egui::Frame::new()
        .fill(theme::PANEL)
        .inner_margin(egui::Margin::symmetric(8, 5))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Acciones hechas").strong().size(14.0));
                ui.colored_label(theme::TEXTO_TENUE, entradas.len().to_string());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(p) = crate::auditoria::ruta() {
                        if ui
                            .small_button("copiar ruta")
                            .on_hover_text(p.display().to_string())
                            .clicked()
                        {
                            ui.ctx().copy_text(p.display().to_string());
                        }
                    }
                });
            });
        });

    if entradas.is_empty() {
        super::centrado(
            ui,
            "Todavía no hiciste ninguna acción sobre un cluster desde kubo.",
            theme::TEXTO_TENUE,
        );
        return;
    }

    // Las mutaciones son pocas y valen leerse enteras; sin virtualizar.
    egui::ScrollArea::vertical()
        .id_salt("auditoria")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(6.0);
            for e in &entradas {
                egui::Frame::new()
                    .fill(theme::PANEL_ALT)
                    .stroke(egui::Stroke::new(
                        1.0,
                        if e.ok { theme::BORDE } else { theme::BAD },
                    ))
                    .corner_radius(5)
                    .inner_margin(7)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.colored_label(
                                if e.ok { theme::OK } else { theme::BAD },
                                if e.ok { "●" } else { "✗" },
                            );
                            ui.colored_label(
                                color_verbo(&e.verbo),
                                egui::RichText::new(&e.verbo).strong(),
                            );
                            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                            ui.add(
                                egui::Label::new(format!("{} «{}»", e.kind, e.name)).truncate(),
                            );
                            if let Some(d) = &e.detalle {
                                ui.colored_label(theme::TEXTO_TENUE, d);
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.colored_label(theme::TEXTO_TENUE, fecha_legible(&e.ts));
                            ui.colored_label(theme::TEXTO_TENUE, "·");
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(&e.contexto).color(theme::ACENTO),
                                )
                                .truncate(),
                            )
                            .on_hover_text(&e.contexto);
                            if let Some(ns) = &e.ns {
                                ui.colored_label(theme::TEXTO_TENUE, format!("· {ns}"));
                            }
                        });
                        if let Some(err) = &e.error {
                            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                            ui.colored_label(theme::BAD, err);
                        }
                    });
                ui.add_space(4.0);
            }
        });
}

fn color_verbo(v: &str) -> egui::Color32 {
    match v {
        "borrar" => theme::BAD,
        "escalar" | "reiniciar" => theme::WARN,
        _ => theme::TEXTO,
    }
}

/// El timestamp se guarda ISO completo; en pantalla alcanza con la fecha y la
/// hora al segundo.
fn fecha_legible(ts: &str) -> String {
    ts.replace('T', " ")
        .split('.')
        .next()
        .unwrap_or(ts)
        .trim_end_matches('Z')
        .to_string()
}
