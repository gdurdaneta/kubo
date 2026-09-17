//! Mutaciones confirmadas: borrar, escalar, reiniciar y aplicar YAML.

use super::*;

impl App {
    // ------------------------------------------------------------ acciones

    pub(super) fn client_del_pane(&self, pane_id: u64) -> Option<Client> {
        self.panes
            .iter()
            .find(|p| p.id == pane_id)
            .and_then(|p| p.contexto.as_ref())
            .and_then(|c| self.clusters.get(c))
            .and_then(|c| c.client.clone())
    }

    pub(super) fn ar_del_pane(&self, pane_id: u64) -> Option<kube::discovery::ApiResource> {
        self.panes
            .iter()
            .find(|p| p.id == pane_id)
            .and_then(|p| p.item.as_ref())
            .map(|i| i.res.ar.clone())
    }

    /// Nombre del contexto que mira un panel. Va al registro de auditoría:
    /// saber qué se tocó sin saber en qué cluster no sirve de nada.
    pub(super) fn contexto_del_pane(&self, pane_id: u64) -> String {
        self.panes
            .iter()
            .find(|p| p.id == pane_id)
            .and_then(|p| p.contexto.clone())
            .unwrap_or_else(|| "(desconocido)".into())
    }

    /// Ejecuta la acción ya confirmada del modal.
    pub fn ejecutar_confirmada(&mut self) {
        let Some(c) = self.confirm.take() else { return };
        let (Some(client), Some(ar)) = (self.client_del_pane(c.pane), self.ar_del_pane(c.pane))
        else {
            return;
        };
        let ctx = self.contexto_del_pane(c.pane);
        let bridge = self.bridge.clone();
        match c.verbo {
            Verbo::Borrar => {
                self.rt.spawn(async move {
                    k8s::actions::borrar(client, ar, c.ns, c.name, ctx, bridge).await;
                });
            }
            Verbo::Reiniciar => {
                if c.kind == "Pod" {
                    // Reiniciar un pod es borrarlo: el controlador lo repone.
                    self.rt.spawn(async move {
                        k8s::actions::borrar(client, ar, c.ns, c.name, ctx, bridge).await;
                    });
                } else {
                    self.rt.spawn(async move {
                        k8s::actions::reiniciar(client, ar, c.ns, c.name, ctx, bridge).await;
                    });
                }
            }
            Verbo::Escalar(n) => {
                self.rt.spawn(async move {
                    k8s::actions::escalar(client, ar, c.ns, c.name, n, ctx, bridge).await;
                });
            }
            Verbo::AplicarYaml(yaml) => {
                self.rt.spawn(async move {
                    k8s::actions::aplicar_yaml(client, ar, yaml, c.name, c.ns, ctx, bridge).await;
                });
            }
        }
    }

    pub fn aplicar_yaml(&mut self, pane_id: u64, yaml: String) {
        // El nombre/ns esperados salen del detalle abierto, no del YAML.
        let Some((name, ns, kind, revelar)) = self
            .panes
            .iter()
            .find(|p| p.id == pane_id)
            .and_then(|p| p.detalle.as_ref())
            .map(|d| (d.name.clone(), d.ns.clone(), d.kind.clone(), d.revelar))
        else {
            return;
        };
        // Aplicar un Secret enmascarado escribiría el marcador como valor.
        if kind == "Secret" && !revelar {
            self.toast(
                "revelá el Secret antes de aplicarlo: los valores están ocultos",
                true,
            );
            return;
        }
        self.confirm = Some(Confirmacion {
            pane: pane_id,
            verbo: Verbo::AplicarYaml(yaml),
            kind,
            ns,
            name,
        });
    }
}
