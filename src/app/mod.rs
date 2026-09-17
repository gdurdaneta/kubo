//! Estado de la aplicación: clusters compartidos por contexto y N paneles
//! independientes, cada uno mirando un cluster/recurso propio.

use std::collections::{HashMap, HashSet, VecDeque};

use kube::Client;
use tokio::task::JoinHandle;

use crate::k8s::contexts::ContextInfo;
use crate::k8s::mapa::Mapa;
use crate::k8s::search::Hit;
use crate::k8s::watch::Target;
use crate::k8s::{self, ClusterInfo, EventRow, K8sEvent, UiBridge, WatchMsg};
use crate::nav::{NavCategory, NavItem, VistaLocal};
use crate::store::Store;

/// Modo solo lectura (`--solo-lectura` o `KUBO_SOLO_LECTURA=1`): la UI no
/// ofrece nada que mute el cluster ni abra una shell, y aunque algo llegue a
/// pedirlo, se frena acá. Se fija una vez al arrancar.
pub static SOLO_LECTURA: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn solo_lectura() -> bool {
    SOLO_LECTURA.load(std::sync::atomic::Ordering::Relaxed)
}

/// Tope de líneas en el visor de logs; más que esto no se lee y cuesta memoria.
const MAX_LINEAS_LOG: usize = 5_000;
pub const MAX_PANES: usize = 4;
/// Tope para conectar a un cluster antes de darlo por inalcanzable.
const TIMEOUT_CONEXION: u64 = 20;
/// Espera tras la última tecla antes de disparar la búsqueda de la paleta.
const DEBOUNCE_BUSQUEDA: f32 = 0.25;

mod acciones;
mod conexion;
mod detalle;
mod eventos;
mod forward;
mod logs_shell;
mod paleta;
mod pruebas;
mod vistas;

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum Conn {
    Conectando,
    Lista,
    Error,
}

/// Conexión a un contexto, compartida por todos los paneles que lo miran.
pub struct Cluster {
    pub conn: Conn,
    pub error: Option<String>,
    pub token: u64,
    pub client: Option<Client>,
    pub info: Option<ClusterInfo>,
    pub nav: Vec<NavCategory>,
    pub namespaces: Vec<String>,
    /// Qué permite el RBAC, por (namespace, recurso). Se llena a medida que se
    /// abren vistas y se comparte entre los paneles del mismo contexto.
    pub permisos: HashMap<String, k8s::permisos::Permisos>,
    /// Columnas de tabla por Kind custom, leídas de su CRD una sola vez.
    pub columnas_crd: HashMap<String, Vec<k8s::printer::ColumnaCrd>>,
}

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum TabDetalle {
    Resumen,
    Yaml,
    Eventos,
    Mapa,
}

pub struct Detalle {
    pub key: String,
    pub kind: String,
    pub name: String,
    pub ns: Option<String>,
    pub tab: TabDetalle,
    pub yaml: Option<String>,
    pub yaml_edit: Option<String>,
    pub yaml_token: u64,
    /// `yaml` es la copia autoritativa del API server y no el adelanto que se
    /// arma con el objeto que ya tenía el watch. Solo sobre la copia fresca se
    /// habilita editar: aplicar un PUT armado sobre datos viejos pisaría
    /// cambios ajenos.
    pub yaml_fresco: bool,
    /// Backends del Service (solo para Kind Service).
    pub backends: Vec<k8s::endpoints::Backend>,
    pub backends_token: u64,
    /// Hay una consulta de backends en vuelo: sin esto la UI decía "ninguno"
    /// mientras todavía estaba pidiéndolos.
    pub backends_pedidos: bool,
    pub eventos: Vec<EventRow>,
    pub eventos_token: u64,
    pub eventos_pedidos: bool,
    /// Solo aplica a Secrets: si está en false el YAML viene enmascarado.
    pub revelar: bool,
    /// El YAML arranca en solo lectura. Editar un manifiesto en vivo es una
    /// escritura contra el cluster: hay que pedirla, no caer en ella por
    /// tipear encima de lo que se estaba mirando.
    pub editando: bool,
    pub mapa: Option<Box<Mapa>>,
    pub mapa_token: u64,
}

