//! Conexión a clusters: clientes compartidos por contexto, reconexión y selección inicial.

use super::*;

impl App {
    // ------------------------------------------------------------ conexión

    /// Lanza la conexión si nadie la pidió todavía.
    pub(super) fn asegurar_cluster(&mut self, contexto: &str) {
        if self.clusters.contains_key(contexto) {
            return;
        }
        let token = self.token();
        self.clusters.insert(
            contexto.to_string(),
            Cluster {
                conn: Conn::Conectando,
                error: None,
                token,
                client: None,
                info: None,
                nav: Vec::new(),
                namespaces: Vec::new(),
                permisos: HashMap::new(),
                columnas_crd: HashMap::new(),
            },
        );
        let bridge = self.bridge.clone();
        let ctx_name = contexto.to_string();
        self.rt.spawn(async move {
            // Sin tope, un cluster inalcanzable (VPN caída, endpoint viejo)
            // dejaba el panel en «Conectando…» hasta que se rindiera el
            // sistema operativo, minutos después y sin forma de cancelar.
            let intento = tokio::time::timeout(
                std::time::Duration::from_secs(TIMEOUT_CONEXION),
                k8s::session::connect(&ctx_name),
            )
            .await;
            let (client, info, desde_cache) = match intento {
                Ok(Ok(v)) => v,
                Ok(Err(e)) => {
                    bridge.send(K8sEvent::ConnectFailed {
                        token,
                        error: format!("{e:#}"),
                    });
                    return;
                }
                Err(_) => {
                    bridge.send(K8sEvent::ConnectFailed {
                        token,
                        error: format!(
                            "el cluster no respondió en {TIMEOUT_CONEXION} s. \
                             ¿Está levantado y tenés red o VPN hacia él?"
                        ),
                    });
                    return;
                }
            };
            // La UI ya puede pintar la navegación; lo demás llega solo.
            let server = info.server.clone();
            let n_recursos = info.resources.len();
            bridge.send(K8sEvent::Connected {
                token,
                info: Box::new(info),
                client: client.clone(),
            });

            let (c1, c2, b1, b2) = (
                client.clone(),
                client.clone(),
                bridge.clone(),
                bridge.clone(),
            );
            tokio::join!(
                async move {
                    let version = k8s::session::version(c1).await;
                    b1.send(K8sEvent::Version { token, version });
                },
                async move {
                    let list = k8s::session::namespaces(c2).await;
                    b2.send(K8sEvent::Namespaces { token, list });
                },
                async move {
                    // Con caché la lista mostrada puede estar vieja (un CRD
                    // nuevo, un operador instalado); se rehace por detrás.
                    if !desde_cache {
                        return;
                    }
                    if let Some(resources) =
                        k8s::session::refrescar_discovery(client, server, n_recursos).await
                    {
                        bridge.send(K8sEvent::Resources { token, resources });
                    }
                },
            );
        });
    }

    /// Reintenta una conexión fallida.
    pub fn reconectar(&mut self, contexto: &str) {
        self.clusters.remove(contexto);
        self.asegurar_cluster(contexto);
    }

    pub fn cambiar_contexto(&mut self, pane_id: u64, contexto: String) {
        let ns = k8s::session::default_namespace(&contexto);
        if let Some(pane) = self.pane(pane_id) {
            pane.limpiar_vista();
            pane.contexto = Some(contexto.clone());
            pane.ns_sel = ns;
        }
        self.asegurar_cluster(&contexto);
        self.autoseleccionar(pane_id);
        self.soltar_clusters_sin_uso();
        self.guardar_layout();
    }

    /// Descarta las conexiones que ya no mira ningún panel.
    ///
    /// Cada `Cluster` retiene un cliente HTTP con su pool; ir y volver entre
    /// contextos los iba acumulando para toda la sesión.
    pub(super) fn soltar_clusters_sin_uso(&mut self) {
        let en_uso: HashSet<&str> = self
            .panes
            .iter()
            .filter_map(|p| p.contexto.as_deref())
            .collect();
        // Un forward vivo sigue usando su cliente aunque el panel ya no esté.
        let con_forward: HashSet<&str> =
            self.forwards.iter().map(|f| f.contexto.as_str()).collect();
        self.clusters
            .retain(|k, _| en_uso.contains(k.as_str()) || con_forward.contains(k.as_str()));
    }

    /// Abre Pods si el panel no tiene nada seleccionado y su cluster ya está.
    pub(super) fn autoseleccionar(&mut self, pane_id: u64) {
        let Some(pane) = self.panes.iter().find(|p| p.id == pane_id) else {
            return;
        };
        if pane.item.is_some() {
            return;
        }
        let Some(cluster) = self.cluster_de(pane) else {
            return;
        };
        if cluster.conn != Conn::Lista {
            return;
        }
        // Lo que estaba abierto la sesión pasada gana; Pods es el default.
        let deseado = pane.recurso_pendiente.clone();
        let elegido = cluster
            .nav
            .iter()
            .flat_map(|c| c.items.iter())
            .find(|i| match &deseado {
                Some(k) => i.res.key() == *k,
                None => i.res.ar.kind == "Pod",
            })
            .or_else(|| {
                // El recurso guardado ya no está (se desinstaló un operador).
                cluster
                    .nav
                    .iter()
                    .flat_map(|c| c.items.iter())
                    .find(|i| i.res.ar.kind == "Pod")
            })
            .cloned();
        if let Some(item) = elegido {
            if let Some(p) = self.pane(pane_id) {
                p.recurso_pendiente = None;
            }
            self.seleccionar(pane_id, item);
        }
    }
}
