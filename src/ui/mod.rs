//! Composición de la ventana: barra de app + N paneles lado a lado, cada uno
//! con su propio sidebar, tabla, detalle y franja inferior (logs o shell).
//! Cada panel dibuja y devuelve acciones; el estado se muta después.

mod atajos;
mod confirm;
mod detail;
mod forward;
mod logs;
mod map;
mod palette;
mod picker;
mod sidebar;
mod table;
mod term;
mod topbar;

use crate::app::{App, Bottom, Conn, MAX_PANES};
use crate::theme;

/// Lo que el usuario pidió durante este frame, siempre referido a un panel.
pub enum Accion {
    Ninguna,
    AbrirPane,
    CerrarPane(u64),
    Conectar(u64, String),
    Reconectar(String),
    Seleccionar(u64, crate::nav::NavItem),
    CambiarNamespace(u64, Option<String>),
    AbrirDetalle(u64, String),
    /// Abrir el detalle aterrizando en una pestaña concreta (editar YAML, mapa).
    AbrirDetalleTab(u64, String, crate::app::TabDetalle),
    /// Abrir el detalle en el YAML y ya en modo edición.
    EditarManifiesto(u64, String),
    CerrarDetalle(u64),
    AbrirLogs(u64, String),
    AbrirShell(u64, String),
    CerrarBottom(u64),
    ReiniciarLogs(u64),
    Refrescar(u64),
    PedirMapa(u64),
    /// Navegar a un recurso concreto: (panel, kind, ns, nombre).
    IrA(u64, String, Option<String>, String),
    AplicarYaml(u64, String),
    AlternarRevelar(u64),
    /// Abrir el diálogo de port-forward de un Service.
    PedirForward(u64, String),
    AbrirForward,
    CerrarForward(u64),
    /// Abrir una vista local (port-forwards) en el panel.
    VerVistaLocal(u64, crate::nav::VistaLocal),
    Confirmar(crate::app::Confirmacion),
}

/// Panel al que se refiere la acción, si es que apunta a alguno.
fn pane_de(a: &Accion) -> Option<u64> {
    match a {
        Accion::CerrarPane(id)
        | Accion::Conectar(id, _)
        | Accion::Seleccionar(id, _)
        | Accion::CambiarNamespace(id, _)
        | Accion::AbrirDetalle(id, _)
        | Accion::AbrirDetalleTab(id, _, _)
        | Accion::EditarManifiesto(id, _)
        | Accion::CerrarDetalle(id)
        | Accion::AbrirLogs(id, _)
        | Accion::AbrirShell(id, _)
        | Accion::CerrarBottom(id)
        | Accion::ReiniciarLogs(id)
        | Accion::Refrescar(id)
        | Accion::PedirMapa(id)
        | Accion::IrA(id, _, _, _)
        | Accion::AplicarYaml(id, _)
        | Accion::AlternarRevelar(id)
        | Accion::PedirForward(id, _)
        | Accion::VerVistaLocal(id, _) => Some(*id),
        Accion::Confirmar(c) => Some(c.pane),
        Accion::Ninguna
        | Accion::AbrirPane
        | Accion::Reconectar(_)
        | Accion::AbrirForward
        | Accion::CerrarForward(_) => None,
    }
}

/// Lo que se le reserva a la tabla cuando el detalle está abierto.
const ANCHO_MIN_TABLA: f32 = 180.0;
/// Menos que esto el detalle no se lee.
const ANCHO_MIN_DETALLE: f32 = 280.0;

fn marco(fondo: egui::Color32, margen: i8) -> egui::Frame {
    egui::Frame::new().fill(fondo).inner_margin(margen)
}

