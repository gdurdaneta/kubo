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
        // Aplicar YAML es de a uno; el resto acepta lote: una tarea por
        // objetivo, cada una con su toast y su línea de auditoría.
        if let Verbo::AplicarYaml(yaml) = c.verbo {
            self.rt.spawn(async move {
                k8s::actions::aplicar_yaml(client, ar, yaml, c.name, c.ns, ctx, bridge).await;
            });
            return;
        }
        let objetivos = std::iter::once((c.ns.clone(), c.name.clone())).chain(c.extra.into_iter());
        for (ns, name) in objetivos {
            let (client, ar, ctx, bridge) =
                (client.clone(), ar.clone(), ctx.clone(), bridge.clone());
            match c.verbo {
                Verbo::Borrar => {
                    self.rt.spawn(async move {
                        k8s::actions::borrar(client, ar, ns, name, ctx, bridge).await;
                    });
                }
                Verbo::Reiniciar => {
                    if c.kind == "Pod" {
                        // Reiniciar un pod es borrarlo: el controlador lo repone.
                        self.rt.spawn(async move {
                            k8s::actions::borrar(client, ar, ns, name, ctx, bridge).await;
                        });
                    } else {
                        self.rt.spawn(async move {
                            k8s::actions::reiniciar(client, ar, ns, name, ctx, bridge).await;
                        });
                    }
                }
                Verbo::Escalar(n) => {
                    self.rt.spawn(async move {
                        k8s::actions::escalar(client, ar, ns, name, n, ctx, bridge).await;
                    });
                }
                Verbo::AplicarYaml(_) => unreachable!(),
            }
        }
        // La selección ya se consumió.
        if let Some(p) = self.pane(c.pane) {
            p.seleccion.clear();
        }
    }

    pub fn aplicar_yaml(&mut self, pane_id: u64, yaml: String) {
        // El nombre/ns esperados salen del detalle abierto, no del YAML.
        let Some((name, ns, kind, revelar, original)) = self
            .panes
            .iter()
            .find(|p| p.id == pane_id)
            .and_then(|p| p.detalle.as_ref())
            .map(|d| {
                (
                    d.name.clone(),
                    d.ns.clone(),
                    d.kind.clone(),
                    d.revelar,
                    d.yaml.clone().unwrap_or_default(),
                )
            })
        else {
            return;
        };
        // El diff es lo que se confirma: sin cambios no hay nada que aplicar.
        let diff = diff_unificado(&original, &yaml);
        if diff.is_empty() {
            self.toast("el manifiesto no tiene cambios", false);
            return;
        }
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
            diff: Some(diff),
            tecleado: String::new(),
            extra: Vec::new(),
        });
    }
}

/// Diff unificado (3 líneas de contexto) entre dos textos; vacío si son iguales.
pub fn diff_unificado(antes: &str, despues: &str) -> String {
    if antes == despues {
        return String::new();
    }
    similar::TextDiff::from_lines(antes, despues)
        .unified_diff()
        .context_radius(3)
        .header("api-server", "editado")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::diff_unificado;

    #[test]
    fn diff_vacio_si_no_cambia_y_unificado_si_cambia() {
        assert_eq!(diff_unificado("a: 1\n", "a: 1\n"), "");
        let d = diff_unificado("a: 1\nb: 2\n", "a: 1\nb: 3\n");
        assert!(d.contains("-b: 2"), "{d}");
        assert!(d.contains("+b: 3"), "{d}");
        assert!(d.starts_with("--- api-server"), "{d}");
    }
}
