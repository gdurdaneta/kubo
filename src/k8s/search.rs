//! Búsqueda global de recursos por nombre, para la paleta de comandos.

use futures::future::join_all;
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

/// Barre los kinds en paralelo y devuelve las coincidencias por substring.
///
/// El `limit` por Kind acota el costo en clusters grandes: en ese caso la
/// búsqueda es parcial, y la UI lo avisa.
pub async fn buscar(
    client: Client,
    recursos: Vec<(ApiResource, bool)>,
    query: String,
    ns: Option<String>,
    token: u64,
    bridge: UiBridge,
) {
    let q = query.to_lowercase();
    let consultas = recursos.into_iter().map(|(ar, namespaced)| {
        let client = client.clone();
        let q = q.clone();
        let ns = ns.clone();
        async move {
            let api: Api<DynamicObject> = match (&ns, namespaced) {
                (Some(ns), true) => Api::namespaced_with(client, ns, &ar),
                _ => Api::all_with(client, &ar),
            };
            let lista = match api.list(&ListParams::default().limit(500)).await {
                Ok(l) => l,
                Err(_) => return Vec::new(),
            };
            lista
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
                .collect::<Vec<_>>()
        }
    });

    let mut hits: Vec<Hit> = join_all(consultas).await.into_iter().flatten().collect();
    hits.sort_by(|a, b| {
        a.rango
            .cmp(&b.rango)
            .then_with(|| a.name.len().cmp(&b.name.len()))
            .then_with(|| a.name.cmp(&b.name))
    });
    hits.truncate(60);

    bridge.send(K8sEvent::Search { token, hits });
}
