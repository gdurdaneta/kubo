//! Árbol de navegación. El orden es fijo (como en Lens) pero el contenido se
//! cruza contra discovery: solo se muestra lo que el cluster realmente sirve.
//!
//! Además de los recursos de siempre, detecta operadores instalados (Istio,
//! Argo CD, Gateway API…) por sus grupos de API y les da su propia sección en
//! vez de dejar los CRDs sueltos en una lista alfabética.

use std::collections::BTreeMap;

use crate::k8s::Discovered;

/// (grupo de la API, Kind). Grupo vacío = core/v1.
type Gk = (&'static str, &'static str);

pub struct CategorySpec {
    pub name: &'static str,
    pub icono: &'static str,
    pub kinds: &'static [Gk],
}

pub const CATALOG: &[CategorySpec] = &[
    CategorySpec {
        name: "Cluster",
        icono: "◈",
        kinds: &[
            ("", "Node"),
            ("", "Namespace"),
            ("", "Event"),
            ("apiextensions.k8s.io", "CustomResourceDefinition"),
        ],
    },
    CategorySpec {
        name: "Workloads",
        icono: "▦",
        kinds: &[
            ("", "Pod"),
            ("apps", "Deployment"),
            ("apps", "DaemonSet"),
            ("apps", "StatefulSet"),
            ("apps", "ReplicaSet"),
            ("", "ReplicationController"),
            ("batch", "Job"),
            ("batch", "CronJob"),
        ],
    },
    CategorySpec {
        name: "Config",
        icono: "◎",
        kinds: &[
            ("", "ConfigMap"),
            ("", "Secret"),
            ("", "ResourceQuota"),
            ("", "LimitRange"),
            ("autoscaling", "HorizontalPodAutoscaler"),
            ("policy", "PodDisruptionBudget"),
            ("scheduling.k8s.io", "PriorityClass"),
        ],
    },
    CategorySpec {
        name: "Network",
        icono: "⇅",
        kinds: &[
            ("", "Service"),
            ("", "Endpoints"),
            ("discovery.k8s.io", "EndpointSlice"),
            ("networking.k8s.io", "Ingress"),
            ("networking.k8s.io", "IngressClass"),
            ("networking.k8s.io", "NetworkPolicy"),
        ],
    },
    CategorySpec {
        name: "Storage",
        icono: "▤",
        kinds: &[
            ("", "PersistentVolumeClaim"),
            ("", "PersistentVolume"),
            ("storage.k8s.io", "StorageClass"),
        ],
    },
    CategorySpec {
        name: "Access Control",
        icono: "◐",
        kinds: &[
            ("", "ServiceAccount"),
            ("rbac.authorization.k8s.io", "Role"),
            ("rbac.authorization.k8s.io", "RoleBinding"),
            ("rbac.authorization.k8s.io", "ClusterRole"),
            ("rbac.authorization.k8s.io", "ClusterRoleBinding"),
        ],
    },
];

/// Un operador o extensión que, si está instalado, merece sección propia.
struct ExtSpec {
    nombre: &'static str,
    icono: &'static str,
    /// Grupos de API que delatan la extensión. Un patrón `*.suf.ijo`
    /// hace match por sufijo (Flux publica varios `*.toolkit.fluxcd.io`).
    grupos: &'static [&'static str],
    /// Kinds que van primero, en este orden. El resto queda detrás, alfabético.
    orden: &'static [&'static str],
    /// Si está, la sección se anida dentro de esa categoría del catálogo base.
    dentro_de: Option<&'static str>,
}

