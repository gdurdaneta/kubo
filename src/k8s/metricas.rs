//! Uso de CPU y memoria vía `metrics.k8s.io` (metrics-server).
//!
//! La API de métricas no soporta watch, así que se sondea. metrics-server
//! actualiza cada ~15 s de todos modos; preguntar más seguido solo devuelve
//! el mismo número.

use std::collections::{HashMap, VecDeque};

use kube::api::{Api, DynamicObject, ListParams};
use kube::discovery::ApiResource;
use kube::Client;
use kube::ResourceExt;

use super::watch::Target;
use super::{K8sEvent, UiBridge};

/// Cada cuánto se vuelve a preguntar.
pub const INTERVALO_S: u64 = 15;
/// Muestras que se guardan por objeto para el sparkline (~10 min a 15 s).
pub const HISTORIAL: usize = 40;

/// Consumo instantáneo de un pod o un nodo.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Uso {
    /// Milicores.
    pub cpu_m: u64,
    pub mem_bytes: u64,
}

pub fn ar_pods() -> ApiResource {
    ApiResource {
        group: "metrics.k8s.io".into(),
        version: "v1beta1".into(),
        api_version: "metrics.k8s.io/v1beta1".into(),
        kind: "PodMetrics".into(),
        plural: "pods".into(),
    }
}

pub fn ar_nodes() -> ApiResource {
    ApiResource {
        group: "metrics.k8s.io".into(),
        version: "v1beta1".into(),
        api_version: "metrics.k8s.io/v1beta1".into(),
        kind: "NodeMetrics".into(),
        plural: "nodes".into(),
    }
}

/// Cantidad de CPU de Kubernetes a milicores: "250m", "1", "1.5", "29621n".
pub fn parse_cpu(s: &str) -> Option<u64> {
    let s = s.trim();
    if let Some(n) = s.strip_suffix('n') {
        return Some(n.parse::<u64>().ok()? / 1_000_000);
    }
    if let Some(u) = s.strip_suffix('u') {
        return Some(u.parse::<u64>().ok()? / 1_000);
    }
    if let Some(m) = s.strip_suffix('m') {
        return m.parse::<u64>().ok();
    }
    let cores: f64 = s.parse().ok()?;
    Some((cores * 1000.0).round() as u64)
}

/// Cantidad de memoria a bytes: "128Mi", "1Gi", "20824Ki", "1000000", "1M".
pub fn parse_mem(s: &str) -> Option<u64> {
    let s = s.trim();
    const BIN: &[(&str, u64)] = &[
        ("Ei", 1 << 60),
        ("Pi", 1 << 50),
        ("Ti", 1 << 40),
        ("Gi", 1 << 30),
        ("Mi", 1 << 20),
        ("Ki", 1 << 10),
    ];
    const DEC: &[(&str, u64)] = &[
        ("E", 1_000_000_000_000_000_000),
        ("P", 1_000_000_000_000_000),
        ("T", 1_000_000_000_000),
        ("G", 1_000_000_000),
        ("M", 1_000_000),
        ("k", 1_000),
    ];
    for (suf, mult) in BIN.iter().chain(DEC) {
        if let Some(n) = s.strip_suffix(suf) {
            let v: f64 = n.parse().ok()?;
            return Some((v * *mult as f64) as u64);
        }
    }
    // "1e3" también es válido en la API, pero no aparece en métricas.
    s.parse::<f64>().ok().map(|v| v as u64)
}

/// "12m" hasta un core, después "1.25".
pub fn fmt_cpu(m: u64) -> String {
    if m < 1000 {
        format!("{m}m")
    } else {
        format!("{:.2}", m as f64 / 1000.0)
    }
}

/// Binario con una decimal desde Mi, entero por debajo.
pub fn fmt_mem(b: u64) -> String {
    const GI: f64 = (1u64 << 30) as f64;
    const MI: f64 = (1u64 << 20) as f64;
    const KI: f64 = (1u64 << 10) as f64;
    let b = b as f64;
    if b >= GI {
        format!("{:.1}Gi", b / GI)
    } else if b >= MI {
        format!("{:.0}Mi", b / MI)
    } else if b >= KI {
        format!("{:.0}Ki", b / KI)
    } else {
        format!("{b:.0}")
    }
}

/// Suma el uso de todos los contenedores de un PodMetrics, o lee el de un
/// NodeMetrics.
fn leer(o: &DynamicObject) -> Option<(String, Uso)> {
    let clave = match o.namespace() {
        Some(ns) => format!("{ns}/{}", o.name_any()),
        None => o.name_any(),
    };
    let mut uso = Uso::default();
    let sumar = |uso: &mut Uso, u: &serde_json::Value| {
        if let Some(c) = u.get("cpu").and_then(|v| v.as_str()).and_then(parse_cpu) {
            uso.cpu_m += c;
        }
        if let Some(m) = u.get("memory").and_then(|v| v.as_str()).and_then(parse_mem) {
            uso.mem_bytes += m;
        }
    };
    if let Some(cs) = o.data.get("containers").and_then(|c| c.as_array()) {
        for c in cs {
            if let Some(u) = c.get("usage") {
                sumar(&mut uso, u);
            }
        }
    } else if let Some(u) = o.data.get("usage") {
        sumar(&mut uso, u);
    } else {
        return None;
    }
    Some((clave, uso))
}

