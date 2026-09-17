//! Panel de detalle: resumen legible, YAML aplicable y eventos del objeto.

use serde_json::Value;

use super::Accion;
use crate::app::{App, TabDetalle};
use crate::theme;

/// Kinds con pestaña "Mapa": Services y workloads (config del micro).
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

pub fn dibujar(app: &mut App, ui: &mut egui::Ui, id: u64, ancho: f32, accion: &mut Accion) {
    let permisos = app.permisos_del_pane(id).cloned();
    // Clava el contenido al ancho que eligió el panel. Leer `available_width`
    // no sirve: durante la pasada de medición no está acotado, y el contenido
    // terminaba maquetado más ancho que el panel.
    ui.set_max_width(ancho);

    let Some(pane) = app.panes.iter_mut().find(|p| p.id == id) else {
        return;
    };
    let Some(det) = pane.detalle.as_mut() else {
        return;
    };

    // ---- cabecera con acciones ------------------------------------------
    // Los botones se colocan primero, de derecha a izquierda; el título usa lo
    // que sobra y se trunca. Al revés (título primero) se desbordaba encima de
    // los botones en paneles angostos.
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 7.0;
            if ui.button("×").on_hover_text("Cerrar").clicked() {
                *accion = Accion::CerrarDetalle(id);
            }
            ui.menu_button("⋮", |ui| {
                crate::ui::table::menu_acciones(
                    ui,
                    id,
                    &det.kind.clone(),
                    &det.key.clone(),
                    det.ns.clone(),
                    det.name.clone(),
                    permisos.as_ref(),
                    accion,
                );
            })
            .response
            .on_hover_text("Acciones");
            // Barra de iconos como la de Lens, de derecha a izquierda. Borrar
            // no está acá a propósito: queda en ⋮, detrás de un clic más.
            let icono = |ui: &mut egui::Ui, glifo: &str, ayuda: &str, habilitado: bool| {
                ui.add_enabled(
                    habilitado,
                    egui::Button::new(egui::RichText::new(glifo).size(15.0)).frame(false),
                )
                .on_hover_text(ayuda)
                .on_disabled_hover_text("Tu credencial no puede hacer esto acá")
                .clicked()
            };
            let puede = |verbo: &str| permisos.as_ref().is_none_or(|p| !p.prohibido(verbo));
            let kind = det.kind.clone();
            let (ns, name, key) = (det.ns.clone(), det.name.clone(), det.key.clone());
            let confirmar = |verbo: crate::app::Verbo| {
                Accion::Confirmar(crate::app::Confirmacion {
                    pane: id,
                    verbo,
                    kind: kind.clone(),
                    ns: ns.clone(),
                    name: name.clone(),
                    diff: None,
                    tecleado: String::new(),
                    extra: Vec::new(),
                })
            };

            let ro = crate::app::solo_lectura();
            if !ro && icono(ui, "✎", "Editar el manifiesto YAML y aplicarlo", true) {
                // El YAML ya se pidió al abrir el detalle: basta con saltar a
                // la pestaña, y como el tab bar se dibuja después se ve en este
                // mismo frame. Si todavía no llegó la copia del API server, el
                // botón de la pestaña queda deshabilitado y el usuario lo ve ahí.
                det.tab = TabDetalle::Yaml;
                det.editando = det.yaml_fresco;
            }
            let es_workload = crate::ui::table::escalable(&kind) || kind == "DaemonSet";
            if kind == "Pod" {
                if !ro && icono(ui, ">_", "Shell en el pod", true) {
                    *accion = Accion::AbrirShell(id, key.clone());
                }
                if icono(ui, "≡", "Logs del pod", true) {
                    *accion = Accion::AbrirLogs(id, key.clone());
                }
            } else if es_workload {
                if !ro && icono(ui, ">_", "Shell en un pod del workload", true) {
                    *accion =
                        Accion::PodDeWorkload(id, key.clone(), crate::k8s::pods::QuePod::Shell);
                }
                if icono(ui, "≡", "Logs de un pod del workload", true) {
                    *accion =
                        Accion::PodDeWorkload(id, key.clone(), crate::k8s::pods::QuePod::Logs);
                }
            }
            if !ro && crate::ui::table::reiniciable(&kind) {
                let verbo = if kind == "Pod" { "delete" } else { "patch" };
                let ayuda = if kind == "Pod" {
                    "Reiniciar: borra el pod y su controlador lo recrea"
                } else {
                    "Rollout restart"
                };
                if icono(ui, "↻", ayuda, puede(verbo)) {
                    *accion = confirmar(crate::app::Verbo::Reiniciar);
                }
            }
            if !ro
                && crate::ui::table::escalable(&kind)
                && icono(ui, "⇅", "Escalar réplicas", puede("patch"))
            {
                *accion = confirmar(crate::app::Verbo::Escalar(-1));
            }
            if matches!(kind.as_str(), "Service" | "Pod")
                && icono(ui, "⇄", "Port-forward: exponerlo en local", true)
            {
                *accion = Accion::PedirForward(id, key.clone());
            }

            // Lo que queda a la izquierda de los botones.
            ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                ui.add(
                    egui::Label::new(egui::RichText::new(&det.name).strong().size(13.5)).truncate(),
                )
                .on_hover_text(&det.name);
                let sub = match &det.ns {
                    Some(ns) => format!("{}  ·  {ns}", det.kind),
                    None => det.kind.clone(),
                };
                ui.add(
                    egui::Label::new(egui::RichText::new(sub).color(theme::TEXTO_TENUE)).truncate(),
                );
            });
        });
    });

    ui.separator();
    let mut pedir_mapa = false;
    ui.horizontal(|ui| {
        for (tab, etiqueta) in [
            (TabDetalle::Resumen, "Resumen"),
            (TabDetalle::Yaml, "YAML"),
            (TabDetalle::Eventos, "Eventos"),
        ] {
            if ui.selectable_label(det.tab == tab, etiqueta).clicked() {
                det.tab = tab;
            }
        }
        if tiene_mapa(&det.kind)
            && ui
                .selectable_label(det.tab == TabDetalle::Mapa, "Mapa")
                .clicked()
        {
            det.tab = TabDetalle::Mapa;
            if det.mapa.is_none() {
                pedir_mapa = true;
            }
        }
    });
    ui.add_space(4.0);

    // ---- cuerpo por pestaña ---------------------------------------------
    let tab = det.tab;
    let key = det.key.clone();
    let kind = det.kind.clone();
    let ns_detalle = det.ns.clone();
    let eventos = det.eventos.clone();
    let eventos_pedidos = det.eventos_pedidos;
    let backends_lista = det.backends.clone();
    let backends_pedidos = det.backends_pedidos;
    // Historial de métricas del objeto abierto (Pods y Nodes), para el sparkline.
    let historial: Vec<crate::k8s::metricas::Uso> = pane
        .historial
        .por_clave
        .get(&key)
        .map(|h| h.iter().copied().collect())
        .unwrap_or_default();

    match tab {
        TabDetalle::Yaml => {
            // Solo lectura por defecto. El buffer editable existe únicamente
            // mientras dura la edición: así no hay forma de tipear encima del
            // manifiesto que se estaba mirando.
            let mut aplicar: Option<String> = None;
            let mut recargar = false;
            let mut entrar_edicion = false;
            let mut salir_edicion = false;
            let es_secret = det.kind == "Secret";
            let revelado = det.revelar;
            let editando = det.editando;

            ui.horizontal(|ui| {
                if editando {
                    let editado = det.yaml_edit.as_ref() != det.yaml.as_ref();
                    if ui
                        .add_enabled(
                            editado,
                            egui::Button::new(egui::RichText::new("Aplicar").color(theme::OK)),
                        )
                        .on_hover_text("PUT del YAML editado al API server")
                        .clicked()
                    {
                        aplicar = det.yaml_edit.clone();
                    }
                    if ui
                        .button("Cancelar")
                        .on_hover_text("Descartar los cambios y volver a solo lectura")
                        .clicked()
                    {
                        salir_edicion = true;
                    }
                    if ui.button("Copiar").clicked() {
                        if let Some(y) = det.yaml_edit.as_ref().or(det.yaml.as_ref()) {
                            ui.ctx().copy_text(y.clone());
                        }
                    }
                    if editado {
                        ui.colored_label(theme::WARN, "· editado");
                    }
                } else {
                    let boton = ui.add_enabled(
                        det.yaml_fresco,
                        egui::Button::new(egui::RichText::new("✎ Editar").color(theme::WARN)),
                    );
                    if boton.clicked() {
                        entrar_edicion = true;
                    }
                    boton.on_hover_text(if det.yaml_fresco {
                        "Habilita la edición de este manifiesto"
                    } else {
                        "esperando la copia del API server"
                    });
                    if ui
                        .button("Recargar")
                        .on_hover_text("Releer del API server")
                        .clicked()
                    {
                        recargar = true;
                    }
                    if ui.button("Copiar").clicked() {
                        if let Some(y) = det.yaml.as_ref() {
                            ui.ctx().copy_text(y.clone());
                        }
                    }
                    if es_secret {
                        let (txt, color) = if revelado {
                            ("Ocultar", theme::WARN)
                        } else {
                            ("Revelar", theme::TEXTO_TENUE)
                        };
                        if ui
                            .button(egui::RichText::new(txt).color(color))
                            .on_hover_text(
                                "base64 no es cifrado: los valores están ocultos por defecto",
                            )
                            .clicked()
                        {
                            *accion = Accion::AlternarRevelar(id);
                        }
                    }
                    if det.yaml_fresco {
                        ui.colored_label(theme::TEXTO_TENUE, "· solo lectura");
                    } else {
                        ui.spinner();
                        ui.colored_label(theme::TEXTO_TENUE, "· releyendo del API server");
                    }
                }
            });
            ui.add_space(2.0);

            match det.yaml.clone() {
                Some(y) => {
                    egui::ScrollArea::vertical()
                        .id_salt(("yaml_scroll", id))
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            if editando {
                                let buffer = det.yaml_edit.get_or_insert(y);
                                ui.add(
                                    egui::TextEdit::multiline(buffer)
                                        .code_editor()
                                        .desired_width(f32::INFINITY)
                                        .desired_rows(30),
                                );
                            } else {
                                // `&str` implementa TextBuffer como inmutable:
                                // se puede seleccionar y copiar, no escribir.
                                ui.add(
                                    egui::TextEdit::multiline(&mut y.as_str())
                                        .code_editor()
                                        .desired_width(f32::INFINITY)
                                        .desired_rows(30),
                                );
                            }
                        });
                }
                None => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.colored_label(theme::TEXTO_TENUE, "leyendo del API server…");
                    });
                }
            }

            if entrar_edicion {
                det.yaml_edit = det.yaml.clone();
                det.editando = true;
            }
            if salir_edicion {
                det.yaml_edit = None;
                det.editando = false;
            }
            if let Some(y) = aplicar {
                *accion = Accion::AplicarYaml(id, y);
            }
            if recargar {
                *accion = Accion::AbrirDetalleTab(id, key.clone(), TabDetalle::Yaml);
            }
        }
        TabDetalle::Mapa => match det.mapa.as_deref() {
            Some(data) => {
                let data = data.clone();
                egui::ScrollArea::both()
                    .id_salt(("mapa_scroll", id))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if ui.button("↻ actualizar").clicked() {
                            pedir_mapa = true;
                        }
                        match &data {
                            crate::k8s::mapa::Mapa::Service(d) => crate::ui::map::dibujar(ui, d),
                            crate::k8s::mapa::Mapa::Workload(d) => {
                                crate::ui::map::dibujar_workload(ui, d, id, &ns_detalle, accion)
                            }
                        }
                    });
            }
            None => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.colored_label(theme::TEXTO_TENUE, "armando el mapa…");
                });
            }
        },
        TabDetalle::Resumen | TabDetalle::Eventos => {
            let obj_existe = pane.store.as_ref().and_then(|s| s.objeto(&key)).is_some();
            egui::ScrollArea::vertical()
                .id_salt(("detalle_scroll", id))
                .auto_shrink([false, false])
                .show(ui, |ui| match tab {
                    TabDetalle::Resumen => {
                        if obj_existe {
                            let obj = pane.store.as_ref().and_then(|s| s.objeto(&key)).unwrap();
                            let crd = pane.store.as_ref().map(|s| s.columnas_crd()).unwrap_or(&[]);
                            resumen(ui, &kind, obj, &historial, crd);
                            if kind == "Service" {
                                backends(
                                    ui,
                                    id,
                                    &ns_detalle,
                                    &backends_lista,
                                    backends_pedidos,
                                    accion,
                                );
                            }
                        } else {
                            ui.colored_label(
                                theme::TEXTO_TENUE,
                                "el objeto ya no está en la vista",
                            );
                        }
                    }
                    _ => {
                        if eventos.is_empty() {
                            ui.colored_label(
                                theme::TEXTO_TENUE,
                                if eventos_pedidos {
                                    "sin eventos recientes"
                                } else {
                                    "el objeto no tiene UID; no se pueden resolver sus eventos"
                                },
                            );
                        }
                        for e in &eventos {
                            let color = if e.type_ == "Warning" {
                                theme::BAD
                            } else {
                                theme::TEXTO_TENUE
                            };
                            egui::Frame::new()
                                .fill(theme::PANEL_ALT)
                                .corner_radius(2)
                                .inner_margin(6)
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.colored_label(color, &e.reason);
                                        if e.count > 1 {
                                            ui.colored_label(
                                                theme::TEXTO_TENUE,
                                                format!("×{}", e.count),
                                            );
                                        }
                                        if let Some(t) = e.last {
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    ui.colored_label(
                                                        theme::TEXTO_TENUE,
                                                        t.strftime("%H:%M:%S").to_string(),
                                                    );
                                                },
                                            );
                                        }
                                    });
                                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                                    ui.label(&e.message);
                                });
                            ui.add_space(3.0);
                        }
                    }
                });
        }
    }

    if pedir_mapa {
        *accion = Accion::PedirMapa(id);
    }
}