const EXTENSIONES: &[ExtSpec] = &[
    ExtSpec {
        nombre: "Gateway API",
        icono: "⇉",
        grupos: &["gateway.networking.k8s.io"],
        orden: &[
            "GatewayClass",
            "Gateway",
            "HTTPRoute",
            "GRPCRoute",
            "ReferenceGrant",
            "TCPRoute",
            "TLSRoute",
            "UDPRoute",
            "BackendTLSPolicy",
        ],
        dentro_de: Some("Network"),
    },
    ExtSpec {
        nombre: "Argo CD",
        icono: "◆",
        grupos: &["argoproj.io"],
        orden: &[
            "Application",
            "ApplicationSet",
            "AppProject",
            "Rollout",
            "AnalysisTemplate",
            "AnalysisRun",
            "Workflow",
            "WorkflowTemplate",
            "CronWorkflow",
        ],
        dentro_de: None,
    },
    ExtSpec {
        nombre: "Istio",
        icono: "◆",
        grupos: &[
            "networking.istio.io",
            "security.istio.io",
            "telemetry.istio.io",
            "install.istio.io",
            "extensions.istio.io",
        ],
        orden: &[
            "VirtualService",
            "DestinationRule",
            "Gateway",
            "ServiceEntry",
            "WorkloadEntry",
            "WorkloadGroup",
            "Sidecar",
            "EnvoyFilter",
            "PeerAuthentication",
            "RequestAuthentication",
            "AuthorizationPolicy",
            "Telemetry",
            "WasmPlugin",
            "IstioOperator",
        ],
        dentro_de: None,
    },
    ExtSpec {
        nombre: "cert-manager",
        icono: "◆",
        grupos: &["cert-manager.io", "acme.cert-manager.io"],
        orden: &[
            "Certificate",
            "CertificateRequest",
            "Issuer",
            "ClusterIssuer",
            "Order",
            "Challenge",
        ],
        dentro_de: None,
    },
    ExtSpec {
        nombre: "Prometheus Operator",
        icono: "◆",
        grupos: &["monitoring.coreos.com"],
        orden: &[
            "Prometheus",
            "Alertmanager",
            "ServiceMonitor",
            "PodMonitor",
            "PrometheusRule",
            "Probe",
            "ThanosRuler",
        ],
        dentro_de: None,
    },
    ExtSpec {
        nombre: "Flux",
        icono: "◆",
        grupos: &["*.toolkit.fluxcd.io"],
        orden: &[
            "GitRepository",
            "OCIRepository",
            "HelmRepository",
            "HelmChart",
            "Kustomization",
            "HelmRelease",
            "Bucket",
            "Receiver",
            "Alert",
            "Provider",
        ],
        dentro_de: None,
    },
    ExtSpec {
        nombre: "KEDA",
        icono: "◆",
        grupos: &["keda.sh"],
        orden: &[
            "ScaledObject",
            "ScaledJob",
            "TriggerAuthentication",
            "ClusterTriggerAuthentication",
        ],
        dentro_de: None,
    },
    ExtSpec {
        nombre: "MetalLB",
        icono: "◆",
        grupos: &["metallb.io"],
        orden: &[
            "IPAddressPool",
            "L2Advertisement",
            "BGPAdvertisement",
            "BGPPeer",
            "Community",
            "ServiceL2Status",
        ],
        dentro_de: Some("Network"),
    },
    ExtSpec {
        nombre: "RabbitMQ Operator",
        icono: "◆",
        grupos: &["rabbitmq.com"],
        orden: &["RabbitmqCluster", "Queue", "Exchange", "Binding", "Policy", "User", "Vhost"],
        dentro_de: None,
    },
    ExtSpec {
        nombre: "External Secrets",
        icono: "◆",
        grupos: &["external-secrets.io", "generators.external-secrets.io"],
        orden: &[
            "ExternalSecret",
            "SecretStore",
            "ClusterSecretStore",
            "PushSecret",
            "ClusterExternalSecret",
        ],
        dentro_de: None,
    },
];

/// Vista que no es un recurso del cluster sino estado local de kubo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VistaLocal {
    PortForwards,
    Auditoria,
}

impl VistaLocal {
    pub fn label(self) -> &'static str {
        match self {
            Self::PortForwards => "Port Forwards",
            Self::Auditoria => "Acciones hechas",
        }
    }
}

/// Una entrada clickeable del sidebar.
#[derive(Clone, Debug)]
pub struct NavItem {
    pub label: String,
    pub res: Discovered,
}

