//! Datos para el mapa de un Service: quién le manda tráfico (Ingress) y a
//! quién se lo reparte (workloads → pods que matchean el selector).

use std::collections::BTreeMap;

use kube::api::{Api, DynamicObject, ListParams};
use kube::discovery::ApiResource;
use kube::Client;
use kube::ResourceExt;

use super::{K8sEvent, UiBridge};

#[derive(Clone, Debug)]
pub struct PodDot {
    pub name: String,
    pub ready: bool,
    pub estado: String,
}

#[derive(Clone, Debug)]
pub struct Workload {
    pub kind: String,
    pub name: String,
    pub pods: Vec<PodDot>,
}

#[derive(Clone, Debug)]
pub struct IngressRef {
    pub name: String,
    pub hosts: Vec<String>,
}

/// Referencia a un recurso de configuración, con cómo se usa y si existe.
#[derive(Clone, Debug)]
pub struct RefConfig {
    pub name: String,
    /// "envFrom", "env", "volumen data", "imagePullSecret"…
    pub usos: Vec<String>,
    pub existe: bool,
}

/// Mapa de configuración de un workload: qué monta, de dónde lee variables y
/// quién le manda tráfico.
#[derive(Clone, Debug, Default)]
pub struct WorkloadMapaData {
    pub kind: String,
    pub name: String,
    pub imagenes: Vec<String>,
    pub configmaps: Vec<RefConfig>,
    pub secrets: Vec<RefConfig>,
    pub pvcs: Vec<RefConfig>,
    pub service_account: Option<RefConfig>,
    pub services: Vec<String>,
    pub ingresses: Vec<IngressRef>,
    pub error: Option<String>,
}

/// Lo que viaja a la UI: mapa de Service o de workload.
#[derive(Clone, Debug)]
pub enum Mapa {
    Service(MapaData),
    Workload(WorkloadMapaData),
}

#[derive(Clone, Debug, Default)]
pub struct MapaData {
    pub service: String,
    pub tipo: String,
    pub cluster_ip: String,
    pub puertos: Vec<String>,
    pub selector: BTreeMap<String, String>,
    pub ingresses: Vec<IngressRef>,
    pub workloads: Vec<Workload>,
    pub error: Option<String>,
}

fn ar_core(kind: &str, plural: &str) -> ApiResource {
    ApiResource {
        group: String::new(),
        version: "v1".into(),
        api_version: "v1".into(),
        kind: kind.into(),
        plural: plural.into(),
    }
}

pub async fn fetch_service(client: Client, ns: String, service: String, token: u64, bridge: UiBridge) {
    let data = armar(client, &ns, &service).await;
    bridge.send(K8sEvent::Mapa {
        token,
        data: Box::new(Mapa::Service(data)),
    });
}

pub async fn fetch_workload(
    client: Client,
    ar: kube::discovery::ApiResource,
    ns: String,
    name: String,
    token: u64,
    bridge: UiBridge,
) {
    let data = armar_workload(client, ar, &ns, &name).await;
    bridge.send(K8sEvent::Mapa {
        token,
        data: Box::new(Mapa::Workload(data)),
    });
}

