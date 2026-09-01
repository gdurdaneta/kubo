//! Tabla de recursos de un panel. Virtualizada: se dibujan solo las filas
//! visibles, así da igual que el namespace tenga 50 pods o 5.000.

use egui_extras::{Column, TableBuilder};

use super::Accion;
use crate::app::{App, Confirmacion, TabDetalle, Verbo};
use crate::columns::{self, ColSpec};
use crate::theme;

const ALTO_FILA: f32 = 24.0;
/// Ancho asumido para la columna elástica al medir si la tabla entra a lo ancho.
const ANCHO_ELASTICA: f32 = 220.0;
/// Hasta dónde se puede achicar una columna antes de dejar de servir.
const ANCHO_MIN_COL: f32 = 56.0;
/// La primera columna es el nombre: achicarla de más vuelve la tabla inútil.
const ANCHO_MIN_NOMBRE: f32 = 130.0;

/// Reparte el ancho disponible entre las columnas.
///
/// Los anchos declarados son los deseados, no un mínimo: si la ventana es más
/// angosta se encogen proporcionalmente hasta su piso. Solo cuando ni con los
/// pisos entra se cae al scroll horizontal — antes se iba a scroll siempre, y
/// al achicar la ventana las columnas ni se enteraban.
fn repartir_anchos(cabeceras: &[ColSpec], disponible: f32, sep: f32) -> (Vec<f32>, bool) {
    let huecos = sep * cabeceras.len() as f32;
    let deseados: Vec<f32> = cabeceras
        .iter()
        .map(|c| c.width.unwrap_or(ANCHO_ELASTICA))
        .collect();
    let natural: f32 = deseados.iter().sum();
    if natural + huecos <= disponible {
        return (deseados, false);
    }

    let piso = |i: usize| if i == 0 { ANCHO_MIN_NOMBRE } else { ANCHO_MIN_COL };
    let minimo: f32 = (0..deseados.len()).map(piso).sum();
    if minimo + huecos > disponible {
        // Ni encogidas al máximo entran: se scrollea.
        return ((0..deseados.len()).map(piso).collect(), true);
    }

    // Encoge proporcionalmente y reparte entre las que todavía tienen margen
    // lo que las que tocaron su piso no pudieron ceder.
    let objetivo = disponible - huecos;
    let mut anchos = deseados.clone();
    for _ in 0..8 {
        let total: f32 = anchos.iter().sum();
        let sobra = total - objetivo;
        if sobra <= 0.5 {
            break;
        }
        let elasticas: f32 = anchos
            .iter()
            .enumerate()
            .map(|(i, w)| (w - piso(i)).max(0.0))
            .sum();
        if elasticas <= 0.5 {
            break;
        }
        for (i, w) in anchos.iter_mut().enumerate() {
            let margen = (*w - piso(i)).max(0.0);
            *w -= sobra * (margen / elasticas);
            *w = w.max(piso(i));
        }
    }
    (anchos, false)
}

/// Kinds con pestaña "Mapa" (misma lista que usa el detalle).
fn tiene_mapa(kind: &str) -> bool {
    matches!(
        kind,
        "Service"
            | "Deployment"
            | "StatefulSet"
            | "DaemonSet"
            | "ReplicaSet"
            | "CronJob"
            | "Job"
            | "Pod"
    )
}

/// Kinds que soportan escalar y rollout restart.
fn escalable(kind: &str) -> bool {
    matches!(kind, "Deployment" | "StatefulSet" | "ReplicaSet" | "ReplicationController")
}
fn reiniciable(kind: &str) -> bool {
    matches!(kind, "Deployment" | "StatefulSet" | "DaemonSet" | "Pod")
}

/// Id del campo de búsqueda de un panel, para poder enfocarlo desde afuera
/// (escribir en cualquier lado cae acá).
pub fn id_busqueda(pane_id: u64) -> egui::Id {
    egui::Id::new(("busqueda_tabla", pane_id))
}