#[derive(Clone, Debug)]
pub struct NavCategory {
    pub name: String,
    pub icono: String,
    pub items: Vec<NavItem>,
    pub subgrupos: Vec<NavCategory>,
    /// Grupos de API que la originaron; se muestra al pasar el mouse.
    pub detalle: Option<String>,
    /// Sección aportada por un operador, no parte de Kubernetes base.
    pub extension: bool,
    /// Vistas locales de kubo que viven en esta categoría.
    pub locales: Vec<VistaLocal>,
}

impl NavCategory {
    fn nueva(name: impl Into<String>, icono: &str) -> Self {
        Self {
            name: name.into(),
            icono: icono.to_string(),
            items: Vec::new(),
            subgrupos: Vec::new(),
            locales: Vec::new(),
            detalle: None,
            extension: false,
        }
    }

    /// Cuenta los recursos propios y los de sus subgrupos.
    pub fn total(&self) -> usize {
        self.items.len()
            + self.locales.len()
            + self.subgrupos.iter().map(|s| s.total()).sum::<usize>()
    }
}

fn grupo_coincide(grupo: &str, patron: &str) -> bool {
    match patron.strip_prefix("*.") {
        Some(sufijo) => grupo == sufijo || grupo.ends_with(&format!(".{sufijo}")),
        None => grupo == patron,
    }
}

/// Arma el árbol: categorías canónicas, después las extensiones detectadas y
/// al final lo que quede, agrupado por grupo de API.
pub fn build(resources: &[Discovered]) -> Vec<NavCategory> {
    let mut usados: Vec<String> = Vec::new();
    let mut base: Vec<NavCategory> = Vec::new();

    for spec in CATALOG {
        let mut cat = NavCategory::nueva(spec.name, spec.icono);
        for (group, kind) in spec.kinds {
            // Discovery puede exponer varias versiones; `resources` ya viene
            // priorizado por estabilidad, así que la primera coincidencia sirve.
            if let Some(r) = resources
                .iter()
                .find(|r| r.ar.group == *group && r.ar.kind == *kind && r.watchable())
            {
                usados.push(r.key());
                cat.items.push(NavItem {
                    label: plural_legible(&r.ar.kind),
                    res: r.clone(),
                });
            }
        }
        // Los port-forward son de red y se manejan desde acá, aunque el estado
        // sea local y no un recurso del cluster.
        if spec.name == "Network" {
            cat.locales.push(VistaLocal::PortForwards);
        }
        // El registro de lo que uno hizo es del cliente, no del cluster, pero
        // se busca junto con Events: por eso va en Cluster.
        if spec.name == "Cluster" {
            cat.locales.push(VistaLocal::Auditoria);
        }
        if !cat.items.is_empty() || !cat.locales.is_empty() {
            base.push(cat);
        }
    }

    // --- extensiones instaladas -------------------------------------------
    let mut sueltas: Vec<NavCategory> = Vec::new();
    for ext in EXTENSIONES {
        let encontrados: Vec<&Discovered> = resources
            .iter()
            .filter(|r| {
                r.watchable()
                    && !usados.contains(&r.key())
                    && ext.grupos.iter().any(|p| grupo_coincide(&r.ar.group, p))
            })
            .collect();
        if encontrados.is_empty() {
            continue;
        }

        let mut cat = NavCategory::nueva(ext.nombre, ext.icono);
        cat.extension = true;

        let mut grupos_vistos: Vec<&str> = Vec::new();
        let mut items: Vec<(usize, NavItem)> = Vec::new();
        for r in encontrados {
            usados.push(r.key());
            if !grupos_vistos.contains(&r.ar.group.as_str()) {
                grupos_vistos.push(&r.ar.group);
            }
            // Los del orden preferido van primero; el resto detrás.
            let peso = ext
                .orden
                .iter()
                .position(|k| *k == r.ar.kind)
                .unwrap_or(ext.orden.len());
            items.push((
                peso,
                NavItem {
                    label: plural_legible(&r.ar.kind),
                    res: r.clone(),
                },
            ));
        }
        items.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.label.cmp(&b.1.label)));
        cat.items = items.into_iter().map(|(_, i)| i).collect();
        cat.detalle = Some(grupos_vistos.join(", "));

        match ext.dentro_de {
            Some(padre) => match base.iter_mut().find(|c| c.name == padre) {
                Some(c) => c.subgrupos.push(cat),
                // Si la categoría padre no existe en este cluster, no se pierde.
                None => sueltas.push(cat),
            },
            None => sueltas.push(cat),
        }
    }

    let mut out = base;
    out.append(&mut sueltas);

    // --- lo que no encajó en ningún lado ----------------------------------
    let mut extra: BTreeMap<String, Vec<NavItem>> = BTreeMap::new();
    for r in resources {
        if usados.contains(&r.key()) || !r.watchable() {
            continue;
        }
        // Ruido de sistema que nunca se navega a mano.
        if matches!(
            r.ar.group.as_str(),
            "authentication.k8s.io"
                | "authorization.k8s.io"
                | "metrics.k8s.io"
                | "coordination.k8s.io"
        ) {
            continue;
        }
        let grupo = if r.ar.group.is_empty() {
            "core".to_string()
        } else {
            r.ar.group.clone()
        };
        extra.entry(grupo).or_default().push(NavItem {
            label: plural_legible(&r.ar.kind),
            res: r.clone(),
        });
    }
    for (grupo, mut items) in extra {
        items.sort_by(|a, b| a.label.cmp(&b.label));
        let mut cat = NavCategory::nueva(grupo, "▪");
        cat.items = items;
        out.push(cat);
    }

    out
}