fn resumen(
    ui: &mut egui::Ui,
    kind: &str,
    o: &kube::api::DynamicObject,
    historial: &[crate::k8s::metricas::Uso],
    crd: &[crate::k8s::printer::ColumnaCrd],
) {
    seccion(ui, "Metadata", |ui| {
        campo(ui, "Nombre", &kube::ResourceExt::name_any(o));
        if let Some(ns) = kube::ResourceExt::namespace(o) {
            campo(ui, "Namespace", &ns);
        }
        if let Some(t) = kube::ResourceExt::creation_timestamp(o) {
            campo(
                ui,
                "Creado",
                &t.0.strftime("%Y-%m-%d %H:%M:%S UTC").to_string(),
            );
        }
        if let Some(uid) = kube::ResourceExt::uid(o) {
            campo(ui, "UID", &uid);
        }
        if let Some(dueño) = o.metadata.owner_references.as_ref().and_then(|r| r.first()) {
            campo(
                ui,
                "Controlado por",
                &format!("{}/{}", dueño.kind, dueño.name),
            );
        }
    });

    // El uso actual es lo primero que uno busca en un pod o un nodo: va antes
    // que labels y anotaciones, que empujan todo hacia abajo.
    if crate::columns::tiene_metricas(kind) {
        uso_recursos(ui, kind, o, historial);
    }

    let labels = kube::ResourceExt::labels(o);
    if !labels.is_empty() {
        seccion(ui, "Labels", |ui| chips(ui, labels));
    }
    // last-applied-configuration es un JSON entero: no aporta acá.
    let anns: Vec<(&String, &String)> = kube::ResourceExt::annotations(o)
        .iter()
        .filter(|(k, _)| !k.ends_with("last-applied-configuration"))
        .collect();
    if !anns.is_empty() {
        seccion(ui, "Annotations", |ui| {
            for (k, v) in anns {
                campo(ui, k, v);
            }
        });
    }

    match kind {
        "Pod" => resumen_pod(ui, o),
        "Secret" | "ConfigMap" => datos_clave_valor(ui, kind, o),
        _ => {
            if !super::resumen::por_kind(ui, kind, o) {
                // Recurso custom o kind sin resumen propio: las columnas del
                // CRD, los escalares del spec y los del status.
                super::resumen::columnas_crd(ui, crd, o);
                super::resumen::spec_generico(ui, o);
                resumen_generico(ui, o);
            }
        }
    }

    if let Some(conds) = o
        .data
        .get("status")
        .and_then(|s| s.get("conditions"))
        .and_then(|v| v.as_array())
    {
        seccion(ui, "Conditions", |ui| {
            for c in conds {
                let tipo = str_de(c, "type");
                let estado = str_de(c, "status");
                let color = match estado.as_str() {
                    "True" => theme::OK,
                    "False" => theme::BAD,
                    _ => theme::WARN,
                };
                ui.horizontal(|ui| {
                    ui.colored_label(color, "●");
                    ui.label(&tipo);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let motivo = str_de(c, "reason");
                        if !motivo.is_empty() {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(&motivo).color(theme::TEXTO_TENUE),
                                )
                                .truncate(),
                            )
                            .on_hover_text(&motivo);
                        }
                    });
                });
                let msg = str_de(c, "message");
                if !msg.is_empty() {
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                    ui.colored_label(theme::TEXTO_TENUE, msg);
                }
            }
        });
    }
}