pub fn dibujar(app: &mut App, ui: &mut egui::Ui) {
    let mut accion = Accion::Ninguna;

    let mut ver_atajos = app.ver_atajos;
    barra_app(app, ui, &mut accion, &mut ver_atajos);
    app.ver_atajos = ver_atajos;

    // Escribir en cualquier lado cae en el buscador del panel activo, como en
    // Lens. Solo si no hay nada más con foco (ni la shell, ni el editor de
    // YAML, ni el filtro de la nav) y sin modales abiertos; se pide el foco
    // antes de dibujar el campo para que el TextEdit consuma las teclas en
    // este mismo frame y no se pierda el primer carácter.
    if app.palette.is_none()
        && app.confirm.is_none()
        && app.dialogo_pf.is_none()
        && app.picker.is_none()
        && !app.ver_atajos
    {
        let sin_foco = ui.ctx().memory(|m| m.focused().is_none());
        let hay_texto = ui.ctx().input(|i| {
            i.events.iter().any(|e| match e {
                egui::Event::Text(t) => !t.trim().is_empty(),
                _ => false,
            })
        });
        if sin_foco && hay_texto {
            if let Some(activo) = app.pane_activo() {
                ui.ctx()
                    .memory_mut(|m| m.request_focus(table::id_busqueda(activo)));
            }
        }
    }

    // Los paneles se reparten el ancho en partes iguales.
    let n = app.panes.len().max(1);
    let ids: Vec<u64> = app.panes.iter().map(|p| p.id).collect();
    ui.columns(n, |cols| {
        for (i, id) in ids.iter().enumerate() {
            let ui = &mut cols[i];
            egui::Frame::new()
                .fill(theme::FONDO)
                .stroke(egui::Stroke::new(1.0, theme::BORDE))
                .show(ui, |ui| {
                    dibujar_pane(app, ui, *id, n, &mut accion);
                });
        }
    });

    // Ctrl+K abre la paleta sobre el último panel usado.
    if ui.ctx().input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::K)) {
        if app.palette.is_some() {
            app.palette = None;
        } else if let Some(id) = app.pane_activo() {
            app.abrir_palette(id);
        }
        ui.ctx().request_repaint();
    }
    forward::dialogo(app, ui.ctx(), &mut accion);

    // Un modal que pide una decisión (confirmar una acción destructiva,
    // configurar un forward) se queda con el teclado: abrir otro panel o el
    // selector de cluster por encima solo confunde.
    let modal_abierto = app.confirm.is_some() || app.dialogo_pf.is_some();

    // Ctrl+T abre otro panel. Se consume después de dibujar los paneles, así
    // que si la shell embebida tiene el foco la tecla es suya y no llega acá.
    if !modal_abierto
        && ui
        .ctx()
        .input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::T))
    {
        if app.panes.len() < MAX_PANES {
            accion = Accion::AbrirPane;
        } else {
            app.toast(format!("ya hay {MAX_PANES} paneles abiertos"), true);
        }
        ui.ctx().request_repaint();
    }

    // Ctrl+P cambia de cluster, Ctrl+N de namespace, sobre el panel activo.
    for (tecla, modo) in [
        (egui::Key::P, crate::app::PickerModo::Contexto),
        (egui::Key::N, crate::app::PickerModo::Namespace),
    ] {
        if !modal_abierto
            && ui
                .ctx()
                .input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, tecla))
        {
            match app.picker.as_ref().map(|p| p.modo) {
                // La misma tecla otra vez cierra; la otra cambia de modo.
                Some(m) if m == modo => app.picker = None,
                _ => {
                    if let Some(id) = app.pane_activo() {
                        app.abrir_picker(id, modo);
                    }
                }
            }
            ui.ctx().request_repaint();
        }
    }
    // F1 (y Ctrl+/) muestran la ayuda de atajos. `?` no se usa: sin foco,
    // cualquier carácter cae en el buscador de la tabla y se lo comería.
    if ui.ctx().input_mut(|i| {
        i.consume_key(egui::Modifiers::NONE, egui::Key::F1)
            || i.consume_key(egui::Modifiers::COMMAND, egui::Key::Slash)
    }) {
        app.ver_atajos = !app.ver_atajos;
        ui.ctx().request_repaint();
    }
    atajos::dibujar(app, ui.ctx());
    picker::dibujar(app, ui.ctx(), &mut accion);
    palette::dibujar(app, ui.ctx(), &mut accion);
    confirm::dibujar(app, ui.ctx(), &mut accion);
    toasts(app, ui.ctx());

    // El estado se muta recién al final del frame, así que lo que ya se pintó
    // muestra lo viejo. `clicked()` se dispara al soltar el botón —el último
    // evento de entrada—, de modo que sin pedir repintado explícito no hay más
    // frames hasta el tick de 1 s o hasta que el mouse se mueva: el clic parece
    // no hacer nada.
    if !matches!(accion, Accion::Ninguna) {
        ui.ctx().request_repaint();
    }
    if let Some(p) = pane_de(&accion) {
        app.pane_activo = p;
    }

    // Red de seguridad para lo que se muta durante el dibujo y recién se
    // recalcula en el `logic()` siguiente (orden, filtro).
    if app
        .panes
        .iter()
        .any(|p| p.store.as_ref().is_some_and(|s| s.sucio()))
    {
        ui.ctx().request_repaint();
    }

    match accion {
        Accion::Ninguna => {}
        Accion::AbrirPane => app.abrir_pane(),
        Accion::CerrarPane(id) => app.cerrar_pane(id),
        Accion::Conectar(id, c) => app.cambiar_contexto(id, c),
        Accion::Reconectar(c) => app.reconectar(&c),
        Accion::Seleccionar(id, i) => app.seleccionar(id, i),
        Accion::CambiarNamespace(id, ns) => app.cambiar_namespace(id, ns),
        Accion::AbrirDetalle(id, k) => app.abrir_detalle(id, &k),
        Accion::AbrirDetalleTab(id, k, t) => app.abrir_detalle_en(id, &k, t),
        Accion::EditarManifiesto(id, k) => {
            app.abrir_detalle_en(id, &k, crate::app::TabDetalle::Yaml);
            if let Some(d) = app.pane(id).and_then(|p| p.detalle.as_mut()) {
                d.editando = true;
            }
        }
        Accion::CerrarDetalle(id) => {
            if let Some(p) = app.pane(id) {
                p.detalle = None;
            }
        }
        Accion::AbrirLogs(id, k) => app.abrir_logs(id, &k),
        Accion::AbrirShell(id, k) => app.abrir_shell(id, &k),
        Accion::CerrarBottom(id) => {
            if let Some(p) = app.pane(id) {
                p.cerrar_bottom();
            }
        }
        Accion::ReiniciarLogs(id) => app.reiniciar_logs(id),
        Accion::Refrescar(id) => app.refrescar(id),
        Accion::PedirMapa(id) => app.pedir_mapa(id),
        Accion::IrA(id, kind, ns, name) => app.ir_a(id, &kind, ns, &name),
        Accion::AplicarYaml(id, y) => app.aplicar_yaml(id, y),
        Accion::AlternarRevelar(id) => app.alternar_revelar(id),
        Accion::PedirForward(id, k) => app.pedir_forward(id, &k),
        Accion::AbrirForward => app.abrir_forward(),
        Accion::CerrarForward(id) => app.cerrar_forward(id),
        Accion::VerVistaLocal(id, v) => app.ver_vista_local(id, v),
        Accion::Confirmar(c) => app.confirm = Some(c),
    }
}

