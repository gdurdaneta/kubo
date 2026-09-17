//! Lo que llega de la capa async al hilo de UI, y la persistencia del layout.

use super::*;

impl App {
    // ------------------------------------------------------------ canal -> UI

    /// Persiste qué está mirando cada panel, para volver acá al reabrir.
    pub(super) fn guardar_layout(&self) {
        crate::layout::guardar(&crate::layout::Estado {
            panes: self
                .panes
                .iter()
                .map(|p| crate::layout::PaneGuardado {
                    contexto: p.contexto.clone(),
                    ns: p.ns_sel.clone(),
                    recurso: p
                        .item
                        .as_ref()
                        .map(|i| i.res.key())
                        .or_else(|| p.recurso_pendiente.clone()),
                    favoritos: {
                        let mut favoritos: Vec<_> = p.favoritos.iter().cloned().collect();
                        favoritos.sort();
                        favoritos
                    },
                })
                .collect(),
        });
    }

    /// Panel activo, cayendo al primero si el recordado ya se cerró.
    pub fn pane_activo(&self) -> Option<u64> {
        if self.panes.iter().any(|p| p.id == self.pane_activo) {
            Some(self.pane_activo)
        } else {
            self.panes.first().map(|p| p.id)
        }
    }

    pub fn drenar_eventos(&mut self) {
        while let Ok(ev) = self.rx.try_recv() {
            self.aplicar_evento(ev);
        }
    }