/// CPU y memoria en el tiempo, desde metrics.k8s.io.
fn uso_recursos(
    ui: &mut egui::Ui,
    kind: &str,
    o: &kube::api::DynamicObject,
    historial: &[crate::k8s::metricas::Uso],
) {
    use crate::k8s::metricas::{fmt_cpu, fmt_mem, parse_cpu, parse_mem, HISTORIAL, INTERVALO_S};

    seccion(ui, "Uso", |ui| {
        let Some(ultimo) = historial.last() else {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.colored_label(
                    theme::TEXTO_TENUE,
                    "esperando la primera muestra de metrics-server…",
                );
            });
            return;
        };

        // Tope del gráfico: para un Node lo asignable, para un Pod el límite
        // del contenedor si lo declara. Sin tope, el máximo de la serie.
        let (tope_cpu, tope_mem) = if kind == "Node" {
            let a = o.data.get("status").and_then(|s| s.get("allocatable"));
            (
                a.and_then(|a| a.get("cpu"))
                    .and_then(|v| v.as_str())
                    .and_then(parse_cpu),
                a.and_then(|a| a.get("memory"))
                    .and_then(|v| v.as_str())
                    .and_then(parse_mem),
            )
        } else {
            limites_del_pod(o)
        };

        let ancho = ui.available_width();
        let mitad = ((ancho - 12.0) / 2.0).max(80.0);
        ui.horizontal_top(|ui| {
            sparkline(
                ui,
                mitad,
                "CPU",
                &historial.iter().map(|u| u.cpu_m as f64).collect::<Vec<_>>(),
                tope_cpu.map(|t| t as f64),
                &fmt_cpu(ultimo.cpu_m),
                tope_cpu.map(fmt_cpu),
                theme::ACENTO,
            );
            ui.add_space(12.0);
            sparkline(
                ui,
                mitad,
                "Memoria",
                &historial
                    .iter()
                    .map(|u| u.mem_bytes as f64)
                    .collect::<Vec<_>>(),
                tope_mem.map(|t| t as f64),
                &fmt_mem(ultimo.mem_bytes),
                tope_mem.map(fmt_mem),
                theme::OK,
            );
        });
        ui.colored_label(
            theme::TEXTO_TENUE,
            format!(
                "últimos {} min, una muestra cada {INTERVALO_S} s",
                HISTORIAL as u64 * INTERVALO_S / 60
            ),
        );
    });
}