pub struct VistaLogs {
    pub ns: String,
    /// Nombre del pod, o del workload cuando `pods` trae varios.
    pub pod: String,
    /// Pods cuyos logs se mezclan (vacío = solo `pod`).
    pub pods: Vec<String>,
    pub contenedores: Vec<String>,
    pub contenedor: Option<String>,
    pub lineas: VecDeque<String>,
    pub filtro: String,
    pub follow: bool,
    pub previous: bool,
    pub tail: i64,
    pub token: u64,
    pub cerrado: Option<String>,
    pub tarea: Option<JoinHandle<()>>,
}

pub struct VistaTerm {
    pub ns: String,
    pub pod: String,
    pub contenedor: Option<String>,
    pub parser: vt100::Parser,
    pub handles: k8s::exec::TermHandles,
    pub token: u64,
    pub cerrado: Option<String>,
    pub cols: u16,
    pub rows: u16,
    pub tarea: Option<JoinHandle<()>>,
}

/// Qué ocupa la franja inferior de un panel.
///
/// `VistaTerm` lleva el parser de vt100 y pesa bastante más que `VistaLogs`;
/// va en Box para que el enum no ocupe siempre el tamaño de la más grande.
pub enum Bottom {
    Logs(Box<VistaLogs>),
    Term(Box<VistaTerm>),
}

/// Un panel: una vista independiente sobre algún cluster.
pub struct Pane {
    pub id: u64,
    pub contexto: Option<String>,
    pub ns_sel: Option<String>,
    pub nav_filtro: String,
    pub nav_cerradas: HashSet<String>,
    pub favoritos: HashSet<String>,
    pub nav_visible: bool,
    pub item: Option<NavItem>,
    /// Vista local activa (port-forwards); tapa la tabla de recursos.
    pub vista_local: Option<VistaLocal>,
    pub store: Option<Store>,
    pub busqueda: String,
    /// Fila resaltada por teclado. Índice sobre la vista filtrada y ordenada,
    /// así que se re-acota en cada dibujo.
    pub cursor: Option<usize>,
    /// Filas marcadas con la casilla (claves `ns/nombre`) para actuar en lote.
    pub seleccion: HashSet<String>,
    pub watch_token: u64,
    pub watch_tarea: Option<JoinHandle<()>>,
    /// Backends por Service (`ns/servicio -> conteo`), solo en la vista Services.
    pub endpoints: HashMap<String, k8s::endpoints::Conteo>,
    pub endpoints_token: u64,
    pub endpoints_tarea: Option<JoinHandle<()>>,
    /// Última muestra de CPU/memoria por objeto, en Pods y Nodes.
    pub metricas: HashMap<String, k8s::metricas::Uso>,
    /// Muestras anteriores, para el sparkline del detalle.
    pub historial: k8s::metricas::Historial,
    pub metricas_token: u64,
    pub metricas_tarea: Option<JoinHandle<()>>,
    /// Ámbito del watch en curso, para no relistar si no cambió nada.
    pub watch_target: Option<Target>,
    pub detalle: Option<Detalle>,
    pub detalle_tareas: Vec<JoinHandle<()>>,
    pub bottom: Option<Bottom>,
    /// Recurso guardado de la sesión anterior, a seleccionar apenas el cluster
    /// conecte y se sepa qué sirve.
    pub recurso_pendiente: Option<String>,
    /// Detalle a abrir apenas el watch de la vista nueva termine de cargar
    /// (navegación "ir al recurso" desde el mapa).
    pub pendiente_detalle: Option<String>,
}