/// Pluraliza el Kind para el sidebar: "NetworkPolicy" -> "Network Policies".
///
/// Respeta los acrónimos, que abundan en los CRDs de red: "HTTPRoute" tiene
/// que quedar "HTTP Routes", no "H T T P Routes".
fn plural_legible(kind: &str) -> String {
    let cs: Vec<char> = kind.chars().collect();
    let mut palabras = String::new();
    for (i, c) in cs.iter().enumerate() {
        if i > 0 && c.is_uppercase() {
            let anterior = cs[i - 1];
            let siguiente = cs.get(i + 1);
            // Corta al salir de una palabra en minúsculas, o al final de un
            // acrónimo (la mayúscula que arranca la palabra siguiente).
            let fin_de_palabra = anterior.is_lowercase() || anterior.is_numeric();
            let fin_de_acronimo =
                anterior.is_uppercase() && siguiente.is_some_and(|n| n.is_lowercase());
            if fin_de_palabra || fin_de_acronimo {
                palabras.push(' ');
            }
        }
        palabras.push(*c);
    }
    let vocal_antes_de_y = palabras
        .chars()
        .rev()
        .nth(1)
        .is_some_and(|c| "aeiou".contains(c.to_ascii_lowercase()));
    if palabras.ends_with('y') && !vocal_antes_de_y {
        // Policy -> Policies, pero Gateway -> Gateways.
        format!("{}ies", &palabras[..palabras.len() - 1])
    } else if palabras.ends_with("ss") {
        // Ingress -> Ingresses
        format!("{palabras}es")
    } else if palabras.ends_with('s') {
        // Endpoints ya viene en plural desde la API.
        palabras
    } else {
        format!("{palabras}s")
    }
}

#[cfg(test)]
mod tests {
    use super::{build, grupo_coincide, plural_legible};
    use crate::k8s::Discovered;
    use kube::discovery::ApiResource;

    fn recurso(group: &str, version: &str, kind: &str) -> Discovered {
        let api_version = if group.is_empty() {
            version.to_string()
        } else {
            format!("{group}/{version}")
        };
        Discovered {
            ar: ApiResource {
                group: group.into(),
                version: version.into(),
                api_version,
                kind: kind.into(),
                plural: format!("{}s", kind.to_lowercase()),
            },
            namespaced: true,
            verbs: vec!["list".into(), "watch".into(), "get".into()],
        }
    }

