//! Un pod representativo de un workload, para abrir logs o shell desde el
//! Deployment/StatefulSet/DaemonSet sin ir a buscarlo a mano.

use kube::api::{Api, DynamicObject, ListParams};
use kube::discovery::ApiResource;
use kube::Client;
use serde_json::Value;

use super::{K8sEvent, UiBridge};

/// Qué se quería abrir sobre el pod resuelto.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuePod {
    Logs,
    Shell,
}

pub fn ar_pod() -> ApiResource {
    ApiResource {
        group: String::new(),
        version: "v1".into(),
        api_version: "v1".into(),
        kind: "Pod".into(),
        plural: "pods".into(),
    }
}

/// Selector de labels del workload como lo entiende `kubectl get -l`.
/// ReplicationController usa un mapa plano; el resto un LabelSelector.
pub fn selector_de(obj: &DynamicObject) -> Option<String> {
    let sel = obj.data.get("spec")?.get("selector")?;
    let mapa = sel.get("matchLabels").unwrap_or(sel).as_object()?;
    let mut partes: Vec<String> = mapa
        .iter()
        .filter_map(|(k, v)| v.as_str().map(|s| format!("{k}={s}")))
        .collect();
    if partes.is_empty() {
        return None;
    }
    partes.sort();
    Some(partes.join(","))
}

fn fase(p: &DynamicObject) -> &str {
    p.data
        .get("status")
        .and_then(|s| s.get("phase"))
        .and_then(Value::as_str)
        .unwrap_or("")
}

/// Tope de pods cuyos logs se mezclan en un visor: más es ilegible.
pub const MAX_PODS_LOGS: usize = 20;

/// Lista los pods del selector. Para shell manda el mejor candidato (un
/// Running, o el primero); para logs, todos (Running primero, con tope).
/// Sin pods se avisa con un toast.
pub async fn resolver(
    client: Client,
    ns: String,
    selector: String,
    titulo: String,
    pane: u64,
    que: QuePod,
    bridge: UiBridge,
) {
    let api: Api<DynamicObject> = Api::namespaced_with(client, &ns, &ar_pod());
    let lp = ListParams::default().labels(&selector).limit(200);
    let mut items = match api.list(&lp).await {
        Ok(l) => l.items,
        Err(e) => {
            bridge.toast(format!("no se pudieron listar los pods: {e}"), true);
            return;
        }
    };
    if items.is_empty() {
        bridge.toast(format!("no hay pods con {selector} en {ns}"), true);
        return;
    }
    items.sort_by_key(|p| fase(p) != "Running");
    match que {
        QuePod::Shell => bridge.send(K8sEvent::PodResuelto {
            pane,
            que,
            pod: Box::new(items.remove(0)),
        }),
        QuePod::Logs => {
            if items.len() > MAX_PODS_LOGS {
                bridge.toast(
                    format!(
                        "{titulo}: {} pods, se muestran los primeros {MAX_PODS_LOGS}",
                        items.len()
                    ),
                    false,
                );
                items.truncate(MAX_PODS_LOGS);
            }
            bridge.send(K8sEvent::PodsResueltos {
                pane,
                titulo,
                pods: items,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn obj(spec: Value) -> DynamicObject {
        serde_json::from_value(json!({
            "apiVersion": "apps/v1", "kind": "Deployment",
            "metadata": {"name": "x", "namespace": "ns"}, "spec": spec
        }))
        .unwrap()
    }

    #[test]
    fn selector_de_label_selector_y_mapa_plano() {
        let d = obj(json!({"selector": {"matchLabels": {"app": "web", "tier": "fe"}}}));
        assert_eq!(selector_de(&d).as_deref(), Some("app=web,tier=fe"));
        let rc = obj(json!({"selector": {"app": "web"}}));
        assert_eq!(selector_de(&rc).as_deref(), Some("app=web"));
        assert_eq!(selector_de(&obj(json!({}))), None);
    }
}