/// Suma de los límites de CPU y memoria de los contenedores del pod, si todos
/// los declaran. Con uno solo sin límite el total no significa nada.
fn limites_del_pod(o: &kube::api::DynamicObject) -> (Option<u64>, Option<u64>) {
    use crate::k8s::metricas::{parse_cpu, parse_mem};
    let Some(cs) = o
        .data
        .get("spec")
        .and_then(|s| s.get("containers"))
        .and_then(|c| c.as_array())
    else {
        return (None, None);
    };
    let mut cpu = Some(0u64);
    let mut mem = Some(0u64);
    for c in cs {
        let l = c.get("resources").and_then(|r| r.get("limits"));
        match l
            .and_then(|l| l.get("cpu"))
            .and_then(|v| v.as_str())
            .and_then(parse_cpu)
        {
            Some(v) => cpu = cpu.map(|t| t + v),
            None => cpu = None,
        }
        match l
            .and_then(|l| l.get("memory"))
            .and_then(|v| v.as_str())
            .and_then(parse_mem)
        {
            Some(v) => mem = mem.map(|t| t + v),
            None => mem = None,
        }
    }
    (cpu.filter(|v| *v > 0), mem.filter(|v| *v > 0))
}

/// Serie temporal chica: línea sobre fondo, con el valor actual y el tope.
#[allow(clippy::too_many_arguments)]
fn sparkline(
    ui: &mut egui::Ui,
    ancho: f32,
    titulo: &str,
    valores: &[f64],
    tope: Option<f64>,
    actual: &str,
    tope_texto: Option<String>,
    color: egui::Color32,
) {
    ui.vertical(|ui| {
        ui.set_width(ancho);
        ui.horizontal(|ui| {
            ui.colored_label(theme::TEXTO_TENUE, titulo);
            ui.with_layout(
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| match &tope_texto {
                    Some(t) => {
                        ui.colored_label(theme::TEXTO_TENUE, format!("/ {t}"));
                        ui.label(egui::RichText::new(actual).strong());
                    }
                    None => {
                        ui.label(egui::RichText::new(actual).strong());
                    }
                },
            );
        });

        let alto = 42.0;
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ancho, alto), egui::Sense::hover());
        let p = ui.painter_at(rect);
        p.rect_filled(rect, 3.0, theme::PANEL_ALT);

        if valores.is_empty() {
            return;
        }
        // La escala es el tope si lo hay, si no el máximo de la serie; nunca
        // cero, para no dividir por él con una serie plana en 0.
        let max_serie = valores.iter().cloned().fold(0.0_f64, f64::max);
        let escala = tope.unwrap_or(max_serie).max(max_serie).max(1.0);

        // La serie se dibuja pegada a la derecha: lo último es lo que importa,
        // y así el gráfico "avanza" a medida que llegan muestras.
        let n = crate::k8s::metricas::HISTORIAL.max(2) as f32;
        let paso = (rect.width() - 8.0) / (n - 1.0);
        let x0 = rect.right() - 4.0 - paso * (valores.len().saturating_sub(1)) as f32;
        let puntos: Vec<egui::Pos2> = valores
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let y = rect.bottom() - 4.0 - ((v / escala) as f32) * (rect.height() - 8.0);
                egui::pos2(x0 + paso * i as f32, y)
            })
            .collect();

        // Relleno tenue bajo la línea.
        if puntos.len() >= 2 {
            let mut area = puntos.clone();
            area.push(egui::pos2(puntos.last().unwrap().x, rect.bottom() - 4.0));
            area.push(egui::pos2(puntos[0].x, rect.bottom() - 4.0));
            p.add(egui::Shape::convex_polygon(
                area,
                color.gamma_multiply(0.15),
                egui::Stroke::NONE,
            ));
            p.add(egui::Shape::line(
                puntos.clone(),
                egui::Stroke::new(1.5, color),
            ));
        }
        if let Some(ultimo) = puntos.last() {
            p.circle_filled(*ultimo, 2.5, color);
        }
        // Marca del tope, si el actual está lejos de él se ve como referencia.
        if tope.is_some() {
            p.line_segment(
                [
                    egui::pos2(rect.left() + 4.0, rect.top() + 4.0),
                    egui::pos2(rect.right() - 4.0, rect.top() + 4.0),
                ],
                egui::Stroke::new(1.0, theme::BORDE),
            );
        }
    });
}