    pub(super) fn aplicar_evento(&mut self, ev: K8sEvent) {
        match ev {
            K8sEvent::Connected {
                token,
                info,
                client,
            } => {
                let Some((_, cluster)) = self.clusters.iter_mut().find(|(_, c)| c.token == token)
                else {
                    return;
                };
                cluster.nav = crate::nav::build(&info.resources);
                cluster.info = Some(*info);
                cluster.client = Some(client);
                cluster.conn = Conn::Lista;
                let ids: Vec<u64> = self.panes.iter().map(|p| p.id).collect();
                for id in ids {
                    self.autoseleccionar(id);
                }
            }
            K8sEvent::ConnectFailed { token, error } => {
                if let Some((_, c)) = self.clusters.iter_mut().find(|(_, c)| c.token == token) {
                    c.conn = Conn::Error;
                    c.error = Some(error);
                }
            }
            K8sEvent::Permisos { clave, permisos } => {
                // Se guarda en el cluster, no en el panel: el RBAC es del
                // contexto y lo aprovechan todos los paneles que lo miran.
                let contextos: Vec<String> = self
                    .panes
                    .iter()
                    .filter_map(|p| p.contexto.clone())
                    .collect();
                for c in contextos {
                    if let Some(cl) = self.clusters.get_mut(&c) {
                        cl.permisos.insert(clave.clone(), permisos.clone());
                    }
                }
            }
            K8sEvent::Backends { token, items } => {
                for pane in &mut self.panes {
                    if let Some(d) = pane.detalle.as_mut() {
                        if d.backends_token == token {
                            d.backends = items;
                            d.backends_pedidos = false;
                            return;
                        }
                    }
                }
            }
            K8sEvent::Metricas { token, mapa } => {
                if let Some(p) = self.panes.iter_mut().find(|p| p.metricas_token == token) {
                    p.historial.agregar(&mapa);
                    p.metricas = mapa;
                }
            }
            K8sEvent::Endpoints { token, mapa } => {
                if let Some(p) = self.panes.iter_mut().find(|p| p.endpoints_token == token) {
                    p.endpoints = mapa;
                }
            }
            K8sEvent::PuertosSvc { servicio, puertos } => {
                if let Some(d) = self.dialogo_pf.as_mut() {
                    if d.servicio != servicio {
                        return;
                    }
                    // Por defecto, el mismo puerto que dentro del cluster; si es
                    // privilegiado se remapea a uno alto y la UI lo avisa.
                    d.puerto_local = puertos
                        .first()
                        .map(|p| k8s::portforward::puerto_local_sugerido(p.puerto).to_string())
                        .unwrap_or_default();
                    d.puertos = puertos;
                    d.cargando = false;
                }
                if std::env::var("KUBO_TEST_PF_AUTO").is_ok_and(|v| !v.is_empty()) {
                    std::env::set_var("KUBO_TEST_PF_AUTO", "");
                    self.abrir_forward();
                }
            }
            K8sEvent::Pf { id, msg } => {
                let Some(f) = self.forwards.iter_mut().find(|f| f.id == id) else {
                    return;
                };
                match msg {
                    k8s::PfMsg::Escuchando => {
                        f.estado = EstadoPf::Activo;
                        f.error = None;
                    }
                    k8s::PfMsg::Conexion(d) => f.conexiones = (f.conexiones + d).max(0),
                    k8s::PfMsg::Fatal(e) => {
                        f.estado = EstadoPf::Caido;
                        f.error = Some(e.clone());
                        self.toast(e, true);
                    }
                    // El listener sigue vivo: se anota en la fila y nada más.
                    k8s::PfMsg::FalloConexion(e) => f.error = Some(e),
                }
            }
            K8sEvent::Alias { id, error } => {
                if let Some(e) = error {
                    if let Some(f) = self.forwards.iter_mut().find(|f| f.id == id) {
                        // El listener ya está atado a su IP propia, así que
                        // `.localhost` no llegaría: se ofrece la IP directa.
                        f.alias = false;
                        f.host = f.bind.to_string();
                    }
                    self.toast(format!("alias no aplicado: {e}"), true);
                } else {
                    self.toast("/etc/hosts actualizado", false);
                }
            }
            K8sEvent::Version { token, version } => {
                if let Some((_, c)) = self.clusters.iter_mut().find(|(_, c)| c.token == token) {
                    if let Some(i) = c.info.as_mut() {
                        i.version = version;
                    }
                }
            }
            K8sEvent::Resources { token, resources } => {
                let Some((_, cluster)) = self.clusters.iter_mut().find(|(_, c)| c.token == token)
                else {
                    return;
                };
                cluster.nav = crate::nav::build(&resources);
                if let Some(i) = cluster.info.as_mut() {
                    i.resources = resources;
                }
                // Un panel que quedó sin vista porque el kind todavía no estaba
                // en la caché ahora sí puede abrirla.
                let ids: Vec<u64> = self.panes.iter().map(|p| p.id).collect();
                for id in ids {
                    self.autoseleccionar(id);
                }
            }
            K8sEvent::Namespaces { token, list } => {
                if let Some((_, c)) = self.clusters.iter_mut().find(|(_, c)| c.token == token) {
                    c.namespaces = list;
                }
            }
            K8sEvent::PodResuelto { pane, que, pod } => match que {
                k8s::pods::QuePod::Logs => self.abrir_logs_de(pane, &pod),
                k8s::pods::QuePod::Shell => self.abrir_shell_de(pane, &pod),
            },
            K8sEvent::ColumnasCrd {
                token,
                clave,
                columnas,
            } => {
                let mut contexto = None;
                if let Some(pane) = self.panes.iter_mut().find(|p| p.watch_token == token) {
                    if let Some(store) = pane.store.as_mut() {
                        store.set_columnas_crd(columnas.clone());
                    }
                    contexto = pane.contexto.clone();
                }
                if let Some(c) = contexto.and_then(|c| self.clusters.get_mut(&c)) {
                    c.columnas_crd.insert(clave, columnas);
                }
            }
            K8sEvent::Watch { token, msg } => {
                let mut error_toast = None;
                let mut init_listo: Option<u64> = None;
                if let Some(pane) = self.panes.iter_mut().find(|p| p.watch_token == token) {
                    if let Some(store) = pane.store.as_mut() {
                        match msg {
                            WatchMsg::Init => store.init_start(),
                            WatchMsg::InitBatch(objs) => store.init_batch(objs),
                            WatchMsg::InitDone => {
                                store.init_done();
                                init_listo = Some(pane.id);
                            }
                            WatchMsg::Apply(o) => store.apply(*o),
                            WatchMsg::Delete(o) => store.delete(&o),
                            WatchMsg::Error(e) => {
                                store.set_error(e.clone());
                                error_toast = Some(e);
                            }
                        }
                    }
                }
                if let Some(e) = error_toast {
                    self.toast(format!("watch: {e}"), true);
                }
                if let Some(pane_id) = init_listo {
                    // Navegación diferida: el usuario clickeó "ir al recurso".
                    let pendiente = self.pane(pane_id).and_then(|p| p.pendiente_detalle.take());
                    // KUBO_TEST_TAB=Yaml|Eventos|Mapa fija la pestaña al navegar.
                    let tab_forzada = std::env::var("KUBO_TEST_TAB")
                        .ok()
                        .filter(|s| !s.is_empty());
                    if let Some(key) = pendiente {
                        let existe = self
                            .panes
                            .iter()
                            .find(|p| p.id == pane_id)
                            .and_then(|p| p.store.as_ref())
                            .and_then(|s| s.objeto(&key))
                            .is_some();
                        if existe {
                            tracing::debug!(key, "navegación: detalle diferido abierto");
                            self.abrir_detalle(pane_id, &key);
                            if let Some(t) = tab_forzada {
                                if let Some(d) = self.pane(pane_id).and_then(|p| p.detalle.as_mut())
                                {
                                    d.tab = match t.as_str() {
                                        "Yaml" => TabDetalle::Yaml,
                                        "Eventos" => TabDetalle::Eventos,
                                        "Mapa" => TabDetalle::Mapa,
                                        _ => TabDetalle::Resumen,
                                    };
                                }
                            }
                        } else {
                            self.toast(format!("«{key}» no está en la vista actual"), true);
                        }
                    }
                    self.gancho_de_prueba(pane_id);
                }
            }
            K8sEvent::Yaml { token, text } => {
                for pane in &mut self.panes {
                    if let Some(d) = pane.detalle.as_mut() {
                        if d.yaml_token == token {
                            d.yaml = Some(text);
                            d.yaml_fresco = true;
                            return;
                        }
                    }
                }
            }
            K8sEvent::ObjectEvents { token, items } => {
                for pane in &mut self.panes {
                    if let Some(d) = pane.detalle.as_mut() {
                        if d.eventos_token == token {
                            d.eventos = items;
                            return;
                        }
                    }
                }
            }
            K8sEvent::Mapa { token, data } => {
                tracing::debug!(token, "mapa recibido");
                for pane in &mut self.panes {
                    if let Some(d) = pane.detalle.as_mut() {
                        if d.mapa_token == token {
                            d.mapa = Some(data);
                            return;
                        }
                    }
                }
            }
            K8sEvent::LogLine { token, line } => {
                for pane in &mut self.panes {
                    if let Some(Bottom::Logs(v)) = pane.bottom.as_mut() {
                        if v.token == token {
                            if v.lineas.len() >= MAX_LINEAS_LOG {
                                v.lineas.pop_front();
                            }
                            // Los logs con color (Nest, chalk, pino-pretty)
                            // traen secuencias ANSI que egui pinta como
                            // cuadraditos: se quitan, el color no se conserva.
                            v.lineas.push_back(super::logs_shell::sin_ansi(&line));
                            return;
                        }
                    }
                }
            }
            K8sEvent::LogClosed { token, error } => {
                for pane in &mut self.panes {
                    if let Some(Bottom::Logs(v)) = pane.bottom.as_mut() {
                        if v.token == token {
                            v.cerrado = Some(error.unwrap_or_else(|| "stream cerrado".into()));
                            return;
                        }
                    }
                }
            }
            K8sEvent::TermData { token, bytes } => {
                for pane in &mut self.panes {
                    if let Some(Bottom::Term(v)) = pane.bottom.as_mut() {
                        if v.token == token {
                            v.parser.process(&bytes);
                            return;
                        }
                    }
                }
            }
            K8sEvent::TermClosed { token, error } => {
                for pane in &mut self.panes {
                    if let Some(Bottom::Term(v)) = pane.bottom.as_mut() {
                        if v.token == token {
                            v.cerrado = Some(error.unwrap_or_else(|| "sesión terminada".into()));
                            return;
                        }
                    }
                }
            }
            K8sEvent::Search {
                token,
                hits,
                completo,
                parcial,
            } => {
                if let Some(p) = self.palette.as_mut() {
                    if p.token == token {
                        p.hits = hits;
                        p.buscando = !completo;
                        p.parcial = parcial;
                    }
                }
            }
            K8sEvent::Toast { text, error } => self.toast(text, error),
        }
    }
}
