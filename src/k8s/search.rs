//! Búsqueda global de recursos por nombre, para la paleta de comandos.

use futures::{stream, StreamExt};
use kube::api::{Api, DynamicObject, ListParams};
use kube::discovery::ApiResource;
use kube::Client;
use kube::ResourceExt;

use super::{K8sEvent, UiBridge};

/// Un resultado de búsqueda.
#[derive(Clone, Debug)]
pub struct Hit {
    pub kind: String,
    pub ns: Option<String>,
    pub name: String,
    /// Menor es mejor: prefijo exacto primero, después substring.
    pub rango: u8,
}

/// Kinds que se barren, en orden de utilidad. No se busca en todo el cluster:
/// un list por Kind ya son varios round-trips, y estos cubren el 95% de lo que
/// uno busca por nombre.
pub const KINDS_BUSCABLES: &[&str] = &[
    "Pod",
    "Deployment",
    "Service",
    "Ingress",
    "ConfigMap",
    "Secret",
    "StatefulSet",
    "DaemonSet",
    "CronJob",
    "Job",
    "PersistentVolumeClaim",
    "Node",
];

const TAM_PAGINA: u32 = 300;
const TIMEOUT_KIND_S: u64 = 4;

fn ordenar_y_acotar(hits: &mut Vec<Hit>) {
    hits.sort_by(|a, b| {
        a.rango
            .cmp(&b.rango)
            .then_with(|| a.name.len().cmp(&b.name.len()))
            .then_with(|| a.name.cmp(&b.name))
    });
    hits.truncate(60);
}

/// Barre una página por kind, con concurrencia acotada, y publica resultados
/// a medida que llegan. Las vistas normales sí paginan completas; la paleta
/// prioriza responder rápido y avisa cuando su resultado es parcial.
pub async fn buscar(
    client: Client,
    recursos: Vec<(ApiResource, bool)>,
    query: String,
    ns: Option<String>,
    token: u64,
    bridge: UiBridge,
) {
    let q = query.to_lowercase();
    let mut consultas = stream::iter(recursos.into_iter().map(|(ar, namespaced)| {
        let client = client.clone();
        let q = q.clone();
        let ns = ns.clone();
        async move {
            let kind = ar.kind.clone();
            let api: Api<DynamicObject> = match (&ns, namespaced) {
                (Some(ns), true) => Api::namespaced_with(client, ns, &ar),
                _ => Api::all_with(client, &ar),
            };
            let pagina = tokio::time::timeout(
                std::time::Duration::from_secs(TIMEOUT_KIND_S),
                api.list(&ListParams::default().limit(TAM_PAGINA)),
            )
            .await
            .map_err(|_| (kind.clone(), "timeout".to_string()))?
            .map_err(|e| (kind, e.to_string()))?;
            let parcial = pagina
                .metadata
                .continue_
                .as_deref()
                .is_some_and(|token| !token.is_empty());
            Ok::<_, (String, String)>((
                pagina
                    .items
                    .into_iter()
                    .filter_map(|o| {
                        let name = o.name_any();
                        let bajo = name.to_lowercase();
                        let rango = if bajo.starts_with(&q) {
                            0
                        } else if bajo.contains(&q) {
                            1
                        } else {
                            return None;
                        };
                        Some(Hit {
                            kind: ar.kind.clone(),
                            ns: o.namespace(),
                            name,
                            rango,
                        })
                    })
                    .collect::<Vec<_>>(),
                parcial,
            ))
        }
    }))
    .buffer_unordered(6);

    let mut hits = Vec::new();
    let mut errores = Vec::new();
    let mut parcial = false;
    while let Some(resultado) = consultas.next().await {
        match resultado {
            Ok((mut encontrados, truncado)) => {
                hits.append(&mut encontrados);
                parcial |= truncado;
            }
            Err((kind, error)) => {
                parcial = true;
                errores.push(format!("{kind}: {error}"));
            }
        }
        ordenar_y_acotar(&mut hits);
        bridge.send(K8sEvent::Search {
            token,
            hits: hits.clone(),
            completo: false,
            parcial,
        });
    }

    if !errores.is_empty() {
        tracing::warn!(errores = ?errores, "búsqueda parcial");
        bridge.toast(
            format!(
                "búsqueda parcial: fallaron {} de los tipos consultados",
                errores.len()
            ),
            true,
        );
    }
    bridge.send(K8sEvent::Search {
        token,
        hits,
        completo: true,
        parcial,
    });
}
