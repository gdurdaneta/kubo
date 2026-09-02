//! Modal de confirmación para acciones destructivas (borrar, reiniciar,
//! escalar). Nada muta el cluster sin pasar por acá.

use super::Accion;
use crate::app::{App, Verbo};
use crate::theme;

pub fn dibujar(app: &mut App, ctx: &egui::Context, _accion: &mut Accion) {
    let Some(confirm) = app.confirm.as_mut() else { return };

    // Escalar(-1) es el sentinel "precargar con las réplicas actuales".
    if let Verbo::Escalar(n) = confirm.verbo {
        if n < 0 {
            let key = match &confirm.ns {
                Some(ns) => format!("{ns}/{}", confirm.name),
                None => confirm.name.clone(),
            };
            let actuales = app
                .panes
                .iter()
                .find(|p| p.id == app.confirm.as_ref().unwrap().pane)
                .and_then(|p| p.store.as_ref())
                .and_then(|s| s.objeto(&key))
                .and_then(|o| o.data.get("spec"))
                .and_then(|s| s.get("replicas"))
                .and_then(|v| v.as_i64())
                .unwrap_or(1);
            if let Some(c) = app.confirm.as_mut() {
                c.verbo = Verbo::Escalar(actuales);
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
        ui.set_width(360.0);
        let c = app.confirm.as_mut().unwrap();
        let destino = match &c.ns {
            Some(ns) => format!("{} «{}» en {ns}", c.kind, c.name),
            None => format!("{} «{}»", c.kind, c.name),
        };

        // El cluster va primero y bien visible. Con varios paneles abiertos
        // sobre contextos distintos, no decirlo es la forma más fácil de tocar
        // producción creyendo que se está en staging.
        if let Some(ctx_nombre) = contexto.as_deref() {
            let prod = parece_produccion(ctx_nombre);
            egui::Frame::new()
                .fill(if prod { theme::BAD_TENUE } else { theme::PANEL_ALT })
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
                                egui::RichText::new(ctx_nombre)
                                    .strong()
                                    .color(if prod { theme::BAD } else { theme::TEXTO }),
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
                ui.label(format!("Se va a borrar {destino}. Esto no se puede deshacer."));
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
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label("Réplicas:");
                    ui.add(egui::DragValue::new(n).range(0..=500));
                    if *n == 0 {
                        ui.colored_label(theme::WARN, "⚠ queda sin pods");
                    }
                });
            }
        }

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            let (texto, color) = match &c.verbo {
                Verbo::Borrar => ("Borrar", theme::BAD),
                Verbo::Reiniciar => ("Reiniciar", theme::WARN),
                Verbo::Escalar(_) => ("Escalar", theme::ACENTO),
            };
            if ui
                .button(egui::RichText::new(texto).color(color).strong())
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
    use super::parece_produccion;

    #[test]
    fn marca_los_contextos_de_produccion() {
        assert!(parece_produccion("justo-prod-mexico"));
        assert!(parece_produccion("arn:aws:eks:us-east-1:1234:cluster/prod"));
        assert!(parece_produccion("PRD-cluster"));
        assert!(!parece_produccion("arn:aws:eks:us-east-2:1234:cluster/staging"));
        assert!(!parece_produccion("inxpirius@217.76.158.104"));
        assert!(!parece_produccion("preprod"));
        assert!(!parece_produccion("non-prod-eu"));
    }
}
