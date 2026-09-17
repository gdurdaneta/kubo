//! Columnas por Kind. Cada Kind decide qué mira del objeto; el fallback
//! genérico (Nombre / Namespace / Edad) cubre cualquier CRD.

use std::borrow::Cow;

use k8s_openapi::jiff::{SignedDuration, Timestamp};
use kube::api::DynamicObject;
use kube::ResourceExt;
use serde_json::Value;

use crate::k8s::printer::{self, ColumnaCrd};

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

#[derive(Clone, Debug)]
pub struct ColSpec {
    /// Prestado para las columnas fijas de cada Kind; propio para las que
    /// vienen del CRD.
    pub title: Cow<'static, str>,
    /// Ancho inicial; `None` = ocupa el resto.
    pub width: Option<f32>,
}

const fn col(title: &'static str, width: f32) -> ColSpec {
    ColSpec {
        title: Cow::Borrowed(title),
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
        title: Cow::Borrowed("Mensaje"),
        width: None,
    },
];
const C_HPA: &[ColSpec] = &[
    col("Objetivo", 180.0),
    col("Min", 50.0),
    col("Max", 50.0),
    col("Actuales", 80.0),
    col("Uso", 130.0),
];
const C_NETPOL: &[ColSpec] = &[col("Selector", 220.0), col("Tipos", 120.0)];
const C_ROLE: &[ColSpec] = &[col("Reglas", 70.0)];
const C_ROLEBINDING: &[ColSpec] = &[col("Rol", 200.0), col("Sujetos", 260.0)];
const C_STORAGECLASS: &[ColSpec] = &[
    col("Provisioner", 220.0),
    col("Reclaim", 90.0),
    col("Binding", 160.0),
    col("Expansion", 80.0),
    col("Default", 70.0),
];
const C_INGRESSCLASS: &[ColSpec] = &[col("Controller", 240.0), col("Default", 70.0)];
const C_PDB: &[ColSpec] = &[
    col("Min disp.", 80.0),
    col("Max no disp.", 100.0),
    col("Permitidas", 90.0),
    col("Sanos", 80.0),
];
const C_QUOTA: &[ColSpec] = &[col("Recursos", 80.0), col("Uso", 260.0)];
const C_LIMITRANGE: &[ColSpec] = &[col("Limites", 80.0), col("Tipos", 160.0)];
const C_PRIORITYCLASS: &[ColSpec] = &[
    col("Valor", 100.0),
    col("Global", 70.0),
    col("Preemption", 130.0),
];
const C_ENDPOINTS: &[ColSpec] = &[col("Direcciones", 90.0), col("Puertos", 160.0)];
const C_ENDPOINTSLICE: &[ColSpec] = &[
    col("Tipo", 80.0),
    col("Endpoints", 90.0),
    col("Puertos", 160.0),
];
/// Para un CRD que no declara columnas: estado inferido del status y un
/// resumen del spec. Ni kubectl ni Lens muestran nada en ese caso.
const C_GENERICO: &[ColSpec] = &[col("Estado", 120.0), col("Spec", 340.0)];
const C_APISERVICE: &[ColSpec] = &[col("Servicio", 220.0), col("Disponible", 100.0)];
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
        "ReplicaSet" | "ReplicationController" => C_REPLICASET,
        "HorizontalPodAutoscaler" => C_HPA,
        "NetworkPolicy" => C_NETPOL,
        "Role" | "ClusterRole" => C_ROLE,
        "RoleBinding" | "ClusterRoleBinding" => C_ROLEBINDING,
        "StorageClass" => C_STORAGECLASS,
        "IngressClass" => C_INGRESSCLASS,
        "PodDisruptionBudget" => C_PDB,
        "ResourceQuota" => C_QUOTA,
        "LimitRange" => C_LIMITRANGE,
        "PriorityClass" => C_PRIORITYCLASS,
        "Endpoints" => C_ENDPOINTS,
        "EndpointSlice" => C_ENDPOINTSLICE,
        "APIService" => C_APISERVICE,
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
pub fn headers(kind: &str, mostrar_ns: bool, crd: &[ColumnaCrd]) -> Vec<ColSpec> {
    let mut v = vec![col("Nombre", 280.0)];
    if mostrar_ns {
        v.push(col("Namespace", 160.0));
    }
    let fijas = extra_cols(kind);
    v.extend_from_slice(fijas);
    // Un recurso custom sin columnas propias usa las que declara su CRD,
    // igual que `kubectl get`; si el CRD tampoco declara, se infieren.
    if fijas.is_empty() && !crd.iter().any(|c| c.prioridad == 0) {
        v.extend_from_slice(C_GENERICO);
    } else if fijas.is_empty() {
        v.extend(crd.iter().filter(|c| c.prioridad == 0).map(|c| ColSpec {
            title: Cow::Owned(c.nombre.clone()),
            width: Some(match c.tipo.as_str() {
                "integer" | "number" | "boolean" => 80.0,
                "date" => 70.0,
                _ => 130.0,
            }),
        }));
    }
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
pub fn titulo_estado(kind: &str, crd: &[ColumnaCrd]) -> Option<String> {
    let fijas = extra_cols(kind);
    if fijas.is_empty() {
        if !crd.iter().any(|c| c.prioridad == 0) {
            return Some("Estado".to_string());
        }
        return crd
            .iter()
            .filter(|c| c.prioridad == 0)
            .find(|c| es_titulo_estado(&c.nombre))
            .map(|c| c.nombre.clone());
    }
    fijas
        .iter()
        .find(|c| es_titulo_estado(&c.title))
        .map(|c| c.title.to_string())
}

/// "Tipo" es el de Events (Normal/Warning): filtrar por Warning es justo lo
/// que uno hace al triagear. El resto son los nombres que usan los CRD para
/// su columna de salud (Argo, cert-manager, Flux, Rollouts…).
fn es_titulo_estado(t: &str) -> bool {
    const TITULOS: &[&str] = &["Estado", "Status", "Fase", "Tipo"];
    const CRD: &[&str] = &[
        "status",
        "ready",
        "phase",
        "state",
        "health",
        "health status",
        "sync status",
        "healthy",
        "synced",
    ];
    TITULOS.contains(&t) || CRD.contains(&t.to_ascii_lowercase().as_str())
}

pub fn indice_estado(kind: &str, mostrar_ns: bool, crd: &[ColumnaCrd]) -> Option<usize> {
    let mut i = 1; // Nombre
    if mostrar_ns {
        i += 1;
    }
    let fijas = extra_cols(kind);
    if fijas.is_empty() {
        if !crd.iter().any(|c| c.prioridad == 0) {
            return Some(i);
        }
        return crd
            .iter()
            .filter(|c| c.prioridad == 0)
            .position(|c| es_titulo_estado(&c.nombre))
            .map(|p| p + i);
    }
    fijas
        .iter()
        .position(|c| es_titulo_estado(&c.title))
        .map(|p| p + i)
}

/// ¿Este Kind tiene columnas que se calculan al dibujar, fuera de la caché?
pub fn tiene_endpoints(kind: &str) -> bool {
    kind == "Service"
}

/// Valores de una fila, en el mismo orden que `headers` pero SIN la columna
/// de edad: esa se calcula al dibujar, si no quedaría congelada en la caché.
pub fn row(kind: &str, o: &DynamicObject, mostrar_ns: bool, crd: &[ColumnaCrd]) -> Vec<Cell> {
    let mut v = vec![Cell::plain(o.name_any())];
    if mostrar_ns {
        v.push(Cell::dim(o.namespace().unwrap_or_default()));
    }
    if extra_cols(kind).is_empty() && !crd.iter().any(|c| c.prioridad == 0) {
        let estado = estado_inferido(&o.data).unwrap_or_default();
        let tono = tono_de(&estado);
        v.push(Cell::toned(estado, tono));
        v.push(Cell::dim(resumen_spec(&o.data)));
    } else if extra_cols(kind).is_empty() {
        v.extend(crd.iter().filter(|c| c.prioridad == 0).map(|c| {
            let valor = printer::celda(c, &o.data);
            let tono = tono_de(&valor);
            Cell::toned(valor, tono)
        }));
    } else {
        v.extend(extra_cells(kind, o));
    }
    v
}

/// Color para un valor de estado que no conocemos de antemano (columnas de
/// CRD): lo que suena a sano en verde, lo que suena a roto en rojo.
pub fn tono_de(v: &str) -> Tone {
    match v.to_ascii_lowercase().as_str() {
        "" => Tone::Dim,
        "true" | "ready" | "healthy" | "synced" | "running" | "succeeded" | "active"
        | "available" | "bound" | "complete" | "completed" | "ok" | "valid" => Tone::Ok,
        "false" | "degraded" | "error" | "failed" | "outofsync" | "missing" | "invalid"
        | "unhealthy" | "notready" | "lost" => Tone::Bad,
        "progressing" | "unknown" | "suspended" | "pending" | "paused" | "warning" => Tone::Warn,
        _ => Tone::Normal,
    }
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
                Cell::plain(
                    num(status, "currentNumberScheduled")
                        .unwrap_or(0)
                        .to_string(),
                ),
                Cell::toned(
                    listos.to_string(),
                    if listos == deseados {
                        Tone::Ok
                    } else {
                        Tone::Warn
                    },
                ),
                Cell::plain(num(status, "numberAvailable").unwrap_or(0).to_string()),
            ]
        }
        "ReplicaSet" | "ReplicationController" => vec![
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
            vec![
                Cell::plain(format!("{ok}/{quiere}")),
                Cell::toned(txt, tono),
            ]
        }
        "CronJob" => {
            let susp = spec
                .and_then(|s| s.get("suspend"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
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
            let tono = if fase == "Active" {
                Tone::Ok
            } else {
                Tone::Warn
            };
            vec![Cell::toned(fase, tono)]
        }
        "PersistentVolumeClaim" => {
            let fase = txt(status, "phase");
            vec![
                Cell::toned(
                    fase.clone(),
                    if fase == "Bound" {
                        Tone::Ok
                    } else {
                        Tone::Warn
                    },
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
                    if fase == "Bound" {
                        Tone::Ok
                    } else {
                        Tone::Warn
                    },
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
                Cell::dim(
                    d.get("type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                ),
            ]
        }
        "Event" => {
            let t = d
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
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
                Cell::plain(
                    d.get("reason")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                ),
                Cell::dim(obj),
                Cell::plain(
                    d.get("message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                ),
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
        "HorizontalPodAutoscaler" => celdas_hpa(spec, status),
        "NetworkPolicy" => {
            let sel = selector_corto(spec.and_then(|s| s.get("podSelector")));
            let tipos = lista_str(spec, "policyTypes");
            vec![Cell::dim(sel), Cell::plain(tipos)]
        }
        "Role" | "ClusterRole" => vec![Cell::plain(
            d.get("rules")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0)
                .to_string(),
        )],
        "RoleBinding" | "ClusterRoleBinding" => {
            let r = d.get("roleRef");
            let rol = format!("{}/{}", txt(r, "kind"), txt(r, "name"));
            let sujetos: Vec<String> = d
                .get("subjects")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .map(|x| format!("{}:{}", str_de(x, "kind"), str_de(x, "name")))
                        .collect()
                })
                .unwrap_or_default();
            vec![Cell::plain(rol), Cell::dim(resumir(&sujetos, 3))]
        }
        "StorageClass" => {
            let por_defecto = o
                .annotations()
                .get("storageclass.kubernetes.io/is-default-class")
                .is_some_and(|v| v == "true");
            vec![
                Cell::plain(txt(Some(d), "provisioner")),
                Cell::dim(txt(Some(d), "reclaimPolicy")),
                Cell::dim(txt(Some(d), "volumeBindingMode")),
                Cell::dim(si_no(
                    d.get("allowVolumeExpansion").and_then(|v| v.as_bool()),
                )),
                Cell::toned(tilde(por_defecto), Tone::Ok),
            ]
        }
        "IngressClass" => {
            let por_defecto = o
                .annotations()
                .get("ingressclass.kubernetes.io/is-default-class")
                .is_some_and(|v| v == "true");
            vec![
                Cell::plain(txt(spec, "controller")),
                Cell::toned(tilde(por_defecto), Tone::Ok),
            ]
        }
        "PodDisruptionBudget" => {
            let permitidas = num(status, "disruptionsAllowed").unwrap_or(0);
            let sanos = num(status, "currentHealthy").unwrap_or(0);
            let esperados = num(status, "expectedPods").unwrap_or(0);
            vec![
                Cell::plain(cantidad(spec, "minAvailable")),
                Cell::plain(cantidad(spec, "maxUnavailable")),
                Cell::toned(
                    permitidas.to_string(),
                    if permitidas > 0 { Tone::Ok } else { Tone::Warn },
                ),
                Cell::dim(format!("{sanos}/{esperados}")),
            ]
        }
        "ResourceQuota" => {
            let hard = status
                .and_then(|s| s.get("hard"))
                .and_then(|v| v.as_object());
            let used = status
                .and_then(|s| s.get("used"))
                .and_then(|v| v.as_object());
            let n = hard.map(|m| m.len()).unwrap_or(0);
            let uso: Vec<String> = hard
                .map(|m| {
                    m.iter()
                        .map(|(k, v)| {
                            let u = used
                                .and_then(|u| u.get(k))
                                .and_then(|x| x.as_str())
                                .unwrap_or("0");
                            format!("{k} {u}/{}", v.as_str().unwrap_or(""))
                        })
                        .collect()
                })
                .unwrap_or_default();
            vec![Cell::plain(n.to_string()), Cell::dim(resumir(&uso, 3))]
        }
        "LimitRange" => {
            let limites = spec
                .and_then(|s| s.get("limits"))
                .and_then(|v| v.as_array());
            let tipos: Vec<String> = limites
                .map(|a| a.iter().map(|l| str_de(l, "type")).collect())
                .unwrap_or_default();
            vec![
                Cell::plain(limites.map(|a| a.len()).unwrap_or(0).to_string()),
                Cell::dim(tipos.join(",")),
            ]
        }
        "PriorityClass" => vec![
            Cell::plain(num(Some(d), "value").unwrap_or(0).to_string()),
            Cell::toned(
                tilde(d.get("globalDefault").and_then(|v| v.as_bool()) == Some(true)),
                Tone::Ok,
            ),
            Cell::dim(txt(Some(d), "preemptionPolicy")),
        ],
        "Endpoints" => {
            let subsets = d.get("subsets").and_then(|v| v.as_array());
            let direcciones: usize = subsets
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.get("addresses").and_then(|v| v.as_array()))
                        .map(|v| v.len())
                        .sum()
                })
                .unwrap_or(0);
            let puertos: Vec<String> = subsets
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.get("ports").and_then(|v| v.as_array()))
                        .flatten()
                        .map(|p| num(Some(p), "port").unwrap_or(0).to_string())
                        .collect()
                })
                .unwrap_or_default();
            vec![
                Cell::toned(
                    direcciones.to_string(),
                    if direcciones > 0 {
                        Tone::Ok
                    } else {
                        Tone::Warn
                    },
                ),
                Cell::dim(puertos.join(",")),
            ]
        }
        "APIService" => {
            let svc = spec
                .and_then(|s| s.get("service"))
                .filter(|s| !s.is_null())
                .map(|s| format!("{}/{}", str_de(s, "namespace"), str_de(s, "name")))
                .unwrap_or_else(|| "Local".into());
            let disp = status
                .and_then(|s| s.get("conditions"))
                .and_then(|v| v.as_array())
                .and_then(|a| a.iter().find(|c| str_de(c, "type") == "Available"))
                .map(|c| str_de(c, "status"))
                .unwrap_or_default();
            vec![Cell::dim(svc), Cell::toned(disp.clone(), tono_de(&disp))]
        }
        "EndpointSlice" => {
            let n = d
                .get("endpoints")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            let puertos: Vec<String> = d
                .get("ports")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .map(|p| num(Some(p), "port").unwrap_or(0).to_string())
                        .collect()
                })
                .unwrap_or_default();
            vec![
                Cell::dim(txt(Some(d), "addressType")),
                Cell::toned(n.to_string(), if n > 0 { Tone::Ok } else { Tone::Warn }),
                Cell::dim(puertos.join(",")),
            ]
        }
        _ => Vec::new(),
    }
}

