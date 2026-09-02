//! Capa de acceso a Kubernetes. Todo corre sobre el runtime de tokio y se
//! comunica con la UI por canal; el hilo de render nunca bloquea.

pub mod actions;
pub mod cache;
pub mod contexts;
pub mod detail;
pub mod endpoints;
pub mod exec;
pub mod logs;
pub mod mapa;
pub mod metricas;
pub mod permisos;
pub mod portforward;
pub mod search;
pub mod session;
pub mod watch;

use kube::api::DynamicObject;
use kube::discovery::ApiResource;

/// Un recurso servido por el cluster, tal como lo reporta discovery.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Discovered {
    pub ar: ApiResource,
    pub namespaced: bool,
    pub verbs: Vec<String>,
}

impl Discovered {
    /// Identificador estable `group/version/Kind` para indexar en la UI.
    pub fn key(&self) -> String {
        if self.ar.group.is_empty() {
            format!("{}/{}", self.ar.version, self.ar.kind)
        } else {
            format!("{}/{}/{}", self.ar.group, self.ar.version, self.ar.kind)
        }
    }

    pub fn watchable(&self) -> bool {
        self.verbs.iter().any(|v| v == "watch") && self.verbs.iter().any(|v| v == "list")
    }
}

/// Datos de una conexión establecida a un cluster.
#[derive(Clone, Debug)]
pub struct ClusterInfo {
    pub server: String,
    pub version: String,
    pub resources: Vec<Discovered>,
}

/// Evento del watcher, ya normalizado para la UI.
pub enum WatchMsg {
    /// Empieza una resincronización: la UI debe acumular en un buffer.
    Init,
    /// Tanda de objetos del listado inicial. Va en lotes y no de a uno porque
    /// cada mensaje despierta el hilo de render: con 5.000 pods, mandarlos
    /// sueltos son 5.000 repintados durante la carga.
    InitBatch(Vec<DynamicObject>),
    /// Fin del listado inicial: se reemplaza el contenido de la tabla.
    InitDone,
    Apply(Box<DynamicObject>),
    Delete(Box<DynamicObject>),
    Error(String),
}

/// Fila de la pestaña de eventos del panel de detalle.
#[derive(Clone, Debug)]
pub struct EventRow {
    pub type_: String,
    pub reason: String,
    pub message: String,
    pub count: i64,
    pub last: Option<k8s_openapi::jiff::Timestamp>,
}

/// Todo lo que la capa async le manda al hilo de UI.
pub enum K8sEvent {
    Connected {
        token: u64,
        info: Box<ClusterInfo>,
        /// El cliente viaja con el evento: reconstruirlo en el hilo de UI
        /// bloquearía el render (el exec plugin de EKS tarda segundos).
        client: kube::Client,
    },
    ConnectFailed {
        token: u64,
        error: String,
    },
    /// Versión del API server, que llega después del `Connected` cuando la
    /// conexión arrancó con discovery cacheado.
    Version {
        token: u64,
        version: String,
    },
    /// Discovery fresco: reemplaza al cacheado y reconstruye la navegación.
    Resources {
        token: u64,
        resources: Vec<Discovered>,
    },
    Namespaces {
        token: u64,
        list: Vec<String>,
    },
    Watch {
        token: u64,
        msg: WatchMsg,
    },
    /// YAML ya serializado del objeto pedido (se recarga desde el API server).
    Yaml {
        token: u64,
        text: String,
    },
    ObjectEvents {
        token: u64,
        items: Vec<EventRow>,
    },
    LogLine {
        token: u64,
        line: String,
    },
    LogClosed {
        token: u64,
        error: Option<String>,
    },
    /// Bytes crudos que salieron del PTY del pod.
    TermData {
        token: u64,
        bytes: Vec<u8>,
    },
    TermClosed {
        token: u64,
        error: Option<String>,
    },
    /// Datos del mapa (de un Service o de un workload).
    Mapa {
        token: u64,
        data: Box<mapa::Mapa>,
    },
    Search {
        token: u64,
        hits: Vec<search::Hit>,
    },
    Toast {
        text: String,
        error: bool,
    },
    /// Backends por Service: `ns/servicio -> conteo`.
    Endpoints {
        token: u64,
        mapa: std::collections::HashMap<String, endpoints::Conteo>,
    },
    /// Muestra de CPU/memoria de los objetos de la vista.
    Metricas {
        token: u64,
        mapa: std::collections::HashMap<String, metricas::Uso>,
    },
    /// Qué verbos permite el RBAC sobre el recurso de la vista.
    Permisos {
        clave: String,
        permisos: permisos::Permisos,
    },
    /// Backends concretos de un Service, para el panel de detalle.
    Backends {
        token: u64,
        items: Vec<endpoints::Backend>,
    },
    /// Puertos que publica un Service, para el diálogo de port-forward.
    PuertosSvc {
        servicio: String,
        puertos: Vec<portforward::PuertoSvc>,
    },
    /// Novedad de un port-forward activo.
    Pf {
        id: u64,
        msg: PfMsg,
    },
    /// Resultado de tocar /etc/hosts (corre bloqueando por el diálogo de polkit).
    Alias {
        id: u64,
        error: Option<String>,
    },
}

pub enum PfMsg {
    /// El listener local ya está arriba.
    Escuchando,
    /// Delta de conexiones vivas: +1 al entrar, -1 al cerrarse.
    Conexion(i64),
    /// El forward entero se murió (no se pudo bindear, se cayó el accept).
    Fatal(String),
    /// Falló una conexión suelta. Se anota en la fila, no se avisa con toast:
    /// que un cliente corte la conexión es lo normal, no un problema.
    FalloConexion(String),
}

/// Canal hacia la UI; despierta el repintado en cuanto llega algo.
#[derive(Clone)]
pub struct UiBridge {
    tx: flume::Sender<K8sEvent>,
    ctx: egui::Context,
}

impl UiBridge {
    pub fn new(tx: flume::Sender<K8sEvent>, ctx: egui::Context) -> Self {
        Self { tx, ctx }
    }

    pub fn send(&self, ev: K8sEvent) {
        // Si el receptor murió la app está cerrando: no es un error.
        if self.tx.send(ev).is_ok() {
            self.ctx.request_repaint();
        }
    }

    pub fn toast(&self, text: impl Into<String>, error: bool) {
        self.send(K8sEvent::Toast {
            text: text.into(),
            error,
        });
    }
}