/// Backends detrás del Service: IP, pod y nodo, con el pod clickeable.
fn backends(
    ui: &mut egui::Ui,
    id: u64,
    ns: &Option<String>,
    lista: &[crate::k8s::endpoints::Backend],
    pedidos: bool,
    accion: &mut Accion,
) {
    let listos = lista.iter().filter(|b| b.listo).count();
    let titulo = if lista.is_empty() {
        "Backends".to_string()
    } else {
        format!("Backends ({listos}/{})", lista.len())
    };
    seccion(ui, &titulo, |ui| {
        if lista.is_empty() {
            if pedidos {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.colored_label(theme::TEXTO_TENUE, "buscando los endpoints…");
                });
            } else {
                ui.colored_label(theme::BAD, "ninguno: el Service no resuelve a ningún pod");
            }
            return;
        }
        // Los puertos son del slice, iguales para todas las direcciones.
        if let Some(p) = lista.first().filter(|b| !b.puertos.is_empty()) {
            campo(ui, "Puertos destino", &p.puertos.join(", "));
            ui.add_space(4.0);
        }
        for b in lista {
            ui.horizontal(|ui| {
                let (punto, color, ayuda) = if b.listo {
                    ("●", theme::OK, "Ready")
                } else {
                    ("●", theme::WARN, "no está Ready: no recibe tráfico")
                };
                ui.colored_label(color, punto).on_hover_text(ayuda);
                ui.add(
                    egui::Label::new(egui::RichText::new(&b.ip).monospace())
                        .sense(egui::Sense::click()),
                )
                .on_hover_text("clic para copiar")
                .clicked()
                .then(|| ui.ctx().copy_text(b.ip.clone()));

                match &b.pod {
                    Some(pod) => {
                        if ui
                            .add(egui::Button::new(
                                egui::RichText::new(pod).color(theme::ACENTO),
                            ))
                            .on_hover_text("ir al pod")
                            .clicked()
                        {
                            *accion = Accion::IrA(id, "Pod".into(), ns.clone(), pod.clone());
                        }
                    }
                    None => {
                        // Un endpoint sin targetRef es una dirección externa.
                        ui.colored_label(theme::TEXTO_TENUE, "externo");
                    }
                }
                if let Some(n) = &b.nodo {
                    ui.colored_label(theme::TEXTO_TENUE, n);
                }
                if let Some(z) = &b.zona {
                    ui.colored_label(theme::TEXTO_TENUE, z);
                }
            });
        }
    });
}

