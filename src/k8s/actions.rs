//! Mutaciones sobre el cluster: borrar, escalar, reiniciar y aplicar YAML.
//! Todas reportan por toast; la tabla se actualiza sola vía el watch.

use k8s_openapi::jiff::Timestamp;
use kube::api::{DeleteParams, DynamicObject, Patch, PatchParams, PostParams};
use kube::discovery::ApiResource;
use kube::Client;
use serde_json::json;

use super::UiBridge;

pub async fn borrar(
    client: Client,
    ar: ApiResource,
    ns: Option<String>,
    name: String,
    contexto: String,
    bridge: UiBridge,
) {
    let api = super::api_for(client, &ar, ns.as_deref());
    let r = api.delete(&name, &DeleteParams::default()).await;
    let resultado = match &r {
        Ok(_) => {
            bridge.toast(format!("{} «{name}» borrado", ar.kind), false);
            Ok(())
        }
        Err(e) => {
            bridge.toast(format!("no se pudo borrar {name}: {e}"), true);
            Err(e.to_string())
        }
    };
    if !crate::auditoria::anotar(&contexto, "borrar", &ar.kind, &ns, &name, None, resultado) {
        bridge.toast("no se pudo escribir la auditoría local", true);
    }
}

pub async fn escalar(
    client: Client,
    ar: ApiResource,
    ns: Option<String>,
    name: String,
    replicas: i64,
    contexto: String,
    bridge: UiBridge,
) {
    let api = super::api_for(client, &ar, ns.as_deref());
    let patch = json!({ "spec": { "replicas": replicas } });
    let r = api
        .patch(&name, &PatchParams::default(), &Patch::Merge(&patch))
        .await;
    let resultado = match &r {
        Ok(_) => {
            bridge.toast(format!("«{name}» escalado a {replicas} réplicas"), false);
            Ok(())
        }
        Err(e) => {
            bridge.toast(format!("no se pudo escalar {name}: {e}"), true);
            Err(e.to_string())
        }
    };
    if !crate::auditoria::anotar(
        &contexto,
        "escalar",
        &ar.kind,
        &ns,
        &name,
        Some(format!("{replicas} réplicas")),
        resultado,
    ) {
        bridge.toast("no se pudo escribir la auditoría local", true);
    }
}

