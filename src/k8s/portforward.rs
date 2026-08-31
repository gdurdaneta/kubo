//! Port-forward de un Service: escucha en local y tuneliza cada conexión a un
//! pod detrás del servicio, igual que `kubectl port-forward svc/...`.
//!
//! El túnel es por conexión: cada TCP entrante abre su propio stream contra el
//! API server. Multiplexar uno solo sería más barato en handshakes, pero un
//! corte se llevaría puestas todas las conexiones a la vez.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use anyhow::{anyhow, Context as _, Result};
use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, DynamicObject, ListParams};
use kube::discovery::ApiResource;
use kube::Client;
use tokio::net::TcpListener;

use super::{K8sEvent, PfMsg, UiBridge};

/// Un puerto publicado por el Service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PuertoSvc {
    pub nombre: Option<String>,
    /// Puerto del Service (el que se usa dentro del cluster).
    pub puerto: u16,
    /// `targetPort`: número, o nombre a resolver contra el pod.
    pub target: Target,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Numero(u16),
    Nombre(String),
}

impl PuertoSvc {
    /// Etiqueta para el combo del modal.
    pub fn etiqueta(&self) -> String {
        match &self.nombre {
            Some(n) => format!("{} · {}", self.puerto, n),
            None => self.puerto.to_string(),
        }
    }
}

fn ar_service() -> ApiResource {
    ApiResource {
        group: String::new(),
        version: "v1".into(),
        api_version: "v1".into(),
        kind: "Service".into(),
        plural: "services".into(),
    }
}

/// Puertos que publica el Service, para que el usuario elija.
pub async fn puertos_de(client: Client, ns: &str, svc: &str) -> Result<Vec<PuertoSvc>> {
    let api: Api<DynamicObject> = Api::namespaced_with(client, ns, &ar_service());
    let obj = api.get(svc).await.context("no se pudo leer el Service")?;
    let puertos = obj
        .data
        .get("spec")
        .and_then(|s| s.get("ports"))
        .and_then(|p| p.as_array())
        .ok_or_else(|| anyhow!("el Service no declara puertos"))?;

    let mut out = Vec::new();
    for p in puertos {
        // Solo TCP: el port-forward del API server no tuneliza UDP.
        if p.get("protocol").and_then(|v| v.as_str()).unwrap_or("TCP") != "TCP" {
            continue;
        }
        let Some(puerto) = p.get("port").and_then(|v| v.as_u64()) else {
            continue;
        };
        let target = match p.get("targetPort") {
            Some(serde_json::Value::Number(n)) => Target::Numero(n.as_u64().unwrap_or(0) as u16),
            Some(serde_json::Value::String(s)) => Target::Nombre(s.clone()),
            // Sin targetPort, el default de Kubernetes es el mismo `port`.
            _ => Target::Numero(puerto as u16),
        };
        out.push(PuertoSvc {
            nombre: p
                .get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            puerto: puerto as u16,
            target,
        });
    }
    if out.is_empty() {
        return Err(anyhow!("el Service no publica ningún puerto TCP"));
    }
    Ok(out)
}

/// Pod listo detrás del Service y el puerto del contenedor al que hay que ir.
pub async fn elegir_pod(
    client: Client,
    ns: &str,
    svc: &str,
    puerto: &PuertoSvc,
) -> Result<(String, u16)> {
    let api: Api<DynamicObject> = Api::namespaced_with(client.clone(), ns, &ar_service());
    let obj = api.get(svc).await.context("no se pudo leer el Service")?;
    let selector = obj
        .data
        .get("spec")
        .and_then(|s| s.get("selector"))
        .and_then(|s| s.as_object())
        .ok_or_else(|| anyhow!("el Service no tiene selector (¿headless o ExternalName?)"))?;
    if selector.is_empty() {
        return Err(anyhow!("el Service no tiene selector"));
    }
    let etiquetas: Vec<String> = selector
        .iter()
        .filter_map(|(k, v)| v.as_str().map(|v| format!("{k}={v}")))
        .collect();

    let pods: Api<Pod> = Api::namespaced(client, ns);
    let lista = pods
        .list(&ListParams::default().labels(&etiquetas.join(",")).limit(100))
        .await
        .context("no se pudieron listar los pods del Service")?;

    let pod = lista
        .items
        .iter()
        .find(|p| pod_listo(p))
        .ok_or_else(|| anyhow!("ningún pod del Service está Ready"))?;
    let nombre = pod
        .metadata
        .name
        .clone()
        .ok_or_else(|| anyhow!("el pod no tiene nombre"))?;

    let puerto_pod = match &puerto.target {
        Target::Numero(n) => *n,
        Target::Nombre(n) => resolver_puerto_nombrado(pod, n)
            .ok_or_else(|| anyhow!("el pod no declara el puerto '{n}'"))?,
    };
    Ok((nombre, puerto_pod))
}

