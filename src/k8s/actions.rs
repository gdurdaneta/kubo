//! Mutaciones sobre el cluster: borrar, escalar, reiniciar y aplicar YAML.
//! Todas reportan por toast; la tabla se actualiza sola vía el watch.

use k8s_openapi::jiff::Timestamp;
use kube::api::{Api, DeleteParams, DynamicObject, Patch, PatchParams, PostParams};
use kube::discovery::ApiResource;
use kube::Client;
use serde_json::json;

use super::UiBridge;

fn api_for(client: Client, ar: &ApiResource, ns: Option<&str>) -> Api<DynamicObject> {
    match ns {
        Some(ns) => Api::namespaced_with(client, ns, ar),
        None => Api::all_with(client, ar),
    }
}

pub async fn borrar(
    client: Client,
    ar: ApiResource,
    ns: Option<String>,
    name: String,
    bridge: UiBridge,
) {
    let api = api_for(client, &ar, ns.as_deref());
    match api.delete(&name, &DeleteParams::default()).await {
        Ok(_) => bridge.toast(format!("{} «{name}» borrado", ar.kind), false),
        Err(e) => bridge.toast(format!("no se pudo borrar {name}: {e}"), true),
    }
}

pub async fn escalar(
    client: Client,
    ar: ApiResource,
    ns: Option<String>,
    name: String,
    replicas: i64,
    bridge: UiBridge,
) {
    let api = api_for(client, &ar, ns.as_deref());
    let patch = json!({ "spec": { "replicas": replicas } });
    match api
        .patch(&name, &PatchParams::default(), &Patch::Merge(&patch))
        .await
    {
        Ok(_) => bridge.toast(format!("«{name}» escalado a {replicas} réplicas"), false),
        Err(e) => bridge.toast(format!("no se pudo escalar {name}: {e}"), true),
    }
}

/// Rollout restart: la misma anotación que pone `kubectl rollout restart`.
pub async fn reiniciar(
    client: Client,
    ar: ApiResource,
    ns: Option<String>,
    name: String,
    bridge: UiBridge,
) {
    let api = api_for(client, &ar, ns.as_deref());
    let ahora = Timestamp::now().to_string();
    let patch = json!({
        "spec": { "template": { "metadata": { "annotations": {
            "kubectl.kubernetes.io/restartedAt": ahora
        }}}}
    });
    match api
        .patch(&name, &PatchParams::default(), &Patch::Merge(&patch))
        .await
    {
        Ok(_) => bridge.toast(format!("rollout restart de «{name}» disparado"), false),
        Err(e) => bridge.toast(format!("no se pudo reiniciar {name}: {e}"), true),
    }
}

/// Reemplaza el objeto con el YAML editado (PUT, optimista por resourceVersion).
///
/// El nombre y el namespace tienen que coincidir con el objeto abierto: en
/// Kubernetes no existe renombrar, y aplicar sobre otro nombre sería editar
/// un recurso distinto al que el usuario cree estar tocando.
pub async fn aplicar_yaml(
    client: Client,
    ar: ApiResource,
    yaml: String,
    esperado_name: String,
    esperado_ns: Option<String>,
    bridge: UiBridge,
) {
    let obj: DynamicObject = match serde_yaml_ng::from_str(&yaml) {
        Ok(o) => o,
        Err(e) => {
            bridge.toast(format!("YAML inválido: {e}"), true);
            return;
        }
    };
    let Some(name) = obj.metadata.name.clone() else {
        bridge.toast("el YAML no tiene metadata.name", true);
        return;
    };
    if name != esperado_name {
        bridge.toast(
            format!("no se puede cambiar el nombre («{esperado_name}» → «{name}»): los recursos de Kubernetes no se renombran"),
            true,
        );
        return;
    }
    if obj.metadata.namespace != esperado_ns {
        bridge.toast("no se puede cambiar el namespace del objeto", true);
        return;
    }
    // Sin resourceVersion el PUT no es optimista: pisa lo que haya en el
    // cluster aunque otro lo haya cambiado mientras tanto. Es fácil borrarla
    // sin querer al limpiar el manifiesto, así que se exige.
    if obj
        .metadata
        .resource_version
        .as_deref()
        .is_none_or(str::is_empty)
    {
        bridge.toast(
            "falta metadata.resourceVersion: sin eso el cambio pisaría lo que \
             haya en el cluster. Recargá y volvé a editar.",
            true,
        );
        return;
    }
    let api = api_for(client, &ar, esperado_ns.as_deref());
    match api.replace(&name, &PostParams::default(), &obj).await {
        Ok(_) => bridge.toast(format!("«{name}» actualizado"), false),
        Err(e) => {
            // 409 es el caso esperable: alguien más tocó el objeto.
            let msg = if format!("{e}").contains("409") || format!("{e}").contains("Conflict") {
                format!("«{name}» cambió en el cluster desde que lo abriste. Recargá y volvé a aplicar.")
            } else {
                format!("no se pudo aplicar: {e}")
            };
            bridge.toast(msg, true)
        }
    }
}