impl Pane {
    fn nueva(id: u64, contexto: Option<String>) -> Self {
        Self {
            id,
            contexto,
            ns_sel: None,
            nav_filtro: String::new(),
            nav_cerradas: HashSet::new(),
            favoritos: HashSet::new(),
            nav_visible: true,
            item: None,
            vista_local: None,
            store: None,
            busqueda: String::new(),
            cursor: None,
            seleccion: HashSet::new(),
            watch_token: 0,
            watch_tarea: None,
            endpoints: HashMap::new(),
            endpoints_token: 0,
            endpoints_tarea: None,
            metricas: HashMap::new(),
            historial: k8s::metricas::Historial::default(),
            metricas_token: 0,
            metricas_tarea: None,
            watch_target: None,
            detalle: None,
            detalle_tareas: Vec::new(),
            bottom: None,
            recurso_pendiente: None,
            pendiente_detalle: None,
        }
    }

    fn limpiar_vista(&mut self) {
        if let Some(t) = self.watch_tarea.take() {
            t.abort();
        }
        self.parar_endpoints();
        self.parar_metricas();
        for t in self.detalle_tareas.drain(..) {
            t.abort();
        }
        self.cerrar_bottom();
        self.item = None;
        self.vista_local = None;
        self.store = None;
        self.watch_target = None;
        self.detalle = None;
        self.busqueda.clear();
        self.cursor = None;
        self.seleccion.clear();
    }

    fn parar_endpoints(&mut self) {
        if let Some(t) = self.endpoints_tarea.take() {
            t.abort();
        }
        self.endpoints.clear();
        self.endpoints_token = 0;
    }

    fn parar_metricas(&mut self) {
        if let Some(t) = self.metricas_tarea.take() {
            t.abort();
        }
        self.metricas.clear();
        self.historial = k8s::metricas::Historial::default();
        self.metricas_token = 0;
    }

    pub fn cerrar_bottom(&mut self) {
        match self.bottom.take() {
            Some(Bottom::Logs(mut v)) => {
                if let Some(t) = v.tarea.take() {
                    t.abort();
                }
            }
            Some(Bottom::Term(mut v)) => {
                if let Some(t) = v.tarea.take() {
                    t.abort();
                }
            }
            None => {}
        }
    }
}

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum EstadoPf {
    Levantando,
    Activo,
    Caido,
}

/// Un port-forward activo hacia un Service.
pub struct Forward {
    pub id: u64,
    /// Túnel directo a un pod (sin pasar por un Service).
    pub pod: bool,
    pub contexto: String,
    pub ns: String,
    pub servicio: String,
    pub puerto_svc: u16,
    pub puerto_local: u16,
    pub bind: std::net::IpAddr,
    /// Host por el que se consume: `agent-ops` con alias, `agent-ops.localhost` sin él.
    pub host: String,
    pub alias: bool,
    pub estado: EstadoPf,
    pub conexiones: i64,
    pub error: Option<String>,
    pub tarea: Option<JoinHandle<()>>,
}

impl Forward {
    pub fn url(&self) -> String {
        let esquema = if self.puerto_svc == 443 {
            "https"
        } else {
            "http"
        };
        format!("{esquema}://{}:{}", self.host, self.puerto_local)
    }

    /// El puerto local no coincide con el del servicio (privilegiado u ocupado).
    pub fn remapeado(&self) -> bool {
        self.puerto_local != self.puerto_svc
    }
}

/// Diálogo de configuración de un port-forward, antes de levantarlo.
pub struct DialogoPf {
    /// El destino es un pod concreto (`servicio` lleva su nombre), no un Service.
    pub pod: bool,
    /// Panel desde el que se pidió; ahí se muestra la lista al levantarlo.
    pub pane: u64,
    pub contexto: String,
    pub ns: String,
    pub servicio: String,
    pub puertos: Vec<crate::k8s::portforward::PuertoSvc>,
    pub sel: usize,
    pub puerto_local: String,
    pub alias: bool,
    pub cargando: bool,
}

/// Acción peligrosa pendiente de confirmación.
pub struct Confirmacion {
    pub pane: u64,
    pub verbo: Verbo,
    pub kind: String,
    pub ns: Option<String>,
    pub name: String,
    /// Diff unificado entre el YAML del API server y el editado (solo aplicar).
    pub diff: Option<String>,
    /// Lo que el usuario tecleó para confirmar en producción.
    pub tecleado: String,
    /// Más objetivos del mismo kind (lote): `(namespace, nombre)`.
    pub extra: Vec<(Option<String>, String)>,
}