fn pod_listo(p: &Pod) -> bool {
    p.status
        .as_ref()
        .and_then(|s| s.conditions.as_ref())
        .is_some_and(|cs| {
            cs.iter()
                .any(|c| c.type_ == "Ready" && c.status == "True")
        })
}

/// `targetPort: http` se resuelve contra los `containerPort` del pod.
fn resolver_puerto_nombrado(p: &Pod, nombre: &str) -> Option<u16> {
    p.spec.as_ref()?.containers.iter().find_map(|c| {
        c.ports.as_ref()?.iter().find_map(|cp| {
            (cp.name.as_deref() == Some(nombre)).then_some(cp.container_port as u16)
        })
    })
}

/// IP de loopback estable y propia de cada servicio.
///
/// Todo 127.0.0.0/8 es loopback en Linux, así que se puede bindear cualquiera
/// sin permisos ni configurar interfaces. Una IP por servicio permite que dos
/// servicios distintos usen el mismo puerto, como en el cluster.
pub fn ip_para(nombre: &str) -> Ipv4Addr {
    use std::hash::{Hash as _, Hasher as _};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    nombre.hash(&mut h);
    let x = h.finish();
    // 127.30.b.c, con b y c fuera de .0 y .255 para no pisar red ni broadcast.
    let b = (x >> 8) as u8;
    let c = (x & 0xff) as u8;
    Ipv4Addr::new(127, 30, b.clamp(1, 254), c.clamp(1, 254))
}

/// Escucha en `addr` y tuneliza cada conexión al pod. Corre hasta que se aborta
/// la tarea.
pub async fn servir(
    client: Client,
    ns: String,
    pod: String,
    puerto_pod: u16,
    addr: SocketAddr,
    id: u64,
    bridge: UiBridge,
) {
    let listener = match TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            let detalle = if addr.port() < 1024 {
                format!("{e} (los puertos bajo 1024 necesitan privilegios)")
            } else {
                e.to_string()
            };
            bridge.send(K8sEvent::Pf {
                id,
                msg: PfMsg::Fatal(format!("no se pudo escuchar en {addr}: {detalle}")),
            });
            return;
        }
    };
    tracing::info!(%addr, %ns, %pod, puerto_pod, "port-forward: escuchando");
    bridge.send(K8sEvent::Pf {
        id,
        msg: PfMsg::Escuchando,
    });

    loop {
        let (mut sock, origen) = match listener.accept().await {
            Ok(v) => v,
            Err(e) => {
                bridge.send(K8sEvent::Pf {
                    id,
                    msg: PfMsg::Fatal(format!("accept falló: {e}")),
                });
                return;
            }
        };
        let api: Api<Pod> = Api::namespaced(client.clone(), &ns);
        let (pod, b) = (pod.clone(), bridge.clone());
        tokio::spawn(async move {
            b.send(K8sEvent::Pf {
                id,
                msg: PfMsg::Conexion(1),
            });
            let resultado = async {
                let mut pf = api
                    .portforward(&pod, &[puerto_pod])
                    .await
                    .context("el API server rechazó el port-forward")?;
                let mut arriba = pf
                    .take_stream(puerto_pod)
                    .ok_or_else(|| anyhow!("el pod no expone el puerto {puerto_pod}"))?;
                tokio::io::copy_bidirectional(&mut sock, &mut arriba)
                    .await
                    .context("se cortó el túnel")?;
                Ok::<_, anyhow::Error>(())
            }
            .await;
            if let Err(e) = resultado {
                if corte_normal(&e) {
                    // El cliente cortó: pasa todo el tiempo y no es un fallo.
                    tracing::debug!(%origen, error = %e, "port-forward: conexión cerrada");
                } else {
                    tracing::warn!(%origen, error = %e, "port-forward: conexión caída");
                    b.send(K8sEvent::Pf {
                        id,
                        msg: PfMsg::FalloConexion(format!("{e:#}")),
                    });
                }
            }
            b.send(K8sEvent::Pf {
                id,
                msg: PfMsg::Conexion(-1),
            });
        });
    }
}

/// ¿El error es solo un cliente que cerró la conexión?
///
/// `copy_bidirectional` devuelve error cuando cualquiera de las dos puntas se
/// va sin cerrar prolijo: un navegador que cierra la pestaña, un keep-alive que
/// vence. Tratarlo como fallo llenaba la pantalla de avisos.
fn corte_normal(e: &anyhow::Error) -> bool {
    use std::io::ErrorKind::*;
    e.chain().any(|c| {
        c.downcast_ref::<std::io::Error>().is_some_and(|io| {
            matches!(
                io.kind(),
                ConnectionReset | ConnectionAborted | BrokenPipe | UnexpectedEof | NotConnected
            )
        })
    })
}

