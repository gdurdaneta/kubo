//! Panel de detalle: YAML, eventos del objeto, secrets revelados y mapa.

use super::*;

impl App {
    // ------------------------------------------------------------- detalle

    pub fn abrir_detalle(&mut self, pane_id: u64, key: &str) {
        self.abrir_detalle_en(pane_id, key, TabDetalle::Resumen);
    }

    /// Igual que `abrir_detalle` pero aterrizando en una pestaña concreta:
    /// "Editar manifiesto" entra directo al YAML y "Mapa" al mapa, sin obligar
    /// a pasar por Resumen.
    pub fn abrir_detalle_en(&mut self, pane_id: u64, key: &str, tab: TabDetalle) {
        tracing::debug!(pane_id, key, "abrir_detalle");
        let ar_endpoints_del_pane = self.ar_endpoints(pane_id);
        let yaml_token = self.token();
        let eventos_token = self.token();
        let backends_token = self.token();
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
        let (Some(store), Some(item)) = (pane.store.as_ref(), pane.item.as_ref()) else {
            return;
        };
        let Some(obj) = store.objeto(key) else { return };

        let name = kube::ResourceExt::name_any(obj);
        let ns = kube::ResourceExt::namespace(obj);
        let uid = kube::ResourceExt::uid(obj);
        let ar = item.res.ar.clone();
        let es_service = ar.kind == "Service" && ar.group.is_empty();

        // El watch ya tiene el objeto: mostrarlo al instante evita que el panel
        // quede en blanco durante el round trip. Los Secrets no: los enmascara
        // la capa async y acá se filtrarían en claro.
        let adelanto = if ar.kind == "Secret" {
            None
        } else {
            let mut o = obj.clone();
            o.metadata.managed_fields = None;
            // El watch entrega los objetos sin TypeMeta; sin esto el adelanto
            // arrancaría en `metadata:` y se vería distinto de la copia buena.
            if o.types.is_none() {
                o.types = Some(kube::core::TypeMeta {
                    api_version: ar.api_version.clone(),
                    kind: ar.kind.clone(),
                });
            }
            serde_yaml_ng::to_string(&o).ok()
        };

        for t in pane.detalle_tareas.drain(..) {
            t.abort();
        }

        pane.detalle = Some(Detalle {
            key: key.to_string(),
            kind: ar.kind.clone(),
            name: name.clone(),
            ns: ns.clone(),
            tab,
            yaml: adelanto,
            yaml_edit: None,
            yaml_token,
            yaml_fresco: false,
            backends: Vec::new(),
            backends_token,
            backends_pedidos: false,
            eventos: Vec::new(),
            eventos_token,
            eventos_pedidos: uid.is_some(),
            revelar: false,
            editando: false,
            mapa: None,
            mapa_token: 0,
        });

        let b = self.bridge.clone();
        let (c, n, nn) = (client.clone(), ns.clone(), name.clone());
        pane.detalle_tareas.push(self.rt.spawn(async move {
            k8s::detail::fetch_yaml(c, ar, n, nn, false, yaml_token, b).await;
        }));
        if es_service {
            if let (Some(ns), Some(ar_ep)) = (ns.clone(), ar_endpoints_del_pane) {
                if let Some(d) = pane.detalle.as_mut() {
                    d.backends_pedidos = true;
                }
                let (c, b, n) = (client.clone(), self.bridge.clone(), name.clone());
                pane.detalle_tareas.push(self.rt.spawn(async move {
                    k8s::endpoints::backends(c, ar_ep, ns, n, backends_token, b).await;
                }));
            }
        }
        if let Some(uid) = uid {
            let b = self.bridge.clone();
            pane.detalle_tareas.push(self.rt.spawn(async move {
                k8s::detail::fetch_events(client, uid, ns, eventos_token, b).await;
            }));
        }
        // El mapa normalmente se pide al entrar en la pestaña; si abrimos ya
        // parados ahí, nadie lo dispararía.
        if tab == TabDetalle::Mapa {
            self.pedir_mapa(pane_id);
        }
    }

    /// Re-pide el YAML del objeto abierto, revelando o volviendo a ocultar los
    /// valores de un Secret.
    pub fn alternar_revelar(&mut self, pane_id: u64) {
        let token = self.token();
        let Some(client) = self.client_del_pane(pane_id) else {
            return;
        };
        let Some(ar) = self.ar_del_pane(pane_id) else {
            return;
        };
        let bridge = self.bridge.clone();
        let rt_ref = &self.rt;
        let Some(pane) = self.panes.iter_mut().find(|p| p.id == pane_id) else {
            return;
        };
        let Some(det) = pane.detalle.as_mut() else {
            return;
        };
        det.revelar = !det.revelar;
        det.yaml = None;
        det.yaml_edit = None;
        det.yaml_fresco = false;
        det.editando = false;
        det.yaml_token = token;
        let (ns, name, revelar) = (det.ns.clone(), det.name.clone(), det.revelar);
        pane.detalle_tareas.push(rt_ref.spawn(async move {
            k8s::detail::fetch_yaml(client, ar, ns, name, revelar, token, bridge).await;
        }));
    }

    /// Pide (o re-pide) el mapa del objeto abierto en el detalle: de tráfico
    /// para un Service, de configuración para un workload.
    pub fn pedir_mapa(&mut self, pane_id: u64) {
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
        let ar = pane.item.as_ref().map(|i| i.res.ar.clone());
        let Some(det) = pane.detalle.as_mut() else {
            return;
        };
        let Some(ns) = det.ns.clone() else { return };
        det.mapa_token = token;
        det.mapa = None;
        let name = det.name.clone();
        let kind = det.kind.clone();
        let bridge = self.bridge.clone();
        if kind == "Service" {
            pane.detalle_tareas.push(self.rt.spawn(async move {
                k8s::mapa::fetch_service(client, ns, name, token, bridge).await;
            }));
        } else if let Some(ar) = ar {
            pane.detalle_tareas.push(self.rt.spawn(async move {
                k8s::mapa::fetch_workload(client, ar, ns, name, token, bridge).await;
            }));
        }
    }
}
