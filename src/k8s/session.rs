//! Conexión a un contexto: cliente, versión del server y discovery completo
//! (incluye CRDs, por eso la navegación no está hardcodeada).

use anyhow::{Context as _, Result};
use kube::api::{Api, ListParams};
use kube::config::{KubeConfigOptions, Kubeconfig};
use kube::discovery::{Discovery, Scope};
use kube::{Client, Config};

use super::{cache, ClusterInfo, Discovered};

/// Construye un cliente para el contexto indicado.
pub async fn client_for(context: &str) -> Result<(Client, String)> {
    let opts = KubeConfigOptions {
        context: Some(context.to_string()),
        cluster: None,
        user: None,
    };
    let config = Config::from_kubeconfig(&opts)
        .await
        .with_context(|| format!("kubeconfig inválido para el contexto {context}"))?;
    let server = config.cluster_url.to_string();
    let client = Client::try_from(config).context("no se pudo construir el cliente HTTP")?;
    Ok((client, server))
}

/// Namespace por defecto declarado en el contexto del kubeconfig, si hay.
pub fn default_namespace(context: &str) -> Option<String> {
    let cfg = Kubeconfig::read().ok()?;
    cfg.contexts
        .iter()
        .find(|c| c.name == context)
        .and_then(|c| c.context.as_ref())
        .and_then(|c| c.namespace.clone())
}

/// Aplana el resultado del discovery a la lista que consume la navegación.
fn aplanar(discovery: &Discovery) -> Vec<Discovered> {
    let mut resources = Vec::new();
    for group in discovery.groups() {
        for (ar, caps) in group.resources_by_stability() {
            // Los subrecursos (pods/log, pods/exec) no son navegables.
            if ar.plural.contains('/') {
                continue;
            }
            resources.push(Discovered {
                namespaced: matches!(caps.scope, Scope::Namespaced),
                verbs: caps.operations.clone(),
                ar,
            });
        }
    }
    resources.sort_by(|a, b| a.ar.kind.cmp(&b.ar.kind));
    resources
}

/// Enumera la API del cluster. Usa el endpoint agregado (2 requests) con
/// fallback al recorrido grupo por grupo (~2 por grupo) para API servers
/// viejos: es la diferencia entre un round trip y cincuenta.
async fn descubrir(client: Client) -> Result<Vec<Discovered>> {
    let d = match Discovery::new(client.clone()).run_aggregated().await {
        Ok(d) => d,
        Err(e) => {
            tracing::info!(error = %e, "discovery agregado no disponible; fallback al secuencial");
            Discovery::new(client).run().await?
        }
    };
    Ok(aplanar(&d))
}

/// Conecta al contexto y devuelve con qué pintar la UI cuanto antes.
///
/// Con discovery cacheado no se espera ninguna request: la navegación aparece
/// al instante y el refresco corre por detrás. Sin caché hay que enumerar la
/// API sí o sí, que en un cluster remoto son un par de segundos.
pub async fn connect(context: &str) -> Result<(Client, ClusterInfo, bool)> {
    let t0 = std::time::Instant::now();
    // Ojo con este tramo: en EKS/GKE el kubeconfig usa un exec plugin
    // (`aws eks get-token`) y armar el cliente lanza un proceso externo. Suele
    // ser el tramo más caro y no depende de la red.
    let (client, server) = client_for(context).await?;
    let ms_cliente = t0.elapsed().as_millis() as u64;

    if let Some((resources, vencida)) = cache::leer(&server) {
        tracing::info!(
            ms = t0.elapsed().as_millis() as u64,
            ms_cliente,
            n = resources.len(),
            vencida,
            "conexión establecida (discovery cacheado)"
        );
        return Ok((
            client,
            ClusterInfo {
                server,
                version: String::new(),
                resources,
            },
            true,
        ));
    }

    let t1 = std::time::Instant::now();
    let resources = descubrir(client.clone()).await?;
    tracing::info!(
        ms = t0.elapsed().as_millis() as u64,
        ms_cliente,
        ms_discovery = t1.elapsed().as_millis() as u64,
        n = resources.len(),
        "conexión establecida"
    );
    cache::escribir(&server, &resources);

    Ok((
        client,
        ClusterInfo {
            server,
            version: String::new(),
            resources,
        },
        false,
    ))
}

/// Versión del API server. Sin permiso sobre /version se puede trabajar igual.
pub async fn version(client: Client) -> String {
    match client.apiserver_version().await {
        Ok(v) => format!("{}.{}", v.major, v.minor),
        Err(_) => "?".to_string(),
    }
}

/// Rehace el discovery y actualiza la caché. Devuelve None si no cambió nada
/// respecto de lo que ya tiene la UI.
pub async fn refrescar_discovery(
    client: Client,
    server: String,
    actual: usize,
) -> Option<Vec<Discovered>> {
    let t = std::time::Instant::now();
    let resources = descubrir(client).await.ok()?;
    tracing::info!(
        ms = t.elapsed().as_millis() as u64,
        n = resources.len(),
        "discovery refrescado"
    );
    cache::escribir(&server, &resources);
    if resources.len() == actual {
        return None;
    }
    Some(resources)
}

/// Lista de namespaces. Si el usuario no tiene permiso devuelve vacío en vez
/// de fallar: en clusters con RBAC acotado eso es lo normal.
pub async fn namespaces(client: Client) -> Vec<String> {
    let ar = kube::discovery::ApiResource {
        group: String::new(),
        version: "v1".into(),
        api_version: "v1".into(),
        kind: "Namespace".into(),
        plural: "namespaces".into(),
    };
    let api: Api<kube::api::DynamicObject> = Api::all_with(client, &ar);
    match api.list(&ListParams::default().limit(500)).await {
        Ok(list) => {
            let mut names: Vec<String> = list
                .items
                .into_iter()
                .filter_map(|o| o.metadata.name)
                .collect();
            names.sort();
            names
        }
        Err(_) => Vec::new(),
    }
}
