//! Ayuda de atajos (F1).
//!
//! La lista vive acá y en un solo lugar: si un atajo cambia, se cambia el
//! handler y esta tabla, que están a un archivo de distancia. Peor sería que la
//! ayuda mintiera.

use crate::app::App;
use crate::theme;

/// (grupo, tecla, qué hace). El grupo se repite para agrupar en la vista.
const ATAJOS: &[(&str, &str, &str)] = &[
    ("Navegación", "Ctrl+K", "Buscar cualquier recurso del cluster"),
    ("Navegación", "Ctrl+P", "Cambiar de cluster"),
    ("Navegación", "Ctrl+N", "Cambiar de namespace"),
    ("Navegación", "escribir", "Cae en el buscador de la tabla"),
    ("Paneles", "Ctrl+T", "Abrir otro panel (hasta 4)"),
    ("Listas y diálogos", "↑ ↓", "Moverse"),
    ("Listas y diálogos", "Enter", "Elegir"),
    ("Listas y diálogos", "Esc", "Cerrar"),
    ("Tabla", "clic", "Abrir el detalle"),
    ("Tabla", "doble clic", "Logs del pod"),
    ("Tabla", "clic derecho", "Acciones: escalar, reiniciar, borrar, forward"),
    ("Tabla", "clic en cabecera", "Ordenar por esa columna"),
    ("Ayuda", "F1", "Esta ventana"),
];

pub fn dibujar(app: &mut App, ctx: &egui::Context) {
    if !app.ver_atajos {
        return;
    }
    let mut cerrar = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));

    let modal = egui::Modal::new(egui::Id::new("atajos")).show(ctx, |ui| {
        ui.set_width(520.0);
        ui.horizontal(|ui| {
            ui.heading("Atajos");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("×").clicked() {
                    cerrar = true;
                }
            });
        });
        ui.add_space(8.0);

        let mut grupo_actual = "";
        for (grupo, tecla, que) in ATAJOS {
            if *grupo != grupo_actual {
                if !grupo_actual.is_empty() {
                    ui.add_space(10.0);
                }
                grupo_actual = grupo;
                ui.colored_label(
                    theme::ACENTO,
                    egui::RichText::new(grupo.to_uppercase()).size(11.0),
                );
                ui.add_space(2.0);
            }
            ui.horizontal(|ui| {
                // Ancho fijo para que las teclas queden en columna.
                let (rect, _) = ui.allocate_exact_size(
                    egui::vec2(120.0, 20.0),
                    egui::Sense::hover(),
                );
                let mut tecla_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(rect)
                        .layout(egui::Layout::right_to_left(egui::Align::Center)),
                );
                tecla_ui.add(
                    egui::Label::new(
                        egui::RichText::new(*tecla)
                            .monospace()
                            .color(theme::TEXTO)
                            .background_color(theme::PANEL_ALT),
                    )
                    .selectable(false),
                );
                ui.add_space(10.0);
                ui.colored_label(theme::TEXTO_TENUE, *que);
            });
        }
    });

    if modal.should_close() || cerrar {
        app.ver_atajos = false;
    }
}
