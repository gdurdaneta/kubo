//! Resumen del panel de detalle para cada Kind que no sea Pod ni datos
//! clave-valor: lo que `kubectl describe` cuenta y el YAML esconde.
//!
//! Cada función lee el JSON crudo del objeto (`DynamicObject.data`): así sirve
//! para cualquier versión del API sin tipar cada recurso.

use kube::ResourceExt;
use serde_json::Value;

use super::detail::{campo, chips, opt_str, recursos, seccion, str_de};
use crate::columns;
use crate::k8s::metricas::{fmt_cpu, fmt_mem, parse_cpu, parse_mem};
use crate::k8s::printer::ColumnaCrd;
use crate::theme;

/// Dibuja el resumen propio del Kind. Devuelve `false` si no hay uno
/// específico, para que el llamador caiga en el genérico.
pub fn por_kind(ui: &mut egui::Ui, kind: &str, o: &kube::api::DynamicObject) -> bool {
    let d = &o.data;
    let spec = d.get("spec");
    let status = d.get("status");
    match kind {
        "Deployment" | "StatefulSet" | "DaemonSet" | "ReplicaSet" | "ReplicationController" => {
            workload(ui, kind, o, spec, status)
        }
        "Job" => job(ui, spec, status),
        "CronJob" => cronjob(ui, spec, status),
        "Service" => service(ui, spec, status),
        "Ingress" => ingress(ui, spec, status),
        "Node" => node(ui, spec, status),
        "PersistentVolumeClaim" => pvc(ui, spec, status),
        "PersistentVolume" => pv(ui, spec, status),
        "HorizontalPodAutoscaler" => hpa(ui, spec, status),
        "NetworkPolicy" => network_policy(ui, spec),
        "Role" | "ClusterRole" => role(ui, d),
        "RoleBinding" | "ClusterRoleBinding" => role_binding(ui, d),
        "StorageClass" => storage_class(ui, d),
        "IngressClass" => {
            seccion(ui, "IngressClass", |ui| {
                campo(ui, "Controller", &opt_str(spec, "controller"));
                if let Some(p) = spec.and_then(|s| s.get("parameters")) {
                    campo(
                        ui,
                        "Parámetros",
                        &format!("{}/{}", str_de(p, "kind"), str_de(p, "name")),
                    );
                }
            });
        }
        "PodDisruptionBudget" => pdb(ui, spec, status),
        "ResourceQuota" => quota(ui, spec, status),
        "LimitRange" => limit_range(ui, spec),
        "PriorityClass" => {
            seccion(ui, "PriorityClass", |ui| {
                campo(ui, "Valor", &num_str(Some(d), "value"));
                campo(ui, "Global por defecto", &si_no(d, "globalDefault"));
                campo(ui, "Preemption", &opt_str(Some(d), "preemptionPolicy"));
                campo(ui, "Descripción", &opt_str(Some(d), "description"));
            });
        }
        "ServiceAccount" => {
            seccion(ui, "ServiceAccount", |ui| {
                campo(ui, "Secrets", &nombres(d.get("secrets")).join(", "));
                campo(
                    ui,
                    "Image pull secrets",
                    &nombres(d.get("imagePullSecrets")).join(", "),
                );
                campo(
                    ui,
                    "Automount token",
                    &si_no(d, "automountServiceAccountToken"),
                );
            });
        }
        "CustomResourceDefinition" => crd(ui, spec, status),
        "Event" => event(ui, d),
        "Endpoints" => endpoints(ui, d),
        "EndpointSlice" => endpoint_slice(ui, d),
        "Namespace" => {
            seccion(ui, "Namespace", |ui| {
                campo(ui, "Fase", &opt_str(status, "phase"));
                campo(ui, "Finalizers", &lista(spec, "finalizers").join(", "));
            });
        }
        _ => return false,
    }
    true
}

/// Para recursos custom: las mismas columnas que la tabla, como campos.
pub fn columnas_crd(ui: &mut egui::Ui, cols: &[ColumnaCrd], o: &kube::api::DynamicObject) {
    if cols.is_empty() {
        // Sin columnas declaradas: el mismo estado inferido que la tabla.
        if let Some(e) = columns::estado_inferido(&o.data) {
            seccion(ui, "Estado", |ui| {
                campo_tono(ui, "Estado", &e, theme::color_tono(columns::tono_de(&e)));
            });
        }
        return;
    }
    seccion(ui, "Estado", |ui| {
        for c in cols {
            let mut v = crate::k8s::printer::celda(c, &o.data);
            if v.is_empty() {
                continue;
            }
            // Cantidades de memoria (`16144396Ki`) legibles, como en Nodes.
            if v.ends_with("Ki") || v.ends_with("Mi") || v.ends_with("Gi") || v.ends_with("Ti") {
                if let Some(b) = parse_mem(&v) {
                    v = fmt_mem(b);
                }
            }
            campo_tono(ui, &c.nombre, &v, theme::color_tono(columns::tono_de(&v)));
        }
    });
}

