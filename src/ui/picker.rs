//! Selector rápido de contexto (Ctrl+P) y namespace (Ctrl+N).
//!
//! Mismo patrón que la paleta: modal, filtro con foco puesto y teclado.

use super::Accion;
use crate::app::{App, PickerModo};
use crate::theme;

const MAX_VISIBLE: usize = 12;

pub fn dibujar(app: &mut App, ctx: &egui::Context, accion: &mut Accion) {
    let Some(p) = app.picker.as_mut() else { return };

    // Opciones ya filtradas. El namespace lleva "todos" adelante como opción
    // real: es el estado por defecto del panel.
    let q = p.query.trim().to_lowercase();
    let opciones: Vec<String> = match p.modo {
        PickerModo::Contexto => app
            .contextos
            .iter()
            .map(|c| c.name.clone())
            .filter(|n| q.is_empty() || n.to_lowercase().contains(&q))
            .collect(),
        PickerModo::Namespace => {
            let mut v = vec![TODOS.to_string()];
            v.extend(
                app.panes
                    .iter()
                    .find(|x| x.id == p.pane)
                    .and_then(|x| x.contexto.as_ref())
                    .and_then(|c| app.clusters.get(c))
                    .map(|c| c.namespaces.clone())
                    .unwrap_or_default(),
            );
            v.into_iter()
                .filter(|n| q.is_empty() || n.to_lowercase().contains(&q))
                .collect()
        }
    };
    p.sel = p.sel.min(opciones.len().saturating_sub(1));

    let (arriba, abajo, entrar, salir) = ctx.input_mut(|i| {
        (
            i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
            i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
            i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
            i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
        )
    });
    if arriba {
        p.sel = p.sel.saturating_sub(1);
    }
    if abajo && p.sel + 1 < opciones.len() {
        p.sel += 1;
    }

    let (modo, pane, sel) = (p.modo, p.pane, p.sel);
    let mut elegido: Option<String> = None;
    let mut cerrar = salir;

    let modal = egui::Modal::new(egui::Id::new("picker")).show(ctx, |ui| {
        ui.set_width(560.0);
        let Some(p) = app.picker.as_mut() else { return };
        ui.heading(match modo {
            PickerModo::Contexto => "Cambiar de cluster",
            PickerModo::Namespace => "Cambiar de namespace",
        });
        ui.add_space(6.0);

        let campo = ui.add(
            egui::TextEdit::singleline(&mut p.query)
                .hint_text("filtrar")
                .desired_width(f32::INFINITY),
        );
        campo.request_focus();
        ui.add_space(6.0);

        if opciones.is_empty() {
            ui.colored_label(theme::TEXTO_TENUE, "nada coincide");
            return;
        }
        egui::ScrollArea::vertical()
            .max_height(MAX_VISIBLE as f32 * 26.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for (i, o) in opciones.iter().enumerate() {
                    let resp = ui.add_sized(
                        [ui.available_width(), 24.0],
                        egui::Button::selectable(i == sel, (o.as_str(), egui::Atom::grow())),
                    );
                    if i == sel {
                        resp.scroll_to_me(None);
                    }
                    if resp.clicked() {
                        elegido = Some(o.clone());
                    }
                }
            });
        ui.add_space(4.0);
        ui.colored_label(theme::TEXTO_TENUE, "↑↓ mover · ↵ elegir · esc cerrar");
    });

    if entrar {
        elegido = opciones.get(sel).cloned();
    }
    if let Some(o) = elegido {
        *accion = match modo {
            PickerModo::Contexto => Accion::Conectar(pane, o),
            PickerModo::Namespace if o == TODOS => Accion::CambiarNamespace(pane, None),
            PickerModo::Namespace => Accion::CambiarNamespace(pane, Some(o)),
        };
        cerrar = true;
    }
    if modal.should_close() || cerrar {
        app.picker = None;
    }
}

pub const TODOS: &str = "todos los namespaces";
