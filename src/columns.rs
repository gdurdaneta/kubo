//! Columnas por Kind. Cada Kind decide qué mira del objeto; el fallback
//! genérico (Nombre / Namespace / Edad) cubre cualquier CRD.

use k8s_openapi::jiff::{SignedDuration, Timestamp};
use kube::api::DynamicObject;
use kube::ResourceExt;
use serde_json::Value;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tone {
    Normal,
    Ok,
    Warn,
    Bad,
    Dim,
}

#[derive(Clone, Debug)]
pub struct Cell {
    pub text: String,
    pub tone: Tone,
}

impl Cell {
    fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            tone: Tone::Normal,
        }
    }
    fn dim(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            tone: Tone::Dim,
        }
    }
    fn toned(text: impl Into<String>, tone: Tone) -> Self {
        Self {
            text: text.into(),
            tone,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ColSpec {
    pub title: &'static str,
    /// Ancho inicial; `None` = ocupa el resto.
    pub width: Option<f32>,
}

const fn col(title: &'static str, width: f32) -> ColSpec {
    ColSpec {
        title,
        width: Some(width),
    }
}

// Los arrays van como `const` con nombre: un `&[...]` construido dentro del
// match no se promueve a `'static` porque `col()` es una llamada a función.
const C_POD: &[ColSpec] = &[
    col("Ready", 60.0),
    col("Estado", 130.0),
    col("Restarts", 70.0),
    col("Node", 180.0),
    col("IP", 120.0),
];
const C_DEPLOY: &[ColSpec] = &[
    col("Ready", 70.0),
    col("Actualizados", 100.0),
    col("Disponibles", 100.0),
];
const C_DAEMONSET: &[ColSpec] = &[
    col("Deseados", 80.0),
    col("Actuales", 80.0),
    col("Ready", 70.0),
    col("Disponibles", 90.0),
];
const C_REPLICASET: &[ColSpec] = &[
    col("Deseados", 80.0),
    col("Actuales", 80.0),
    col("Ready", 70.0),
];
const C_JOB: &[ColSpec] = &[col("Completions", 100.0), col("Estado", 110.0)];
const C_CRONJOB: &[ColSpec] = &[
    col("Schedule", 130.0),
    col("Suspend", 80.0),
    col("Activos", 70.0),
    col("Ultimo", 110.0),
];
const C_SERVICE: &[ColSpec] = &[
    col("Tipo", 110.0),
    col("Cluster IP", 130.0),
    col("Externo", 150.0),
    col("Puertos", 160.0),
];
const C_INGRESS: &[ColSpec] = &[
    col("Clase", 110.0),
    col("Hosts", 220.0),
    col("Direccion", 160.0),
];
const C_NODE: &[ColSpec] = &[
    col("Estado", 110.0),
    col("Roles", 130.0),
    col("Version", 110.0),
    col("IP interna", 130.0),
];
const C_NAMESPACE: &[ColSpec] = &[col("Estado", 110.0)];
const C_PVC: &[ColSpec] = &[
    col("Estado", 100.0),
    col("Volumen", 200.0),
    col("Capacidad", 90.0),
    col("Clase", 120.0),
];
const C_PV: &[ColSpec] = &[
    col("Capacidad", 90.0),
    col("Modo", 90.0),
    col("Reclaim", 100.0),
    col("Estado", 100.0),
    col("Clase", 120.0),
];
const C_CONFIGMAP: &[ColSpec] = &[col("Claves", 70.0), col("Tipo", 200.0)];
const C_EVENT: &[ColSpec] = &[
    col("Tipo", 80.0),
    col("Razon", 150.0),
    col("Objeto", 200.0),
    ColSpec {
        title: "Mensaje",
        width: None,
    },
];
const C_SERVICEACCOUNT: &[ColSpec] = &[col("Secrets", 80.0)];
const C_CRD: &[ColSpec] = &[
    col("Grupo", 200.0),
    col("Alcance", 110.0),
    col("Versiones", 140.0),
];

/// Columnas específicas del Kind, sin contar Nombre/Namespace/Edad.
fn extra_cols(kind: &str) -> &'static [ColSpec] {
    match kind {
        "Pod" => C_POD,
        "Deployment" | "StatefulSet" => C_DEPLOY,
        "DaemonSet" => C_DAEMONSET,
        "ReplicaSet" => C_REPLICASET,
        "Job" => C_JOB,
        "CronJob" => C_CRONJOB,
        "Service" => C_SERVICE,
        "Ingress" => C_INGRESS,
        "Node" => C_NODE,
        "Namespace" => C_NAMESPACE,
        "PersistentVolumeClaim" => C_PVC,
        "PersistentVolume" => C_PV,
        "ConfigMap" | "Secret" => C_CONFIGMAP,
        "Event" => C_EVENT,
        "ServiceAccount" => C_SERVICEACCOUNT,
        "CustomResourceDefinition" => C_CRD,
        _ => &[],
    }
}

/// Cabecera completa de la tabla para un Kind.
pub fn headers(kind: &str, mostrar_ns: bool) -> Vec<ColSpec> {
    let mut v = vec![ColSpec {
        title: "Nombre",
        width: Some(280.0),
    }];
    if mostrar_ns {
        v.push(col("Namespace", 160.0));
    }
    v.extend_from_slice(extra_cols(kind));
    // Los backends no están en el Service: llegan de un watch aparte y cambian
    // solos, así que la columna se pinta al dibujar igual que la edad.
    if kind == "Service" {
        v.push(col("Endpoints", 100.0));
    }
    // Igual que los endpoints: vienen de otra API y cambian solos.
    if tiene_metricas(kind) {
        v.push(col("CPU", 80.0));
        v.push(col("Mem", 90.0));
    }
    v.push(col("Edad", 70.0));
    v
}

/// Kinds con columnas de CPU/memoria desde metrics.k8s.io.
pub fn tiene_metricas(kind: &str) -> bool {
    matches!(kind, "Pod" | "Node")
}

/// Índice de la columna que hace de "estado" para este Kind, si tiene una.
///
/// Se busca por título en vez de hardcodear posiciones: así sigue andando si
/// alguien reordena las columnas de un Kind.
pub fn indice_estado(kind: &str, mostrar_ns: bool) -> Option<usize> {
    const TITULOS: &[&str] = &["Estado", "Status", "Fase"];
    let mut i = 1; // Nombre
    if mostrar_ns {
        i += 1;
    }
    extra_cols(kind)
        .iter()
        .position(|c| TITULOS.contains(&c.title))
        .map(|p| p + i)
}

/// ¿Este Kind tiene columnas que se calculan al dibujar, fuera de la caché?
pub fn tiene_endpoints(kind: &str) -> bool {
    kind == "Service"
}

/// Valores de una fila, en el mismo orden que `headers` pero SIN la columna
/// de edad: esa se calcula al dibujar, si no quedaría congelada en la caché.
pub fn row(kind: &str, o: &DynamicObject, mostrar_ns: bool) -> Vec<Cell> {
    let mut v = vec![Cell::plain(o.name_any())];
    if mostrar_ns {
        v.push(Cell::dim(o.namespace().unwrap_or_default()));
    }
    v.extend(extra_cells(kind, o));
    v
}

fn extra_cells(kind: &str, o: &DynamicObject) -> Vec<Cell> {
    let d = &o.data;
    let spec = d.get("spec");
    let status = d.get("status");

    match kind {
        "Pod" => celdas_pod(o, spec, status),
        "Deployment" | "StatefulSet" => {
            let deseado = num(spec, "replicas").unwrap_or(0);
            let listos = num(status, "readyReplicas").unwrap_or(0);
            vec![
                Cell::toned(
                    format!("{listos}/{deseado}"),
                    if listos == deseado && deseado > 0 {
                        Tone::Ok
                    } else if listos == 0 && deseado > 0 {
                        Tone::Bad
                    } else if deseado == 0 {
                        Tone::Dim
                    } else {
                        Tone::Warn
                    },
                ),
                Cell::plain(num(status, "updatedReplicas").unwrap_or(0).to_string()),
                Cell::plain(num(status, "availableReplicas").unwrap_or(0).to_string()),
            ]
        }
        "DaemonSet" => {
            let deseados = num(status, "desiredNumberScheduled").unwrap_or(0);
            let listos = num(status, "numberReady").unwrap_or(0);
            vec![
                Cell::plain(deseados.to_string()),
                Cell::plain(num(status, "currentNumberScheduled").unwrap_or(0).to_string()),
                Cell::toned(
                    listos.to_string(),
                    if listos == deseados { Tone::Ok } else { Tone::Warn },
                ),
                Cell::plain(num(status, "numberAvailable").unwrap_or(0).to_string()),
            ]
        }
        "ReplicaSet" => vec![
            Cell::plain(num(spec, "replicas").unwrap_or(0).to_string()),
            Cell::plain(num(status, "replicas").unwrap_or(0).to_string()),
            Cell::plain(num(status, "readyReplicas").unwrap_or(0).to_string()),
        ],
        "Job" => {
            let quiere = num(spec, "completions").unwrap_or(1);
            let ok = num(status, "succeeded").unwrap_or(0);
            let fallo = num(status, "failed").unwrap_or(0);
            let (txt, tono) = if fallo > 0 {
                ("Failed".to_string(), Tone::Bad)
            } else if ok >= quiere {
                ("Complete".to_string(), Tone::Ok)
            } else {
                ("Running".to_string(), Tone::Warn)
            };
            vec![Cell::plain(format!("{ok}/{quiere}")), Cell::toned(txt, tono)]
        }
        "CronJob" => {
            let susp = spec.and_then(|s| s.get("suspend")).and_then(|v| v.as_bool()).unwrap_or(false);
            let activos = status
                .and_then(|s| s.get("active"))
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            vec![
                Cell::plain(txt(spec, "schedule")),
                Cell::toned(
                    if susp { "sí" } else { "no" },
                    if susp { Tone::Warn } else { Tone::Dim },
                ),
                Cell::plain(activos.to_string()),
                Cell::dim(
                    status
                        .and_then(|s| s.get("lastScheduleTime"))
                        .and_then(|v| v.as_str())
                        .map(desde_rfc3339_relativo)
                        .unwrap_or_default(),
                ),
            ]
        }
        "Service" => celdas_service(spec, status),
        "Ingress" => celdas_ingress(spec, status),
        "Node" => celdas_node(o, spec, status),
        "Namespace" => {
            let fase = txt(status, "phase");
            let tono = if fase == "Active" { Tone::Ok } else { Tone::Warn };
            vec![Cell::toned(fase, tono)]
        }
        "PersistentVolumeClaim" => {
            let fase = txt(status, "phase");
            vec![
                Cell::toned(
                    fase.clone(),
                    if fase == "Bound" { Tone::Ok } else { Tone::Warn },
                ),
                Cell::plain(txt(spec, "volumeName")),
                Cell::plain(
                    status
                        .and_then(|s| s.get("capacity"))
                        .and_then(|c| c.get("storage"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                ),
                Cell::dim(txt(spec, "storageClassName")),
            ]
        }
        "PersistentVolume" => {
            let fase = txt(status, "phase");
            vec![
                Cell::plain(
                    spec.and_then(|s| s.get("capacity"))
                        .and_then(|c| c.get("storage"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                ),
                Cell::dim(
                    spec.and_then(|s| s.get("accessModes"))
                        .and_then(|v| v.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|x| x.as_str())
                                .map(modo_corto)
                                .collect::<Vec<_>>()
                                .join(",")
                        })
                        .unwrap_or_default(),
                ),
                Cell::dim(txt(spec, "persistentVolumeReclaimPolicy")),
                Cell::toned(
                    fase.clone(),
                    if fase == "Bound" { Tone::Ok } else { Tone::Warn },
                ),
                Cell::dim(txt(spec, "storageClassName")),
            ]
        }
        "ConfigMap" | "Secret" => {
            let n = d
                .get("data")
                .and_then(|v| v.as_object())
                .map(|m| m.len())
                .unwrap_or(0)
                + d.get("binaryData")
                    .and_then(|v| v.as_object())
                    .map(|m| m.len())
                    .unwrap_or(0);
            vec![
                Cell::plain(n.to_string()),
                Cell::dim(d.get("type").and_then(|v| v.as_str()).unwrap_or("").to_string()),
            ]
        }
        "Event" => {
            let t = d.get("type").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let obj = d
                .get("involvedObject")
                .map(|io| {
                    format!(
                        "{}/{}",
                        io.get("kind").and_then(|v| v.as_str()).unwrap_or(""),
                        io.get("name").and_then(|v| v.as_str()).unwrap_or("")
                    )
                })
                .unwrap_or_default();
            vec![
                Cell::toned(
                    t.clone(),
                    if t == "Warning" { Tone::Bad } else { Tone::Dim },
                ),
                Cell::plain(d.get("reason").and_then(|v| v.as_str()).unwrap_or("").to_string()),
                Cell::dim(obj),
                Cell::plain(d.get("message").and_then(|v| v.as_str()).unwrap_or("").to_string()),
            ]
        }
        "ServiceAccount" => vec![Cell::plain(
            d.get("secrets")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0)
                .to_string(),
        )],
        "CustomResourceDefinition" => vec![
            Cell::plain(txt(spec, "group")),
            Cell::dim(txt(spec, "scope")),
            Cell::dim(
                spec.and_then(|s| s.get("versions"))
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.get("name").and_then(|n| n.as_str()))
                            .collect::<Vec<_>>()
                            .join(",")
                    })
                    .unwrap_or_default(),
            ),
        ],
        _ => Vec::new(),
    }
}