/// Barra superior de la aplicación: nombre + gestor de paneles.
fn barra_app(app: &App, ui: &mut egui::Ui, accion: &mut Accion, ver_atajos: &mut bool) {
    egui::Panel::top("barra_app")
        .exact_size(30.0)
        .frame(marco(theme::PANEL_ALT, 4))
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.label(egui::RichText::new("kubo").strong().color(theme::ACENTO));
                ui.separator();
                if app.panes.len() < MAX_PANES
                    && ui
                        .button("+ panel")
                        .on_hover_text("Abrir otro panel (otro cluster u otro recurso)")
                        .clicked()
                {
                    *accion = Accion::AbrirPane;
                }
                ui.colored_label(
                    theme::TEXTO_TENUE,
                    format!("{} de {MAX_PANES}", app.panes.len()),
                );
                ui.separator();
                let n_pf = app.forwards.len();
                let activos = app
                    .forwards
                    .iter()
                    .filter(|f| f.estado == crate::app::EstadoPf::Activo)
                    .count();
                let etiqueta = if n_pf == 0 {
                    "⇄ forwards".to_string()
                } else {
                    format!("⇄ {activos}/{n_pf}")
                };
                if ui
                    .button(egui::RichText::new(etiqueta).color(if n_pf > 0 {
                        theme::ACENTO
                    } else {
                        theme::TEXTO_TENUE
                    }))
                    .on_hover_text("Port forwards (también están en Network)")
                    .clicked()
                {
                    if let Some(p) = app.pane_activo() {
                        *accion = Accion::VerVistaLocal(p, crate::nav::VistaLocal::PortForwards);
                    }
                }
                ui.separator();
                if ui
                    .button(egui::RichText::new("⌨ atajos").color(theme::TEXTO_TENUE))
                    .on_hover_text("Atajos de teclado (F1)")
                    .clicked()
                {
                    *ver_atajos = true;
                }
            });
        });
}

