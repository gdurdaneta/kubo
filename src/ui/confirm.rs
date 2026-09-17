//! Modal de confirmación para mutaciones (borrar, reiniciar, escalar y aplicar
//! YAML). Nada muta el cluster sin pasar por acá.

use super::Accion;
use crate::app::{App, Verbo};
use crate::theme;

pub fn dibujar(app: &mut App, ctx: &egui::Context, _accion: &mut Accion) {
    let Some(confirm) = app.confirm.as_mut() else {
        return;
    };

    // Réplicas actuales del objeto: precargan el valor (sentinel -1) y se
    // muestran al lado para saber de dónde se parte.
    let mut actuales: Option<i64> = None;
    if let Verbo::Escalar(n) = confirm.verbo {
        let key = match &confirm.ns {
            Some(ns) => format!("{ns}/{}", confirm.name),
            None => confirm.name.clone(),
        };
        let pane = confirm.pane;
        actuales = app
            .panes
            .iter()
            .find(|p| p.id == pane)
            .and_then(|p| p.store.as_ref())
            .and_then(|s| s.objeto(&key))
            .and_then(|o| o.data.get("spec"))
            .and_then(|s| s.get("replicas"))
            .and_then(|v| v.as_i64());
        if n < 0 {
            if let Some(c) = app.confirm.as_mut() {
                c.verbo = Verbo::Escalar(actuales.unwrap_or(1));
            }
        }
    }

    let mut ejecutar = false;
    let mut cancelar = false;

    let contexto = app
        .panes
        .iter()
        .find(|p| Some(p.id) == app.confirm.as_ref().map(|c| c.pane))
        .and_then(|p| p.contexto.clone());

    let modal = egui::Modal::new(egui::Id::new("confirmacion")).show(ctx, |ui| {
        let c = app.confirm.as_mut().unwrap();
        ui.set_width(if c.diff.is_some() { 640.0 } else { 360.0 });
        let destino = if c.extra.is_empty() {
            match &c.ns {
                Some(ns) => format!("{} «{}» en {ns}", c.kind, c.name),
                None => format!("{} «{}»", c.kind, c.name),
            }
        } else {
            format!(
                "{} {}",
                c.cantidad(),
                crate::nav::plural_legible(&c.kind).to_lowercase()
            )
        };
        let lote: Vec<String> = if c.extra.is_empty() {
            Vec::new()
        } else {
            std::iter::once((&c.ns, &c.name))
                .chain(c.extra.iter().map(|(ns, n)| (ns, n)))
                .map(|(ns, n)| match ns {
                    Some(ns) => format!("{ns}/{n}"),
                    None => n.clone(),
                })
                .collect()
        };

        // El cluster va primero y bien visible. Con varios paneles abiertos
        // sobre contextos distintos, no decirlo es la forma más fácil de tocar
        // producción creyendo que se está en staging.
        if let Some(ctx_nombre) = contexto.as_deref() {
            let prod = parece_produccion(ctx_nombre);
            egui::Frame::new()
                .fill(if prod {
                    theme::BAD_TENUE
                } else {
                    theme::PANEL_ALT
                })
                .stroke(egui::Stroke::new(
                    1.0,
                    if prod { theme::BAD } else { theme::BORDE },
                ))
                .corner_radius(4)
                .inner_margin(6)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.colored_label(
                            if prod { theme::BAD } else { theme::TEXTO_TENUE },
                            if prod { "⚠ cluster" } else { "cluster" },
                        );
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(ctx_nombre).strong().color(if prod {
                                    theme::BAD
                                } else {
                                    theme::TEXTO
                                }),
                            )
                            .truncate(),
                        )
                        .on_hover_text(ctx_nombre);
                    });
                });
            ui.add_space(8.0);
        }

        match &mut c.verbo {
            Verbo::Borrar => {
                ui.label(egui::RichText::new("Borrar recurso").strong().size(15.0));
                ui.add_space(6.0);
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                ui.label(format!(
                    "Se va a borrar {destino}. Esto no se puede deshacer."
                ));
            }
            Verbo::Reiniciar => {
                ui.label(egui::RichText::new("Reiniciar").strong().size(15.0));
                ui.add_space(6.0);
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                if c.kind == "Pod" {
                    ui.label(format!(
                        "Se va a borrar el pod «{}»; su controlador lo va a recrear.",
                        c.name
                    ));
                } else {
                    ui.label(format!("Rollout restart de {destino}."));
                }
            }
            Verbo::Escalar(n) => {
                ui.label(egui::RichText::new("Escalar").strong().size(15.0));
                ui.add_space(6.0);
                ui.label(destino);
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label("Réplicas:");
                    if ui.add_enabled(*n > 0, egui::Button::new("−")).clicked() {
                        *n -= 1;
                    }
                    ui.add(egui::DragValue::new(n).range(0..=500).speed(0.1));
                    if ui.add_enabled(*n < 500, egui::Button::new("+")).clicked() {
                        *n += 1;
                    }
                    if let Some(a) = actuales {
                        ui.colored_label(
                            theme::TEXTO_TENUE,
                            if a == *n {
                                format!("(actual: {a})")
                            } else {
                                format!("(actual: {a} → {n})")
                            },
                        );
                    }
                });
                // Atajos para los valores de siempre.
                ui.horizontal(|ui| {
                    ui.colored_label(theme::TEXTO_TENUE, "rápido:");
                    for v in [0_i64, 1, 2, 3, 5, 10] {
                        if ui.selectable_label(*n == v, v.to_string()).clicked() {
                            *n = v;
                        }
                    }
                });
                if *n == 0 {
                    ui.colored_label(theme::WARN, "⚠ queda sin pods");
                }
            }
            Verbo::AplicarYaml(_) => {
                ui.label(
                    egui::RichText::new("Aplicar manifiesto")
                        .strong()
                        .size(15.0),
                );
                ui.add_space(6.0);
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                ui.label(format!("Se va a reemplazar {destino} con el YAML editado."));
                ui.colored_label(
                    theme::WARN,
                    "Los cambios se enviarán directamente al API server.",
                );
                if let Some(d) = c.diff.as_deref() {
                    ui.add_space(6.0);
                    dibujar_diff(ui, d);
                }
            }
        }

        if !lote.is_empty() {
            ui.add_space(4.0);
            egui::ScrollArea::vertical()
                .max_height(120.0)
                .show(ui, |ui| {
                    for l in &lote {
                        ui.colored_label(theme::TEXTO_TENUE, format!("• {l}"));
                    }
                });
        }

        // En producción, borrar, aplicar o dejar sin pods exige teclear el
        // nombre: el clic reflejo no alcanza.
        let prod = contexto.as_deref().is_some_and(parece_produccion)
            || std::env::var("KUBO_TEST_PROD").is_ok_and(|v| !v.is_empty());
        let tecleo = prod && requiere_tecleo(&c.verbo);
        let mut habilitado = true;
        if tecleo {
            // De a uno se teclea el nombre; en lote, la cantidad: obliga a
            // mirar cuántos son.
            let esperado = if c.extra.is_empty() {
                c.name.clone()
            } else {
                c.cantidad().to_string()
            };
            ui.add_space(10.0);
            ui.colored_label(
                theme::BAD,
                if c.extra.is_empty() {
                    format!("Producción: escribí «{esperado}» para confirmar")
                } else {
                    format!("Producción: escribí «{esperado}» (la cantidad) para confirmar")
                },
            );
            let resp = ui.add(
                egui::TextEdit::singleline(&mut c.tecleado)
                    .hint_text(&esperado)
                    .desired_width(f32::INFINITY),
            );
            resp.request_focus();
            habilitado = c.tecleado.trim() == esperado;
        }

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            let (texto, color) = match &c.verbo {
                Verbo::Borrar => ("Borrar", theme::BAD),
                Verbo::Reiniciar => ("Reiniciar", theme::WARN),
                Verbo::Escalar(_) => ("Escalar", theme::ACENTO),
                Verbo::AplicarYaml(_) => ("Aplicar", theme::WARN),
            };
            if ui
                .add_enabled(
                    habilitado,
                    egui::Button::new(egui::RichText::new(texto).color(color).strong()),
                )
                .on_disabled_hover_text("Escribí el nombre exacto del recurso")
                .clicked()
            {
                ejecutar = true;
            }
            if ui.button("Cancelar").clicked() {
                cancelar = true;
            }
        });
    });

    if (modal.should_close() || cancelar) && !ejecutar {
        app.confirm = None;
    }
    if ejecutar {
        app.ejecutar_confirmada();
    }
}