/// Puertos bajo 1024 necesitan privilegios; se remapea a uno alto equivalente.
pub fn puerto_local_sugerido(puerto_svc: u16) -> u16 {
    if puerto_svc >= 1024 {
        return puerto_svc;
    }
    match puerto_svc {
        80 => 8080,
        443 => 8443,
        // 22 -> 8022, 3306 ya es alto, etc.
        p => 8000 + p,
    }
}

/// Dirección donde bindear según el modo de resolución elegido.
pub fn bind_de(alias: bool, servicio: &str) -> IpAddr {
    if alias && crate::hosts::soportado() {
        // Con alias en /etc/hosts cada servicio tiene su IP: pueden convivir
        // dos servicios en el mismo puerto. Solo Linux enruta todo
        // 127.0.0.0/8 sin configurar interfaces.
        IpAddr::V4(ip_para(servicio))
    } else {
        IpAddr::V4(Ipv4Addr::LOCALHOST)
    }
}

/// Host por el que se consume el forward.
///
/// `*.localhost` lo resuelve systemd-resolved sin configurar nada, pero eso es
/// de Linux: en macOS y Windows no es confiable, así que ahí se ofrece la IP.
pub fn host_de(alias: bool, servicio: &str) -> String {
    if alias && crate::hosts::soportado() {
        servicio.to_string()
    } else if cfg!(target_os = "linux") {
        format!("{servicio}.localhost")
    } else {
        Ipv4Addr::LOCALHOST.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_ip_es_estable_y_valida() {
        let a = ip_para("agent-ops");
        assert_eq!(a, ip_para("agent-ops"), "misma entrada, misma IP");
        assert_ne!(a, ip_para("billing-api"));
        for ip in ["agent-ops", "billing-api", "x", ""].map(ip_para) {
            let o = ip.octets();
            assert_eq!([o[0], o[1]], [127, 30]);
            assert!(o[2] >= 1 && o[2] <= 254, "octeto de red inválido: {ip}");
            assert!(o[3] >= 1 && o[3] <= 254, "octeto de host inválido: {ip}");
            assert!(ip.is_loopback());
        }
    }

    /// Prueba de punta a punta contra un cluster real: resuelve el Service,
    /// levanta el túnel y hace una request HTTP por él.
    ///
    /// `KUBECONFIG=... KUBO_IT_SVC=default/test-nginx cargo test --release -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "necesita un cluster: definí KUBO_IT_SVC=ns/servicio"]
    async fn tuneliza_un_service_real() {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

        let spec = std::env::var("KUBO_IT_SVC").expect("falta KUBO_IT_SVC=ns/servicio");
        let (ns, svc) = spec.split_once('/').expect("formato ns/servicio");
        let client = kube::Client::try_default().await.expect("sin cliente k8s");

        let puertos = puertos_de(client.clone(), ns, svc).await.expect("puertos");
        println!("puertos del Service: {puertos:?}");
        let puerto = puertos.first().cloned().expect("al menos un puerto");

        let (pod, puerto_pod) = elegir_pod(client.clone(), ns, svc, &puerto)
            .await
            .expect("pod detrás del Service");
        println!("pod elegido: {pod} :{puerto_pod}");

        // Puerto efímero: el test no puede chocar con nada que esté corriendo.
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind local");
        let addr = listener.local_addr().unwrap();
        drop(listener);

        let bridge = UiBridge::new(flume::unbounded().0, egui::Context::default());
        let tarea = tokio::spawn(servir(
            client, ns.to_string(), pod, puerto_pod, addr, 1, bridge,
        ));
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;

        let mut sock = tokio::net::TcpStream::connect(addr)
            .await
            .expect("conectar al túnel");
        let req = format!("GET / HTTP/1.1\r\nHost: {svc}\r\nConnection: close\r\n\r\n");
        sock.write_all(req.as_bytes()).await.expect("escribir");

        let mut resp = Vec::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            sock.read_to_end(&mut resp),
        )
        .await
        .expect("timeout leyendo del túnel")
        .expect("leer");
        tarea.abort();

        let texto = String::from_utf8_lossy(&resp);
        println!("--- respuesta ({} bytes) ---\n{}", resp.len(), &texto[..texto.len().min(300)]);
        assert!(
            texto.starts_with("HTTP/1."),
            "no volvió una respuesta HTTP por el túnel"
        );
    }

    #[test]
    fn remapea_solo_los_puertos_privilegiados() {
        assert_eq!(puerto_local_sugerido(80), 8080);
        assert_eq!(puerto_local_sugerido(443), 8443);
        assert_eq!(puerto_local_sugerido(3000), 3000);
        assert_eq!(puerto_local_sugerido(8080), 8080);
        assert!(puerto_local_sugerido(22) >= 1024);
    }
}