    #[test]
    fn pluraliza_los_kinds_de_kubernetes() {
        assert_eq!(plural_legible("Pod"), "Pods");
        assert_eq!(plural_legible("NetworkPolicy"), "Network Policies");
        assert_eq!(plural_legible("Ingress"), "Ingresses");
        assert_eq!(plural_legible("Endpoints"), "Endpoints");
        assert_eq!(plural_legible("PodDisruptionBudget"), "Pod Disruption Budgets");
        assert_eq!(
            plural_legible("CustomResourceDefinition"),
            "Custom Resource Definitions"
        );
        assert_eq!(plural_legible("EndpointSlice"), "Endpoint Slices");
        assert_eq!(plural_legible("HTTPRoute"), "HTTP Routes");
        assert_eq!(plural_legible("GRPCRoute"), "GRPC Routes");
        assert_eq!(plural_legible("OCIRepository"), "OCI Repositories");
        assert_eq!(plural_legible("IstioOperator"), "Istio Operators");
        assert_eq!(plural_legible("APIService"), "API Services");
        assert_eq!(plural_legible("Gateway"), "Gateways");
        assert_eq!(plural_legible("GatewayClass"), "Gateway Classes");
    }

    #[test]
    fn detecta_extensiones_y_las_anida() {
        let recursos = vec![
            recurso("", "v1", "Pod"),
            recurso("", "v1", "Service"),
            recurso("gateway.networking.k8s.io", "v1", "Gateway"),
            recurso("gateway.networking.k8s.io", "v1", "HTTPRoute"),
            recurso("argoproj.io", "v1alpha1", "Application"),
            recurso("argoproj.io", "v1alpha1", "AppProject"),
            recurso("networking.istio.io", "v1", "VirtualService"),
            recurso("security.istio.io", "v1", "AuthorizationPolicy"),
            recurso("un.grupo.desconocido.io", "v1", "Widget"),
        ];
        let arbol = build(&recursos);

        // Gateway API queda anidado dentro de Network.
        let network = arbol.iter().find(|c| c.name == "Network").expect("Network");
        let gw = network
            .subgrupos
            .iter()
            .find(|s| s.name == "Gateway API")
            .expect("Gateway API anidado");
        assert!(gw.extension);
        // El orden preferido: Gateways antes que HTTP Routes.
        assert_eq!(gw.items[0].label, "Gateways");
        assert_eq!(gw.items[1].label, "HTTP Routes");

        // Argo CD e Istio como secciones de primer nivel.
        let argo = arbol.iter().find(|c| c.name == "Argo CD").expect("Argo CD");
        assert!(argo.extension);
        assert_eq!(argo.items[0].label, "Applications");
        let istio = arbol.iter().find(|c| c.name == "Istio").expect("Istio");
        // Istio junta kinds de varios grupos en una sola sección.
        assert_eq!(istio.items.len(), 2);
        assert_eq!(istio.items[0].label, "Virtual Services");

        // Lo desconocido cae agrupado por su grupo de API, no se pierde.
        assert!(arbol.iter().any(|c| c.name == "un.grupo.desconocido.io"));

        // El Gateway de Istio no se confunde con el de Gateway API.
        assert!(!istio.items.iter().any(|i| i.label == "Gateways"));
    }

    #[test]
    fn detecta_grupos_exactos_y_por_sufijo() {
        assert!(grupo_coincide("argoproj.io", "argoproj.io"));
        assert!(!grupo_coincide("argoproj.io", "cert-manager.io"));
        assert!(grupo_coincide("source.toolkit.fluxcd.io", "*.toolkit.fluxcd.io"));
        assert!(grupo_coincide("helm.toolkit.fluxcd.io", "*.toolkit.fluxcd.io"));
        assert!(!grupo_coincide("toolkit.fluxcd.io.evil.com", "*.toolkit.fluxcd.io"));
    }
}