pub fn dibujar(app: &mut App, ui: &mut egui::Ui, id: u64, accion: &mut Accion) {
    // Las celdas son objetivos de clic, no texto para seleccionar: arrastrando
    // sobre la tabla se pintaba media pantalla de azul. En el detalle sí queda
    // seleccionable, que ahí uno copia UIDs e IPs.
    ui.style_mut().interaction.selectable_labels = false;

    let Some(pane) = app.panes.iter_mut().find(|p| p.id == id) else {
        return;
    };
    let Some(item) = pane.item.clone() else {
        super::centrado(ui, "Elegí un recurso en la barra lateral", theme::TEXTO_TENUE);
        return;
    };

    // ---- barra de herramientas del panel --------------------------------
    let (vis, tot) = match pane.store.as_ref() {
        Some(s) => (s.visibles(), s.total()),
        None => (0, 0),
    };
    egui::Frame::new()
        .fill(theme::PANEL)
        .inner_margin(egui::Margin::symmetric(8, 5))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(&item.label).strong().size(14.0));
                ui.colored_label(
                    theme::TEXTO_TENUE,
                    if vis == tot {
                        format!("{tot}")
                    } else {
                        format!("{vis} / {tot}")
                    },
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut pane.busqueda)
                            .id(id_busqueda(id))
                            .hint_text("buscar")
                            .desired_width(180.0),
                    );
                    if let Some(store) = pane.store.as_mut() {
                        selector_estado(ui, store, id);
                    }
                });
            });
        });

    let busqueda = pane.busqueda.clone();
    let mostrar_ns = item.res.namespaced && pane.ns_sel.is_none();
    let sel_key = pane.detalle.as_ref().map(|d| d.key.clone());
    let kind = item.res.ar.kind.clone();

    // Préstamos disjuntos: `endpoints` y `store` son campos distintos del panel.
    // Antes esto sacaba el mapa con `mem::take` y lo devolvía al final, pero los
    // `return` de abajo (mientras la tabla carga o está vacía) se lo comían: si
    // los endpoints llegaban antes que las filas, la columna quedaba en `—`
    // para siempre, porque el watch solo reenvía cuando algo cambia.
    let endpoints = columns::tiene_endpoints(&kind).then_some(&pane.endpoints);
    let col_estado = columns::indice_estado(&kind, mostrar_ns);
    let Some(store) = pane.store.as_mut() else { return };
    store.set_col_estado(col_estado);
    store.set_filtro(&busqueda);
    store.refrescar();

    if let Some(err) = store.error.clone() {
        ui.horizontal(|ui| {
            ui.colored_label(theme::BAD, "⚠");
            ui.colored_label(theme::BAD, err);
        });
    }

    let filas = store.visibles();
    if store.cargando && filas == 0 {
        ui.vertical_centered(|ui| {
            ui.add_space(60.0);
            ui.spinner();
        });
        return;
    }
    if filas == 0 {
        let msg = if *store.filtro_estado() != crate::store::FiltroEstado::Todos {
            "nada con ese estado"
        } else if !busqueda.is_empty() {
            "nada coincide con la búsqueda"
        } else {
            "sin recursos"
        };
        super::centrado(ui, msg, theme::TEXTO_TENUE);
        return;
    }

    let cabeceras = columns::headers(&kind, mostrar_ns);
    let sep = ui.spacing().item_spacing.x;
    let (anchos, scrollear) = repartir_anchos(&cabeceras, ui.available_width(), sep);
    let ancho_pedido: f32 = anchos.iter().sum::<f32>() + sep * cabeceras.len() as f32;

    // Con el detalle abierto (o en paneles muy angostos) ni encogidas entran;
    // sin esto las últimas columnas quedan cortadas e inalcanzables.
    if scrollear {
        egui::ScrollArea::horizontal()
            .id_salt(("tabla_h", id))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_min_width(ancho_pedido);
                cuerpo(
                    ui, store, &cabeceras, &anchos, filas, &sel_key, &kind, id, endpoints,
                    accion,
                );
            });
    } else {
        cuerpo(
            ui, store, &cabeceras, &anchos, filas, &sel_key, &kind, id, endpoints, accion,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn cuerpo(
    ui: &mut egui::Ui,
    store: &mut crate::store::Store,
    cabeceras: &[ColSpec],
    anchos: &[f32],
    filas: usize,
    sel_key: &Option<String>,
    kind: &str,
    pane_id: u64,
    endpoints: Option<&std::collections::HashMap<String, crate::k8s::endpoints::Conteo>>,
    accion: &mut Accion,
) {
    let es_pod = kind == "Pod";
    let sort_col = store.sort_col;
    let sort_desc = store.sort_desc;
    let mut nuevo_sort: Option<usize> = None;

    // El id lleva el kind y el ancho: egui_extras guarda los anchos de columna
    // y, con `resizable`, los conserva para siempre. Sin esto los de Pods se
    // colaban en Nodes, y al achicar la ventana las columnas no se enteraban.
    // Con el ancho en cuartos, arrastrar una columna se mantiene mientras la
    // ventana no cambie de tamaño, pero un resize sí rearma el reparto.
    let salt = (kind, cabeceras.len(), (anchos.iter().sum::<f32>() / 24.0) as i32);
    let mut builder = TableBuilder::new(ui)
        .id_salt(salt)
        .striped(true)
        .resizable(true)
        .sense(egui::Sense::click())
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .min_scrolled_height(0.0)
        .auto_shrink([false, false]);

    for (i, _) in cabeceras.iter().enumerate() {
        let w = anchos.get(i).copied().unwrap_or(ANCHO_ELASTICA);
        builder = builder.column(Column::initial(w).at_least(40.0).clip(true));
    }

    builder
        .header(26.0, |mut header| {
            for (i, c) in cabeceras.iter().enumerate() {
                header.col(|ui| {
                    let flecha = if i == sort_col {
                        if sort_desc { " ▼" } else { " ▲" }
                    } else {
                        ""
                    };
                    let resp = ui.add(
                        egui::Label::new(
                            egui::RichText::new(format!("{}{flecha}", c.title))
                                .size(11.5)
                                .color(if i == sort_col { theme::TEXTO } else { theme::TEXTO_TENUE }),
                        )
                        .sense(egui::Sense::click()),
                    );
                    if resp.clicked() {
                        nuevo_sort = Some(i);
                    }
                });
            }
        })
        .body(|body| {
            body.rows(ALTO_FILA, filas, |mut row| {
                let idx = row.index();
                let Some((key, celdas, creado)) = store.fila(idx) else {
                    return;
                };
                let key = key.to_string();
                row.set_selected(sel_key.as_deref() == Some(key.as_str()));

                for celda in celdas {
                    row.col(|ui| {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(&celda.text)
                                    .color(theme::color_tono(celda.tone)),
                            )
                            .truncate(),
                        );
                    });
                }
                if let Some(eps) = endpoints {
                    row.col(|ui| celda_endpoints(ui, eps.get(&key)));
                }
                row.col(|ui| {
                    ui.colored_label(theme::TEXTO_TENUE, columns::edad(creado));
                });

                let resp = row.response();
                if resp.clicked() {
                    *accion = Accion::AbrirDetalle(pane_id, key.clone());
                }
                if es_pod && resp.double_clicked() {
                    *accion = Accion::AbrirLogs(pane_id, key.clone());
                }
                let (ns, nombre) = partir_key(&key);
                resp.context_menu(|ui| {
                    menu_acciones(ui, pane_id, kind, &key, ns.clone(), nombre.clone(), accion);
                });
            });
        });

    if let Some(c) = nuevo_sort {
        // El reordenamiento se aplica con la tabla ya dibujada: sin repintar,
        // el clic en la cabecera no se vería hasta mover el mouse.
        store.set_sort(c);
        ui.ctx().request_repaint();
    }
}