fn resumen_pod(ui: &mut egui::Ui, o: &kube::api::DynamicObject) {
    let spec = o.data.get("spec");
    let status = o.data.get("status");

    seccion(ui, "Pod", |ui| {
        campo(ui, "Nodo", &opt_str(spec, "nodeName"));
        campo(ui, "IP", &opt_str(status, "podIP"));
        campo(ui, "IP del nodo", &opt_str(status, "hostIP"));
        campo(ui, "QoS", &opt_str(status, "qosClass"));
        campo(ui, "Service account", &opt_str(spec, "serviceAccountName"));
        campo(ui, "Restart policy", &opt_str(spec, "restartPolicy"));
    });

    let estados: Vec<&Value> = status
        .and_then(|s| s.get("containerStatuses"))
        .and_then(|v| v.as_array())
        .map(|a| a.iter().collect())
        .unwrap_or_default();

    if let Some(cs) = spec
        .and_then(|s| s.get("containers"))
        .and_then(|v| v.as_array())
    {
        seccion(ui, "Contenedores", |ui| {
            for c in cs {
                let nombre = str_de(c, "name");
                let st = estados.iter().find(|s| str_de(s, "name") == nombre);
                let listo = st
                    .and_then(|s| s.get("ready"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let restarts = st
                    .and_then(|s| s.get("restartCount"))
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);

                egui::Frame::new()
                    .fill(theme::PANEL_ALT)
                    .corner_radius(2)
                    .inner_margin(6)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.colored_label(if listo { theme::OK } else { theme::BAD }, "●");
                            ui.label(egui::RichText::new(&nombre).strong());
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if restarts > 0 {
                                        ui.colored_label(
                                            theme::WARN,
                                            format!("{restarts} reinicios"),
                                        );
                                    }
                                },
                            );
                        });
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                        ui.colored_label(theme::TEXTO_TENUE, str_de(c, "image"));

                        if let Some(estado) = st.and_then(|s| s.get("state")) {
                            if let Some(w) = estado.get("waiting") {
                                let razon = str_de(w, "reason");
                                let msg = str_de(w, "message");
                                ui.colored_label(
                                    theme::BAD,
                                    if msg.is_empty() {
                                        razon
                                    } else {
                                        format!("{razon}: {msg}")
                                    },
                                );
                            } else if let Some(t) = estado.get("terminated") {
                                ui.colored_label(theme::WARN, terminacion("terminated", t));
                            }
                        }

                        // Por qué se murió la vez anterior: es lo que explica un
                        // CrashLoopBackOff o un contador de reinicios alto, y no
                        // está en el estado actual.
                        if let Some(t) = st
                            .and_then(|s| s.get("lastState"))
                            .and_then(|l| l.get("terminated"))
                        {
                            ui.colored_label(theme::BAD, terminacion("murió antes", t));
                        }

                        if let Some(r) = c.get("resources") {
                            let req = recursos(r.get("requests"));
                            let lim = recursos(r.get("limits"));
                            if !req.is_empty() || !lim.is_empty() {
                                ui.colored_label(
                                    theme::TEXTO_TENUE,
                                    format!("requests: {req}   limits: {lim}"),
                                );
                            }
                        }
                    });
                ui.add_space(3.0);
            }
        });
    }
}

