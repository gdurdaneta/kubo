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

    let modal = egui::Modal::new(egui::Id::new("confirmacion")).show(ctx, |ui| {
        ui.set_width(360.0);
        let c = app.confirm.as_mut().unwrap();
        let destino = match &c.ns {
            Some(ns) => format!("{} «{}» en {ns}", c.kind, c.name),
            None => format!("{} «{}»", c.kind, c.name),
        };

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

    if modal.should_close() || cancelar {
        if !ejecutar {
            app.confirm = None;
        }
    }
    if ejecutar {
        app.ejecutar_confirmada();
    }
}
