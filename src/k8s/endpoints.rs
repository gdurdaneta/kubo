//! Conteo de endpoints por Service, para saber de un vistazo si un servicio
//! tiene backends vivos o está apuntando al vacío.
//!
//! Va como watch aparte de la tabla porque el dato no está en el Service: vive
//! en EndpointSlice (o en el Endpoints viejo). Se manda el mapa entero y no
//! cambios sueltos: son cientos de entradas de dos números, y así la tabla
//! nunca queda con un estado a medio aplicar.

use std::collections::HashMap;

use futures::StreamExt;
use kube::api::{Api, DynamicObject};
use kube::discovery::ApiResource;
use kube::runtime::watcher::{self, Event};
use kube::Client;
use kube::ResourceExt;

use super::watch::Target;
use super::{K8sEvent, UiBridge};

/// Backends de un Service.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Conteo {
    pub listos: u32,
    pub total: u32,
}

pub fn ar_endpointslice() -> ApiResource {
    ApiResource {
        group: "discovery.k8s.io".into(),
        version: "v1".into(),
        api_version: "discovery.k8s.io/v1".into(),
        kind: "EndpointSlice".into(),
        plural: "endpointslices".into(),
    }
}

pub fn ar_endpoints() -> ApiResource {
    ApiResource {
        group: String::new(),
        version: "v1".into(),
        api_version: "v1".into(),
        kind: "Endpoints".into(),
        plural: "endpoints".into(),
    }
}

/// A qué Service pertenece el objeto y cuántas direcciones aporta.
fn leer(o: &DynamicObject, slices: bool) -> Option<(String, Conteo)> {
    let ns = o.namespace()?;
    if slices {
        // El slice declara su Service por label; puede haber varios por Service.
        let svc = o.labels().get("kubernetes.io/service-name")?.clone();
        let mut c = Conteo::default();
        for e in o.data.get("endpoints")?.as_array()? {
            let n = e
                .get("addresses")
                .and_then(|a| a.as_array())
                .map(|a| a.len() as u32)
                .unwrap_or(0);
            // `ready` ausente significa listo, según la API.
            let listo = e
                .get("conditions")
                .and_then(|c| c.get("ready"))
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            c.total += n;
            if listo {
                c.listos += n;
            }
        }
        Some((format!("{ns}/{svc}"), c))
    } else {
        // El Endpoints viejo se llama igual que su Service.
        let mut c = Conteo::default();
        for s in o.data.get("subsets").and_then(|s| s.as_array())? {
            let listos = s
                .get("addresses")
                .and_then(|a| a.as_array())
                .map(|a| a.len() as u32)
                .unwrap_or(0);
            let no = s
                .get("notReadyAddresses")
                .and_then(|a| a.as_array())
                .map(|a| a.len() as u32)
                .unwrap_or(0);
            c.listos += listos;
            c.total += listos + no;
        }
        Some((format!("{ns}/{}", o.name_any()), c))
    }
}

/// Un backend concreto detrás de un Service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Backend {
    pub ip: String,
    /// Pod al que pertenece la dirección, si el endpoint lo referencia.
    pub pod: Option<String>,
    pub nodo: Option<String>,
    pub zona: Option<String>,
    pub listo: bool,
    pub puertos: Vec<String>,
}

