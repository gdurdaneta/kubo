//! Ganchos `KUBO_TEST_*` para capturas y pruebas sin clickear.

use super::*;

impl App {
    /// Harness de depuración: `KUBO_TEST_SHELL=ns/pod` abre una shell apenas
    /// carga el primer listado; `KUBO_TEST_SHELL_CMD` manda un comando. Sirve
    /// para probar el exec sin clickear.
    pub(super) fn gancho_de_prueba(&mut self, pane_id: u64) {
        if let Ok(key) = std::env::var("KUBO_TEST_SHELL") {
            if !key.is_empty() {
                std::env::set_var("KUBO_TEST_SHELL", "");
                self.abrir_shell(pane_id, &key);
                if let Ok(cmd) = std::env::var("KUBO_TEST_SHELL_CMD") {
                    if let Some(pane) = self.panes.iter().find(|p| p.id == pane_id) {
                        if let Some(Bottom::Term(v)) = pane.bottom.as_ref() {
                            let _ = v.handles.stdin.send(format!("{cmd}\n").into_bytes());
                        }
                    }
                }
            }
        }

        // KUBO_TEST_PF=ns/servicio — levanta un port-forward con los valores
        // por defecto (sin alias, así no dispara el diálogo de polkit).
        // Espera a la vista que tenga el recurso (un IRA previo puede estar
        // cambiando de vista todavía).
        if let Ok(spec) = std::env::var("KUBO_TEST_PF") {
            if !spec.is_empty() && self.objeto_del_pane(pane_id, &spec).is_some() {
                std::env::set_var("KUBO_TEST_PF", "");
                std::env::set_var("KUBO_TEST_PF_AUTO", "1");
                self.pedir_forward(pane_id, &spec);
                // Con un pod el diálogo queda listo al instante y no hay evento
                // que dispare el auto-abrir: se hace acá.
                if self.dialogo_pf.as_ref().is_some_and(|d| d.pod) {
                    std::env::set_var("KUBO_TEST_PF_AUTO", "");
                    self.abrir_forward();
                }
            }
        }

        // KUBO_TEST_NAV=0 — oculta el sidebar, para capturas de la tabla en
        // ventanas angostas.
        if std::env::var("KUBO_TEST_NAV").is_ok_and(|v| v == "0") {
            std::env::set_var("KUBO_TEST_NAV", "");
            if let Some(p) = self.pane(pane_id) {
                p.nav_visible = false;
            }
        }

        // KUBO_TEST_VISTA=auditoria|forwards — abre una vista local.
        if let Ok(v) = std::env::var("KUBO_TEST_VISTA") {
            if !v.is_empty() {
                std::env::set_var("KUBO_TEST_VISTA", "");
                let vista = match v.as_str() {
                    "auditoria" => Some(VistaLocal::Auditoria),
                    "forwards" => Some(VistaLocal::PortForwards),
                    _ => None,
                };
                if let Some(vl) = vista {
                    self.ver_vista_local(pane_id, vl);
                }
            }
        }

        // KUBO_TEST_SEL=ns/a,ns/b — marca filas para probar el lote. Espera a
        // la vista que las tenga (el IRA de arriba puede cambiar de namespace).
        if let Ok(claves) = std::env::var("KUBO_TEST_SEL") {
            if !claves.is_empty() {
                let claves: Vec<String> = claves.split(',').map(|s| s.trim().to_string()).collect();
                if let Some(p) = self.pane(pane_id) {
                    let todas = p
                        .store
                        .as_ref()
                        .is_some_and(|s| claves.iter().all(|k| s.objeto(k).is_some()));
                    if todas {
                        std::env::set_var("KUBO_TEST_SEL", "");
                        p.seleccion.extend(claves);
                    }
                }
            }
        }

        // KUBO_TEST_CONFIRM=[escalar:|aplicar:]Kind:ns:nombre[,nombre…] — abre
        // el modal de confirmación sin ejecutarlo; con varios nombres, en lote.
        if let Ok(spec) = std::env::var("KUBO_TEST_CONFIRM") {
            if !spec.is_empty() {
                std::env::set_var("KUBO_TEST_CONFIRM", "");
                let (verbo, spec) = match spec.strip_prefix("escalar:") {
                    Some(resto) => (Verbo::Escalar(-1), resto.to_string()),
                    None => match spec.strip_prefix("aplicar:") {
                        Some(resto) => (Verbo::AplicarYaml(String::new()), resto.to_string()),
                        None => (Verbo::Borrar, spec.clone()),
                    },
                };
                let diff = matches!(verbo, Verbo::AplicarYaml(_)).then(|| {
                    super::acciones::diff_unificado(
                        "spec:\n  replicas: 1\n  template:\n    spec:\n      containers:\n      - image: app:1.0\n        name: app\n",
                        "spec:\n  replicas: 3\n  template:\n    spec:\n      containers:\n      - image: app:1.1\n        name: app\n",
                    )
                });
                let partes: Vec<&str> = spec.split(':').collect();
                if let [kind, ns, nombres] = partes[..] {
                    let ns = (!ns.is_empty()).then(|| ns.to_string());
                    let mut nombres = nombres.split(',');
                    let nombre = nombres.next().unwrap_or_default();
                    self.confirm = Some(Confirmacion {
                        pane: pane_id,
                        verbo,
                        kind: kind.to_string(),
                        ns: ns.clone(),
                        name: nombre.to_string(),
                        diff,
                        tecleado: String::new(),
                        extra: nombres.map(|n| (ns.clone(), n.to_string())).collect(),
                    });
                }
            }
        }

        // KUBO_TEST_PALETTE=texto — abre la paleta con la query puesta.
        if let Ok(q) = std::env::var("KUBO_TEST_PALETTE") {
            if !q.is_empty() {
                std::env::set_var("KUBO_TEST_PALETTE", "");
                self.abrir_palette(pane_id);
                if let Some(p) = self.palette.as_mut() {
                    p.query = q;
                    p.desde_cambio = 1.0;
                }
                return;
            }
        }

        // KUBO_TEST_WL=logs|shell:ns/name — resuelve un pod del workload y
        // abre logs o shell (espera a que el recurso esté en la vista).
        if let Ok(spec) = std::env::var("KUBO_TEST_WL") {
            if let Some((que, key)) = spec.split_once(':') {
                let que = match que {
                    "logs" => Some(k8s::pods::QuePod::Logs),
                    "shell" => Some(k8s::pods::QuePod::Shell),
                    _ => None,
                };
                if let Some(que) = que {
                    if self.objeto_del_pane(pane_id, key).is_some() {
                        std::env::set_var("KUBO_TEST_WL", "");
                        self.resolver_pod_de(pane_id, key, que);
                    }
                }
            }
        }

        // KUBO_TEST_IRA=Kind:ns:name — prueba la navegación "ir al recurso".
        if let Ok(spec) = std::env::var("KUBO_TEST_IRA") {
            if !spec.is_empty() {
                std::env::set_var("KUBO_TEST_IRA", "");
                let partes: Vec<&str> = spec.splitn(3, ':').collect();
                if let [kind, ns, name] = partes[..] {
                    let (kind, name) = (kind.to_string(), name.to_string());
                    let ns = Some(ns.to_string());
                    self.ir_a(pane_id, &kind, ns, &name);
                    return;
                }
            }
        }

        // KUBO_TEST_MAPA=ns/svc (o KUBO_TEST_WMAPA=ns/deploy): navega al
        // kind y abre el mapa.
        let (var, kind_buscado) = if std::env::var("KUBO_TEST_WMAPA")
            .map(|v| !v.is_empty())
            .unwrap_or(false)
        {
            ("KUBO_TEST_WMAPA", "Deployment")
        } else {
            ("KUBO_TEST_MAPA", "Service")
        };
        if let Ok(spec) = std::env::var(var) {
            if !spec.is_empty() {
                let kind_actual = self
                    .panes
                    .iter()
                    .find(|p| p.id == pane_id)
                    .and_then(|p| p.item.as_ref())
                    .map(|i| i.res.ar.kind.clone());
                if kind_actual.as_deref() != Some(kind_buscado) {
                    let destino = self
                        .panes
                        .iter()
                        .find(|p| p.id == pane_id)
                        .and_then(|p| self.cluster_de(p))
                        .and_then(|c| {
                            c.nav
                                .iter()
                                .flat_map(|cat| cat.items.iter())
                                .find(|i| i.res.ar.kind == kind_buscado)
                                .cloned()
                        });
                    if let Some(item) = destino {
                        self.seleccionar(pane_id, item);
                    }
                    return;
                }
                std::env::set_var(var, "");
                tracing::debug!(spec, "gancho: abriendo detalle+mapa");
                self.abrir_detalle(pane_id, &spec);
                if let Some(pane) = self.pane(pane_id) {
                    if let Some(d) = pane.detalle.as_mut() {
                        d.tab = crate::app::TabDetalle::Mapa;
                    }
                }
                self.pedir_mapa(pane_id);
            }
        }
    }
}