fn celdas_hpa(spec: Option<&Value>, status: Option<&Value>) -> Vec<Cell> {
    let r = spec.and_then(|s| s.get("scaleTargetRef"));
    let objetivo = format!("{}/{}", txt(r, "kind"), txt(r, "name"));
    let min = num(spec, "minReplicas").unwrap_or(1);
    let max = num(spec, "maxReplicas").unwrap_or(0);
    let actual = num(status, "currentReplicas").unwrap_or(0);
    // Primera métrica de recurso (cpu/memoria): uso actual contra objetivo,
    // que es lo que uno mira para saber si el HPA está al tope.
    let objetivo_pct = spec
        .and_then(|s| s.get("metrics"))
        .and_then(|v| v.as_array())
        .and_then(|a| a.iter().find_map(|m| m.get("resource")))
        .map(|r| {
            (
                str_de(r, "name"),
                r.get("target")
                    .and_then(|t| t.get("averageUtilization"))
                    .and_then(|v| v.as_i64()),
            )
        });
    let actual_pct = status
        .and_then(|s| s.get("currentMetrics"))
        .and_then(|v| v.as_array())
        .and_then(|a| a.iter().find_map(|m| m.get("resource")))
        .and_then(|r| r.get("current"))
        .and_then(|c| c.get("averageUtilization"))
        .and_then(|v| v.as_i64());
    let uso = match (objetivo_pct, actual_pct) {
        (Some((nombre, Some(obj))), Some(act)) => format!("{nombre} {act}%/{obj}%"),
        (Some((nombre, Some(obj))), None) => format!("{nombre} ?/{obj}%"),
        _ => String::new(),
    };
    vec![
        Cell::dim(objetivo),
        Cell::plain(min.to_string()),
        Cell::plain(max.to_string()),
        Cell::toned(
            actual.to_string(),
            if max > 0 && actual >= max {
                Tone::Warn
            } else {
                Tone::Normal
            },
        ),
        Cell::dim(uso),
    ]
}

