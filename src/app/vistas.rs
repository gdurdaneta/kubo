//! Qué mira cada panel: kind, namespace, watch, permisos, métricas y endpoints.

use super::*;

impl App {
    // -------------------------------------------------------------- vistas

    pub fn seleccionar(&mut self, pane_id: u64, item: NavItem) {
        self.seleccionar_con(pane_id, item, false);
    }

    /// `forzar` salta la optimización de "misma vista": lo usa el botón de
    /// recargar, que justamente quiere volver a listar contra el API server.
    pub(super) fn seleccionar_con(&mut self, pane_id: u64, item: NavItem, forzar: bool) {
        let token = self.token();
        let Some(pane) = self.panes.iter_mut().find(|p| p.id == pane_id) else {
            return;
        };
        let Some(client) = pane
            .contexto
            .as_ref()
            .and_then(|c| self.clusters.get(c))
            .and_then(|c| c.client.clone())
        else {
            return;
        };

        let target = match (&pane.ns_sel, item.res.namespaced) {
            (Some(ns), true) => Target::Namespace(ns.clone()),
            _ => Target::AllNamespaces,
        };

        // Volver a pedir la misma vista (pasa al navegar desde el mapa o la
        // paleta a un recurso del kind que ya se está mirando) costaría un
        // listado completo contra el API server para llegar a lo mismo.
        let misma_vista = !forzar
            && pane.item.as_ref().map(|i| i.res.key()) == Some(item.res.key())
            && pane.watch_target.as_ref() == Some(&target)
            && pane.watch_tarea.as_ref().is_some_and(|t| !t.is_finished())
            && pane.store.is_some();
        pane.vista_local = None;
        if misma_vista {
            return;
        }

        if let Some(t) = pane.watch_tarea.take() {
            t.abort();
        }
        pane.detalle = None;
        pane.cerrar_bottom();
        pane.busqueda.clear();

        let mostrar_ns = item.res.namespaced && pane.ns_sel.is_none();
        let es_service = item.res.ar.kind == "Service" && item.res.ar.group.is_empty();
        let con_metricas =
            item.res.ar.group.is_empty() && matches!(item.res.ar.kind.as_str(), "Pod" | "Node");
        pane.store = Some(Store::new(item.res.ar.kind.clone(), mostrar_ns));
        pane.watch_token = token;

        // Un recurso custom muestra las columnas de su CRD, como kubectl. Si ya
        // se pidió para este cluster se reutiliza; si no, llega por el bridge.
        if !k8s::printer::es_grupo_nativo(&item.res.ar.group) {
            let cacheadas = pane
                .contexto
                .as_ref()
                .and_then(|c| self.clusters.get(c))
                .and_then(|c| c.columnas_crd.get(&item.res.ar.kind))
                .cloned();
            match (cacheadas, pane.store.as_mut()) {
                (Some(cols), Some(store)) => store.set_columnas_crd(cols),
                _ => {
                    let (client, ar, bridge) =
                        (client.clone(), item.res.ar.clone(), self.bridge.clone());
                    self.rt.spawn(async move {
                        k8s::printer::obtener(client, ar, token, bridge).await;
                    });
                }
            }
        }

        pane.parar_endpoints();
        pane.parar_metricas();
        pane.watch_target = Some(target.clone());
        let ar = item.res.ar.clone();
        let namespaced = item.res.namespaced;
        let bridge = self.bridge.clone();
        pane.watch_tarea = Some(self.rt.spawn(async move {
            k8s::watch::run(client, ar, namespaced, target, token, bridge).await;
        }));
        pane.item = Some(item);
        self.consultar_permisos(pane_id);
        if es_service {
            self.seguir_endpoints(pane_id);
        }
        if con_metricas {
            self.sondear_metricas(pane_id);
        }
        self.guardar_layout();
    }