fn celdas_pod(o: &DynamicObject, spec: Option<&Value>, status: Option<&Value>) -> Vec<Cell> {
    let cs = status
        .and_then(|s| s.get("containerStatuses"))
        .and_then(|v| v.as_array());
    let total = spec
        .and_then(|s| s.get("containers"))
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    let listos = cs
        .map(|a| a.iter().filter(|c| c.get("ready").and_then(|v| v.as_bool()).unwrap_or(false)).count())
        .unwrap_or(0);
    let restarts: i64 = cs
        .map(|a| a.iter().filter_map(|c| c.get("restartCount").and_then(|v| v.as_i64())).sum())
        .unwrap_or(0);

    let (estado, tono) = estado_pod(o, status, cs, listos, total);

    vec![
        Cell::toned(
            format!("{listos}/{total}"),
            if listos == total && total > 0 { Tone::Ok } else { Tone::Warn },
        ),
        Cell::toned(estado, tono),
        Cell::toned(
            restarts.to_string(),
            if restarts > 5 {
                Tone::Bad
            } else if restarts > 0 {
                Tone::Warn
            } else {
                Tone::Dim
            },
        ),
        Cell::dim(txt(spec, "nodeName")),
        Cell::dim(txt(status, "podIP")),
    ]
}

/// Réplica de la lógica de `kubectl get pods`: el motivo del contenedor
/// bloqueado pesa más que la fase, que casi siempre dice "Running".
fn estado_pod(
    o: &DynamicObject,
    status: Option<&Value>,
    cs: Option<&Vec<Value>>,
    listos: usize,
    total: usize,
) -> (String, Tone) {
    if o.metadata.deletion_timestamp.is_some() {
        return ("Terminating".into(), Tone::Warn);
    }
    if let Some(arr) = cs {
        for c in arr {
            let st = c.get("state");
            if let Some(w) = st.and_then(|s| s.get("waiting")) {
                let r = w.get("reason").and_then(|v| v.as_str()).unwrap_or("Waiting");
                return (r.to_string(), Tone::Bad);
            }
            if let Some(t) = st.and_then(|s| s.get("terminated")) {
                let code = t.get("exitCode").and_then(|v| v.as_i64()).unwrap_or(0);
                if code != 0 {
                    let r = t.get("reason").and_then(|v| v.as_str()).unwrap_or("Error");
                    return (r.to_string(), Tone::Bad);
                }
            }
        }
    }
    let fase = txt(status, "phase");
    let tono = match fase.as_str() {
        "Running" if listos == total && total > 0 => Tone::Ok,
        "Running" => Tone::Warn,
        "Succeeded" => Tone::Dim,
        "Failed" => Tone::Bad,
        _ => Tone::Warn,
    };
    (fase, tono)
}