/// Estado de un objeto cuyo CRD no dice cómo mostrarlo: se prueban los
/// campos que usan casi todos los operadores (`phase`, `state`, `health`,
/// la condición Ready/Available/Succeeded) y, si falla, dice por qué.
pub fn estado_inferido(d: &Value) -> Option<String> {
    let status = d.get("status")?;
    for k in ["phase", "state", "status", "syncStatus"] {
        if let Some(s) = status.get(k).and_then(|v| v.as_str()) {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    if let Some(s) = status
        .get("health")
        .and_then(|h| h.get("status"))
        .and_then(|v| v.as_str())
    {
        return Some(s.to_string());
    }
    if let Some(b) = status.get("ready").and_then(|v| v.as_bool()) {
        return Some(if b { "Ready" } else { "NotReady" }.to_string());
    }
    let conds = status.get("conditions")?.as_array()?;
    for tipo in [
        "Ready",
        "Available",
        "Succeeded",
        "Healthy",
        "Synced",
        "Reconciled",
        "Accepted",
        "Programmed",
    ] {
        if let Some(c) = conds.iter().find(|c| str_de(c, "type") == tipo) {
            let ok = str_de(c, "status");
            if ok == "True" {
                return Some(tipo.to_string());
            }
            let razon = str_de(c, "reason");
            return Some(if razon.is_empty() {
                format!("Not{tipo}")
            } else {
                format!("Not{tipo}: {razon}")
            });
        }
    }
    // Cualquier condición en False con razón: mejor que nada.
    conds
        .iter()
        .find(|c| str_de(c, "status") == "False")
        .map(|c| {
            let razon = str_de(c, "reason");
            if razon.is_empty() {
                format!("{} False", str_de(c, "type"))
            } else {
                razon
            }
        })
}

/// Lo más identificador del spec en una línea: primero campos conocidos
/// (schedule, hosts, selector, namespaces…) y si no, los escalares.
pub fn resumen_spec(d: &Value) -> String {
    let Some(spec) = d.get("spec") else {
        return String::new();
    };
    let mut partes: Vec<String> = Vec::new();
    let texto = |v: &Value| -> String {
        match v {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Array(a) => {
                let items: Vec<String> = a
                    .iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect();
                if items.is_empty() {
                    format!("{} items", a.len())
                } else if items.len() > 3 {
                    format!("{} +{}", items[..3].join(","), items.len() - 3)
                } else {
                    items.join(",")
                }
            }
            Value::Object(m) => m
                .iter()
                .filter_map(|(k, v)| v.as_str().map(|s| format!("{k}={s}")))
                .collect::<Vec<_>>()
                .join(","),
            Value::Null => String::new(),
        }
    };
    const CONOCIDOS: &[&str] = &[
        "schedule",
        "hosts",
        "host",
        "includedNamespaces",
        "namespaces",
        "targetRef",
        "scaleTargetRef",
        "type",
        "provider",
        "source",
        "destination",
        "repoURL",
        "url",
        "endpoint",
        "address",
        "port",
        "ports",
        "backupName",
        "storageLocation",
        "ttl",
        "suspend",
        "template",
    ];
    for k in CONOCIDOS {
        if let Some(v) = spec.get(k) {
            let t = match (k, v) {
                (&"selector", _) | (&"template", _) => String::new(),
                (_, Value::Object(m)) if m.contains_key("matchLabels") => selector_corto(Some(v)),
                (_, Value::Object(m)) => m
                    .iter()
                    .filter_map(|(k2, v2)| v2.as_str().map(|s| format!("{k2}={s}")))
                    .take(3)
                    .collect::<Vec<_>>()
                    .join(","),
                _ => texto(v),
            };
            if !t.is_empty() {
                partes.push(format!("{k}: {t}"));
            }
        }
        if partes.len() >= 3 {
            break;
        }
    }
    for k in [
        "selector",
        "podSelector",
        "workloadSelector",
        "namespaceSelector",
    ] {
        if partes.len() >= 3 {
            break;
        }
        if let Some(v) = spec.get(k) {
            let t = if v.get("matchLabels").is_some() {
                selector_corto(Some(v))
            } else {
                texto(v)
            };
            if !t.is_empty() && t != "<todos>" {
                partes.push(format!("{k}: {t}"));
            }
        }
    }
    if partes.is_empty() {
        if let Some(m) = spec.as_object() {
            partes = m
                .iter()
                .filter(|(_, v)| v.is_string() || v.is_number() || v.is_boolean())
                .take(3)
                .map(|(k, v)| format!("{k}: {}", texto(v)))
                .collect();
        }
    }
    partes.join("  ·  ")
}

/// `matchLabels` como `k=v,k=v`; vacío significa "todos los pods".
pub fn selector_corto(sel: Option<&Value>) -> String {
    let pares: Vec<String> = sel
        .and_then(|s| s.get("matchLabels"))
        .and_then(|v| v.as_object())
        .map(|m| {
            m.iter()
                .map(|(k, v)| format!("{k}={}", v.as_str().unwrap_or("")))
                .collect()
        })
        .unwrap_or_default();
    if pares.is_empty() {
        "<todos>".to_string()
    } else {
        pares.join(",")
    }
}

fn lista_str(v: Option<&Value>, k: &str) -> String {
    v.and_then(|v| v.get(k))
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str())
                .collect::<Vec<_>>()
                .join(",")
        })
        .unwrap_or_default()
}

