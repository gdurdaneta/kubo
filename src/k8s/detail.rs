//! Datos del panel de detalle: YAML fresco del objeto y sus eventos.

use k8s_openapi::jiff::Timestamp;
use kube::api::{Api, DynamicObject, ListParams};
use kube::discovery::ApiResource;
use kube::Client;

use super::{EventRow, K8sEvent, UiBridge};

fn api_for(client: Client, ar: &ApiResource, ns: Option<&str>) -> Api<DynamicObject> {
    match ns {
        Some(ns) => Api::namespaced_with(client, ns, ar),
        None => Api::all_with(client, ar),
    }
}

/// Sustituye los valores de `data`/`stringData` por un marcador.
///
/// base64 no es cifrado: mostrar un Secret entero en pantalla es filtrar la
/// credencial a cualquiera que mire (o a un screenshot). Se enmascara salvo
/// que el usuario pida explícitamente revelar.
fn enmascarar_secreto(obj: &mut DynamicObject) {
    const MARCA: &str = "<oculto por kubo>";

    for campo in ["data", "stringData"] {
        if let Some(serde_json::Value::Object(m)) = obj.data.get_mut(campo) {
            for (_, v) in m.iter_mut() {
                *v = serde_json::Value::String(MARCA.into());
            }
        }
    }

    // `last-applied-configuration` guarda el objeto entero tal como se aplicó,
    // incluido `data`: enmascarar solo el campo dejaría el secreto igual de
    // expuesto un par de líneas más abajo.
    if let Some(anns) = obj.metadata.annotations.as_mut() {
        for (k, v) in anns.iter_mut() {
            if k.ends_with("last-applied-configuration") {
                *v = MARCA.to_string();
            }
        }
    }
}

/// Relee el objeto del API server y lo entrega como YAML.
///
/// `revelar` solo tiene efecto sobre Secrets; el resto se devuelve tal cual.
pub async fn fetch_yaml(
    client: Client,
    ar: ApiResource,
    ns: Option<String>,
    name: String,
    revelar: bool,
    token: u64,
    bridge: UiBridge,
) {
    let api = api_for(client, &ar, ns.as_deref());
    let t0 = std::time::Instant::now();
    let text = match api.get(&name).await {
        Ok(mut obj) => {
            if ar.kind == "Secret" && !revelar {
                enmascarar_secreto(&mut obj);
            }
            // El API server no devuelve apiVersion/kind en GETs tipados; los
            // reponemos para que el YAML sea aplicable tal cual.
            if obj.types.is_none() {
                obj.types = Some(kube::core::TypeMeta {
                    api_version: ar.api_version.clone(),
                    kind: ar.kind.clone(),
                });
            }
            obj.metadata.managed_fields = None;
            serde_yaml_ng::to_string(&obj).unwrap_or_else(|e| format!("# error serializando: {e}"))
        }
        Err(e) => format!("# no se pudo leer el objeto:\n# {e}"),
    };
    tracing::info!(kind = %ar.kind, %name, ms = t0.elapsed().as_millis(), "detalle: yaml listo");
    bridge.send(K8sEvent::Yaml { token, text });
}

/// Eventos asociados al objeto, resueltos por UID.
pub async fn fetch_events(
    client: Client,
    uid: String,
    ns: Option<String>,
    token: u64,
    bridge: UiBridge,
) {
    let ar = ApiResource {
        group: String::new(),
        version: "v1".into(),
        api_version: "v1".into(),
        kind: "Event".into(),
        plural: "events".into(),
    };
    let api = api_for(client, &ar, ns.as_deref());
    let lp = ListParams::default()
        .fields(&format!("involvedObject.uid={uid}"))
        .limit(200);

    let t0 = std::time::Instant::now();
    let items = match api.list(&lp).await {
        Ok(list) => {
            let mut rows: Vec<EventRow> = list
                .items
                .into_iter()
                .map(|o| {
                    let d = &o.data;
                    let last = d
                        .get("lastTimestamp")
                        .or_else(|| d.get("eventTime"))
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.parse::<Timestamp>().ok());
                    EventRow {
                        type_: d
                            .get("type")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        reason: d
                            .get("reason")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        message: d
                            .get("message")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        count: d.get("count").and_then(|v| v.as_i64()).unwrap_or(1),
                        last,
                    }
                })
                .collect();
            // Más recientes arriba, como en kubectl describe.
            rows.sort_by(|a, b| b.last.cmp(&a.last));
            rows
        }
        Err(_) => Vec::new(),
    };
    tracing::info!(n = items.len(), ms = t0.elapsed().as_millis(), "detalle: eventos listos");
    bridge.send(K8sEvent::ObjectEvents { token, items });
}