/// Describe un `terminated` completo: código, razón, señal y mensaje.
///
/// El `reason` solo suele decir `Error` o `ContainerStatusUnknown`; lo que
/// explica de verdad la caída está en `message` (y el 137 en `signal`).
fn terminacion(prefijo: &str, t: &Value) -> String {
    let code = t.get("exitCode").and_then(|v| v.as_i64()).unwrap_or(0);
    let mut out = format!("{prefijo} ({code})");
    let razon = str_de(t, "reason");
    if !razon.is_empty() {
        out.push_str(&format!(" {razon}"));
    }
    if let Some(sig) = t.get("signal").and_then(|v| v.as_i64()).filter(|s| *s != 0) {
        out.push_str(&format!(" · señal {sig}"));
    }
    // 137 = 128+9 (SIGKILL): casi siempre OOM o un límite de memoria.
    if code == 137 && !razon.contains("OOM") {
        out.push_str(" · matado (SIGKILL): suele ser OOM o el límite de memoria");
    }
    let msg = str_de(t, "message");
    if !msg.is_empty() {
        out.push_str(&format!("\n{msg}"));
    }
    out
}

/// Para lo que no tiene vista propia: los escalares del status, que suelen ser
/// justo lo que uno quiere ver de un CRD.
fn resumen_generico(ui: &mut egui::Ui, o: &kube::api::DynamicObject) {
    let Some(status) = o.data.get("status").and_then(|v| v.as_object()) else {
        return;
    };
    let escalares: Vec<(&String, &Value)> = status
        .iter()
        .filter(|(_, v)| v.is_string() || v.is_number() || v.is_boolean())
        .collect();
    if escalares.is_empty() {
        return;
    }
    seccion(ui, "Status", |ui| {
        for (k, v) in escalares {
            let texto = match v {
                Value::String(s) => s.clone(),
                otro => otro.to_string(),
            };
            campo(ui, k, &texto);
        }
    });
}

pub(super) fn recursos(v: Option<&Value>) -> String {
    v.and_then(|v| v.as_object())
        .map(|m| {
            m.iter()
                .map(|(k, v)| format!("{k}={}", v.as_str().unwrap_or("")))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default()
}

pub(super) fn str_de(v: &Value, k: &str) -> String {
    v.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string()
}

pub(super) fn opt_str(v: Option<&Value>, k: &str) -> String {
    v.and_then(|v| v.get(k))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

pub(super) fn seccion(ui: &mut egui::Ui, titulo: &str, contenido: impl FnOnce(&mut egui::Ui)) {
    // Título tenue con una regla fina: separa sin usar el acento como cromo.
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(titulo.to_uppercase())
                .size(10.5)
                .color(theme::TEXTO_TENUE),
        );
        let resto = ui.available_width();
        if resto > 8.0 {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(resto, 1.0), egui::Sense::hover());
            let y = rect.center().y;
            ui.painter().hline(
                rect.left() + 4.0..=rect.right(),
                y,
                egui::Stroke::new(1.0, theme::BORDE),
            );
        }
    });
    ui.add_space(3.0);
    contenido(ui);
}

pub(super) fn campo(ui: &mut egui::Ui, clave: &str, valor: &str) {
    if valor.is_empty() {
        return;
    }
    // La columna de claves era fija en 150 px: en un panel angosto se comía
    // todo el ancho y los valores quedaban afuera.
    let ancho_clave = (ui.available_width() * 0.38).clamp(70.0, 150.0);
    ui.horizontal_top(|ui| {
        ui.add_sized(
            [ancho_clave, 16.0],
            egui::Label::new(
                egui::RichText::new(clave)
                    .size(12.0)
                    .color(theme::TEXTO_TENUE),
            )
            .truncate(),
        )
        .on_hover_text(clave);
        // El valor va en su propio hueco de ancho conocido. Ni heredar el wrap
        // del estilo ni `.wrap()` alcanzaban: dentro de un layout horizontal el
        // label tomaba su ancho natural y los valores largos salían cortados.
        let resto = ui.available_width().max(40.0);
        ui.allocate_ui_with_layout(
            egui::vec2(resto, 0.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_max_width(resto);
                // Truncado y no envuelto: envolver corta en espacios, y estos
                // valores no los tienen (`unix:///var/run/...`, un JSON), así
                // que se desbordaban del panel. El valor entero va en el
                // tooltip y el clic lo copia.
                let resp = ui
                    .add(
                        egui::Label::new(egui::RichText::new(valor).size(12.0))
                            .truncate()
                            .sense(egui::Sense::click()),
                    )
                    .on_hover_text(valor);
                if resp.clicked() {
                    ui.ctx().copy_text(valor.to_string());
                }
            },
        );
    });
}

pub(super) fn chips(ui: &mut egui::Ui, mapa: &std::collections::BTreeMap<String, String>) {
    // Dentro de un hueco de ancho exacto. Si la fila desborda, egui agranda el
    // `max_rect` del ui padre y todo lo que se dibuja después —las anotaciones,
    // las conditions— se maqueta más ancho que el panel y sale cortado.
    let ancho = ui.available_width().max(60.0);
    ui.allocate_ui_with_layout(
        egui::vec2(ancho, 0.0),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_max_width(ancho);
            chips_fila(ui, ancho, mapa);
        },
    );
}