/// Un IntOrString (`3` o `"25%"`) como texto.
pub fn cantidad(v: Option<&Value>, k: &str) -> String {
    match v.and_then(|v| v.get(k)) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

fn si_no(v: Option<bool>) -> String {
    match v {
        Some(true) => "sí".into(),
        Some(false) => "no".into(),
        None => String::new(),
    }
}

fn tilde(v: bool) -> String {
    if v {
        "✓".into()
    } else {
        String::new()
    }
}

/// Los primeros `n` elementos y cuántos quedaron afuera.
pub fn resumir(items: &[String], n: usize) -> String {
    if items.len() <= n {
        items.join(", ")
    } else {
        format!("{} +{}", items[..n].join(", "), items.len() - n)
    }
}

fn str_de(v: &Value, k: &str) -> String {
    v.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string()
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
        .map(|a| {
            a.iter()
                .filter(|c| c.get("ready").and_then(|v| v.as_bool()).unwrap_or(false))
                .count()
        })
        .unwrap_or(0);
    let restarts: i64 = cs
        .map(|a| {
            a.iter()
                .filter_map(|c| c.get("restartCount").and_then(|v| v.as_i64()))
                .sum()
        })
        .unwrap_or(0);

    let (estado, tono) = estado_pod(o, status, cs, listos, total);

    vec![
        Cell::toned(
            format!("{listos}/{total}"),
            if listos == total && total > 0 {
                Tone::Ok
            } else {
                Tone::Warn
            },
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
                let r = w
                    .get("reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Waiting");
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
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            })
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
                .filter_map(|i| {
                    i.get("hostname")
                        .or_else(|| i.get("ip"))
                        .and_then(|v| v.as_str())
                })
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
        .and_then(|a| {
            a.iter()
                .find(|c| c.get("type").and_then(|v| v.as_str()) == Some("Ready"))
        })
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
        if r.is_empty() {
            "<none>".to_string()
        } else {
            r.join(",")
        }
    };

    let ip = status
        .and_then(|s| s.get("addresses"))
        .and_then(|v| v.as_array())
        .and_then(|a| {
            a.iter()
                .find(|x| x.get("type").and_then(|v| v.as_str()) == Some("InternalIP"))
        })
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

/// Edad relativa a partir de una fecha RFC 3339; si no parsea, va tal cual.
pub fn edad_desde_rfc3339(s: &str) -> String {
    desde_rfc3339_relativo(s)
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn estado_inferido_de_fase_condiciones_y_salud() {
        assert_eq!(
            estado_inferido(&json!({"status": {"phase": "Completed"}})).as_deref(),
            Some("Completed")
        );
        assert_eq!(
            estado_inferido(&json!({"status": {"health": {"status": "Degraded"}}})).as_deref(),
            Some("Degraded")
        );
        let c = json!({"status": {"conditions": [
            {"type": "Progressing", "status": "True"},
            {"type": "Ready", "status": "False", "reason": "ImagePull"}]}});
        assert_eq!(estado_inferido(&c).as_deref(), Some("NotReady: ImagePull"));
        let ok = json!({"status": {"conditions": [{"type": "Ready", "status": "True"}]}});
        assert_eq!(estado_inferido(&ok).as_deref(), Some("Ready"));
        assert_eq!(estado_inferido(&json!({"spec": {}})), None);
    }

    #[test]
    fn resumen_spec_prefiere_campos_conocidos() {
        let backup = json!({"spec": {"includedNamespaces": ["a", "b"], "ttl": "720h0m0s", "storageLocation": "default"}});
        let r = resumen_spec(&backup);
        assert!(r.starts_with("includedNamespaces: a,b"), "{r}");
        assert!(r.contains("storageLocation: default"), "{r}");
        let sm = json!({"spec": {"selector": {"matchLabels": {"app": "x"}}, "endpoints": [{"port": "http"}]}});
        assert_eq!(resumen_spec(&sm), "selector: app=x");
        let raro = json!({"spec": {"foo": 1, "bar": true, "nested": {"x": 1}}});
        assert_eq!(resumen_spec(&raro), "bar: true  ·  foo: 1");
        assert_eq!(resumen_spec(&json!({})), "");
    }
}