impl Confirmacion {
    pub fn simple(pane: u64, verbo: Verbo, kind: String, ns: Option<String>, name: String) -> Self {
        Self {
            pane,
            verbo,
            kind,
            ns,
            name,
            diff: None,
            tecleado: String::new(),
            extra: Vec::new(),
        }
    }

    /// Cuántos recursos toca en total.
    pub fn cantidad(&self) -> usize {
        1 + self.extra.len()
    }
}

#[derive(PartialEq, Eq)]
pub enum Verbo {
    Borrar,
    Reiniciar,
    /// Lleva el valor editable del modal.
    Escalar(i64),
    /// YAML validado por la UI y pendiente de confirmación.
    AplicarYaml(String),
}

/// Paleta de comandos (Ctrl+K): salta a un Kind o a un recurso por nombre.
pub struct Palette {
    /// Panel sobre el que actúa.
    pub pane: u64,
    pub query: String,
    pub hits: Vec<Hit>,
    pub buscando: bool,
    pub parcial: bool,
    pub token: u64,
    /// Índice seleccionado sobre la lista combinada (kinds + hits).
    pub sel: usize,
    /// Query ya despachada; sirve para el debounce.
    pub query_buscada: String,
    /// Segundos desde el último cambio del texto.
    pub desde_cambio: f32,
    /// Búsqueda en curso; se aborta al cambiar la query o cerrar la paleta.
    pub tarea: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for Palette {
    fn drop(&mut self) {
        if let Some(tarea) = self.tarea.take() {
            tarea.abort();
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PickerModo {
    Contexto,
    Namespace,
}

/// Selector rápido de contexto/namespace (Ctrl+P / Ctrl+N).
pub struct Picker {
    pub modo: PickerModo,
    pub pane: u64,
    pub query: String,
    pub sel: usize,
}

pub struct App {
    pub rt: tokio::runtime::Runtime,
    pub rx: flume::Receiver<K8sEvent>,
    pub bridge: UiBridge,

    pub contextos: Vec<ContextInfo>,
    pub clusters: HashMap<String, Cluster>,
    pub panes: Vec<Pane>,
    pub confirm: Option<Confirmacion>,
    pub palette: Option<Palette>,
    pub picker: Option<Picker>,
    pub ver_atajos: bool,
    /// Port-forwards activos. Van en App y no en un panel: ocupan puertos de la
    /// máquina, así que sobreviven a cerrar el panel que los abrió.
    pub forwards: Vec<Forward>,
    pub dialogo_pf: Option<DialogoPf>,

    pub toasts: Vec<(String, bool, f64)>,
    /// Último panel con el que se interactuó: destino de Ctrl+K y de lo que se
    /// teclea sin foco.
    pub pane_activo: u64,
    siguiente_token: u64,
    siguiente_pane: u64,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        crate::theme::aplicar(&cc.egui_ctx);

        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("no se pudo crear el runtime de tokio");

        let (tx, rx) = flume::unbounded();
        let bridge = UiBridge::new(tx, cc.egui_ctx.clone());

        let mut app = Self {
            rt,
            rx,
            bridge,
            contextos: Vec::new(),
            clusters: HashMap::new(),
            panes: Vec::new(),
            confirm: None,
            palette: None,
            picker: None,
            ver_atajos: false,
            forwards: Vec::new(),
            dialogo_pf: None,
            toasts: Vec::new(),
            pane_activo: 0,
            siguiente_token: 1,
            siguiente_pane: 1,
        };

        let (contextos, actual) = match k8s::contexts::load() {
            Ok(v) => v,
            Err(e) => {
                app.toast(format!("kubeconfig: {e}"), true);
                (Vec::new(), None)
            }
        };
        app.contextos = contextos;

        // Se restaura lo último que se estuvo mirando; el `current-context` del
        // kubeconfig es solo el fallback de la primera vez. Los contextos que
        // ya no existen se descartan: el kubeconfig pudo cambiar.
        let guardado = crate::layout::cargar();
        let mut panes: Vec<crate::layout::PaneGuardado> = guardado
            .panes
            .into_iter()
            .filter(|p| {
                p.contexto
                    .as_ref()
                    .is_none_or(|c| app.contextos.iter().any(|x| &x.name == c))
            })
            .take(MAX_PANES)
            .collect();
        if panes.is_empty() {
            panes.push(crate::layout::PaneGuardado {
                contexto: actual.clone(),
                ns: actual.as_deref().and_then(k8s::session::default_namespace),
                recurso: None,
                favoritos: Vec::new(),
            });
        }

        for g in panes {
            let id = app.pane_id();
            let mut pane = Pane::nueva(id, g.contexto.clone());
            pane.ns_sel = g.ns;
            pane.recurso_pendiente = g.recurso;
            pane.favoritos = g.favoritos.into_iter().collect();
            app.panes.push(pane);
            if let Some(ctx) = g.contexto {
                app.asegurar_cluster(&ctx);
            }
        }
        app.pane_activo = app.panes.first().map(|p| p.id).unwrap_or(0);
        // Harness: KUBO_TEST_PANES=n abre n-1 paneles extra al arrancar.
        if let Ok(n) = std::env::var("KUBO_TEST_PANES") {
            for _ in 1..n.parse::<usize>().unwrap_or(1) {
                app.abrir_pane();
            }
        }
        app
    }

    fn token(&mut self) -> u64 {
        self.siguiente_token += 1;
        self.siguiente_token
    }

    fn pane_id(&mut self) -> u64 {
        self.siguiente_pane += 1;
        self.siguiente_pane
    }

    pub fn toast(&mut self, texto: impl Into<String>, error: bool) {
        self.toasts.push((texto.into(), error, 5.0));
    }

    pub fn pane(&mut self, id: u64) -> Option<&mut Pane> {
        self.panes.iter_mut().find(|p| p.id == id)
    }

    pub fn cluster_de(&self, pane: &Pane) -> Option<&Cluster> {
        pane.contexto.as_ref().and_then(|c| self.clusters.get(c))
    }

    // ------------------------------------------------------------- paneles

    pub fn abrir_pane(&mut self) {
        if self.panes.len() >= MAX_PANES {
            return;
        }
        // El panel nuevo arranca en el mismo contexto que el último activo.
        let ctx = self.panes.last().and_then(|p| p.contexto.clone());
        let id = self.pane_id();
        let mut pane = Pane::nueva(id, ctx.clone());
        if let Some(c) = &ctx {
            pane.ns_sel = k8s::session::default_namespace(c);
            self.asegurar_cluster(c);
        }
        self.panes.push(pane);
        // Si el cluster ya está listo, abrir Pods de una.
        self.autoseleccionar(id);
        self.guardar_layout();
    }

    pub fn cerrar_pane(&mut self, id: u64) {
        if self.panes.len() <= 1 {
            return;
        }
        if let Some(pos) = self.panes.iter().position(|p| p.id == id) {
            let mut pane = self.panes.remove(pos);
            pane.limpiar_vista();
        }
        self.soltar_clusters_sin_uso();
        self.guardar_layout();
    }
}

impl eframe::App for App {
    /// Todo lo que no pinta: drenar el canal, envejecer los toasts y pedir el
    /// próximo repintado.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drenar_eventos();

        // La columna "Edad" avanza sola: un repintado por segundo alcanza.
        ctx.request_repaint_after(std::time::Duration::from_secs(1));
        for pane in &mut self.panes {
            if let Some(s) = pane.store.as_mut() {
                s.refrescar();
            }
        }

        let dt = ctx.input(|i| i.stable_dt) as f64;
        self.toasts.retain_mut(|(_, _, t)| {
            *t -= dt;
            *t > 0.0
        });
        self.quizas_buscar(dt as f32, ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        crate::ui::dibujar(self, ui);
    }
}