/// Selector de estado: "todos", "con problemas" y cada valor presente.
///
/// Los valores salen de lo que hay cargado, no de una lista fija: así sirve
/// igual para Pods que para un CRD con estados propios.
fn selector_estado(ui: &mut egui::Ui, store: &mut crate::store::Store, id: u64) {
    use crate::store::FiltroEstado;

    let estados = store.estados();
    if estados.is_empty() {
        return;
    }
    let actual = store.filtro_estado().clone();
    let (texto, color) = match &actual {
        FiltroEstado::Todos => ("estado: todos".to_string(), theme::TEXTO_TENUE),
        FiltroEstado::Problemas => ("estado: con problemas".to_string(), theme::WARN),
        FiltroEstado::Valor(v) => (format!("estado: {v}"), theme::ACENTO),
    };
    let mut nuevo: Option<FiltroEstado> = None;

    egui::ComboBox::from_id_salt(("estado", id))
        .selected_text(egui::RichText::new(texto).color(color))
        .width(190.0)
        .show_ui(ui, |ui| {
            let total: usize = estados.iter().map(|(_, n)| n).sum();
            if ui
                .selectable_label(actual == FiltroEstado::Todos, format!("todos  ({total})"))
                .clicked()
            {
                nuevo = Some(FiltroEstado::Todos);
            }
            let con_problemas: usize = estados
                .iter()
                .filter(|(e, _)| !crate::store::estado_sano(e))
                .map(|(_, n)| n)
                .sum();
            if con_problemas > 0
                && ui
                    .selectable_label(
                        actual == FiltroEstado::Problemas,
                        egui::RichText::new(format!("con problemas  ({con_problemas})"))
                            .color(theme::WARN),
                    )
                    .clicked()
            {
                nuevo = Some(FiltroEstado::Problemas);
            }
            ui.separator();
            for (e, n) in &estados {
                let sel = actual == FiltroEstado::Valor(e.clone());
                let color = if crate::store::estado_sano(e) {
                    theme::TEXTO
                } else {
                    theme::BAD
                };
                if ui
                    .selectable_label(
                        sel,
                        egui::RichText::new(format!("{e}  ({n})")).color(color),
                    )
                    .clicked()
                {
                    nuevo = Some(if sel {
                        FiltroEstado::Todos
                    } else {
                        FiltroEstado::Valor(e.clone())
                    });
                }
            }
        });

    if let Some(f) = nuevo {
        store.set_filtro_estado(f);
        ui.ctx().request_repaint();
    }
}