/// Qué verbos piden teclear el nombre en producción: los que destruyen o
/// reemplazan. Reiniciar y escalar a más de cero son reversibles al toque.
fn requiere_tecleo(v: &Verbo) -> bool {
    match v {
        Verbo::Borrar | Verbo::AplicarYaml(_) => true,
        Verbo::Escalar(n) => *n == 0,
        Verbo::Reiniciar => false,
    }
}

/// Diff unificado con colores, en monoespaciada y con scroll propio.
fn dibujar_diff(ui: &mut egui::Ui, diff: &str) {
    let (mas, menos) = diff.lines().fold((0, 0), |(m, n), l| {
        if l.starts_with('+') && !l.starts_with("+++") {
            (m + 1, n)
        } else if l.starts_with('-') && !l.starts_with("---") {
            (m, n + 1)
        } else {
            (m, n)
        }
    });
    ui.horizontal(|ui| {
        ui.colored_label(theme::TEXTO_TENUE, "cambios:");
        ui.colored_label(theme::OK, format!("+{mas}"));
        ui.colored_label(theme::BAD, format!("−{menos}"));
    });
    egui::Frame::new()
        .fill(theme::EXTREMO)
        .stroke(egui::Stroke::new(1.0, theme::BORDE))
        .inner_margin(6)
        .show(ui, |ui| {
            egui::ScrollArea::both()
                .max_height(320.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for l in diff.lines() {
                        let color = if l.starts_with("+++") || l.starts_with("---") {
                            theme::TEXTO_TENUE
                        } else if l.starts_with('+') {
                            theme::OK
                        } else if l.starts_with('-') {
                            theme::BAD
                        } else if l.starts_with("@@") {
                            theme::ACENTO
                        } else {
                            theme::TEXTO
                        };
                        ui.label(egui::RichText::new(l).monospace().size(11.5).color(color));
                    }
                });
        });
}