    /// Qué API de endpoints sirve este cluster: EndpointSlice es lo actual,
    /// Endpoints quedó deprecado pero es lo único que hay en clusters viejos.
    pub(super) fn ar_endpoints(&self, pane_id: u64) -> Option<kube::discovery::ApiResource> {
        let pane = self.panes.iter().find(|p| p.id == pane_id)?;
        let cluster = self.cluster_de(pane)?;
        let sirve = |k: &str| {
            cluster
                .info
                .as_ref()
                .is_some_and(|i| i.resources.iter().any(|r| r.ar.kind == k))
        };
        if sirve("EndpointSlice") {
            Some(k8s::endpoints::ar_endpointslice())
        } else if sirve("Endpoints") {
            Some(k8s::endpoints::ar_endpoints())
        } else {
            None
        }
    }

    /// Pregunta al API server qué verbos permite tu credencial sobre la vista
    /// recién abierta, para no ofrecer acciones que van a rebotar con un 403.
    pub(super) fn consultar_permisos(&mut self, pane_id: u64) {
        let Some(client) = self.client_del_pane(pane_id) else {
            return;
        };
        let Some(ar) = self.ar_del_pane(pane_id) else {
            return;
        };
        let Some(pane) = self.panes.iter().find(|p| p.id == pane_id) else {
            return;
        };
        // Un recurso cluster-scoped se consulta sin namespace: preguntar por
        // «nodes en default» devuelve una respuesta que no es la que aplica.
        let ns = pane
            .item
            .as_ref()
            .filter(|i| i.res.namespaced)
            .and_then(|_| pane.ns_sel.clone());
        let clave = k8s::permisos::clave(&ar, ns.as_deref());
        // Ya preguntado para este (recurso, namespace).
        if self
            .cluster_de(pane)
            .is_some_and(|c| c.permisos.contains_key(&clave))
        {
            return;
        }
        let bridge = self.bridge.clone();
        self.rt.spawn(async move {
            k8s::permisos::consultar(client, ar, ns, bridge).await;
        });
    }

    /// Permisos vigentes para lo que está mirando el panel.
    pub fn permisos_del_pane(&self, pane_id: u64) -> Option<&k8s::permisos::Permisos> {
        let pane = self.panes.iter().find(|p| p.id == pane_id)?;
        let item = pane.item.as_ref()?;
        let ar = item.res.ar.clone();
        let ns = item.res.namespaced.then(|| pane.ns_sel.clone()).flatten();
        let clave = k8s::permisos::clave(&ar, ns.as_deref());
        self.cluster_de(pane)?.permisos.get(&clave)
    }

    /// Sondeo de `metrics.k8s.io` para las columnas de CPU/memoria. Solo si el
    /// cluster lo sirve: sin metrics-server las columnas no aparecen.
    pub(super) fn sondear_metricas(&mut self, pane_id: u64) {
        let token = self.token();
        let Some(client) = self.client_del_pane(pane_id) else {
            return;
        };
        let Some(pane) = self.panes.iter().find(|p| p.id == pane_id) else {
            return;
        };
        let Some(cluster) = self.cluster_de(pane) else {
            return;
        };
        let kind = pane
            .item
            .as_ref()
            .map(|i| i.res.ar.kind.clone())
            .unwrap_or_default();
        let sirve = |k: &str| {
            cluster.info.as_ref().is_some_and(|i| {
                i.resources
                    .iter()
                    .any(|r| r.ar.kind == k && r.ar.group == "metrics.k8s.io")
            })
        };
        let ar = match kind.as_str() {
            "Pod" if sirve("PodMetrics") => k8s::metricas::ar_pods(),
            "Node" if sirve("NodeMetrics") => k8s::metricas::ar_nodes(),
            _ => return,
        };
        let Some(target) = pane.watch_target.clone() else {
            return;
        };
        let bridge = self.bridge.clone();
        let rt = &self.rt;
        let Some(pane) = self.panes.iter_mut().find(|p| p.id == pane_id) else {
            return;
        };
        pane.metricas_token = token;
        pane.metricas_tarea = Some(rt.spawn(async move {
            k8s::metricas::sondear(client, ar, target, token, bridge).await;
        }));
    }

