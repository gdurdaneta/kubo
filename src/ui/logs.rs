//! Visor de logs en streaming, con filtro y selector de contenedor.

use super::Accion;
use crate::app::{App, Bottom};
use crate::theme;

pub fn dibujar(app: &mut App, ui: &mut egui::Ui, id: u64, accion: &mut Accion) {
    let Some(pane) = app.panes.iter_mut().find(|p| p.id == id) else {
        return;
    };
    let Some(Bottom::Logs(v)) = pane.bottom.as_mut() else { return };
    let mut reiniciar = false;

    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(&v.pod).strong());
        ui.colored_label(theme::TEXTO_TENUE, &v.ns);

        if v.contenedores.len() > 1 {
            let actual = v.contenedor.clone().unwrap_or_default();
            egui::ComboBox::from_id_salt(("cont", id))
                .selected_text(actual.clone())
                .width(180.0)
                .show_ui(ui, |ui| {
                    for c in v.contenedores.clone() {
                        if ui.selectable_label(actual == c, &c).clicked() && actual != c {
                            v.contenedor = Some(c);
                            reiniciar = true;
                        }
                    }
                });
        }

        if ui.checkbox(&mut v.follow, "seguir").changed() {
            reiniciar = true;
        }
        if ui
            .checkbox(&mut v.previous, "anterior")
            .on_hover_text("Logs del contenedor anterior (tras un crash)")
            .changed()
        {
            reiniciar = true;
        }

        ui.add(
            egui::TextEdit::singleline(&mut v.filtro)
                .hint_text("filtrar líneas")
                .desired_width(220.0),
        );

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("×").on_hover_text("Cerrar").clicked() {
                *accion = Accion::CerrarBottom(id);
            }
            if ui.button("Copiar").clicked() {
                let todo: Vec<&str> = v.lineas.iter().map(|s| s.as_str()).collect();
                ui.ctx().copy_text(todo.join("\n"));
            }
            ui.colored_label(theme::TEXTO_TENUE, format!("{} líneas", v.lineas.len()));
        });
    });

    if let Some(motivo) = &v.cerrado {
        ui.colored_label(theme::WARN, format!("stream terminado: {motivo}"));
    }

    let filtro = v.filtro.to_lowercase();
    let seguir = v.follow;

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .stick_to_bottom(seguir)
        .id_salt(("logs_scroll", id))
        .show(ui, |ui| {
            ui.style_mut().override_font_id = Some(egui::FontId::monospace(11.5));
            ui.spacing_mut().item_spacing.y = 0.0;
            for l in v.lineas.iter() {
                if !filtro.is_empty() && !l.to_lowercase().contains(&filtro) {
                    continue;
                }
                ui.add(
                    egui::Label::new(egui::RichText::new(l).color(color_linea(l)))
                        .wrap_mode(egui::TextWrapMode::Extend),
                );
            }
        });

    if reiniciar {
        *accion = Accion::ReiniciarLogs(id);
    }
}

/// Coloreado por nivel: barato y suficiente para leer un crash de un vistazo.
fn color_linea(l: &str) -> egui::Color32 {
    let s = l.to_ascii_lowercase();
    if s.contains("error") || s.contains("fatal") || s.contains("panic") {
        theme::BAD
    } else if s.contains("warn") {
        theme::WARN
    } else {
        theme::TEXTO
    }
}