fn celdas_service(spec: Option<&Value>, status: Option<&Value>) -> Vec<Cell> {
    let tipo = txt(spec, "type");
    let externo = if tipo == "LoadBalancer" {
        status
            .and_then(|s| s.get("loadBalancer"))
            .and_then(|lb| lb.get("ingress"))
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|i| {
                        i.get("hostname")
                            .or_else(|| i.get("ip"))
                            .and_then(|v| v.as_str())
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "<pending>".into())
    } else {
        spec.and_then(|s| s.get("externalIPs"))
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(","))
            .unwrap_or_else(|| "-".into())
    };
    let puertos = spec
        .and_then(|s| s.get("ports"))
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .map(|p| {
                    let port = p.get("port").and_then(|v| v.as_i64()).unwrap_or(0);
                    let proto = p.get("protocol").and_then(|v| v.as_str()).unwrap_or("TCP");
                    match p.get("nodePort").and_then(|v| v.as_i64()) {
                        Some(np) => format!("{port}:{np}/{proto}"),
                        None => format!("{port}/{proto}"),
                    }
                })
                .collect::<Vec<_>>()
                .join(",")
        })
        .unwrap_or_default();

    vec![
        Cell::plain(tipo),
        Cell::dim(txt(spec, "clusterIP")),
        Cell::plain(externo),
        Cell::dim(puertos),
    ]
}