/// Escalares de primer nivel de `spec`: para un CRD sin resumen propio es
/// lo mínimo para entender qué pide el objeto sin abrir el YAML.
pub fn spec_generico(ui: &mut egui::Ui, o: &kube::api::DynamicObject) {
    let Some(spec) = o.data.get("spec").and_then(|v| v.as_object()) else {
        return;
    };
    let escalares: Vec<(&String, String)> = spec
        .iter()
        .filter_map(|(k, v)| match v {
            Value::String(s) => Some((k, s.clone())),
            Value::Number(n) => Some((k, n.to_string())),
            Value::Bool(b) => Some((k, b.to_string())),
            _ => None,
        })
        .collect();
    let resumen = columns::resumen_spec(&o.data);
    if escalares.is_empty() && resumen.is_empty() {
        return;
    }
    seccion(ui, "Spec", |ui| {
        // La misma línea que la columna Spec de la tabla: selector, hosts,
        // schedule… Lo que no es escalar solo se ve así o en el YAML.
        if !resumen.is_empty() {
            for parte in resumen.split("  ·  ") {
                if let Some((k, v)) = parte.split_once(": ") {
                    campo(ui, k, v);
                }
            }
        }
        for (k, v) in escalares {
            if !resumen.contains(&format!("{k}: ")) {
                campo(ui, k, &v);
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Workloads

fn workload(
    ui: &mut egui::Ui,
    kind: &str,
    o: &kube::api::DynamicObject,
    spec: Option<&Value>,
    status: Option<&Value>,
) {
    seccion(ui, kind, |ui| {
        match kind {
            "DaemonSet" => {
                let deseados = num(status, "desiredNumberScheduled");
                let listos = num(status, "numberReady");
                campo_tono(
                    ui,
                    "Pods",
                    &format!(
                        "{listos}/{deseados} listos · {} actuales · {} disponibles · {} desactualizados",
                        num(status, "currentNumberScheduled"),
                        num(status, "numberAvailable"),
                        num(status, "numberMisscheduled")
                    ),
                    if listos == deseados { theme::OK } else { theme::WARN },
                );
                let us = spec.and_then(|s| s.get("updateStrategy"));
                campo(ui, "Estrategia", &estrategia(us, "rollingUpdate"));
            }
            _ => {
                let deseados = num(spec, "replicas");
                let listos = num(status, "readyReplicas");
                let mut partes = vec![format!("{listos}/{deseados} listas")];
                if kind != "ReplicaSet" && kind != "ReplicationController" {
                    partes.push(format!("{} actualizadas", num(status, "updatedReplicas")));
                }
                partes.push(format!("{} disponibles", num(status, "availableReplicas")));
                let no_disp = num(status, "unavailableReplicas");
                if no_disp > 0 {
                    partes.push(format!("{no_disp} no disponibles"));
                }
                campo_tono(
                    ui,
                    "Réplicas",
                    &partes.join(" · "),
                    if listos == deseados && deseados > 0 {
                        theme::OK
                    } else if deseados == 0 {
                        theme::TEXTO_TENUE
                    } else {
                        theme::WARN
                    },
                );
                if kind == "Deployment" {
                    campo(
                        ui,
                        "Estrategia",
                        &estrategia(spec.and_then(|s| s.get("strategy")), "rollingUpdate"),
                    );
                    if let Some(r) = o.annotations().get("deployment.kubernetes.io/revision") {
                        campo(ui, "Revisión", r);
                    }
                    if spec.and_then(|s| s.get("paused")).and_then(|v| v.as_bool()) == Some(true) {
                        campo_tono(ui, "Pausado", "sí", theme::WARN);
                    }
                    campo(ui, "Min ready", &segundos(spec, "minReadySeconds"));
                    campo(
                        ui,
                        "Progress deadline",
                        &segundos(spec, "progressDeadlineSeconds"),
                    );
                }
                if kind == "StatefulSet" {
                    campo(ui, "Service", &opt_str(spec, "serviceName"));
                    campo(
                        ui,
                        "Estrategia",
                        &estrategia(spec.and_then(|s| s.get("updateStrategy")), "rollingUpdate"),
                    );
                    campo(ui, "Pod management", &opt_str(spec, "podManagementPolicy"));
                    let actual = opt_str(status, "currentRevision");
                    let nueva = opt_str(status, "updateRevision");
                    if !actual.is_empty() {
                        if actual == nueva || nueva.is_empty() {
                            campo(ui, "Revisión", &actual);
                        } else {
                            campo_tono(ui, "Revisión", &format!("{actual} → {nueva}"), theme::WARN);
                        }
                    }
                    let vcts: Vec<String> = spec
                        .and_then(|s| s.get("volumeClaimTemplates"))
                        .and_then(|v| v.as_array())
                        .map(|a| {
                            a.iter()
                                .map(|t| {
                                    let nombre = t
                                        .get("metadata")
                                        .map(|m| str_de(m, "name"))
                                        .unwrap_or_default();
                                    let cap = t
                                        .get("spec")
                                        .and_then(|s| s.get("resources"))
                                        .and_then(|r| r.get("requests"))
                                        .map(|r| str_de(r, "storage"))
                                        .unwrap_or_default();
                                    format!("{nombre} ({cap})")
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    campo(ui, "Volume claims", &vcts.join(", "));
                }
            }
        }
        campo(
            ui,
            "Selector",
            &columns::selector_corto(spec.and_then(|s| s.get("selector"))),
        );
    });

    if let Some(pod) = spec
        .and_then(|s| s.get("template"))
        .and_then(|t| t.get("spec"))
    {
        plantilla_pod(ui, pod);
    }
}

fn job(ui: &mut egui::Ui, spec: Option<&Value>, status: Option<&Value>) {
    seccion(ui, "Job", |ui| {
        let ok = num(status, "succeeded");
        let fallo = num(status, "failed");
        let activos = num(status, "active");
        let quiere = num_o(spec, "completions", 1);
        let (texto, color) = if fallo > 0 {
            (format!("{ok}/{quiere} · {fallo} fallidos"), theme::BAD)
        } else if ok >= quiere {
            (format!("{ok}/{quiere} completado"), theme::OK)
        } else {
            (format!("{ok}/{quiere} · {activos} activos"), theme::WARN)
        };
        campo_tono(ui, "Completions", &texto, color);
        campo(ui, "Parallelism", &num_str(spec, "parallelism"));
        campo(ui, "Backoff limit", &num_str(spec, "backoffLimit"));
        campo(ui, "Deadline", &segundos(spec, "activeDeadlineSeconds"));
        campo(
            ui,
            "TTL al terminar",
            &segundos(spec, "ttlSecondsAfterFinished"),
        );
        let inicio = opt_str(status, "startTime");
        let fin = opt_str(status, "completionTime");
        campo(ui, "Inicio", &fecha(&inicio));
        campo(ui, "Fin", &fecha(&fin));
        if let (Ok(a), Ok(b)) = (
            inicio.parse::<k8s_openapi::jiff::Timestamp>(),
            fin.parse::<k8s_openapi::jiff::Timestamp>(),
        ) {
            campo(
                ui,
                "Duración",
                &duracion(b.duration_since(a).as_secs().max(0) as u64),
            );
        }
    });
    if let Some(pod) = spec
        .and_then(|s| s.get("template"))
        .and_then(|t| t.get("spec"))
    {
        plantilla_pod(ui, pod);
    }
}

fn cronjob(ui: &mut egui::Ui, spec: Option<&Value>, status: Option<&Value>) {
    seccion(ui, "CronJob", |ui| {
        campo(ui, "Schedule", &opt_str(spec, "schedule"));
        campo(ui, "Zona horaria", &opt_str(spec, "timeZone"));
        if spec
            .and_then(|s| s.get("suspend"))
            .and_then(|v| v.as_bool())
            == Some(true)
        {
            campo_tono(ui, "Suspendido", "sí", theme::WARN);
        }
        campo(ui, "Concurrencia", &opt_str(spec, "concurrencyPolicy"));
        campo(
            ui,
            "Starting deadline",
            &segundos(spec, "startingDeadlineSeconds"),
        );
        campo(
            ui,
            "Historial",
            &format!(
                "{} ok · {} fallidos",
                num_o(spec, "successfulJobsHistoryLimit", 3),
                num_o(spec, "failedJobsHistoryLimit", 1)
            ),
        );
        campo(
            ui,
            "Última ejecución",
            &fecha(&opt_str(status, "lastScheduleTime")),
        );
        campo(
            ui,
            "Último éxito",
            &fecha(&opt_str(status, "lastSuccessfulTime")),
        );
        let activos = nombres(status.and_then(|s| s.get("active")));
        if !activos.is_empty() {
            campo_tono(ui, "Activos", &activos.join(", "), theme::WARN);
        }
    });
    if let Some(pod) = spec
        .and_then(|s| s.get("jobTemplate"))
        .and_then(|j| j.get("spec"))
        .and_then(|s| s.get("template"))
        .and_then(|t| t.get("spec"))
    {
        plantilla_pod(ui, pod);
    }
}

/// Contenedores y ajustes de pod de una plantilla (sin estado: no hay pod).
fn plantilla_pod(ui: &mut egui::Ui, pod: &Value) {
    let inits = pod.get("initContainers").and_then(|v| v.as_array());
    let conts = pod.get("containers").and_then(|v| v.as_array());
    if inits.is_some() || conts.is_some() {
        seccion(ui, "Contenedores", |ui| {
            for (c, init) in inits
                .into_iter()
                .flatten()
                .map(|c| (c, true))
                .chain(conts.into_iter().flatten().map(|c| (c, false)))
            {
                tarjeta_contenedor(ui, c, init);
            }
        });
    }

    let sa = str_de(pod, "serviceAccountName");
    let node_sel = pod.get("nodeSelector").and_then(|v| v.as_object());
    let tolerations = pod.get("tolerations").and_then(|v| v.as_array());
    let volumes = pod.get("volumes").and_then(|v| v.as_array());
    let pull = nombres(pod.get("imagePullSecrets"));
    let afinidad = pod.get("affinity").is_some();
    if sa.is_empty()
        && node_sel.is_none()
        && tolerations.is_none()
        && volumes.is_none()
        && pull.is_empty()
        && !afinidad
    {
        return;
    }
    seccion(ui, "Pod", |ui| {
        campo(ui, "Service account", &sa);
        campo(ui, "Image pull secrets", &pull.join(", "));
        campo(ui, "Restart policy", &str_de(pod, "restartPolicy"));
        campo(ui, "Priority class", &str_de(pod, "priorityClassName"));
        campo(ui, "Runtime class", &str_de(pod, "runtimeClassName"));
        if let Some(ns) = node_sel {
            let pares: Vec<String> = ns
                .iter()
                .map(|(k, v)| format!("{k}={}", v.as_str().unwrap_or("")))
                .collect();
            campo(ui, "Node selector", &pares.join(", "));
        }
        if afinidad {
            campo(ui, "Affinity", "definida (ver YAML)");
        }
        if let Some(t) = tolerations {
            let lineas: Vec<String> = t
                .iter()
                .map(|x| {
                    let key = str_de(x, "key");
                    let op = str_de(x, "operator");
                    let val = str_de(x, "value");
                    let eff = str_de(x, "effect");
                    let base = if key.is_empty() {
                        "*".to_string()
                    } else if op == "Exists" || val.is_empty() {
                        key
                    } else {
                        format!("{key}={val}")
                    };
                    if eff.is_empty() {
                        base
                    } else {
                        format!("{base}:{eff}")
                    }
                })
                .collect();
            campo(ui, "Tolerations", &lineas.join(", "));
        }
        if let Some(vs) = volumes {
            let lineas: Vec<String> = vs
                .iter()
                .map(|v| format!("{} ({})", str_de(v, "name"), fuente_volumen(v)))
                .collect();
            campo(ui, "Volúmenes", &lineas.join(", "));
        }
    });
}

/// Tipo y origen de un volumen de pod: `configMap:nombre`, `pvc:claim`, …
fn fuente_volumen(v: &Value) -> String {
    let Some(m) = v.as_object() else {
        return String::new();
    };
    for (k, val) in m {
        if k == "name" {
            continue;
        }
        let detalle = match k.as_str() {
            "configMap" | "secret" => str_de(val, "name").max(str_de(val, "secretName")),
            "persistentVolumeClaim" => str_de(val, "claimName"),
            "hostPath" => str_de(val, "path"),
            "projected" => "…".into(),
            _ => String::new(),
        };
        let tipo = match k.as_str() {
            "persistentVolumeClaim" => "pvc",
            otro => otro,
        };
        return if detalle.is_empty() {
            tipo.to_string()
        } else {
            format!("{tipo}:{detalle}")
        };
    }
    String::new()
}

fn tarjeta_contenedor(ui: &mut egui::Ui, c: &Value, init: bool) {
    // Ancho fijado al del panel: si no, la tarjeta se ajusta al texto y una
    // línea larga de mounts la saca del panel en vez de envolverse.
    let ancho = ui.available_width();
    egui::Frame::new()
        .fill(theme::PANEL_ALT)
        .inner_margin(6)
        .show(ui, |ui| {
            ui.set_min_width(ancho - 12.0);
            ui.set_max_width(ancho - 12.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(str_de(c, "name")).strong());
                if init {
                    ui.colored_label(theme::TEXTO_TENUE, "init");
                }
                let puertos: Vec<String> = c
                    .get("ports")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .map(|p| {
                                let n = num(Some(p), "containerPort");
                                let proto = str_de(p, "protocol");
                                if proto.is_empty() || proto == "TCP" {
                                    n.to_string()
                                } else {
                                    format!("{n}/{proto}")
                                }
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                if !puertos.is_empty() {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.colored_label(theme::TEXTO_TENUE, format!(":{}", puertos.join(" :")));
                    });
                }
            });
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
            let imagen = str_de(c, "image");
            let resp = ui
                .add(
                    egui::Label::new(egui::RichText::new(&imagen).color(theme::TEXTO_TENUE))
                        .sense(egui::Sense::click()),
                )
                .on_hover_text("Clic para copiar");
            if resp.clicked() {
                ui.ctx().copy_text(imagen);
            }
            if let Some(r) = c.get("resources") {
                let req = recursos(r.get("requests"));
                let lim = recursos(r.get("limits"));
                if !req.is_empty() || !lim.is_empty() {
                    ui.colored_label(
                        theme::TEXTO_TENUE,
                        format!("requests: {req}   limits: {lim}"),
                    );
                }
            }
            let env = c
                .get("env")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            let env_from: Vec<String> = c
                .get("envFrom")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|e| {
                            e.get("configMapRef")
                                .map(|r| format!("cm:{}", str_de(r, "name")))
                                .or_else(|| {
                                    e.get("secretRef")
                                        .map(|r| format!("secret:{}", str_de(r, "name")))
                                })
                        })
                        .collect()
                })
                .unwrap_or_default();
            if env > 0 || !env_from.is_empty() {
                let mut t = format!("env: {env} vars");
                if !env_from.is_empty() {
                    t.push_str(&format!(" + {}", env_from.join(", ")));
                }
                ui.colored_label(theme::TEXTO_TENUE, t);
            }
            let sondas: Vec<&str> = ["livenessProbe", "readinessProbe", "startupProbe"]
                .into_iter()
                .filter(|p| c.get(p).is_some())
                .map(|p| p.trim_end_matches("Probe"))
                .collect();
            if !sondas.is_empty() {
                ui.colored_label(theme::TEXTO_TENUE, format!("probes: {}", sondas.join(", ")));
            }
            let montajes: Vec<String> = c
                .get("volumeMounts")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .map(|m| format!("{}→{}", str_de(m, "name"), str_de(m, "mountPath")))
                        .collect()
                })
                .unwrap_or_default();
            if !montajes.is_empty() {
                ui.colored_label(
                    theme::TEXTO_TENUE,
                    format!("mounts: {}", montajes.join(", ")),
                );
            }
        });
    ui.add_space(3.0);
}

// ---------------------------------------------------------------------------
// Red

fn service(ui: &mut egui::Ui, spec: Option<&Value>, status: Option<&Value>) {
    seccion(ui, "Service", |ui| {
        let tipo = opt_str(spec, "type");
        campo(ui, "Tipo", &tipo);
        let cluster_ip = opt_str(spec, "clusterIP");
        if cluster_ip == "None" {
            campo_tono(ui, "Cluster IP", "None (headless)", theme::TEXTO_TENUE);
        } else {
            let ips = lista(spec, "clusterIPs");
            campo(
                ui,
                "Cluster IP",
                &if ips.len() > 1 {
                    ips.join(", ")
                } else {
                    cluster_ip
                },
            );
        }
        campo(ui, "IPs externas", &lista(spec, "externalIPs").join(", "));
        if tipo == "LoadBalancer" {
            let lb = lb_ingress(status);
            if lb.is_empty() {
                campo_tono(ui, "Load balancer", "<pending>", theme::WARN);
            } else {
                campo(ui, "Load balancer", &lb);
            }
            campo(ui, "LB IP pedida", &opt_str(spec, "loadBalancerIP"));
            campo(ui, "LB class", &opt_str(spec, "loadBalancerClass"));
        }
        if tipo == "ExternalName" {
            campo(ui, "External name", &opt_str(spec, "externalName"));
        }
        campo(
            ui,
            "Selector",
            &columns::selector_corto(
                spec.map(|s| s.get("selector"))
                    .map(|sel| {
                        // `selector` en Service es un mapa plano, no un LabelSelector.
                        serde_json::json!({ "matchLabels": sel })
                    })
                    .as_ref(),
            ),
        );
        campo(ui, "Session affinity", &opt_str(spec, "sessionAffinity"));
        campo(
            ui,
            "External traffic",
            &opt_str(spec, "externalTrafficPolicy"),
        );
        campo(
            ui,
            "Internal traffic",
            &opt_str(spec, "internalTrafficPolicy"),
        );
        campo(ui, "IP families", &lista(spec, "ipFamilies").join(", "));
    });
    if let Some(puertos) = spec.and_then(|s| s.get("ports")).and_then(|v| v.as_array()) {
        seccion(ui, "Puertos", |ui| {
            for p in puertos {
                let nombre = str_de(p, "name");
                let port = num(Some(p), "port");
                let target = match p.get("targetPort") {
                    Some(Value::Number(n)) => n.to_string(),
                    Some(Value::String(s)) => s.clone(),
                    _ => port.to_string(),
                };
                let proto = str_de(p, "protocol");
                let mut t = format!("{port} → {target}");
                if let Some(np) = p.get("nodePort").and_then(|v| v.as_i64()) {
                    t.push_str(&format!("  nodePort {np}"));
                }
                if !proto.is_empty() && proto != "TCP" {
                    t.push_str(&format!("  {proto}"));
                }
                if let Some(app) = p.get("appProtocol").and_then(|v| v.as_str()) {
                    t.push_str(&format!("  ({app})"));
                }
                campo(ui, if nombre.is_empty() { "—" } else { &nombre }, &t);
            }
        });
    }
}

fn lb_ingress(status: Option<&Value>) -> String {
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
        .unwrap_or_default()
}

fn ingress(ui: &mut egui::Ui, spec: Option<&Value>, status: Option<&Value>) {
    seccion(ui, "Ingress", |ui| {
        campo(ui, "Clase", &opt_str(spec, "ingressClassName"));
        campo(ui, "Dirección", &lb_ingress(status));
        if let Some(b) = spec.and_then(|s| s.get("defaultBackend")) {
            campo(ui, "Backend por defecto", &backend_ingress(b));
        }
    });
    if let Some(reglas) = spec.and_then(|s| s.get("rules")).and_then(|v| v.as_array()) {
        seccion(ui, "Reglas", |ui| {
            for r in reglas {
                let host = str_de(r, "host");
                ui.label(
                    egui::RichText::new(if host.is_empty() {
                        "*".to_string()
                    } else {
                        host
                    })
                    .strong(),
                );
                let paths = r
                    .get("http")
                    .and_then(|h| h.get("paths"))
                    .and_then(|v| v.as_array());
                for p in paths.into_iter().flatten() {
                    let ruta = str_de(p, "path");
                    let tipo = str_de(p, "pathType");
                    let destino = p.get("backend").map(backend_ingress).unwrap_or_default();
                    campo(
                        ui,
                        &format!(
                            "  {}{}",
                            if ruta.is_empty() { "/" } else { &ruta },
                            if tipo == "Prefix" || tipo.is_empty() {
                                ""
                            } else {
                                "  (exact)"
                            }
                        ),
                        &format!("→ {destino}"),
                    );
                }
            }
        });
    }
    if let Some(tls) = spec.and_then(|s| s.get("tls")).and_then(|v| v.as_array()) {
        seccion(ui, "TLS", |ui| {
            for t in tls {
                let hosts = lista(Some(t), "hosts").join(", ");
                campo(
                    ui,
                    &str_de(t, "secretName"),
                    if hosts.is_empty() { "*" } else { &hosts },
                );
            }
        });
    }
}

fn backend_ingress(b: &Value) -> String {
    if let Some(s) = b.get("service") {
        let puerto = s
            .get("port")
            .map(|p| {
                let n = num(Some(p), "number");
                if n > 0 {
                    n.to_string()
                } else {
                    str_de(p, "name")
                }
            })
            .unwrap_or_default();
        return format!("{}:{puerto}", str_de(s, "name"));
    }
    if let Some(r) = b.get("resource") {
        return format!("{}/{}", str_de(r, "kind"), str_de(r, "name"));
    }
    String::new()
}

fn network_policy(ui: &mut egui::Ui, spec: Option<&Value>) {
    seccion(ui, "NetworkPolicy", |ui| {
        campo(
            ui,
            "Pods",
            &columns::selector_corto(spec.and_then(|s| s.get("podSelector"))),
        );
        campo(ui, "Tipos", &lista(spec, "policyTypes").join(", "));
    });
    for (clave, titulo, dir) in [
        ("ingress", "Ingress", "desde"),
        ("egress", "Egress", "hacia"),
    ] {
        let Some(reglas) = spec.and_then(|s| s.get(clave)).and_then(|v| v.as_array()) else {
            continue;
        };
        seccion(ui, titulo, |ui| {
            if reglas.is_empty() {
                ui.colored_label(theme::BAD, "sin reglas: todo bloqueado");
            }
            for r in reglas {
                let peers: Vec<String> = r
                    .get(if clave == "ingress" { "from" } else { "to" })
                    .and_then(|v| v.as_array())
                    .map(|a| a.iter().map(peer_netpol).collect())
                    .unwrap_or_default();
                let puertos: Vec<String> = r
                    .get("ports")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .map(|p| {
                                let port = match p.get("port") {
                                    Some(Value::Number(n)) => n.to_string(),
                                    Some(Value::String(s)) => s.clone(),
                                    _ => "*".into(),
                                };
                                let proto = str_de(p, "protocol");
                                if proto.is_empty() || proto == "TCP" {
                                    port
                                } else {
                                    format!("{port}/{proto}")
                                }
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                campo(
                    ui,
                    &format!(
                        "{dir} {}",
                        if peers.is_empty() {
                            "cualquiera".into()
                        } else {
                            peers.join(" | ")
                        }
                    ),
                    &if puertos.is_empty() {
                        "todos los puertos".into()
                    } else {
                        format!("puertos {}", puertos.join(", "))
                    },
                );
            }
        });
    }
}

fn peer_netpol(p: &Value) -> String {
    let mut partes = Vec::new();
    if let Some(ip) = p.get("ipBlock") {
        let mut t = str_de(ip, "cidr");
        let exc = lista(Some(ip), "except");
        if !exc.is_empty() {
            t.push_str(&format!(" salvo {}", exc.join(",")));
        }
        partes.push(t);
    }
    if let Some(ns) = p.get("namespaceSelector") {
        partes.push(format!("ns[{}]", columns::selector_corto(Some(ns))));
    }
    if let Some(ps) = p.get("podSelector") {
        partes.push(format!("pods[{}]", columns::selector_corto(Some(ps))));
    }
    partes.join(" & ")
}

fn endpoints(ui: &mut egui::Ui, d: &Value) {
    let Some(subsets) = d.get("subsets").and_then(|v| v.as_array()) else {
        seccion(ui, "Endpoints", |ui| {
            ui.colored_label(theme::WARN, "sin direcciones");
        });
        return;
    };
    seccion(ui, "Endpoints", |ui| {
        for s in subsets {
            let puertos: Vec<String> = s
                .get("ports")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().map(|p| num(Some(p), "port").to_string()).collect())
                .unwrap_or_default();
            for a in s
                .get("addresses")
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
            {
                let destino = a
                    .get("targetRef")
                    .map(|r| format!("{}/{}", str_de(r, "kind"), str_de(r, "name")))
                    .unwrap_or_else(|| str_de(a, "nodeName"));
                campo_tono(
                    ui,
                    &str_de(a, "ip"),
                    &format!("{destino}  :{}", puertos.join(",:")),
                    theme::OK,
                );
            }
            for a in s
                .get("notReadyAddresses")
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
            {
                let destino = a
                    .get("targetRef")
                    .map(|r| format!("{}/{}", str_de(r, "kind"), str_de(r, "name")))
                    .unwrap_or_default();
                campo_tono(
                    ui,
                    &str_de(a, "ip"),
                    &format!("{destino}  (not ready)"),
                    theme::BAD,
                );
            }
        }
    });
}

fn endpoint_slice(ui: &mut egui::Ui, d: &Value) {
    seccion(ui, "EndpointSlice", |ui| {
        campo(ui, "Tipo de dirección", &str_de(d, "addressType"));
        let puertos: Vec<String> = d
            .get("ports")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .map(|p| {
                        let n = str_de(p, "name");
                        let port = num(Some(p), "port");
                        if n.is_empty() {
                            port.to_string()
                        } else {
                            format!("{n}:{port}")
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        campo(ui, "Puertos", &puertos.join(", "));
        for e in d
            .get("endpoints")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
        {
            let ips = lista(Some(e), "addresses").join(", ");
            let listo = e
                .get("conditions")
                .and_then(|c| c.get("ready"))
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let destino = e
                .get("targetRef")
                .map(|r| format!("{}/{}", str_de(r, "kind"), str_de(r, "name")))
                .unwrap_or_default();
            let nodo = str_de(e, "nodeName");
            campo_tono(
                ui,
                &ips,
                &format!(
                    "{destino}{}{}",
                    if nodo.is_empty() {
                        "".into()
                    } else {
                        format!("  @{nodo}")
                    },
                    if listo { "" } else { "  (not ready)" }
                ),
                if listo { theme::OK } else { theme::BAD },
            );
        }
    });
}

// ---------------------------------------------------------------------------
// Cluster

fn node(ui: &mut egui::Ui, spec: Option<&Value>, status: Option<&Value>) {
    seccion(ui, "Node", |ui| {
        if spec
            .and_then(|s| s.get("unschedulable"))
            .and_then(|v| v.as_bool())
            == Some(true)
        {
            campo_tono(ui, "Scheduling", "deshabilitado (cordoned)", theme::WARN);
        }
        for a in status
            .and_then(|s| s.get("addresses"))
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
        {
            campo(ui, &str_de(a, "type"), &str_de(a, "address"));
        }
        campo(
            ui,
            "Pod CIDR",
            &lista(spec, "podCIDRs")
                .join(", ")
                .max(opt_str(spec, "podCIDR")),
        );
        campo(ui, "Provider ID", &opt_str(spec, "providerID"));
        let taints: Vec<String> = spec
            .and_then(|s| s.get("taints"))
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .map(|t| {
                        let v = str_de(t, "value");
                        if v.is_empty() {
                            format!("{}:{}", str_de(t, "key"), str_de(t, "effect"))
                        } else {
                            format!("{}={v}:{}", str_de(t, "key"), str_de(t, "effect"))
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        if !taints.is_empty() {
            campo_tono(ui, "Taints", &taints.join(", "), theme::WARN);
        }
    });
    if let Some(info) = status.and_then(|s| s.get("nodeInfo")) {
        seccion(ui, "Sistema", |ui| {
            campo(ui, "OS", &str_de(info, "osImage"));
            campo(ui, "Kernel", &str_de(info, "kernelVersion"));
            campo(ui, "Runtime", &str_de(info, "containerRuntimeVersion"));
            campo(ui, "Kubelet", &str_de(info, "kubeletVersion"));
            campo(ui, "Kube-proxy", &str_de(info, "kubeProxyVersion"));
            campo(
                ui,
                "Arquitectura",
                &format!(
                    "{}/{}",
                    str_de(info, "operatingSystem"),
                    str_de(info, "architecture")
                ),
            );
        });
    }
    let cap = status.and_then(|s| s.get("capacity"));
    let alloc = status.and_then(|s| s.get("allocatable"));
    if cap.is_some() || alloc.is_some() {
        seccion(ui, "Capacidad", |ui| {
            for (k, etiqueta) in [
                ("cpu", "CPU"),
                ("memory", "Memoria"),
                ("pods", "Pods"),
                ("ephemeral-storage", "Disco efímero"),
                ("nvidia.com/gpu", "GPU"),
            ] {
                let c = opt_str(cap, k);
                let a = opt_str(alloc, k);
                if c.is_empty() && a.is_empty() {
                    continue;
                }
                let f = |s: &str| match k {
                    "cpu" => parse_cpu(s).map(fmt_cpu).unwrap_or_else(|| s.to_string()),
                    "memory" | "ephemeral-storage" => {
                        parse_mem(s).map(fmt_mem).unwrap_or_else(|| s.to_string())
                    }
                    _ => s.to_string(),
                };
                campo(
                    ui,
                    etiqueta,
                    &format!("{} asignable · {} total", f(&a), f(&c)),
                );
            }
        });
    }
    let imagenes = status
        .and_then(|s| s.get("images"))
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    if imagenes > 0 {
        seccion(ui, "Imágenes", |ui| {
            campo(ui, "En caché", &imagenes.to_string());
        });
    }
}

fn hpa(ui: &mut egui::Ui, spec: Option<&Value>, status: Option<&Value>) {
    seccion(ui, "HorizontalPodAutoscaler", |ui| {
        let r = spec.and_then(|s| s.get("scaleTargetRef"));
        campo(
            ui,
            "Objetivo",
            &format!("{}/{}", opt_str(r, "kind"), opt_str(r, "name")),
        );
        let min = num_o(spec, "minReplicas", 1);
        let max = num(spec, "maxReplicas");
        let actual = num(status, "currentReplicas");
        let deseado = num(status, "desiredReplicas");
        campo(ui, "Rango", &format!("{min} – {max}"));
        campo_tono(
            ui,
            "Réplicas",
            &format!("{actual} actuales · {deseado} deseadas"),
            if max > 0 && actual >= max {
                theme::WARN
            } else {
                theme::TEXTO
            },
        );
        campo(
            ui,
            "Última escalada",
            &fecha(&opt_str(status, "lastScaleTime")),
        );
    });
    let metricas = spec
        .and_then(|s| s.get("metrics"))
        .and_then(|v| v.as_array());
    let actuales = status
        .and_then(|s| s.get("currentMetrics"))
        .and_then(|v| v.as_array());
    if let Some(ms) = metricas {
        seccion(ui, "Métricas", |ui| {
            for (i, m) in ms.iter().enumerate() {
                let tipo = str_de(m, "type");
                let clave = tipo.to_lowercase();
                let cuerpo = m.get(clave.as_str()).or_else(|| m.get("resource"));
                let nombre = cuerpo
                    .map(|c| {
                        let n = str_de(c, "name");
                        if n.is_empty() {
                            c.get("metric")
                                .map(|x| str_de(x, "name"))
                                .unwrap_or_default()
                        } else {
                            n
                        }
                    })
                    .unwrap_or_else(|| tipo.clone());
                let objetivo = cuerpo
                    .and_then(|c| c.get("target"))
                    .map(valor_metrica)
                    .unwrap_or_default();
                let actual = actuales
                    .and_then(|a| a.get(i))
                    .and_then(|c| c.get(clave.as_str()).or_else(|| c.get("resource")))
                    .and_then(|c| c.get("current"))
                    .map(valor_metrica)
                    .unwrap_or_else(|| "?".into());
                campo(
                    ui,
                    &format!("{nombre} ({tipo})"),
                    &format!("{actual} / {objetivo}"),
                );
            }
        });
    }
    if let Some(b) = spec.and_then(|s| s.get("behavior")) {
        seccion(ui, "Behavior", |ui| {
            for dir in ["scaleUp", "scaleDown"] {
                if let Some(x) = b.get(dir) {
                    let mut partes = Vec::new();
                    if let Some(s) = x.get("stabilizationWindowSeconds").and_then(|v| v.as_i64()) {
                        partes.push(format!("estabilización {s}s"));
                    }
                    for p in x
                        .get("policies")
                        .and_then(|v| v.as_array())
                        .into_iter()
                        .flatten()
                    {
                        partes.push(format!(
                            "{} {} / {}s",
                            num(Some(p), "value"),
                            str_de(p, "type").to_lowercase(),
                            num(Some(p), "periodSeconds")
                        ));
                    }
                    campo(ui, dir, &partes.join(" · "));
                }
            }
        });
    }
}

/// `averageUtilization: 80` → `80%`; `averageValue: 500m` → `500m`; `value: 3` → `3`.
fn valor_metrica(v: &Value) -> String {
    if let Some(u) = v.get("averageUtilization").and_then(|x| x.as_i64()) {
        return format!("{u}%");
    }
    for k in ["averageValue", "value"] {
        match v.get(k) {
            Some(Value::String(s)) => return s.clone(),
            Some(Value::Number(n)) => return n.to_string(),
            _ => {}
        }
    }
    String::new()
}

fn pdb(ui: &mut egui::Ui, spec: Option<&Value>, status: Option<&Value>) {
    seccion(ui, "PodDisruptionBudget", |ui| {
        campo(
            ui,
            "Min disponibles",
            &columns::cantidad(spec, "minAvailable"),
        );
        campo(
            ui,
            "Max no disponibles",
            &columns::cantidad(spec, "maxUnavailable"),
        );
        campo(
            ui,
            "Selector",
            &columns::selector_corto(spec.and_then(|s| s.get("selector"))),
        );
        let permitidas = num(status, "disruptionsAllowed");
        campo_tono(
            ui,
            "Disrupciones permitidas",
            &permitidas.to_string(),
            if permitidas > 0 {
                theme::OK
            } else {
                theme::WARN
            },
        );
        campo(
            ui,
            "Pods",
            &format!(
                "{} sanos · {} deseados · {} esperados",
                num(status, "currentHealthy"),
                num(status, "desiredHealthy"),
                num(status, "expectedPods")
            ),
        );
    });
}

fn quota(ui: &mut egui::Ui, spec: Option<&Value>, status: Option<&Value>) {
    seccion(ui, "ResourceQuota", |ui| {
        let scopes = lista(spec, "scopes");
        if !scopes.is_empty() {
            campo(ui, "Scopes", &scopes.join(", "));
        }
        let hard = status
            .and_then(|s| s.get("hard"))
            .or_else(|| spec.and_then(|s| s.get("hard")));
        let used = status.and_then(|s| s.get("used"));
        let Some(h) = hard.and_then(|v| v.as_object()) else {
            return;
        };
        for (k, v) in h {
            let u = used
                .and_then(|u| u.get(k))
                .and_then(|x| x.as_str())
                .unwrap_or("0");
            let lim = v.as_str().unwrap_or("");
            let color = match (parse_mem(u), parse_mem(lim)) {
                (Some(a), Some(b)) if b > 0 && a * 10 >= b * 9 => theme::BAD,
                (Some(a), Some(b)) if b > 0 && a * 10 >= b * 7 => theme::WARN,
                _ => theme::TEXTO,
            };
            campo_tono(ui, k, &format!("{u} / {lim}"), color);
        }
    });
}

fn limit_range(ui: &mut egui::Ui, spec: Option<&Value>) {
    let Some(limites) = spec
        .and_then(|s| s.get("limits"))
        .and_then(|v| v.as_array())
    else {
        return;
    };
    seccion(ui, "LimitRange", |ui| {
        for l in limites {
            ui.label(egui::RichText::new(str_de(l, "type")).strong());
            for (k, etiqueta) in [
                ("default", "limit por defecto"),
                ("defaultRequest", "request por defecto"),
                ("min", "mín"),
                ("max", "máx"),
                ("maxLimitRequestRatio", "ratio máx"),
            ] {
                let t = recursos(l.get(k));
                if !t.is_empty() {
                    campo(ui, &format!("  {etiqueta}"), &t);
                }
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Storage

fn pvc(ui: &mut egui::Ui, spec: Option<&Value>, status: Option<&Value>) {
    seccion(ui, "PersistentVolumeClaim", |ui| {
        let fase = opt_str(status, "phase");
        campo_tono(
            ui,
            "Estado",
            &fase,
            if fase == "Bound" {
                theme::OK
            } else {
                theme::WARN
            },
        );
        campo(ui, "Volumen", &opt_str(spec, "volumeName"));
        campo(ui, "Storage class", &opt_str(spec, "storageClassName"));
        campo(ui, "Access modes", &lista(spec, "accessModes").join(", "));
        campo(ui, "Volume mode", &opt_str(spec, "volumeMode"));
        let pedido = spec
            .and_then(|s| s.get("resources"))
            .and_then(|r| r.get("requests"))
            .map(|r| str_de(r, "storage"))
            .unwrap_or_default();
        let real = status
            .and_then(|s| s.get("capacity"))
            .map(|c| str_de(c, "storage"))
            .unwrap_or_default();
        campo(
            ui,
            "Capacidad",
            &if real.is_empty() || real == pedido {
                pedido
            } else {
                format!("{real} (pedido {pedido})")
            },
        );
        if let Some(ds) = spec.and_then(|s| s.get("dataSource")) {
            campo(
                ui,
                "Origen",
                &format!("{}/{}", str_de(ds, "kind"), str_de(ds, "name")),
            );
        }
    });
}

fn pv(ui: &mut egui::Ui, spec: Option<&Value>, status: Option<&Value>) {
    seccion(ui, "PersistentVolume", |ui| {
        let fase = opt_str(status, "phase");
        campo_tono(
            ui,
            "Estado",
            &fase,
            match fase.as_str() {
                "Bound" | "Available" => theme::OK,
                "Failed" => theme::BAD,
                _ => theme::WARN,
            },
        );
        campo(
            ui,
            "Capacidad",
            &spec
                .and_then(|s| s.get("capacity"))
                .map(|c| str_de(c, "storage"))
                .unwrap_or_default(),
        );
        campo(ui, "Access modes", &lista(spec, "accessModes").join(", "));
        campo(
            ui,
            "Reclaim",
            &opt_str(spec, "persistentVolumeReclaimPolicy"),
        );
        campo(ui, "Storage class", &opt_str(spec, "storageClassName"));
        campo(ui, "Volume mode", &opt_str(spec, "volumeMode"));
        if let Some(c) = spec.and_then(|s| s.get("claimRef")) {
            campo(
                ui,
                "Claim",
                &format!("{}/{}", str_de(c, "namespace"), str_de(c, "name")),
            );
        }
        campo(ui, "Mount options", &lista(spec, "mountOptions").join(", "));
        if let Some(m) = spec.and_then(|s| s.as_object()) {
            const CONOCIDOS: &[&str] = &[
                "capacity",
                "accessModes",
                "persistentVolumeReclaimPolicy",
                "storageClassName",
                "volumeMode",
                "claimRef",
                "mountOptions",
                "nodeAffinity",
            ];
            if let Some((k, v)) = m.iter().find(|(k, _)| !CONOCIDOS.contains(&k.as_str())) {
                let detalle = match k.as_str() {
                    "csi" => format!("{} · {}", str_de(v, "driver"), str_de(v, "volumeHandle")),
                    "nfs" => format!("{}:{}", str_de(v, "server"), str_de(v, "path")),
                    "hostPath" | "local" => str_de(v, "path"),
                    "awsElasticBlockStore" => str_de(v, "volumeID"),
                    "gcePersistentDisk" => str_de(v, "pdName"),
                    "azureDisk" => str_de(v, "diskName"),
                    _ => String::new(),
                };
                campo(
                    ui,
                    "Fuente",
                    &if detalle.is_empty() {
                        k.clone()
                    } else {
                        format!("{k}: {detalle}")
                    },
                );
            }
        }
    });
}

fn storage_class(ui: &mut egui::Ui, d: &Value) {
    seccion(ui, "StorageClass", |ui| {
        campo(ui, "Provisioner", &str_de(d, "provisioner"));
        campo(ui, "Reclaim", &str_de(d, "reclaimPolicy"));
        campo(ui, "Binding mode", &str_de(d, "volumeBindingMode"));
        campo(ui, "Expansión", &si_no(d, "allowVolumeExpansion"));
        campo(
            ui,
            "Mount options",
            &lista(Some(d), "mountOptions").join(", "),
        );
        if let Some(p) = d.get("parameters").and_then(|v| v.as_object()) {
            let mapa: std::collections::BTreeMap<String, String> = p
                .iter()
                .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
                .collect();
            if !mapa.is_empty() {
                ui.add_space(2.0);
                chips(ui, &mapa);
            }
        }
    });
}

// ---------------------------------------------------------------------------
// RBAC

fn role(ui: &mut egui::Ui, d: &Value) {
    if let Some(agg) = d.get("aggregationRule") {
        seccion(ui, "Agregación", |ui| {
            for s in agg
                .get("clusterRoleSelectors")
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
            {
                campo(ui, "selector", &columns::selector_corto(Some(s)));
            }
        });
    }
    let Some(reglas) = d.get("rules").and_then(|v| v.as_array()) else {
        return;
    };
    seccion(ui, "Reglas", |ui| {
        for r in reglas {
            let verbos = lista(Some(r), "verbs").join(",");
            let grupos = lista(Some(r), "apiGroups");
            let recursos = lista(Some(r), "resources");
            let nombres = lista(Some(r), "resourceNames");
            let urls = lista(Some(r), "nonResourceURLs");
            let mut objeto = if !urls.is_empty() {
                urls.join(", ")
            } else {
                recursos
                    .iter()
                    .map(|res| {
                        let g = grupos
                            .first()
                            .map(|g| {
                                if g.is_empty() {
                                    "core".to_string()
                                } else {
                                    g.clone()
                                }
                            })
                            .unwrap_or_default();
                        if grupos.len() > 1 {
                            format!("{}/{res}", grupos.join("|"))
                        } else {
                            format!("{g}/{res}")
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            if !nombres.is_empty() {
                objeto.push_str(&format!(" [{}]", nombres.join(",")));
            }
            let peligroso = verbos == "*" || (verbos.contains('*') && objeto.contains('*'));
            campo_tono(
                ui,
                &verbos,
                &objeto,
                if peligroso { theme::WARN } else { theme::TEXTO },
            );
        }
    });
}

fn role_binding(ui: &mut egui::Ui, d: &Value) {
    seccion(ui, "Binding", |ui| {
        if let Some(r) = d.get("roleRef") {
            campo(
                ui,
                "Rol",
                &format!("{}/{}", str_de(r, "kind"), str_de(r, "name")),
            );
        }
    });
    let Some(sujetos) = d.get("subjects").and_then(|v| v.as_array()) else {
        return;
    };
    seccion(ui, "Sujetos", |ui| {
        for s in sujetos {
            let ns = str_de(s, "namespace");
            campo(
                ui,
                &str_de(s, "kind"),
                &if ns.is_empty() {
                    str_de(s, "name")
                } else {
                    format!("{ns}/{}", str_de(s, "name"))
                },
            );
        }
    });
}

// ---------------------------------------------------------------------------
// Otros

fn crd(ui: &mut egui::Ui, spec: Option<&Value>, status: Option<&Value>) {
    seccion(ui, "CustomResourceDefinition", |ui| {
        campo(ui, "Grupo", &opt_str(spec, "group"));
        campo(ui, "Alcance", &opt_str(spec, "scope"));
        if let Some(n) = spec.and_then(|s| s.get("names")) {
            campo(ui, "Kind", &str_de(n, "kind"));
            campo(ui, "Plural", &str_de(n, "plural"));
            campo(ui, "Singular", &str_de(n, "singular"));
            campo(
                ui,
                "Nombres cortos",
                &lista(Some(n), "shortNames").join(", "),
            );
            campo(ui, "Categorías", &lista(Some(n), "categories").join(", "));
        }
        if let Some(c) = spec.and_then(|s| s.get("conversion")) {
            campo(ui, "Conversión", &str_de(c, "strategy"));
        }
        campo(
            ui,
            "Almacenadas",
            &lista(status, "storedVersions").join(", "),
        );
    });
    if let Some(vs) = spec
        .and_then(|s| s.get("versions"))
        .and_then(|v| v.as_array())
    {
        seccion(ui, "Versiones", |ui| {
            for v in vs {
                let mut marcas = Vec::new();
                if v.get("served").and_then(|x| x.as_bool()) == Some(true) {
                    marcas.push("served");
                }
                if v.get("storage").and_then(|x| x.as_bool()) == Some(true) {
                    marcas.push("storage");
                }
                if v.get("deprecated").and_then(|x| x.as_bool()) == Some(true) {
                    marcas.push("deprecated");
                }
                let cols: Vec<String> = v
                    .get("additionalPrinterColumns")
                    .and_then(|c| c.as_array())
                    .map(|a| a.iter().map(|c| str_de(c, "name")).collect())
                    .unwrap_or_default();
                let mut t = marcas.join(", ");
                if !cols.is_empty() {
                    t.push_str(&format!("  · columnas: {}", cols.join(", ")));
                }
                campo(ui, &str_de(v, "name"), &t);
            }
        });
    }
}

fn event(ui: &mut egui::Ui, d: &Value) {
    seccion(ui, "Event", |ui| {
        let tipo = str_de(d, "type");
        campo_tono(
            ui,
            "Tipo",
            &tipo,
            if tipo == "Warning" {
                theme::WARN
            } else {
                theme::OK
            },
        );
        campo(ui, "Razón", &str_de(d, "reason"));
        if let Some(o) = d.get("involvedObject") {
            let ns = str_de(o, "namespace");
            campo(
                ui,
                "Objeto",
                &format!(
                    "{} {}{}",
                    str_de(o, "kind"),
                    if ns.is_empty() {
                        String::new()
                    } else {
                        format!("{ns}/")
                    },
                    str_de(o, "name")
                ),
            );
            campo(ui, "Campo", &str_de(o, "fieldPath"));
        }
        if let Some(s) = d.get("source") {
            let host = str_de(s, "host");
            campo(
                ui,
                "Origen",
                &format!(
                    "{}{}",
                    str_de(s, "component"),
                    if host.is_empty() {
                        String::new()
                    } else {
                        format!(" @ {host}")
                    }
                ),
            );
        }
        campo(ui, "Reporter", &str_de(d, "reportingComponent"));
        let cuenta = num(Some(d), "count");
        if cuenta > 1 {
            campo(ui, "Repeticiones", &cuenta.to_string());
        }
        campo(ui, "Primera vez", &fecha(&str_de(d, "firstTimestamp")));
        campo(
            ui,
            "Última vez",
            &fecha(&str_de(d, "lastTimestamp").max(str_de(d, "eventTime"))),
        );
    });
    let msg = str_de(d, "message");
    if !msg.is_empty() {
        seccion(ui, "Mensaje", |ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
            let resp = ui
                .add(egui::Label::new(&msg).sense(egui::Sense::click()))
                .on_hover_text("Clic para copiar");
            if resp.clicked() {
                ui.ctx().copy_text(msg.clone());
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Helpers

fn campo_tono(ui: &mut egui::Ui, clave: &str, valor: &str, color: egui::Color32) {
    if valor.is_empty() {
        return;
    }
    let ancho_clave = (ui.available_width() * 0.38).clamp(70.0, 150.0);
    ui.horizontal_top(|ui| {
        ui.add_sized(
            [ancho_clave, 16.0],
            egui::Label::new(
                egui::RichText::new(clave)
                    .size(12.0)
                    .color(theme::TEXTO_TENUE),
            )
            .truncate(),
        );
        let resto = ui.available_width().max(40.0);
        ui.allocate_ui_with_layout(
            egui::vec2(resto, 0.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_max_width(resto);
                let resp = ui
                    .add(
                        egui::Label::new(egui::RichText::new(valor).size(12.0).color(color))
                            .truncate()
                            .sense(egui::Sense::click()),
                    )
                    .on_hover_text(valor);
                if resp.clicked() {
                    ui.ctx().copy_text(valor.to_string());
                }
            },
        );
    });
}

fn num(v: Option<&Value>, k: &str) -> i64 {
    v.and_then(|v| v.get(k))
        .and_then(|v| v.as_i64())
        .unwrap_or(0)
}

fn num_o(v: Option<&Value>, k: &str, defecto: i64) -> i64 {
    v.and_then(|v| v.get(k))
        .and_then(|v| v.as_i64())
        .unwrap_or(defecto)
}

fn num_str(v: Option<&Value>, k: &str) -> String {
    v.and_then(|v| v.get(k))
        .and_then(|v| v.as_i64())
        .map(|n| n.to_string())
        .unwrap_or_default()
}

fn segundos(v: Option<&Value>, k: &str) -> String {
    v.and_then(|v| v.get(k))
        .and_then(|v| v.as_i64())
        .map(|s| duracion(s.max(0) as u64))
        .unwrap_or_default()
}

fn duracion(s: u64) -> String {
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m {}s", s / 60, s % 60)
    } else if s < 86_400 {
        format!("{}h {}m", s / 3600, (s % 3600) / 60)
    } else {
        format!("{}d {}h", s / 86_400, (s % 86_400) / 3600)
    }
}

fn si_no(d: &Value, k: &str) -> String {
    match d.get(k).and_then(|v| v.as_bool()) {
        Some(true) => "sí".into(),
        Some(false) => "no".into(),
        None => String::new(),
    }
}

fn lista(v: Option<&Value>, k: &str) -> Vec<String> {
    v.and_then(|v| v.get(k))
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// `[{name: a}, {name: b}]` → `[a, b]`.
fn nombres(v: Option<&Value>) -> Vec<String> {
    v.and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .map(|x| str_de(x, "name"))
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// `type: RollingUpdate` más sus parámetros: `RollingUpdate (surge 25%, unavail 25%)`.
fn estrategia(v: Option<&Value>, clave_params: &str) -> String {
    let Some(v) = v else {
        return String::new();
    };
    let tipo = str_de(v, "type");
    let mut params = Vec::new();
    if let Some(p) = v.get(clave_params) {
        for (k, etiqueta) in [
            ("maxSurge", "surge"),
            ("maxUnavailable", "unavail"),
            ("partition", "partition"),
        ] {
            let t = columns::cantidad(Some(p), k);
            if !t.is_empty() {
                params.push(format!("{etiqueta} {t}"));
            }
        }
    }
    if params.is_empty() {
        tipo
    } else {
        format!("{tipo} ({})", params.join(", "))
    }
}

/// Fecha RFC 3339 como `hace 5m · 2026-09-17 10:00 UTC`.
fn fecha(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }
    match s.parse::<k8s_openapi::jiff::Timestamp>() {
        Ok(t) => format!(
            "hace {} · {}",
            columns::edad_desde_rfc3339(s),
            t.strftime("%Y-%m-%d %H:%M UTC")
        ),
        Err(_) => s.to_string(),
    }
}