/// Heurística sobre el nombre del contexto para marcar los que parecen
/// producción. Falsos positivos son baratos —solo pinta el aviso de rojo—;
/// un falso negativo solo deja el diálogo como estaba.
fn parece_produccion(ctx: &str) -> bool {
    let c = ctx.to_lowercase();
    ["prod", "produccion", "producción", "live", "prd"]
        .iter()
        .any(|p| c.contains(p))
        && !c.contains("preprod")
        && !c.contains("non-prod")
}

#[cfg(test)]
mod tests {
    use super::{parece_produccion, requiere_tecleo};
    use crate::app::Verbo;

    #[test]
    fn teclear_solo_en_lo_destructivo() {
        assert!(requiere_tecleo(&Verbo::Borrar));
        assert!(requiere_tecleo(&Verbo::AplicarYaml(String::new())));
        assert!(requiere_tecleo(&Verbo::Escalar(0)));
        assert!(!requiere_tecleo(&Verbo::Escalar(3)));
        assert!(!requiere_tecleo(&Verbo::Reiniciar));
    }

    #[test]
    fn marca_los_contextos_de_produccion() {
        assert!(parece_produccion("justo-prod-mexico"));
        assert!(parece_produccion("arn:aws:eks:us-east-1:1234:cluster/prod"));
        assert!(parece_produccion("PRD-cluster"));
        assert!(!parece_produccion(
            "arn:aws:eks:us-east-2:1234:cluster/staging"
        ));
        assert!(!parece_produccion("inxpirius@217.76.158.104"));
        assert!(!parece_produccion("preprod"));
        assert!(!parece_produccion("non-prod-eu"));
    }
}