/// Sondea las métricas del ámbito y publica el mapa `clave -> uso` cada
/// `INTERVALO_S`. Corre hasta que se aborta la tarea.
pub async fn sondear(
    client: Client,
    ar: ApiResource,
    target: Target,
    token: u64,
    bridge: UiBridge,
) {
    let api: Api<DynamicObject> = match &target {
        Target::Namespace(ns) => Api::namespaced_with(client, ns, &ar),
        Target::AllNamespaces => Api::all_with(client, &ar),
    };
    let mut intervalo = tokio::time::interval(std::time::Duration::from_secs(INTERVALO_S));
    intervalo.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut avisado = false;
    loop {
        intervalo.tick().await;
        match api.list(&ListParams::default().limit(2000)).await {
            Ok(lista) => {
                avisado = false;
                let mapa: HashMap<String, Uso> = lista.items.iter().filter_map(leer).collect();
                tracing::debug!(kind = %ar.kind, n = mapa.len(), "métricas: muestra");
                bridge.send(K8sEvent::Metricas { token, mapa });
            }
            Err(e) => {
                // metrics-server caído o sin permiso: la columna queda vacía y
                // se sigue intentando, pero se avisa una sola vez.
                if !avisado {
                    avisado = true;
                    tracing::warn!(kind = %ar.kind, error = %e, "métricas: no se pudieron leer");
                }
            }
        }
    }
}

/// Historial acotado por objeto, para el sparkline del detalle.
#[derive(Default)]
pub struct Historial {
    pub por_clave: HashMap<String, VecDeque<Uso>>,
}

impl Historial {
    pub fn agregar(&mut self, mapa: &HashMap<String, Uso>) {
        for (k, u) in mapa {
            let h = self.por_clave.entry(k.clone()).or_default();
            h.push_back(*u);
            while h.len() > HISTORIAL {
                h.pop_front();
            }
        }
        // Lo que ya no aparece (pod borrado) se descarta para no crecer sin tope.
        self.por_clave.retain(|k, _| mapa.contains_key(k));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_en_todas_las_unidades() {
        assert_eq!(parse_cpu("250m"), Some(250));
        assert_eq!(parse_cpu("1"), Some(1000));
        assert_eq!(parse_cpu("1.5"), Some(1500));
        assert_eq!(parse_cpu("0.1"), Some(100));
        // Lo que devuelve metrics-server: nanocores.
        assert_eq!(parse_cpu("29621n"), Some(0), "29 µcores redondea a 0 m");
        assert_eq!(parse_cpu("260700389n"), Some(260));
        assert_eq!(parse_cpu("1500u"), Some(1));
        assert_eq!(parse_cpu("basura"), None);
    }

    #[test]
    fn memoria_binaria_y_decimal() {
        assert_eq!(parse_mem("1Ki"), Some(1024));
        assert_eq!(parse_mem("20824Ki"), Some(20824 * 1024));
        assert_eq!(parse_mem("128Mi"), Some(128 << 20));
        assert_eq!(parse_mem("1Gi"), Some(1 << 30));
        assert_eq!(parse_mem("1M"), Some(1_000_000));
        assert_eq!(parse_mem("1k"), Some(1_000));
        assert_eq!(parse_mem("1000000"), Some(1_000_000));
        assert_eq!(parse_mem("1.5Gi"), Some(3 << 29));
        assert_eq!(parse_mem("nada"), None);
    }

    #[test]
    fn formatea_como_kubectl_top() {
        assert_eq!(fmt_cpu(12), "12m");
        assert_eq!(fmt_cpu(999), "999m");
        assert_eq!(fmt_cpu(1000), "1.00");
        assert_eq!(fmt_cpu(2500), "2.50");
        assert_eq!(fmt_mem(20824 * 1024), "20Mi");
        assert_eq!(fmt_mem(53900792 * 1024), "51.4Gi");
        assert_eq!(fmt_mem(512), "512");
    }

    fn obj(v: serde_json::Value) -> DynamicObject {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn suma_los_contenedores_de_un_pod() {
        let o = obj(serde_json::json!({
            "apiVersion": "metrics.k8s.io/v1beta1", "kind": "PodMetrics",
            "metadata": { "name": "web-1", "namespace": "prod" },
            "containers": [
                { "name": "app",     "usage": { "cpu": "100m", "memory": "100Mi" } },
                { "name": "sidecar", "usage": { "cpu": "50m",  "memory": "20Mi" } }
            ]
        }));
        let (clave, uso) = leer(&o).unwrap();
        assert_eq!(clave, "prod/web-1");
        assert_eq!(uso, Uso { cpu_m: 150, mem_bytes: 120 << 20 });
    }

    #[test]
    fn lee_un_nodo_sin_namespace() {
        let o = obj(serde_json::json!({
            "apiVersion": "metrics.k8s.io/v1beta1", "kind": "NodeMetrics",
            "metadata": { "name": "nautilus" },
            "usage": { "cpu": "260700389n", "memory": "53900792Ki" }
        }));
        let (clave, uso) = leer(&o).unwrap();
        assert_eq!(clave, "nautilus");
        assert_eq!(uso.cpu_m, 260);
        assert_eq!(uso.mem_bytes, 53900792 * 1024);
    }

    #[test]
    fn el_historial_se_acota_y_olvida_lo_borrado() {
        let mut h = Historial::default();
        for i in 0..(HISTORIAL as u64 + 10) {
            let mut m = HashMap::new();
            m.insert("a".to_string(), Uso { cpu_m: i, mem_bytes: 0 });
            h.agregar(&m);
        }
        assert_eq!(h.por_clave["a"].len(), HISTORIAL);
        assert_eq!(h.por_clave["a"].back().unwrap().cpu_m, HISTORIAL as u64 + 9);

        let solo_b: HashMap<_, _> = [("b".to_string(), Uso::default())].into();
        h.agregar(&solo_b);
        assert!(!h.por_clave.contains_key("a"), "lo que dejó de existir se va");
    }
}
