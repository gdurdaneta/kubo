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
        if let Ok(spec) = std::env::var("KUBO_TEST_PF") {
            if !spec.is_empty() {
                std::env::set_var("KUBO_TEST_PF", "");
                std::env::set_var("KUBO_TEST_PF_AUTO", "1");
                self.pedir_forward(pane_id, &spec);
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

        // KUBO_TEST_CONFIRM=[escalar:]Kind:ns:nombre — abre el modal de
        // confirmación (borrado, o escalado con el prefijo) sin ejecutarlo.
        if let Ok(spec) = std::env::var("KUBO_TEST_CONFIRM") {
            if !spec.is_empty() {
                std::env::set_var("KUBO_TEST_CONFIRM", "");
                let (verbo, spec) = match spec.strip_prefix("escalar:") {
                    Some(resto) => (Verbo::Escalar(-1), resto.to_string()),
                    None => (Verbo::Borrar, spec.clone()),
                };
                let partes: Vec<&str> = spec.split(':').collect();
                if let [kind, ns, nombre] = partes[..] {
                    self.confirm = Some(Confirmacion {
                        pane: pane_id,
                        verbo,
                        kind: kind.to_string(),
                        ns: (!ns.is_empty()).then(|| ns.to_string()),
                        name: nombre.to_string(),
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

        // KUBO_TEST_IRA=Kind:ns:name — prueba la navegación "ir al recurso".
        if let Ok(spec) = std::env::var("KUBO_TEST_IRA") {
            if !spec.is_empty() {
                std::env::set_var("KUBO_TEST_IRA", "");
                let partes: Vec<&str> = spec.splitn(3, ':').collect();
                if let [kind, ns, name] = partes[..] {
                    let (kind, name) = (kind.to_string(), name.to_string());
                    let ns = Some(ns.to_string());
                    self.ir_a(pane_id, &kind, ns.clone(), &name);
                    // KUBO_TEST_WL=logs|shell — sobre el recurso recién abierto,
                    // resuelve un pod del workload y abre logs o shell.
                    if let Ok(que) = std::env::var("KUBO_TEST_WL") {
                        let que = match que.as_str() {
                            "logs" => Some(k8s::pods::QuePod::Logs),
                            "shell" => Some(k8s::pods::QuePod::Shell),
                            _ => None,
                        };
                        if let Some(que) = que {
                            std::env::set_var("KUBO_TEST_WL", "");
                            let key = match ns {
                                Some(ns) => format!("{ns}/{name}"),
                                None => name.clone(),
                            };
                            self.resolver_pod_de(pane_id, &key, que);
                        }
                    }
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