async fn armar(client: Client, ns: &str, service: &str) -> MapaData {
    let mut out = MapaData {
        service: service.to_string(),
        ..Default::default()
    };

    // --- el service ------------------------------------------------------
    let svc_api: Api<DynamicObject> = Api::namespaced_with(client.clone(), ns, &ar_core("Service", "services"));
    let svc = match svc_api.get(service).await {
        Ok(s) => s,
        Err(e) => {
            out.error = Some(format!("no se pudo leer el service: {e}"));
            return out;
        }
    };
    let spec = svc.data.get("spec");
    out.tipo = spec
        .and_then(|s| s.get("type"))
        .and_then(|v| v.as_str())
        .unwrap_or("ClusterIP")
        .to_string();
    out.cluster_ip = spec
        .and_then(|s| s.get("clusterIP"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    out.puertos = spec
        .and_then(|s| s.get("ports"))
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .map(|p| {
                    let port = p.get("port").and_then(|v| v.as_i64()).unwrap_or(0);
                    let target = p
                        .get("targetPort")
                        .map(|v| v.to_string().trim_matches('"').to_string())
                        .unwrap_or_default();
                    let nombre = p.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    if nombre.is_empty() {
                        format!("{port} → {target}")
                    } else {
                        format!("{nombre} {port} → {target}")
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    out.selector = spec
        .and_then(|s| s.get("selector"))
        .and_then(|v| v.as_object())
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string())))
                .collect()
        })
        .unwrap_or_default();

    // --- pods que matchean el selector -----------------------------------
    if !out.selector.is_empty() {
        let sel = out
            .selector
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(",");
        let pod_api: Api<DynamicObject> = Api::namespaced_with(client.clone(), ns, &ar_core("Pod", "pods"));
        match pod_api.list(&ListParams::default().labels(&sel).limit(500)).await {
            Ok(list) => {
                // Agrupados por su dueño directo; los ReplicaSet se resuelven
                // después a su Deployment.
                let mut grupos: BTreeMap<(String, String), Vec<PodDot>> = BTreeMap::new();
                for p in list.items {
                    let dueño = p
                        .metadata
                        .owner_references
                        .as_ref()
                        .and_then(|r| r.first())
                        .map(|o| (o.kind.clone(), o.name.clone()))
                        .unwrap_or_else(|| ("Pod".into(), p.name_any()));
                    let status = p.data.get("status");
                    let ready = status
                        .and_then(|s| s.get("containerStatuses"))
                        .and_then(|v| v.as_array())
                        .map(|a| !a.is_empty() && a.iter().all(|c| c.get("ready").and_then(|v| v.as_bool()).unwrap_or(false)))
                        .unwrap_or(false);
                    let estado = status
                        .and_then(|s| s.get("phase"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("?")
                        .to_string();
                    grupos.entry(dueño).or_default().push(PodDot {
                        name: p.name_any(),
                        ready,
                        estado,
                    });
                }

                // ReplicaSet → Deployment (una consulta por RS distinto).
                let rs_api: Api<DynamicObject> =
                    Api::namespaced_with(client.clone(), ns, &ApiResource {
                        group: "apps".into(),
                        version: "v1".into(),
                        api_version: "apps/v1".into(),
                        kind: "ReplicaSet".into(),
                        plural: "replicasets".into(),
                    });
                for ((kind, name), pods) in grupos {
                    let (kind, name) = if kind == "ReplicaSet" {
                        match rs_api.get(&name).await.ok().and_then(|rs| {
                            rs.metadata
                                .owner_references
                                .as_ref()
                                .and_then(|r| r.first())
                                .map(|o| (o.kind.clone(), o.name.clone()))
                        }) {
                            Some(dueño) => dueño,
                            None => (kind, name),
                        }
                    } else {
                        (kind, name)
                    };
                    // Puede haber varios RS del mismo Deployment (rollout en
                    // curso): se fusionan.
                    match out.workloads.iter_mut().find(|w| w.kind == kind && w.name == name) {
                        Some(w) => w.pods.extend(pods),
                        None => out.workloads.push(Workload { kind, name, pods }),
                    }
                }
                for w in &mut out.workloads {
                    w.pods.sort_by(|a, b| a.name.cmp(&b.name));
                }
            }
            Err(e) => out.error = Some(format!("no se pudieron listar los pods: {e}")),
        }
    }

    // --- ingresses que apuntan al service --------------------------------
    let ing_api: Api<DynamicObject> = Api::namespaced_with(client, ns, &ApiResource {
        group: "networking.k8s.io".into(),
        version: "v1".into(),
        api_version: "networking.k8s.io/v1".into(),
        kind: "Ingress".into(),
        plural: "ingresses".into(),
    });
    if let Ok(list) = ing_api.list(&ListParams::default().limit(200)).await {
        for ing in list.items {
            let reglas = ing.data.get("spec").and_then(|s| s.get("rules")).and_then(|v| v.as_array());
            let Some(reglas) = reglas else { continue };
            let mut hosts = Vec::new();
            let mut apunta = false;
            for r in reglas {
                let backend_ok = r
                    .get("http")
                    .and_then(|h| h.get("paths"))
                    .and_then(|v| v.as_array())
                    .map(|paths| {
                        paths.iter().any(|p| {
                            p.get("backend")
                                .and_then(|b| b.get("service"))
                                .and_then(|s| s.get("name"))
                                .and_then(|v| v.as_str())
                                == Some(service)
                        })
                    })
                    .unwrap_or(false);
                if backend_ok {
                    apunta = true;
                    if let Some(h) = r.get("host").and_then(|v| v.as_str()) {
                        hosts.push(h.to_string());
                    }
                }
            }
            if apunta {
                out.ingresses.push(IngressRef {
                    name: ing.name_any(),
                    hosts,
                });
            }
        }
    }

    out
}

// ---------------------------------------------------------------- workload

/// Acumula usos por nombre preservando el orden de aparición.
fn anotar(mapa: &mut Vec<(String, Vec<String>)>, name: &str, uso: String) {
    match mapa.iter_mut().find(|(n, _)| n == name) {
        Some((_, usos)) => {
            if !usos.contains(&uso) {
                usos.push(uso);
            }
        }
        None => mapa.push((name.to_string(), vec![uso])),
    }
}

async fn armar_workload(
    client: Client,
    ar: kube::discovery::ApiResource,
    ns: &str,
    name: &str,
) -> WorkloadMapaData {
    let mut out = WorkloadMapaData {
        kind: ar.kind.clone(),
        name: name.to_string(),
        ..Default::default()
    };

    let api: Api<DynamicObject> = Api::namespaced_with(client.clone(), ns, &ar);
    let obj = match api.get(name).await {
        Ok(o) => o,
        Err(e) => {
            out.error = Some(format!("no se pudo leer el objeto: {e}"));
            return out;
        }
    };

    // El pod template según el Kind: los Pods son su propio template y los
    // CronJobs lo llevan un nivel más adentro.
    let spec = obj.data.get("spec");
    let (pod_spec, labels) = match ar.kind.as_str() {
        "Pod" => (
            spec,
            obj.data.get("metadata").and_then(|m| m.get("labels")),
        ),
        "CronJob" => {
            let tpl = spec
                .and_then(|s| s.get("jobTemplate"))
                .and_then(|j| j.get("spec"))
                .and_then(|s| s.get("template"));
            (
                tpl.and_then(|t| t.get("spec")),
                tpl.and_then(|t| t.get("metadata")).and_then(|m| m.get("labels")),
            )
        }
        _ => {
            let tpl = spec.and_then(|s| s.get("template"));
            (
                tpl.and_then(|t| t.get("spec")),
                tpl.and_then(|t| t.get("metadata")).and_then(|m| m.get("labels")),
            )
        }
    };
    let Some(pod_spec) = pod_spec else {
        out.error = Some("el objeto no tiene pod template".into());
        return out;
    };

    // ---- referencias de configuración desde los contenedores -------------
    let mut cms: Vec<(String, Vec<String>)> = Vec::new();
    let mut secs: Vec<(String, Vec<String>)> = Vec::new();
    let mut pvcs: Vec<(String, Vec<String>)> = Vec::new();

    let contenedores = ["containers", "initContainers"]
        .iter()
        .filter_map(|k| pod_spec.get(k).and_then(|v| v.as_array()))
        .flatten();
    for c in contenedores {
        if let Some(img) = c.get("image").and_then(|v| v.as_str()) {
            if !out.imagenes.contains(&img.to_string()) {
                out.imagenes.push(img.to_string());
            }
        }
        if let Some(envfrom) = c.get("envFrom").and_then(|v| v.as_array()) {
            for e in envfrom {
                if let Some(n) = e.get("configMapRef").and_then(|r| r.get("name")).and_then(|v| v.as_str()) {
                    anotar(&mut cms, n, "envFrom".into());
                }
                if let Some(n) = e.get("secretRef").and_then(|r| r.get("name")).and_then(|v| v.as_str()) {
                    anotar(&mut secs, n, "envFrom".into());
                }
            }
        }
        if let Some(env) = c.get("env").and_then(|v| v.as_array()) {
            for e in env {
                let vf = e.get("valueFrom");
                if let Some(n) = vf
                    .and_then(|v| v.get("configMapKeyRef"))
                    .and_then(|r| r.get("name"))
                    .and_then(|v| v.as_str())
                {
                    anotar(&mut cms, n, "env".into());
                }
                if let Some(n) = vf
                    .and_then(|v| v.get("secretKeyRef"))
                    .and_then(|r| r.get("name"))
                    .and_then(|v| v.as_str())
                {
                    anotar(&mut secs, n, "env".into());
                }
            }
        }
    }

    // ---- volúmenes -------------------------------------------------------
    if let Some(vols) = pod_spec.get("volumes").and_then(|v| v.as_array()) {
        for v in vols {
            let vol = v.get("name").and_then(|n| n.as_str()).unwrap_or("?");
            if let Some(n) = v.get("configMap").and_then(|c| c.get("name")).and_then(|x| x.as_str()) {
                anotar(&mut cms, n, format!("volumen {vol}"));
            }
            if let Some(n) = v.get("secret").and_then(|c| c.get("secretName")).and_then(|x| x.as_str()) {
                anotar(&mut secs, n, format!("volumen {vol}"));
            }
            if let Some(n) = v
                .get("persistentVolumeClaim")
                .and_then(|c| c.get("claimName"))
                .and_then(|x| x.as_str())
            {
                anotar(&mut pvcs, n, format!("volumen {vol}"));
            }
            // Volúmenes projected: mezclan varias fuentes.
            if let Some(fuentes) = v.get("projected").and_then(|p| p.get("sources")).and_then(|x| x.as_array()) {
                for f in fuentes {
                    if let Some(n) = f.get("configMap").and_then(|c| c.get("name")).and_then(|x| x.as_str()) {
                        anotar(&mut cms, n, format!("projected {vol}"));
                    }
                    if let Some(n) = f.get("secret").and_then(|c| c.get("name")).and_then(|x| x.as_str()) {
                        anotar(&mut secs, n, format!("projected {vol}"));
                    }
                }
            }
        }
    }

    // ---- imagePullSecrets y service account ------------------------------
    if let Some(ips) = pod_spec.get("imagePullSecrets").and_then(|v| v.as_array()) {
        for s in ips {
            if let Some(n) = s.get("name").and_then(|v| v.as_str()) {
                anotar(&mut secs, n, "imagePullSecret".into());
            }
        }
    }
    let sa = pod_spec
        .get("serviceAccountName")
        .or_else(|| pod_spec.get("serviceAccount"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // ---- existencia: un listado liviano por tipo -------------------------
    let nombres_de = |plural: &str, kind: &str| {
        let ar = ApiResource {
            group: String::new(),
            version: "v1".into(),
            api_version: "v1".into(),
            kind: kind.into(),
            plural: plural.into(),
        };
        let api: Api<DynamicObject> = Api::namespaced_with(client.clone(), ns, &ar);
        async move {
            api.list(&ListParams::default().limit(500))
                .await
                .map(|l| l.items.into_iter().filter_map(|o| o.metadata.name).collect::<Vec<_>>())
                .unwrap_or_default()
        }
    };
    let (cm_exist, sec_exist, pvc_exist, sa_exist) = tokio::join!(
        nombres_de("configmaps", "ConfigMap"),
        nombres_de("secrets", "Secret"),
        nombres_de("persistentvolumeclaims", "PersistentVolumeClaim"),
        nombres_de("serviceaccounts", "ServiceAccount"),
    );

    let a_refs = |lista: Vec<(String, Vec<String>)>, existentes: &[String]| {
        lista
            .into_iter()
            .map(|(name, usos)| RefConfig {
                existe: existentes.contains(&name),
                name,
                usos,
            })
            .collect::<Vec<_>>()
    };
    out.configmaps = a_refs(cms, &cm_exist);
    out.secrets = a_refs(secs, &sec_exist);
    out.pvcs = a_refs(pvcs, &pvc_exist);
    out.service_account = sa.map(|name| RefConfig {
        existe: sa_exist.contains(&name) || name == "default",
        name,
        usos: vec!["pod".into()],
    });

    // ---- services cuyo selector matchea las labels del template ----------
    let labels: BTreeMap<String, String> = labels
        .and_then(|v| v.as_object())
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string())))
                .collect()
        })
        .unwrap_or_default();
    if !labels.is_empty() {
        let svc_api: Api<DynamicObject> =
            Api::namespaced_with(client.clone(), ns, &ar_core("Service", "services"));
        if let Ok(list) = svc_api.list(&ListParams::default().limit(500)).await {
            for svc in list.items {
                let sel = svc
                    .data
                    .get("spec")
                    .and_then(|s| s.get("selector"))
                    .and_then(|v| v.as_object());
                let Some(sel) = sel else { continue };
                if sel.is_empty() {
                    continue;
                }
                let matchea = sel.iter().all(|(k, v)| {
                    v.as_str().is_some_and(|v| labels.get(k).map(String::as_str) == Some(v))
                });
                if matchea {
                    out.services.push(svc.name_any());
                }
            }
        }
    }

    // ---- ingresses que apuntan a esos services ---------------------------
    if !out.services.is_empty() {
        let ing_api: Api<DynamicObject> = Api::namespaced_with(client, ns, &ApiResource {
            group: "networking.k8s.io".into(),
            version: "v1".into(),
            api_version: "networking.k8s.io/v1".into(),
            kind: "Ingress".into(),
            plural: "ingresses".into(),
        });
        if let Ok(list) = ing_api.list(&ListParams::default().limit(200)).await {
            for ing in list.items {
                let reglas = ing.data.get("spec").and_then(|s| s.get("rules")).and_then(|v| v.as_array());
                let Some(reglas) = reglas else { continue };
                let mut hosts = Vec::new();
                let mut apunta = false;
                for r in reglas {
                    let backend_ok = r
                        .get("http")
                        .and_then(|h| h.get("paths"))
                        .and_then(|v| v.as_array())
                        .map(|paths| {
                            paths.iter().any(|p| {
                                p.get("backend")
                                    .and_then(|b| b.get("service"))
                                    .and_then(|s| s.get("name"))
                                    .and_then(|v| v.as_str())
                                    .is_some_and(|n| out.services.iter().any(|s| s == n))
                            })
                        })
                        .unwrap_or(false);
                    if backend_ok {
                        apunta = true;
                        if let Some(h) = r.get("host").and_then(|v| v.as_str()) {
                            hosts.push(h.to_string());
                        }
                    }
                }
                if apunta {
                    out.ingresses.push(IngressRef {
                        name: ing.name_any(),
                        hosts,
                    });
                }
            }
        }
    }

    out
}