    /// Watch auxiliar de endpoints, para la columna de backends de Services.
    pub(super) fn seguir_endpoints(&mut self, pane_id: u64) {
        let token = self.token();
        let Some(client) = self.client_del_pane(pane_id) else {
            return;
        };
        // EndpointSlice es lo actual; Endpoints quedó deprecado pero es lo único
        // que hay en clusters viejos.
        let ar = {
            let Some(pane) = self.panes.iter().find(|p| p.id == pane_id) else {
                return;
            };
            let Some(cluster) = self.cluster_de(pane) else {
                return;
            };
            let sirve = |k: &str| {
                cluster
                    .info
                    .as_ref()
                    .is_some_and(|i| i.resources.iter().any(|r| r.ar.kind == k && r.watchable()))
            };
            if sirve("EndpointSlice") {
                k8s::endpoints::ar_endpointslice()
            } else if sirve("Endpoints") {
                k8s::endpoints::ar_endpoints()
            } else {
                return;
            }
        };
        let bridge = self.bridge.clone();
        let rt = &self.rt;
        let Some(pane) = self.panes.iter_mut().find(|p| p.id == pane_id) else {
            return;
        };
        let Some(target) = pane.watch_target.clone() else {
            return;
        };
        pane.endpoints_token = token;
        pane.endpoints_tarea = Some(rt.spawn(async move {
            k8s::endpoints::seguir(client, ar, target, token, bridge).await;
        }));
    }

    pub fn cambiar_namespace(&mut self, pane_id: u64, ns: Option<String>) {
        if let Some(pane) = self.pane(pane_id) {
            pane.ns_sel = ns;
            if let Some(item) = pane.item.clone() {
                self.seleccionar(pane_id, item);
            }
        }
        // `seleccionar` no corre si el panel todavía no tenía recurso abierto.
        self.guardar_layout();
    }

    pub fn alternar_favorito(&mut self, pane_id: u64, key: &str) {
        let Some(pane) = self.pane(pane_id) else {
            return;
        };
        if !pane.favoritos.remove(key) {
            pane.favoritos.insert(key.to_string());
        }
        self.guardar_layout();
    }

    /// Vuelve a listar desde cero. Sin `forzar`, la guarda de "misma vista"
    /// hacía que el botón de recargar no hiciera nada.
    pub fn refrescar(&mut self, pane_id: u64) {
        if let Some(pane) = self.pane(pane_id) {
            if let Some(item) = pane.item.clone() {
                self.seleccionar_con(pane_id, item, true);
            }
        }
    }

    /// Navega a otro recurso: selecciona su Kind en el panel y deja marcado
    /// el detalle para abrirlo cuando la tabla cargue.
    pub fn ir_a(&mut self, pane_id: u64, kind: &str, ns: Option<String>, name: &str) {
        let item = self
            .panes
            .iter()
            .find(|p| p.id == pane_id)
            .and_then(|p| self.cluster_de(p))
            .and_then(|c| {
                c.nav
                    .iter()
                    .flat_map(|cat| cat.items.iter())
                    .find(|i| i.res.ar.kind == kind)
                    .cloned()
            });
        let Some(item) = item else {
            self.toast(format!("no hay vista para {kind} en este cluster"), true);
            return;
        };
        self.seleccionar(pane_id, item.clone());
        if name.is_empty() {
            // Solo cambiar de vista, sin abrir ningún detalle.
            return;
        }
        // Si el recurso está en otro namespace que el filtrado, se pasa a
        // "todos" para que aparezca en la tabla.
        if item.res.namespaced {
            let distinto = self
                .panes
                .iter()
                .find(|p| p.id == pane_id)
                .map(|p| p.ns_sel != ns)
                .unwrap_or(false);
            if distinto {
                if let Some(pane) = self.pane(pane_id) {
                    pane.ns_sel = ns.clone();
                }
                self.seleccionar(pane_id, item.clone());
            }
        }
        let key = match (&ns, item.res.namespaced) {
            (Some(ns), true) => format!("{ns}/{name}"),
            _ => name.to_string(),
        };
        // Si la vista ya estaba cargada (no hubo relistado) el objeto está a
        // mano: abrir el detalle ya, sin esperar un InitDone que no va a venir.
        let ya_esta = self
            .panes
            .iter()
            .find(|p| p.id == pane_id)
            .and_then(|p| p.store.as_ref())
            .is_some_and(|s| s.objeto(&key).is_some());
        if ya_esta {
            self.abrir_detalle(pane_id, &key);
        } else if let Some(pane) = self.pane(pane_id) {
            pane.pendiente_detalle = Some(key);
        }
    }
}