fn dibujar_pane(app: &mut App, ui: &mut egui::Ui, id: u64, n_panes: usize, accion: &mut Accion) {
    topbar::dibujar(app, ui, id, n_panes, accion);

    let (conn, error) = {
        let Some(pane) = app.panes.iter().find(|p| p.id == id) else {
            return;
        };
        match app.cluster_de(pane) {
            Some(c) => (Some(c.conn), c.error.clone()),
            None => (None, None),
        }
    };

    match conn {
        Some(Conn::Lista) => {}
        Some(Conn::Conectando) => {
            ui.vertical_centered(|ui| {
                ui.add_space(100.0);
                ui.spinner();
                ui.colored_label(theme::TEXTO_TENUE, "Conectando…");
            });
            return;
        }
        Some(Conn::Error) => {
            let ctx = app
                .panes
                .iter()
                .find(|p| p.id == id)
                .and_then(|p| p.contexto.clone());
            ui.vertical_centered(|ui| {
                ui.add_space(100.0);
                ui.colored_label(theme::BAD, "No se pudo conectar");
                ui.add_space(4.0);
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                ui.set_max_width(500.0);
                ui.colored_label(theme::TEXTO_TENUE, error.unwrap_or_default());
                ui.add_space(8.0);
                if let Some(c) = ctx {
                    if ui.button("Reintentar").clicked() {
                        *accion = Accion::Reconectar(c);
                    }
                }
            });
            return;
        }
        None => {
            centrado(ui, "Elegí un contexto arriba", theme::TEXTO_TENUE);
            return;
        }
    }

    // Sidebar (colapsable: en paneles angostos molesta).
    let nav_visible = app
        .panes
        .iter()
        .find(|p| p.id == id)
        .map(|p| p.nav_visible)
        .unwrap_or(true);
    if nav_visible {
        egui::Panel::left(egui::Id::new(("nav", id)))
            .resizable(true)
            .default_size(210.0)
            .size_range(150.0..=400.0)
            .frame(marco(theme::PANEL, 6))
            .show(ui, |ui| sidebar::dibujar(app, ui, id, accion));
    }

    // Franja inferior: logs o shell.
    let bottom_tipo = app
        .panes
        .iter()
        .find(|p| p.id == id)
        .and_then(|p| p.bottom.as_ref())
        .map(|b| matches!(b, Bottom::Term(_)));
    if let Some(es_term) = bottom_tipo {
        egui::Panel::bottom(egui::Id::new(("bottom", id)))
            .resizable(true)
            .default_size(300.0)
            .size_range(120.0..=900.0)
            .frame(marco(theme::PANEL, 6))
            .show(ui, |ui| {
                if es_term {
                    term::dibujar(app, ui, id, accion);
                } else {
                    logs::dibujar(app, ui, id, accion);
                }
            });
    }

    // Detalle a la derecha, dentro del panel.
    let hay_detalle = app
        .panes
        .iter()
        .find(|p| p.id == id)
        .map(|p| p.detalle.is_some())
        .unwrap_or(false);
    let local = app
        .panes
        .iter()
        .find(|p| p.id == id)
        .and_then(|p| p.vista_local);

    // Tabla y detalle se reparten el área central a mano, en vez de usar un
    // `Panel::right`.
    //
    // egui maqueta el contenido de un panel lateral con el ancho
    // `size_range.max` y recién después lo recorta al ancho real, que sale de
    // un estado guardado. Cuando los dos no coinciden —pasa apenas cambia el
    // tamaño de la ventana— el contenido queda arrancando a la izquierda del
    // panel visible: se veía media palabra de cada línea. Partiendo el espacio
    // acá los dos anchos son el mismo por construcción.
    egui::CentralPanel::no_frame()
        .frame(marco(theme::FONDO, 0))
        .show(ui, |ui| {
            let total = ui.available_width();
            let alto = ui.available_height();
            let sep = ui.spacing().item_spacing.x;
            // En una ventana angosta no entran los dos cómodos. Antes de dejar
            // el detalle inservible se prefiere apretar la tabla, que además
            // sabe scrollear a lo ancho.
            let ancho_detalle = if hay_detalle {
                (total * 0.45)
                    .clamp(ANCHO_MIN_DETALLE, 620.0)
                    .min((total - ANCHO_MIN_TABLA - sep).max(ANCHO_MIN_DETALLE))
                    .min(total - sep)
                    .max(0.0)
            } else {
                0.0
            };
            let ancho_tabla = (total - ancho_detalle - if hay_detalle { sep } else { 0.0 }).max(0.0);

            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(ancho_tabla, alto),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_min_size(egui::vec2(ancho_tabla, alto));
                        ui.set_max_width(ancho_tabla);
                        ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
                        match local {
                            Some(crate::nav::VistaLocal::PortForwards) => {
                                forward::vista(app, ui, id, accion)
                            }
                            None => table::dibujar(app, ui, id, accion),
                        }
                    },
                );

                if hay_detalle && ancho_detalle > 0.0 {
                    ui.allocate_ui_with_layout(
                        egui::vec2(ancho_detalle, alto),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_min_size(egui::vec2(ancho_detalle, alto));
                            ui.set_max_width(ancho_detalle);
                            // Red de seguridad: si algún widget se mide mal, se
                            // recorta acá en vez de pintar encima de la tabla.
                            ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
                            egui::Frame::new()
                                .fill(theme::PANEL)
                                .inner_margin(8)
                                .show(ui, |ui| {
                                    ui.set_min_size(egui::vec2(ancho_detalle - 16.0, alto - 16.0));
                                    ui.set_max_width(ancho_detalle - 16.0);
                                    detail::dibujar(app, ui, id, ancho_detalle - 16.0, accion);
                                });
                        },
                    );
                }
            });
        });
}

pub fn centrado(ui: &mut egui::Ui, texto: &str, color: egui::Color32) {
    ui.vertical_centered(|ui| {
        ui.add_space(120.0);
        ui.colored_label(color, texto);
    });
}

fn toasts(app: &mut App, ctx: &egui::Context) {
    if app.toasts.is_empty() {
        return;
    }
    let pendientes: Vec<(String, bool)> = app
        .toasts
        .iter()
        .rev()
        .take(4)
        .map(|(t, e, _)| (t.clone(), *e))
        .collect();

    egui::Area::new(egui::Id::new("toasts"))
        .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-16.0, -16.0))
        .interactable(false)
        .show(ctx, |ui| {
            for (texto, error) in pendientes {
                egui::Frame::new()
                    .fill(theme::PANEL_ALT)
                    .stroke(egui::Stroke::new(
                        1.0,
                        if error { theme::BAD } else { theme::BORDE },
                    ))
                    .corner_radius(6)
                    .inner_margin(8)
                    .show(ui, |ui| {
                        ui.set_max_width(420.0);
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                        ui.colored_label(if error { theme::BAD } else { theme::TEXTO }, texto);
                    });
            }
        });
}