fn celdas_ingress(spec: Option<&Value>, status: Option<&Value>) -> Vec<Cell> {
    let hosts = spec
        .and_then(|s| s.get("rules"))
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|r| r.get("host").and_then(|v| v.as_str()))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let dir = status
        .and_then(|s| s.get("loadBalancer"))
        .and_then(|lb| lb.get("ingress"))
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|i| i.get("hostname").or_else(|| i.get("ip")).and_then(|v| v.as_str()))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    vec![
        Cell::dim(txt(spec, "ingressClassName")),
        Cell::plain(hosts),
        Cell::dim(dir),
    ]
}

fn celdas_node(o: &DynamicObject, spec: Option<&Value>, status: Option<&Value>) -> Vec<Cell> {
    let ready = status
        .and_then(|s| s.get("conditions"))
        .and_then(|v| v.as_array())
        .and_then(|a| a.iter().find(|c| c.get("type").and_then(|v| v.as_str()) == Some("Ready")))
        .and_then(|c| c.get("status").and_then(|v| v.as_str()))
        .unwrap_or("Unknown")
        .to_string();
    let cordoned = spec
        .and_then(|s| s.get("unschedulable"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let (estado, tono) = match (ready.as_str(), cordoned) {
        ("True", true) => ("Ready,SchedulingDisabled".to_string(), Tone::Warn),
        ("True", false) => ("Ready".to_string(), Tone::Ok),
        _ => ("NotReady".to_string(), Tone::Bad),
    };

    let roles = {
        let r: Vec<String> = o
            .labels()
            .keys()
            .filter_map(|k| k.strip_prefix("node-role.kubernetes.io/"))
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        if r.is_empty() { "<none>".to_string() } else { r.join(",") }
    };

    let ip = status
        .and_then(|s| s.get("addresses"))
        .and_then(|v| v.as_array())
        .and_then(|a| a.iter().find(|x| x.get("type").and_then(|v| v.as_str()) == Some("InternalIP")))
        .and_then(|x| x.get("address").and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_string();

    let ver = status
        .and_then(|s| s.get("nodeInfo"))
        .and_then(|n| n.get("kubeletVersion"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    vec![
        Cell::toned(estado, tono),
        Cell::dim(roles),
        Cell::dim(ver),
        Cell::dim(ip),
    ]
}

fn modo_corto(m: &str) -> &str {
    match m {
        "ReadWriteOnce" => "RWO",
        "ReadOnlyMany" => "ROX",
        "ReadWriteMany" => "RWX",
        "ReadWriteOncePod" => "RWOP",
        otro => otro,
    }
}

fn num(v: Option<&Value>, k: &str) -> Option<i64> {
    v.and_then(|v| v.get(k)).and_then(|v| v.as_i64())
}

fn txt(v: Option<&Value>, k: &str) -> String {
    v.and_then(|v| v.get(k))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

/// Edad legible a partir del instante de creación.
pub fn edad(creado: Option<Timestamp>) -> String {
    match creado {
        Some(t) => humano(Timestamp::now().duration_since(t)),
        None => String::new(),
    }
}

fn desde_rfc3339_relativo(s: &str) -> String {
    match s.parse::<Timestamp>() {
        Ok(t) => humano(Timestamp::now().duration_since(t)),
        Err(_) => s.to_string(),
    }
}

/// Formato compacto estilo kubectl: 3d, 5h, 12m, 40s.
fn humano(d: SignedDuration) -> String {
    let s = d.as_secs().max(0);
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m", s / 60)
    } else if s < 86_400 {
        format!("{}h", s / 3600)
    } else if s < 86_400 * 365 {
        format!("{}d", s / 86_400)
    } else {
        format!("{}a", s / (86_400 * 365))
    }
}