/// Backends de un Service. Sin datos todavía se deja en blanco en vez de
/// mostrar un 0 que se leería como "no tiene".
fn celda_endpoints(ui: &mut egui::Ui, c: Option<&crate::k8s::endpoints::Conteo>) {
    let Some(c) = c else {
        ui.colored_label(theme::TEXTO_TENUE, "—")
            .on_hover_text("sin endpoints para este Service");
        return;
    };
    let (texto, color, ayuda) = if c.total == 0 {
        ("0".to_string(), theme::BAD, "ningún backend: el Service no resuelve a nada")
    } else if c.listos == 0 {
        (
            format!("0 / {}", c.total),
            theme::BAD,
            "hay backends pero ninguno está Ready",
        )
    } else if c.listos < c.total {
        (
            format!("{} / {}", c.listos, c.total),
            theme::WARN,
            "algunos backends no están Ready",
        )
    } else {
        (c.listos.to_string(), theme::OK, "todos los backends Ready")
    };
    ui.colored_label(color, texto).on_hover_text(ayuda);
}

fn partir_key(key: &str) -> (Option<String>, String) {
    match key.split_once('/') {
        Some((ns, n)) => (Some(ns.to_string()), n.to_string()),
        None => (None, key.to_string()),
    }
}

/// Menú contextual con las acciones del Kind. También lo usa el detalle.
pub fn menu_acciones(
    ui: &mut egui::Ui,
    pane_id: u64,
    kind: &str,
    key: &str,
    ns: Option<String>,
    nombre: String,
    accion: &mut Accion,
) {
    if ui.button("Ver detalle").clicked() {
        *accion = Accion::AbrirDetalle(pane_id, key.to_string());
        ui.close();
    }
    if ui
        .button("✎ Editar manifiesto…")
        .on_hover_text("Abre el YAML del objeto para editarlo y aplicarlo")
        .clicked()
    {
        *accion = Accion::EditarManifiesto(pane_id, key.to_string());
        ui.close();
    }
    if kind == "Pod" {
        if ui.button("Logs").clicked() {
            *accion = Accion::AbrirLogs(pane_id, key.to_string());
            ui.close();
        }
        if ui.button("Shell").clicked() {
            *accion = Accion::AbrirShell(pane_id, key.to_string());
            ui.close();
        }
    }
    if kind == "Service"
        && ui
            .button("⇄ Port forward…")
            .on_hover_text("Exponerlo en local")
            .clicked()
    {
        *accion = Accion::PedirForward(pane_id, key.to_string());
        ui.close();
    }
    if tiene_mapa(kind) && ui.button("Mapa").clicked() {
        *accion = Accion::AbrirDetalleTab(pane_id, key.to_string(), TabDetalle::Mapa);
        ui.close();
    }
    ui.separator();
    if escalable(kind) && ui.button("Escalar…").clicked() {
        *accion = Accion::Confirmar(Confirmacion {
            pane: pane_id,
            verbo: Verbo::Escalar(-1), // -1 = precargar réplicas actuales
            kind: kind.to_string(),
            ns: ns.clone(),
            name: nombre.clone(),
        });
        ui.close();
    }
    if reiniciable(kind) && ui.button("Reiniciar").clicked() {
        *accion = Accion::Confirmar(Confirmacion {
            pane: pane_id,
            verbo: Verbo::Reiniciar,
            kind: kind.to_string(),
            ns: ns.clone(),
            name: nombre.clone(),
        });
        ui.close();
    }
    if ui
        .button(egui::RichText::new("Borrar").color(theme::BAD))
        .clicked()
    {
        *accion = Accion::Confirmar(Confirmacion {
            pane: pane_id,
            verbo: Verbo::Borrar,
            kind: kind.to_string(),
            ns,
            name: nombre,
        });
        ui.close();
    }
    ui.separator();
    if ui.button("Copiar nombre").clicked() {
        let n = key.rsplit('/').next().unwrap_or(key).to_string();
        ui.ctx().copy_text(n);
        ui.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cols(anchos: &[f32]) -> Vec<ColSpec> {
        anchos
            .iter()
            .map(|w| ColSpec {
                title: "x",
                width: Some(*w),
            })
            .collect()
    }

    /// Las columnas de un Node: son todas de ancho fijo y suman más que una
    /// ventana angosta, que es donde se rompía.
    const NODE: &[f32] = &[280.0, 110.0, 130.0, 110.0, 130.0, 70.0];

    #[test]
    fn con_espacio_de_sobra_respeta_los_anchos_pedidos() {
        let (anchos, scroll) = repartir_anchos(&cols(NODE), 2000.0, 8.0);
        assert!(!scroll);
        assert_eq!(anchos, NODE);
    }

    #[test]
    fn al_achicarse_encoge_en_vez_de_scrollear() {
        let disponible = 750.0;
        let (anchos, scroll) = repartir_anchos(&cols(NODE), disponible, 8.0);
        assert!(!scroll, "todavía entra encogiendo, no hay que scrollear");
        let total: f32 = anchos.iter().sum::<f32>() + 8.0 * NODE.len() as f32;
        assert!(
            total <= disponible + 0.5,
            "la tabla sigue desbordando: {total} > {disponible}"
        );
        // La columna del nombre cede más que las chicas, pero no baja del piso.
        assert!(anchos[0] >= ANCHO_MIN_NOMBRE);
        assert!(anchos[0] < NODE[0]);
        assert!(anchos.iter().skip(1).all(|w| *w >= ANCHO_MIN_COL));
    }

    #[test]
    fn si_ni_encogidas_entran_avisa_que_hay_que_scrollear() {
        let (anchos, scroll) = repartir_anchos(&cols(NODE), 200.0, 8.0);
        assert!(scroll);
        assert_eq!(anchos[0], ANCHO_MIN_NOMBRE);
        assert!(anchos.iter().skip(1).all(|w| *w == ANCHO_MIN_COL));
    }

    #[test]
    fn nunca_devuelve_anchos_negativos_ni_nan() {
        for d in [0.0, 1.0, 50.0, 300.0, 900.0, 5000.0] {
            let (anchos, _) = repartir_anchos(&cols(NODE), d, 8.0);
            assert!(
                anchos.iter().all(|w| w.is_finite() && *w > 0.0),
                "ancho inválido con disponible={d}: {anchos:?}"
            );
        }
    }
}