/// Rollout restart: la misma anotación que pone `kubectl rollout restart`.
pub async fn reiniciar(
    client: Client,
    ar: ApiResource,
    ns: Option<String>,
    name: String,
    contexto: String,
    bridge: UiBridge,
) {
    let api = super::api_for(client, &ar, ns.as_deref());
    let ahora = Timestamp::now().to_string();
    let patch = json!({
        "spec": { "template": { "metadata": { "annotations": {
            "kubectl.kubernetes.io/restartedAt": ahora
        }}}}
    });
    let r = api
        .patch(&name, &PatchParams::default(), &Patch::Merge(&patch))
        .await;
    let resultado = match &r {
        Ok(_) => {
            bridge.toast(format!("rollout restart de «{name}» disparado"), false);
            Ok(())
        }
        Err(e) => {
            bridge.toast(format!("no se pudo reiniciar {name}: {e}"), true);
            Err(e.to_string())
        }
    };
    if !crate::auditoria::anotar(
        &contexto,
        "reiniciar",
        &ar.kind,
        &ns,
        &name,
        None,
        resultado,
    ) {
        bridge.toast("no se pudo escribir la auditoría local", true);
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
    contexto: String,
    bridge: UiBridge,
) {
    let obj = match validar_manifiesto(&yaml, &esperado_name, esperado_ns.as_deref()) {
        Ok(o) => o,
        Err(e) => {
            bridge.toast(e, true);
            return;
        }
    };
    let name = esperado_name.clone();
    let api = super::api_for(client, &ar, esperado_ns.as_deref());
    let r = api.replace(&name, &PostParams::default(), &obj).await;
    let resultado = match &r {
        Ok(_) => {
            bridge.toast(format!("«{name}» actualizado"), false);
            Ok(())
        }
        Err(e) => {
            // 409 es el caso esperable: alguien más tocó el objeto.
            let msg = if format!("{e}").contains("409") || format!("{e}").contains("Conflict") {
                format!("«{name}» cambió en el cluster desde que lo abriste. Recargá y volvé a aplicar.")
            } else {
                format!("no se pudo aplicar: {e}")
            };
            bridge.toast(msg, true);
            Err(e.to_string())
        }
    };
    // Qué se aplicó, sin guardar el manifiesto entero: líneas y un hash
    // estable para poder cotejar contra el YAML si hace falta.
    let hash = {
        use std::hash::{Hash as _, Hasher as _};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        yaml.hash(&mut h);
        h.finish()
    };
    if !crate::auditoria::anotar(
        &contexto,
        "aplicar",
        &ar.kind,
        &esperado_ns,
        &name,
        Some(format!("{} líneas, hash {hash:016x}", yaml.lines().count())),
        resultado,
    ) {
        bridge.toast("no se pudo escribir la auditoría local", true);
    }
}

/// Lo que se exige de un manifiesto editado antes de mandarlo con PUT: que
/// parsee, que no cambie nombre ni namespace (Kubernetes no renombra) y que
/// traiga `resourceVersion` (sin eso el PUT pisa cambios ajenos).
pub fn validar_manifiesto(
    yaml: &str,
    esperado_name: &str,
    esperado_ns: Option<&str>,
) -> Result<DynamicObject, String> {
    let obj: DynamicObject =
        serde_yaml_ng::from_str(yaml).map_err(|e| format!("YAML inválido: {e}"))?;
    let name = obj
        .metadata
        .name
        .clone()
        .ok_or_else(|| "el YAML no tiene metadata.name".to_string())?;
    if name != esperado_name {
        return Err(format!(
            "no se puede cambiar el nombre («{esperado_name}» → «{name}»): los recursos de Kubernetes no se renombran"
        ));
    }
    if obj.metadata.namespace.as_deref() != esperado_ns {
        return Err("no se puede cambiar el namespace del objeto".to_string());
    }
    if obj
        .metadata
        .resource_version
        .as_deref()
        .is_none_or(str::is_empty)
    {
        return Err(
            "falta metadata.resourceVersion: sin eso el cambio pisaría lo que \
                    haya en el cluster. Recargá y volvé a editar."
                .to_string(),
        );
    }
    Ok(obj)
}

#[cfg(test)]
mod tests {
    use super::validar_manifiesto;

    const OK: &str = "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: cfg\n  namespace: ns\n  resourceVersion: \"12\"\ndata:\n  a: b\n";

    #[test]
    fn acepta_un_manifiesto_coherente() {
        let o = validar_manifiesto(OK, "cfg", Some("ns")).unwrap();
        assert_eq!(o.metadata.resource_version.as_deref(), Some("12"));
    }

    #[test]
    fn rechaza_rename_namespace_y_sin_resource_version() {
        assert!(validar_manifiesto(OK, "otro", Some("ns"))
            .unwrap_err()
            .contains("renombran"));
        assert!(validar_manifiesto(OK, "cfg", Some("otro"))
            .unwrap_err()
            .contains("namespace"));
        let sin_rv = OK.replace("  resourceVersion: \"12\"\n", "");
        assert!(validar_manifiesto(&sin_rv, "cfg", Some("ns"))
            .unwrap_err()
            .contains("resourceVersion"));
        assert!(validar_manifiesto("a: [", "cfg", Some("ns"))
            .unwrap_err()
            .contains("YAML inválido"));
        assert!(validar_manifiesto("apiVersion: v1\nkind: X\n", "cfg", None)
            .unwrap_err()
            .contains("metadata.name"));
    }

    #[test]
    fn cluster_scoped_sin_namespace() {
        let y = "apiVersion: v1\nkind: Namespace\nmetadata:\n  name: n\n  resourceVersion: \"1\"\n";
        assert!(validar_manifiesto(y, "n", None).is_ok());
        assert!(validar_manifiesto(y, "n", Some("ns")).is_err());
    }
}
