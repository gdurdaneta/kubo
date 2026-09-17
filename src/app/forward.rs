//! Port-forward de Services: diálogo, túnel y alias en /etc/hosts.

use super::*;

impl App {
    // ------------------------------------------------- port-forward

    /// Abre el diálogo de port-forward leyendo los puertos del Service.
    pub fn pedir_forward(&mut self, pane_id: u64, key: &str) {
        let Some(pane) = self.panes.iter().find(|p| p.id == pane_id) else {
            return;
        };
        let (Some(contexto), Some(client)) = (pane.contexto.clone(), self.client_del_pane(pane_id))
        else {
            return;
        };
        let (ns, servicio) = match key.split_once('/') {
            Some((ns, n)) => (ns.to_string(), n.to_string()),
            None => {
                self.toast("el Service tiene que estar en un namespace", true);
                return;
            }
        };

        self.dialogo_pf = Some(DialogoPf {
            pane: pane_id,
            contexto,
            ns: ns.clone(),
            servicio: servicio.clone(),
            puertos: Vec::new(),
            sel: 0,
            puerto_local: String::new(),
            alias: false,
            cargando: true,
        });

        let bridge = self.bridge.clone();
        self.rt.spawn(async move {
            match k8s::portforward::puertos_de(client, &ns, &servicio).await {
                Ok(ps) => bridge.send(K8sEvent::PuertosSvc {
                    servicio,
                    puertos: ps,
                }),
                Err(e) => bridge.toast(format!("{e:#}"), true),
            }
        });
    }

    /// Levanta el forward configurado en el diálogo.
    pub fn abrir_forward(&mut self) {
        let Some(d) = self.dialogo_pf.take() else {
            return;
        };
        let Some(puerto) = d.puertos.get(d.sel).cloned() else {
            return;
        };
        let Some(client) = self
            .clusters
            .get(&d.contexto)
            .and_then(|c| c.client.clone())
        else {
            return;
        };
        let puerto_local: u16 = match d.puerto_local.trim().parse() {
            Ok(p) if p > 0 => p,
            _ => {
                self.toast("puerto local inválido", true);
                return;
            }
        };
        if d.alias && !crate::hosts::nombre_valido(&d.servicio) {
            self.toast(
                format!("'{}' no sirve como alias en /etc/hosts", d.servicio),
                true,
            );
            return;
        }
        if self.forwards.iter().any(|f| {
            f.bind == k8s::portforward::bind_de(d.alias, &d.servicio)
                && f.puerto_local == puerto_local
        }) {
            self.toast("ya hay un forward escuchando en esa dirección", true);
            return;
        }

        let id = self.token();
        let bind = k8s::portforward::bind_de(d.alias, &d.servicio);
        let host = k8s::portforward::host_de(d.alias, &d.servicio);
        self.forwards.push(Forward {
            id,
            contexto: d.contexto.clone(),
            ns: d.ns.clone(),
            servicio: d.servicio.clone(),
            puerto_svc: puerto.puerto,
            puerto_local,
            bind,
            host,
            alias: d.alias,
            estado: EstadoPf::Levantando,
            conexiones: 0,
            error: None,
            tarea: None,
        });
        // Mostrar la lista apenas se levanta, para no dejarla escondida.
        if let Some(p) = self.pane(d.pane) {
            p.vista_local = Some(VistaLocal::PortForwards);
        }

        // El alias va primero: si el usuario cancela el diálogo de polkit, no
        // tiene sentido dejar el listener arriba con un nombre que no resuelve.
        if d.alias {
            self.sincronizar_alias(id);
        }

        let (ns, servicio, bridge) = (d.ns.clone(), d.servicio.clone(), self.bridge.clone());
        let addr = std::net::SocketAddr::new(bind, puerto_local);
        let tarea = self.rt.spawn(async move {
            let (pod, puerto_pod) =
                match k8s::portforward::elegir_pod(client.clone(), &ns, &servicio, &puerto).await {
                    Ok(v) => v,
                    Err(e) => {
                        bridge.send(K8sEvent::Pf {
                            id,
                            msg: k8s::PfMsg::Fatal(format!("{e:#}")),
                        });
                        return;
                    }
                };
            k8s::portforward::servir(client, ns, pod, puerto_pod, addr, id, bridge).await;
        });
        if let Some(f) = self.forwards.iter_mut().find(|f| f.id == id) {
            f.tarea = Some(tarea);
        }
    }

    /// Muestra los port-forward en el panel, en vez de la tabla de recursos.
    pub fn ver_vista_local(&mut self, pane_id: u64, v: VistaLocal) {
        if let Some(p) = self.pane(pane_id) {
            p.vista_local = Some(v);
        }
    }

    pub fn cerrar_forward(&mut self, id: u64) {
        let Some(i) = self.forwards.iter().position(|f| f.id == id) else {
            return;
        };
        let mut f = self.forwards.remove(i);
        if let Some(t) = f.tarea.take() {
            t.abort();
        }
        // El alias solo se saca si ningún otro forward lo sigue usando.
        if f.alias {
            self.sincronizar_alias(0);
        }
    }

    /// Deja /etc/hosts con los alias de los forwards vivos. `id` es a quién
    /// culpar si falla (0 = a nadie en particular).
    pub(super) fn sincronizar_alias(&mut self, id: u64) {
        let entradas: Vec<(std::net::IpAddr, String)> = self
            .forwards
            .iter()
            .filter(|f| f.alias)
            .map(|f| (f.bind, f.servicio.clone()))
            .collect();
        if entradas == crate::hosts::actuales() {
            return;
        }
        let bridge = self.bridge.clone();
        // pkexec bloquea hasta que el usuario responde el diálogo.
        self.rt.spawn_blocking(move || {
            let error = crate::hosts::aplicar(&entradas)
                .err()
                .map(|e| format!("{e:#}"));
            bridge.send(K8sEvent::Alias { id, error });
        });
    }
}