fn backends_de(o: &DynamicObject) -> Vec<Backend> {
    let puertos: Vec<String> = o
        .data
        .get("ports")
        .and_then(|p| p.as_array())
        .map(|ps| {
            ps.iter()
                .filter_map(|p| {
                    let n = p.get("port")?.as_i64()?;
                    Some(match p.get("name").and_then(|v| v.as_str()) {
                        Some(nom) if !nom.is_empty() => format!("{n} ({nom})"),
                        _ => n.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let Some(eps) = o.data.get("endpoints").and_then(|e| e.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in eps {
        // `ready` ausente significa listo, según la API.
        let listo = e
            .get("conditions")
            .and_then(|c| c.get("ready"))
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let pod = e
            .get("targetRef")
            .filter(|t| t.get("kind").and_then(|k| k.as_str()) == Some("Pod"))
            .and_then(|t| t.get("name"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let nodo = e
            .get("nodeName")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let zona = e.get("zone").and_then(|v| v.as_str()).map(|s| s.to_string());
        for ip in e
            .get("addresses")
            .and_then(|a| a.as_array())
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str())
        {
            out.push(Backend {
                ip: ip.to_string(),
                pod: pod.clone(),
                nodo: nodo.clone(),
                zona: zona.clone(),
                listo,
                puertos: puertos.clone(),
            });
        }
    }
    out
}

/// Backends concretos de un Service, para el panel de detalle.
pub async fn backends(
    client: Client,
    ar: ApiResource,
    ns: String,
    svc: String,
    token: u64,
    bridge: UiBridge,
) {
    let api: Api<DynamicObject> = Api::namespaced_with(client, &ns, &ar);
    // El EndpointSlice viejo no tiene el label, pero se llama igual que el
    // Service; para ese caso se pide el objeto directo.
    let items = if ar.kind == "EndpointSlice" {
        let lp = kube::api::ListParams::default()
            .labels(&format!("kubernetes.io/service-name={svc}"))
            .limit(100);
        match api.list(&lp).await {
            Ok(l) => l.items.iter().flat_map(backends_de).collect(),
            Err(e) => {
                tracing::debug!(error = %e, "backends: no se pudieron listar los slices");
                Vec::new()
            }
        }
    } else {
        match api.get(&svc).await {
            Ok(o) => backends_endpoints_viejo(&o),
            Err(_) => Vec::new(),
        }
    };
    tracing::info!(%svc, %ns, n = items.len(), kind = %ar.kind, token, "backends: listos");
    bridge.send(K8sEvent::Backends { token, items });
}

/// Mismo dato desde el `Endpoints` clásico de core/v1.
fn backends_endpoints_viejo(o: &DynamicObject) -> Vec<Backend> {
    let mut out = Vec::new();
    for s in o
        .data
        .get("subsets")
        .and_then(|s| s.as_array())
        .into_iter()
        .flatten()
    {
        let puertos: Vec<String> = s
            .get("ports")
            .and_then(|p| p.as_array())
            .map(|ps| {
                ps.iter()
                    .filter_map(|p| Some(p.get("port")?.as_i64()?.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        for (campo, listo) in [("addresses", true), ("notReadyAddresses", false)] {
            for a in s.get(campo).and_then(|a| a.as_array()).into_iter().flatten() {
                let Some(ip) = a.get("ip").and_then(|v| v.as_str()) else {
                    continue;
                };
                out.push(Backend {
                    ip: ip.to_string(),
                    pod: a
                        .get("targetRef")
                        .and_then(|t| t.get("name"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    nodo: a
                        .get("nodeName")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    zona: None,
                    listo,
                    puertos: puertos.clone(),
                });
            }
        }
    }
    out
}

/// Sigue los endpoints del ámbito y publica el mapa `ns/servicio -> conteo`.
pub async fn seguir(
    client: Client,
    ar: ApiResource,
    target: Target,
    token: u64,
    bridge: UiBridge,
) {
    let slices = ar.kind == "EndpointSlice";
    let api: Api<DynamicObject> = match &target {
        Target::Namespace(ns) => Api::namespaced_with(client, ns, &ar),
        Target::AllNamespaces => Api::all_with(client, &ar),
    };

    // Varios slices pueden apuntar al mismo Service, así que se acumula por
    // objeto y recién después se suma por Service.
    let mut por_objeto: HashMap<String, (String, Conteo)> = HashMap::new();
    let mut sucio = false;
    let mut listo = false;

    let cfg = watcher::Config::default().page_size(1_000);
    let mut stream = watcher::watcher(api, cfg).boxed();
    // Un tick lento alcanza: es una columna informativa, no hace falta que siga
    // cada rebote de un pod al milisegundo.
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(700));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            item = stream.next() => {
                let Some(item) = item else { return };
                match item {
                    Ok(Event::Init) => por_objeto.clear(),
                    Ok(Event::InitApply(o)) | Ok(Event::Apply(o)) => {
                        let clave = format!("{}/{}", o.namespace().unwrap_or_default(), o.name_any());
                        if let Some(v) = leer(&o, slices) {
                            por_objeto.insert(clave, v);
                            sucio = true;
                        }
                    }
                    Ok(Event::Delete(o)) => {
                        let clave = format!("{}/{}", o.namespace().unwrap_or_default(), o.name_any());
                        if por_objeto.remove(&clave).is_some() {
                            sucio = true;
                        }
                    }
                    Ok(Event::InitDone) => {
                        listo = true;
                        sucio = true;
                    }
                    Err(e) => {
                        // No vale la pena molestar: la columna queda vacía.
                        tracing::debug!(error = %e, "endpoints: watch con error");
                    }
                }
            }
            _ = tick.tick() => {
                if !sucio || !listo {
                    continue;
                }
                sucio = false;
                let mut mapa: HashMap<String, Conteo> = HashMap::new();
                for (svc, c) in por_objeto.values() {
                    let e = mapa.entry(svc.clone()).or_default();
                    e.listos += c.listos;
                    e.total += c.total;
                }
                bridge.send(K8sEvent::Endpoints { token, mapa });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(v: serde_json::Value) -> DynamicObject {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn suma_direcciones_listas_y_no_listas_de_un_slice() {
        let o = obj(serde_json::json!({
            "apiVersion": "discovery.k8s.io/v1", "kind": "EndpointSlice",
            "metadata": {
                "name": "api-abcde", "namespace": "produccion",
                "labels": { "kubernetes.io/service-name": "api" }
            },
            "endpoints": [
                { "addresses": ["10.0.0.1", "10.0.0.2"], "conditions": { "ready": true } },
                { "addresses": ["10.0.0.3"], "conditions": { "ready": false } },
                // Sin `conditions` la API lo considera listo.
                { "addresses": ["10.0.0.4"] }
            ]
        }));
        let (clave, c) = leer(&o, true).expect("se lee el slice");
        assert_eq!(clave, "produccion/api");
        assert_eq!(c, Conteo { listos: 3, total: 4 });
    }

    #[test]
    fn extrae_ip_pod_y_nodo_de_cada_direccion() {
        let o = obj(serde_json::json!({
            "apiVersion": "discovery.k8s.io/v1", "kind": "EndpointSlice",
            "metadata": {
                "name": "web-abc", "namespace": "default",
                "labels": { "kubernetes.io/service-name": "web" }
            },
            "ports": [{ "port": 8080, "name": "http" }],
            "endpoints": [
                {
                    "addresses": ["10.0.0.1"],
                    "conditions": { "ready": true },
                    "targetRef": { "kind": "Pod", "name": "web-1" },
                    "nodeName": "node-a", "zone": "us-east-2a"
                },
                {
                    "addresses": ["10.0.0.2"],
                    "conditions": { "ready": false },
                    "targetRef": { "kind": "Pod", "name": "web-2" }
                }
            ]
        }));
        let b = backends_de(&o);
        assert_eq!(b.len(), 2);
        assert_eq!(b[0].ip, "10.0.0.1");
        assert_eq!(b[0].pod.as_deref(), Some("web-1"));
        assert_eq!(b[0].nodo.as_deref(), Some("node-a"));
        assert_eq!(b[0].puertos, vec!["8080 (http)"]);
        assert!(b[0].listo);
        assert!(!b[1].listo);
        assert_eq!(b[1].nodo, None);
    }

    #[test]
    fn el_endpoints_viejo_marca_los_no_listos() {
        let o = obj(serde_json::json!({
            "apiVersion": "v1", "kind": "Endpoints",
            "metadata": { "name": "web", "namespace": "default" },
            "subsets": [{
                "ports": [{ "port": 8080 }],
                "addresses": [{ "ip": "10.0.0.1", "targetRef": { "name": "web-1" } }],
                "notReadyAddresses": [{ "ip": "10.0.0.9" }]
            }]
        }));
        let b = backends_endpoints_viejo(&o);
        assert_eq!(b.len(), 2);
        assert!(b[0].listo && b[0].pod.as_deref() == Some("web-1"));
        assert!(!b[1].listo);
    }

    /// `KUBECONFIG=... KUBO_IT_SVC=ns/svc cargo test --release -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "necesita un cluster: definí KUBO_IT_SVC=ns/servicio"]
    async fn lee_los_backends_de_un_service_real() {
        let spec = std::env::var("KUBO_IT_SVC").expect("falta KUBO_IT_SVC=ns/servicio");
        let (ns, svc) = spec.split_once('/').expect("formato ns/servicio");
        let client = kube::Client::try_default().await.expect("sin cliente k8s");

        let (tx, rx) = flume::unbounded();
        let bridge = UiBridge::new(tx, egui::Context::default());
        backends(
            client,
            ar_endpointslice(),
            ns.to_string(),
            svc.to_string(),
            7,
            bridge,
        )
        .await;

        let ev = rx.try_recv().expect("no llegó el evento Backends");
        let K8sEvent::Backends { token, items } = ev else {
            panic!("evento inesperado");
        };
        assert_eq!(token, 7);
        println!("backends: {items:#?}");
        assert!(!items.is_empty(), "el Service no devolvió backends");
    }

    #[test]
    fn un_service_sin_backends_da_cero() {
        let o = obj(serde_json::json!({
            "apiVersion": "discovery.k8s.io/v1", "kind": "EndpointSlice",
            "metadata": {
                "name": "vacio-xyz", "namespace": "default",
                "labels": { "kubernetes.io/service-name": "vacio" }
            },
            "endpoints": []
        }));
        assert_eq!(leer(&o, true).unwrap().1, Conteo { listos: 0, total: 0 });
    }

    #[test]
    fn lee_el_endpoints_viejo() {
        let o = obj(serde_json::json!({
            "apiVersion": "v1", "kind": "Endpoints",
            "metadata": { "name": "legacy", "namespace": "default" },
            "subsets": [{
                "addresses": [{ "ip": "10.0.0.1" }, { "ip": "10.0.0.2" }],
                "notReadyAddresses": [{ "ip": "10.0.0.9" }]
            }]
        }));
        let (clave, c) = leer(&o, false).unwrap();
        assert_eq!(clave, "default/legacy");
        assert_eq!(c, Conteo { listos: 2, total: 3 });
    }
}