fn chips_fila(ui: &mut egui::Ui, ancho: f32, mapa: &std::collections::BTreeMap<String, String>) {
    // `horizontal_wrapped` decide el salto con el tamaño del ítem anterior, y
    // un Frame no tiene tamaño hasta que se dibuja: la fila seguía de largo y
    // salía del panel. Acá se mide cada chip antes y se arman las filas.
    let sep = ui.spacing().item_spacing.x;
    let fuente = egui::FontId::proportional(11.0);
    let textos: Vec<String> = mapa.iter().map(|(k, v)| format!("{k}={v}")).collect();
    let anchos: Vec<f32> = textos
        .iter()
        .map(|t| {
            ui.fonts_mut(|f| f.layout_no_wrap(t.clone(), fuente.clone(), theme::TEXTO_TENUE))
                .size()
                .x
                + 12.0
        })
        .collect();
    let tope = (ancho - 4.0).max(60.0);
    let mut i = 0;
    while i < textos.len() {
        let mut fin = i;
        let mut usado = 0.0;
        while fin < textos.len() {
            let w = anchos[fin].min(tope);
            let con_sep = if fin > i { usado + sep + w } else { w };
            if fin > i && con_sep > tope {
                break;
            }
            usado = con_sep;
            fin += 1;
        }
        ui.horizontal(|ui| {
            for texto in &textos[i..fin] {
                egui::Frame::new()
                    .fill(theme::PANEL_ALT)
                    .corner_radius(2)
                    .inner_margin(egui::Margin::symmetric(5, 2))
                    .show(ui, |ui| {
                        ui.set_max_width(tope - 12.0);
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(texto)
                                    .size(11.0)
                                    .color(theme::TEXTO_TENUE),
                            )
                            .truncate(),
                        )
                        .on_hover_text(texto);
                    });
            }
        });
        i = fin;
    }
}

/// Claves de un Secret o ConfigMap. En Secrets el valor arranca oculto y se
/// revela (ya decodificado de base64) clave por clave, no todo de golpe.
fn datos_clave_valor(ui: &mut egui::Ui, kind: &str, o: &kube::api::DynamicObject) {
    use base64::Engine as _;

    let es_secret = kind == "Secret";
    let mut claves: Vec<(String, String, bool)> = Vec::new(); // (clave, valor, es_binario)

    if let Some(m) = o.data.get("data").and_then(|v| v.as_object()) {
        for (k, v) in m {
            let crudo = v.as_str().unwrap_or_default();
            if es_secret {
                // En Secrets `data` viene en base64; en ConfigMaps es texto.
                match base64::engine::general_purpose::STANDARD.decode(crudo) {
                    Ok(bytes) => match String::from_utf8(bytes) {
                        Ok(txt) => claves.push((k.clone(), txt, false)),
                        Err(e) => claves.push((
                            k.clone(),
                            format!("<binario, {} bytes>", e.as_bytes().len()),
                            true,
                        )),
                    },
                    Err(_) => claves.push((k.clone(), crudo.to_string(), true)),
                }
            } else {
                claves.push((k.clone(), crudo.to_string(), false));
            }
        }
    }
    if let Some(m) = o.data.get("binaryData").and_then(|v| v.as_object()) {
        for (k, v) in m {
            let n = v.as_str().map(|s| s.len()).unwrap_or(0);
            claves.push((k.clone(), format!("<binario, ~{} bytes>", n * 3 / 4), true));
        }
    }

    if claves.is_empty() {
        return;
    }
    claves.sort_by(|a, b| a.0.cmp(&b.0));

    seccion(
        ui,
        if es_secret {
            "Datos (ocultos)"
        } else {
            "Datos"
        },
        |ui| {
            for (clave, valor, binario) in &claves {
                let id = ui.make_persistent_id(("secreto_visible", &clave));
                let mut visible = ui
                    .ctx()
                    .data(|d| d.get_temp::<bool>(id))
                    .unwrap_or(!es_secret);

                egui::Frame::new()
                    .fill(theme::PANEL_ALT)
                    .corner_radius(2)
                    .inner_margin(6)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(clave).size(12.0).strong());
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui
                                        .small_button("copiar")
                                        .on_hover_text(if es_secret {
                                            "copia el valor decodificado"
                                        } else {
                                            "copia el valor"
                                        })
                                        .clicked()
                                    {
                                        ui.ctx().copy_text(valor.clone());
                                    }
                                    if es_secret && !binario {
                                        let txt = if visible { "ocultar" } else { "revelar" };
                                        if ui.small_button(txt).clicked() {
                                            visible = !visible;
                                            ui.ctx().data_mut(|d| d.insert_temp(id, visible));
                                        }
                                    }
                                },
                            );
                        });
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                        if visible {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(recortar_valor(valor))
                                        .monospace()
                                        .size(11.0),
                                )
                                .selectable(true),
                            );
                        } else {
                            ui.colored_label(
                                theme::TEXTO_TENUE,
                                egui::RichText::new("••••••••••••").monospace(),
                            );
                        }
                    });
                ui.add_space(3.0);
            }
        },
    );
}

/// Un valor gigante (un cert, un dump) no aporta nada en el panel.
fn recortar_valor(v: &str) -> String {
    const MAX: usize = 2_000;
    if v.len() <= MAX {
        v.to_string()
    } else {
        format!("{}…\n<{} bytes en total>", &v[..MAX], v.len())
    }
}
