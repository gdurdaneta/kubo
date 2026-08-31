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

/// Tope de líneas en el visor de logs; más que esto no se lee y cuesta memoria.
const MAX_LINEAS_LOG: usize = 5_000;
pub const MAX_PANES: usize = 4;
/// Espera tras la última tecla antes de disparar la búsqueda de la paleta.
const DEBOUNCE_BUSQUEDA: f32 = 0.25;

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
    pub pod: String,
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
pub enum Bottom {
    Logs(VistaLogs),
    Term(VistaTerm),
}

/// Un panel: una vista independiente sobre algún cluster.
pub struct Pane {
    pub id: u64,
    pub contexto: Option<String>,
    pub ns_sel: Option<String>,
    pub nav_filtro: String,
    pub nav_cerradas: HashSet<String>,
    pub nav_visible: bool,
    pub item: Option<NavItem>,
    /// Vista local activa (port-forwards); tapa la tabla de recursos.
    pub vista_local: Option<VistaLocal>,
    pub store: Option<Store>,
    pub busqueda: String,
    pub watch_token: u64,
    pub watch_tarea: Option<JoinHandle<()>>,
    /// Backends por Service (`ns/servicio -> conteo`), solo en la vista Services.
    pub endpoints: HashMap<String, k8s::endpoints::Conteo>,
    pub endpoints_token: u64,
    pub endpoints_tarea: Option<JoinHandle<()>>,
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
            nav_visible: true,
            item: None,
            vista_local: None,
            store: None,
            busqueda: String::new(),
            watch_token: 0,
            watch_tarea: None,
            endpoints: HashMap::new(),
            endpoints_token: 0,
            endpoints_tarea: None,
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
    }

    fn parar_endpoints(&mut self) {
        if let Some(t) = self.endpoints_tarea.take() {
            t.abort();
        }
        self.endpoints.clear();
        self.endpoints_token = 0;
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
        let esquema = if self.puerto_svc == 443 { "https" } else { "http" };
        format!("{esquema}://{}:{}", self.host, self.puerto_local)
    }

    /// El puerto local no coincide con el del servicio (privilegiado u ocupado).
    pub fn remapeado(&self) -> bool {
        self.puerto_local != self.puerto_svc
    }
}

/// Diálogo de configuración de un port-forward, antes de levantarlo.
pub struct DialogoPf {
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
}

#[derive(PartialEq, Eq)]
pub enum Verbo {
    Borrar,
    Reiniciar,
    /// Lleva el valor editable del modal.
    Escalar(i64),
}

/// Paleta de comandos (Ctrl+K): salta a un Kind o a un recurso por nombre.
pub struct Palette {
    /// Panel sobre el que actúa.
    pub pane: u64,
    pub query: String,
    pub hits: Vec<Hit>,
    pub buscando: bool,
    pub token: u64,
    /// Índice seleccionado sobre la lista combinada (kinds + hits).
    pub sel: usize,
    /// Query ya despachada; sirve para el debounce.
    pub query_buscada: String,
    /// Segundos desde el último cambio del texto.
    pub desde_cambio: f32,
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
            });
        }

        for g in panes {
            let id = app.pane_id();
            let mut pane = Pane::nueva(id, g.contexto.clone());
            pane.ns_sel = g.ns;
            pane.recurso_pendiente = g.recurso;
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
        self.guardar_layout();
    }

    // ------------------------------------------------------------ conexión

    /// Lanza la conexión si nadie la pidió todavía.
    fn asegurar_cluster(&mut self, contexto: &str) {
        if self.clusters.contains_key(contexto) {
            return;
        }
        let token = self.token();
        self.clusters.insert(
            contexto.to_string(),
            Cluster {
                conn: Conn::Conectando,
                error: None,
                token,
                client: None,
                info: None,
                nav: Vec::new(),
                namespaces: Vec::new(),
            },
        );
        let bridge = self.bridge.clone();
        let ctx_name = contexto.to_string();
        self.rt.spawn(async move {
            let (client, info, desde_cache) = match k8s::session::connect(&ctx_name).await {
                Ok(v) => v,
                Err(e) => {
                    bridge.send(K8sEvent::ConnectFailed {
                        token,
                        error: format!("{e:#}"),
                    });
                    return;
                }
            };
            // La UI ya puede pintar la navegación; lo demás llega solo.
            let server = info.server.clone();
            let n_recursos = info.resources.len();
            bridge.send(K8sEvent::Connected {
                token,
                info: Box::new(info),
                client: client.clone(),
            });

            let (c1, c2, b1, b2) = (client.clone(), client.clone(), bridge.clone(), bridge.clone());
            tokio::join!(
                async move {
                    let version = k8s::session::version(c1).await;
                    b1.send(K8sEvent::Version { token, version });
                },
                async move {
                    let list = k8s::session::namespaces(c2).await;
                    b2.send(K8sEvent::Namespaces { token, list });
                },
                async move {
                    // Con caché la lista mostrada puede estar vieja (un CRD
                    // nuevo, un operador instalado); se rehace por detrás.
                    if !desde_cache {
                        return;
                    }
                    if let Some(resources) =
                        k8s::session::refrescar_discovery(client, server, n_recursos).await
                    {
                        bridge.send(K8sEvent::Resources { token, resources });
                    }
                },
            );
        });
    }

    /// Reintenta una conexión fallida.
    pub fn reconectar(&mut self, contexto: &str) {
        self.clusters.remove(contexto);
        self.asegurar_cluster(contexto);
    }

    pub fn cambiar_contexto(&mut self, pane_id: u64, contexto: String) {
        let ns = k8s::session::default_namespace(&contexto);
        if let Some(pane) = self.pane(pane_id) {
            pane.limpiar_vista();
            pane.contexto = Some(contexto.clone());
            pane.ns_sel = ns;
        }
        self.asegurar_cluster(&contexto);
        self.autoseleccionar(pane_id);
        self.guardar_layout();
    }

    /// Abre Pods si el panel no tiene nada seleccionado y su cluster ya está.
    fn autoseleccionar(&mut self, pane_id: u64) {
        let Some(pane) = self.panes.iter().find(|p| p.id == pane_id) else {
            return;
        };
        if pane.item.is_some() {
            return;
        }
        let Some(cluster) = self.cluster_de(pane) else { return };
        if cluster.conn != Conn::Lista {
            return;
        }
        // Lo que estaba abierto la sesión pasada gana; Pods es el default.
        let deseado = pane.recurso_pendiente.clone();
        let elegido = cluster
            .nav
            .iter()
            .flat_map(|c| c.items.iter())
            .find(|i| match &deseado {
                Some(k) => i.res.key() == *k,
                None => i.res.ar.kind == "Pod",
            })
            .or_else(|| {
                // El recurso guardado ya no está (se desinstaló un operador).
                cluster
                    .nav
                    .iter()
                    .flat_map(|c| c.items.iter())
                    .find(|i| i.res.ar.kind == "Pod")
            })
            .cloned();
        if let Some(item) = elegido {
            if let Some(p) = self.pane(pane_id) {
                p.recurso_pendiente = None;
            }
            self.seleccionar(pane_id, item);
        }
    }

    // -------------------------------------------------------------- vistas

    pub fn seleccionar(&mut self, pane_id: u64, item: NavItem) {
        let token = self.token();
        let Some(pane) = self.panes.iter_mut().find(|p| p.id == pane_id) else {
            return;
        };
        let Some(client) = pane
            .contexto
            .as_ref()
            .and_then(|c| self.clusters.get(c))
            .and_then(|c| c.client.clone())
        else {
            return;
        };

        let target = match (&pane.ns_sel, item.res.namespaced) {
            (Some(ns), true) => Target::Namespace(ns.clone()),
            _ => Target::AllNamespaces,
        };

        // Volver a pedir la misma vista (pasa al navegar desde el mapa o la
        // paleta a un recurso del kind que ya se está mirando) costaría un
        // listado completo contra el API server para llegar a lo mismo.
        let misma_vista = pane.item.as_ref().map(|i| i.res.key()) == Some(item.res.key())
            && pane.watch_target.as_ref() == Some(&target)
            && pane.watch_tarea.as_ref().is_some_and(|t| !t.is_finished())
            && pane.store.is_some();
        pane.vista_local = None;
        if misma_vista {
            return;
        }

        if let Some(t) = pane.watch_tarea.take() {
            t.abort();
        }
        pane.detalle = None;
        pane.cerrar_bottom();
        pane.busqueda.clear();

        let mostrar_ns = item.res.namespaced && pane.ns_sel.is_none();
        let es_service = item.res.ar.kind == "Service" && item.res.ar.group.is_empty();
        pane.store = Some(Store::new(item.res.ar.kind.clone(), mostrar_ns));
        pane.watch_token = token;

        pane.parar_endpoints();
        pane.watch_target = Some(target.clone());
        let ar = item.res.ar.clone();
        let namespaced = item.res.namespaced;
        let bridge = self.bridge.clone();
        pane.watch_tarea = Some(self.rt.spawn(async move {
            k8s::watch::run(client, ar, namespaced, target, token, bridge).await;
        }));
        pane.item = Some(item);
        if es_service {
            self.seguir_endpoints(pane_id);
        }
        self.guardar_layout();
    }

    /// Qué API de endpoints sirve este cluster: EndpointSlice es lo actual,
    /// Endpoints quedó deprecado pero es lo único que hay en clusters viejos.
    fn ar_endpoints(&self, pane_id: u64) -> Option<kube::discovery::ApiResource> {
        let pane = self.panes.iter().find(|p| p.id == pane_id)?;
        let cluster = self.cluster_de(pane)?;
        let sirve = |k: &str| {
            cluster
                .info
                .as_ref()
                .is_some_and(|i| i.resources.iter().any(|r| r.ar.kind == k))
        };
        if sirve("EndpointSlice") {
            Some(k8s::endpoints::ar_endpointslice())
        } else if sirve("Endpoints") {
            Some(k8s::endpoints::ar_endpoints())
        } else {
            None
        }
    }

    /// Watch auxiliar de endpoints, para la columna de backends de Services.
    fn seguir_endpoints(&mut self, pane_id: u64) {
        let token = self.token();
        let Some(client) = self.client_del_pane(pane_id) else { return };
        // EndpointSlice es lo actual; Endpoints quedó deprecado pero es lo único
        // que hay en clusters viejos.
        let ar = {
            let Some(pane) = self.panes.iter().find(|p| p.id == pane_id) else {
                return;
            };
            let Some(cluster) = self.cluster_de(pane) else { return };
            let sirve = |k: &str| {
                cluster
                    .info
                    .as_ref()
                    .is_some_and(|i| i.resources.iter().any(|r| r.ar.kind == k && r.watchable()))
            };
            if sirve("EndpointSlice") {
                k8s::endpoints::ar_endpointslice()
            } else if sirve("Endpoints") {
                k8s::endpoints::ar_endpoints()
            } else {
                return;
            }
        };
        let bridge = self.bridge.clone();
        let rt = &self.rt;
        let Some(pane) = self.panes.iter_mut().find(|p| p.id == pane_id) else {
            return;
        };
        let Some(target) = pane.watch_target.clone() else { return };
        pane.endpoints_token = token;
        pane.endpoints_tarea = Some(rt.spawn(async move {
            k8s::endpoints::seguir(client, ar, target, token, bridge).await;
        }));
    }

    pub fn cambiar_namespace(&mut self, pane_id: u64, ns: Option<String>) {
        if let Some(pane) = self.pane(pane_id) {
            pane.ns_sel = ns;
            if let Some(item) = pane.item.clone() {
                self.seleccionar(pane_id, item);
            }
        }
        // `seleccionar` no corre si el panel todavía no tenía recurso abierto.
        self.guardar_layout();
    }

    pub fn refrescar(&mut self, pane_id: u64) {
        if let Some(pane) = self.pane(pane_id) {
            if let Some(item) = pane.item.clone() {
                self.seleccionar(pane_id, item);
            }
        }
    }

    /// Navega a otro recurso: selecciona su Kind en el panel y deja marcado
    /// el detalle para abrirlo cuando la tabla cargue.
    pub fn ir_a(&mut self, pane_id: u64, kind: &str, ns: Option<String>, name: &str) {
        let item = self
            .panes
            .iter()
            .find(|p| p.id == pane_id)
            .and_then(|p| self.cluster_de(p))
            .and_then(|c| {
                c.nav
                    .iter()
                    .flat_map(|cat| cat.items.iter())
                    .find(|i| i.res.ar.kind == kind)
                    .cloned()
            });
        let Some(item) = item else {
            self.toast(format!("no hay vista para {kind} en este cluster"), true);
            return;
        };
        self.seleccionar(pane_id, item.clone());
        if name.is_empty() {
            // Solo cambiar de vista, sin abrir ningún detalle.
            return;
        }
        // Si el recurso está en otro namespace que el filtrado, se pasa a
        // "todos" para que aparezca en la tabla.
        if item.res.namespaced {
            let distinto = self
                .panes
                .iter()
                .find(|p| p.id == pane_id)
                .map(|p| p.ns_sel != ns)
                .unwrap_or(false);
            if distinto {
                if let Some(pane) = self.pane(pane_id) {
                    pane.ns_sel = ns.clone();
                }
                self.seleccionar(pane_id, item.clone());
            }
        }
        let key = match (&ns, item.res.namespaced) {
            (Some(ns), true) => format!("{ns}/{name}"),
            _ => name.to_string(),
        };
        // Si la vista ya estaba cargada (no hubo relistado) el objeto está a
        // mano: abrir el detalle ya, sin esperar un InitDone que no va a venir.
        let ya_esta = self
            .panes
            .iter()
            .find(|p| p.id == pane_id)
            .and_then(|p| p.store.as_ref())
            .is_some_and(|s| s.objeto(&key).is_some());
        if ya_esta {
            self.abrir_detalle(pane_id, &key);
        } else if let Some(pane) = self.pane(pane_id) {
            pane.pendiente_detalle = Some(key);
        }
    }

    // ------------------------------------------------- port-forward

    /// Abre el diálogo de port-forward leyendo los puertos del Service.
    pub fn pedir_forward(&mut self, pane_id: u64, key: &str) {
        let Some(pane) = self.panes.iter().find(|p| p.id == pane_id) else {
            return;
        };
        let (Some(contexto), Some(client)) = (pane.contexto.clone(), self.client_del_pane(pane_id))
        else {
            return;
        };
        let (ns, servicio) = match key.split_once('/') {
            Some((ns, n)) => (ns.to_string(), n.to_string()),
            None => {
                self.toast("el Service tiene que estar en un namespace", true);
                return;
            }
        };

        self.dialogo_pf = Some(DialogoPf {
            pane: pane_id,
            contexto,
            ns: ns.clone(),
            servicio: servicio.clone(),
            puertos: Vec::new(),
            sel: 0,
            puerto_local: String::new(),
            alias: false,
            cargando: true,
        });

        let bridge = self.bridge.clone();
        self.rt.spawn(async move {
            match k8s::portforward::puertos_de(client, &ns, &servicio).await {
                Ok(ps) => bridge.send(K8sEvent::PuertosSvc {
                    servicio,
                    puertos: ps,
                }),
                Err(e) => bridge.toast(format!("{e:#}"), true),
            }
        });
    }

    /// Levanta el forward configurado en el diálogo.
    pub fn abrir_forward(&mut self) {
        let Some(d) = self.dialogo_pf.take() else { return };
        let Some(puerto) = d.puertos.get(d.sel).cloned() else { return };
        let Some(client) = self
            .clusters
            .get(&d.contexto)
            .and_then(|c| c.client.clone())
        else {
            return;
        };
        let puerto_local: u16 = match d.puerto_local.trim().parse() {
            Ok(p) if p > 0 => p,
            _ => {
                self.toast("puerto local inválido", true);
                return;
            }
        };
        if d.alias && !crate::hosts::nombre_valido(&d.servicio) {
            self.toast(
                format!("'{}' no sirve como alias en /etc/hosts", d.servicio),
                true,
            );
            return;
        }
        if self
            .forwards
            .iter()
            .any(|f| f.bind == k8s::portforward::bind_de(d.alias, &d.servicio) && f.puerto_local == puerto_local)
        {
            self.toast("ya hay un forward escuchando en esa dirección", true);
            return;
        }

        let id = self.token();
        let bind = k8s::portforward::bind_de(d.alias, &d.servicio);
        let host = k8s::portforward::host_de(d.alias, &d.servicio);
        self.forwards.push(Forward {
            id,
            contexto: d.contexto.clone(),
            ns: d.ns.clone(),
            servicio: d.servicio.clone(),
            puerto_svc: puerto.puerto,
            puerto_local,
            bind,
            host,
            alias: d.alias,
            estado: EstadoPf::Levantando,
            conexiones: 0,
            error: None,
            tarea: None,
        });
        // Mostrar la lista apenas se levanta, para no dejarla escondida.
        if let Some(p) = self.pane(d.pane) {
            p.vista_local = Some(VistaLocal::PortForwards);
        }

        // El alias va primero: si el usuario cancela el diálogo de polkit, no
        // tiene sentido dejar el listener arriba con un nombre que no resuelve.
        if d.alias {
            self.sincronizar_alias(id);
        }

        let (ns, servicio, bridge) = (d.ns.clone(), d.servicio.clone(), self.bridge.clone());
        let addr = std::net::SocketAddr::new(bind, puerto_local);
        let tarea = self.rt.spawn(async move {
            let (pod, puerto_pod) =
                match k8s::portforward::elegir_pod(client.clone(), &ns, &servicio, &puerto).await {
                    Ok(v) => v,
                    Err(e) => {
                        bridge.send(K8sEvent::Pf {
                            id,
                            msg: k8s::PfMsg::Fatal(format!("{e:#}")),
                        });
                        return;
                    }
                };
            k8s::portforward::servir(client, ns, pod, puerto_pod, addr, id, bridge).await;
        });
        if let Some(f) = self.forwards.iter_mut().find(|f| f.id == id) {
            f.tarea = Some(tarea);
        }
    }

    /// Muestra los port-forward en el panel, en vez de la tabla de recursos.
    pub fn ver_vista_local(&mut self, pane_id: u64, v: VistaLocal) {
        if let Some(p) = self.pane(pane_id) {
            p.vista_local = Some(v);
        }
    }

    pub fn cerrar_forward(&mut self, id: u64) {
        let Some(i) = self.forwards.iter().position(|f| f.id == id) else {
            return;
        };
        let mut f = self.forwards.remove(i);
        if let Some(t) = f.tarea.take() {
            t.abort();
        }
        // El alias solo se saca si ningún otro forward lo sigue usando.
        if f.alias {
            self.sincronizar_alias(0);
        }
    }

    /// Deja /etc/hosts con los alias de los forwards vivos. `id` es a quién
    /// culpar si falla (0 = a nadie en particular).
    fn sincronizar_alias(&mut self, id: u64) {
        let entradas: Vec<(std::net::IpAddr, String)> = self
            .forwards
            .iter()
            .filter(|f| f.alias)
            .map(|f| (f.bind, f.servicio.clone()))
            .collect();
        if entradas == crate::hosts::actuales() {
            return;
        }
        let bridge = self.bridge.clone();
        // pkexec bloquea hasta que el usuario responde el diálogo.
        self.rt.spawn_blocking(move || {
            let error = crate::hosts::aplicar(&entradas).err().map(|e| format!("{e:#}"));
            bridge.send(K8sEvent::Alias { id, error });
        });
    }

    // ------------------------------------------------------------- paleta

    pub fn abrir_picker(&mut self, pane_id: u64, modo: PickerModo) {
        self.picker = Some(Picker {
            modo,
            pane: pane_id,
            query: String::new(),
            sel: 0,
        });
    }

    pub fn abrir_palette(&mut self, pane_id: u64) {
        self.palette = Some(Palette {
            pane: pane_id,
            query: String::new(),
            hits: Vec::new(),
            buscando: false,
            token: 0,
            sel: 0,
            query_buscada: String::new(),
            desde_cambio: 0.0,
        });
    }

    /// Dispara la búsqueda si el texto se estabilizó (debounce) y tiene
    /// al menos dos caracteres: con uno solo el barrido no filtra nada.
    fn quizas_buscar(&mut self, dt: f32, ctx: &egui::Context) {
        let Some(p) = self.palette.as_mut() else { return };
        p.desde_cambio += dt;
        let query = p.query.trim().to_string();
        if query == p.query_buscada || query.chars().count() < 2 {
            return;
        }
        // Mientras se escribe hay frames; al soltar el teclado no hay más, así
        // que el debounce nunca vencería sin agendar el repintado nosotros.
        if p.desde_cambio < DEBOUNCE_BUSQUEDA {
            ctx.request_repaint_after(std::time::Duration::from_secs_f32(
                DEBOUNCE_BUSQUEDA - p.desde_cambio,
            ));
            return;
        }
        p.query_buscada = query.clone();
        p.buscando = true;

        let pane_id = p.pane;
        let token = self.token();
        if let Some(p) = self.palette.as_mut() {
            p.token = token;
        }
        let Some(client) = self.client_del_pane(pane_id) else { return };
        let Some(pane) = self.panes.iter().find(|p| p.id == pane_id) else {
            return;
        };
        let ns = pane.ns_sel.clone();
        let Some(cluster) = self.cluster_de(pane) else { return };
        // Solo los kinds buscables que este cluster realmente sirve.
        let recursos: Vec<_> = crate::k8s::search::KINDS_BUSCABLES
            .iter()
            .filter_map(|k| {
                cluster
                    .nav
                    .iter()
                    .flat_map(|c| c.items.iter())
                    .find(|i| i.res.ar.kind == *k)
                    .map(|i| (i.res.ar.clone(), i.res.namespaced))
            })
            .collect();
        let bridge = self.bridge.clone();
        self.rt.spawn(async move {
            crate::k8s::search::buscar(client, recursos, query, ns, token, bridge).await;
        });
    }

    // ------------------------------------------------------------- detalle

    pub fn abrir_detalle(&mut self, pane_id: u64, key: &str) {
        self.abrir_detalle_en(pane_id, key, TabDetalle::Resumen);
    }

    /// Igual que `abrir_detalle` pero aterrizando en una pestaña concreta:
    /// "Editar manifiesto" entra directo al YAML y "Mapa" al mapa, sin obligar
    /// a pasar por Resumen.
    pub fn abrir_detalle_en(&mut self, pane_id: u64, key: &str, tab: TabDetalle) {
        tracing::debug!(pane_id, key, "abrir_detalle");
        let ar_endpoints_del_pane = self.ar_endpoints(pane_id);
        let yaml_token = self.token();
        let eventos_token = self.token();
        let backends_token = self.token();
        let Some(pane) = self.panes.iter_mut().find(|p| p.id == pane_id) else {
            return;
        };
        let Some(client) = pane
            .contexto
            .as_ref()
            .and_then(|c| self.clusters.get(c))
            .and_then(|c| c.client.clone())
        else {
            return;
        };
        let (Some(store), Some(item)) = (pane.store.as_ref(), pane.item.as_ref()) else {
            return;
        };
        let Some(obj) = store.objeto(key) else { return };

        let name = kube::ResourceExt::name_any(obj);
        let ns = kube::ResourceExt::namespace(obj);
        let uid = kube::ResourceExt::uid(obj);
        let ar = item.res.ar.clone();
        let es_service = ar.kind == "Service" && ar.group.is_empty();

        // El watch ya tiene el objeto: mostrarlo al instante evita que el panel
        // quede en blanco durante el round trip. Los Secrets no: los enmascara
        // la capa async y acá se filtrarían en claro.
        let adelanto = if ar.kind == "Secret" {
            None
        } else {
            let mut o = obj.clone();
            o.metadata.managed_fields = None;
            // El watch entrega los objetos sin TypeMeta; sin esto el adelanto
            // arrancaría en `metadata:` y se vería distinto de la copia buena.
            if o.types.is_none() {
                o.types = Some(kube::core::TypeMeta {
                    api_version: ar.api_version.clone(),
                    kind: ar.kind.clone(),
                });
            }
            serde_yaml_ng::to_string(&o).ok()
        };

        for t in pane.detalle_tareas.drain(..) {
            t.abort();
        }

        pane.detalle = Some(Detalle {
            key: key.to_string(),
            kind: ar.kind.clone(),
            name: name.clone(),
            ns: ns.clone(),
            tab,
            yaml: adelanto,
            yaml_edit: None,
            yaml_token,
            yaml_fresco: false,
            backends: Vec::new(),
            backends_token,
            backends_pedidos: false,
            eventos: Vec::new(),
            eventos_token,
            eventos_pedidos: uid.is_some(),
            revelar: false,
            editando: false,
            mapa: None,
            mapa_token: 0,
        });

        let b = self.bridge.clone();
        let (c, n, nn) = (client.clone(), ns.clone(), name.clone());
        pane.detalle_tareas.push(self.rt.spawn(async move {
            k8s::detail::fetch_yaml(c, ar, n, nn, false, yaml_token, b).await;
        }));
        if es_service {
            if let (Some(ns), Some(ar_ep)) = (ns.clone(), ar_endpoints_del_pane) {
                if let Some(d) = pane.detalle.as_mut() {
                    d.backends_pedidos = true;
                }
                let (c, b, n) = (client.clone(), self.bridge.clone(), name.clone());
                pane.detalle_tareas.push(self.rt.spawn(async move {
                    k8s::endpoints::backends(c, ar_ep, ns, n, backends_token, b).await;
                }));
            }
        }
        if let Some(uid) = uid {
            let b = self.bridge.clone();
            pane.detalle_tareas.push(self.rt.spawn(async move {
                k8s::detail::fetch_events(client, uid, ns, eventos_token, b).await;
            }));
        }
        // El mapa normalmente se pide al entrar en la pestaña; si abrimos ya
        // parados ahí, nadie lo dispararía.
        if tab == TabDetalle::Mapa {
            self.pedir_mapa(pane_id);
        }
    }

    /// Re-pide el YAML del objeto abierto, revelando o volviendo a ocultar los
    /// valores de un Secret.
    pub fn alternar_revelar(&mut self, pane_id: u64) {
        let token = self.token();
        let Some(client) = self.client_del_pane(pane_id) else { return };
        let Some(ar) = self.ar_del_pane(pane_id) else { return };
        let bridge = self.bridge.clone();
        let rt_ref = &self.rt;
        let Some(pane) = self.panes.iter_mut().find(|p| p.id == pane_id) else {
            return;
        };
        let Some(det) = pane.detalle.as_mut() else { return };
        det.revelar = !det.revelar;
        det.yaml = None;
        det.yaml_edit = None;
        det.yaml_fresco = false;
        det.editando = false;
        det.yaml_token = token;
        let (ns, name, revelar) = (det.ns.clone(), det.name.clone(), det.revelar);
        pane.detalle_tareas.push(rt_ref.spawn(async move {
            k8s::detail::fetch_yaml(client, ar, ns, name, revelar, token, bridge).await;
        }));
    }

    /// Pide (o re-pide) el mapa del objeto abierto en el detalle: de tráfico
    /// para un Service, de configuración para un workload.
    pub fn pedir_mapa(&mut self, pane_id: u64) {
        let token = self.token();
        let Some(pane) = self.panes.iter_mut().find(|p| p.id == pane_id) else {
            return;
        };
        let Some(client) = pane
            .contexto
            .as_ref()
            .and_then(|c| self.clusters.get(c))
            .and_then(|c| c.client.clone())
        else {
            return;
        };
        let ar = pane.item.as_ref().map(|i| i.res.ar.clone());
        let Some(det) = pane.detalle.as_mut() else { return };
        let Some(ns) = det.ns.clone() else { return };
        det.mapa_token = token;
        det.mapa = None;
        let name = det.name.clone();
        let kind = det.kind.clone();
        let bridge = self.bridge.clone();
        if kind == "Service" {
            pane.detalle_tareas.push(self.rt.spawn(async move {
                k8s::mapa::fetch_service(client, ns, name, token, bridge).await;
            }));
        } else if let Some(ar) = ar {
            pane.detalle_tareas.push(self.rt.spawn(async move {
                k8s::mapa::fetch_workload(client, ar, ns, name, token, bridge).await;
            }));
        }
    }

    // ------------------------------------------------------------ acciones

    fn client_del_pane(&self, pane_id: u64) -> Option<Client> {
        self.panes
            .iter()
            .find(|p| p.id == pane_id)
            .and_then(|p| p.contexto.as_ref())
            .and_then(|c| self.clusters.get(c))
            .and_then(|c| c.client.clone())
    }

    fn ar_del_pane(&self, pane_id: u64) -> Option<kube::discovery::ApiResource> {
        self.panes
            .iter()
            .find(|p| p.id == pane_id)
            .and_then(|p| p.item.as_ref())
            .map(|i| i.res.ar.clone())
    }

    /// Ejecuta la acción ya confirmada del modal.
    pub fn ejecutar_confirmada(&mut self) {
        let Some(c) = self.confirm.take() else { return };
        let (Some(client), Some(ar)) = (self.client_del_pane(c.pane), self.ar_del_pane(c.pane))
        else {
            return;
        };
        let bridge = self.bridge.clone();
        match c.verbo {
            Verbo::Borrar => {
                self.rt.spawn(async move {
                    k8s::actions::borrar(client, ar, c.ns, c.name, bridge).await;
                });
            }
            Verbo::Reiniciar => {
                if c.kind == "Pod" {
                    // Reiniciar un pod es borrarlo: el controlador lo repone.
                    self.rt.spawn(async move {
                        k8s::actions::borrar(client, ar, c.ns, c.name, bridge).await;
                    });
                } else {
                    self.rt.spawn(async move {
                        k8s::actions::reiniciar(client, ar, c.ns, c.name, bridge).await;
                    });
                }
            }
            Verbo::Escalar(n) => {
                self.rt.spawn(async move {
                    k8s::actions::escalar(client, ar, c.ns, c.name, n, bridge).await;
                });
            }
        }
    }

    pub fn aplicar_yaml(&mut self, pane_id: u64, yaml: String) {
        let (Some(client), Some(ar)) = (self.client_del_pane(pane_id), self.ar_del_pane(pane_id))
        else {
            return;
        };
        // El nombre/ns esperados salen del detalle abierto, no del YAML.
        let Some((name, ns, kind, revelar)) = self
            .panes
            .iter()
            .find(|p| p.id == pane_id)
            .and_then(|p| p.detalle.as_ref())
            .map(|d| (d.name.clone(), d.ns.clone(), d.kind.clone(), d.revelar))
        else {
            return;
        };
        // Aplicar un Secret enmascarado escribiría el marcador como valor.
        if kind == "Secret" && !revelar {
            self.toast(
                "revelá el Secret antes de aplicarlo: los valores están ocultos",
                true,
            );
            return;
        }
        let bridge = self.bridge.clone();
        self.rt.spawn(async move {
            k8s::actions::aplicar_yaml(client, ar, yaml, name, ns, bridge).await;
        });
    }

    // -------------------------------------------------------- logs y shell

    pub fn abrir_logs(&mut self, pane_id: u64, key: &str) {
        let Some(pane) = self.panes.iter().find(|p| p.id == pane_id) else {
            return;
        };
        let Some(store) = pane.store.as_ref() else { return };
        let Some(obj) = store.objeto(key) else { return };
        let Some(ns) = kube::ResourceExt::namespace(obj) else { return };
        let pod = kube::ResourceExt::name_any(obj);
        let contenedores = contenedores_de(obj);

        if let Some(pane) = self.pane(pane_id) {
            pane.cerrar_bottom();
            let contenedor = contenedores.first().cloned();
            pane.bottom = Some(Bottom::Logs(VistaLogs {
                ns,
                pod,
                contenedores,
                contenedor,
                lineas: VecDeque::new(),
                filtro: String::new(),
                follow: true,
                previous: false,
                tail: 500,
                token: 0,
                cerrado: None,
                tarea: None,
            }));
        }
        self.reiniciar_logs(pane_id);
    }

    pub fn reiniciar_logs(&mut self, pane_id: u64) {
        let token = self.token();
        let Some(client) = self.client_del_pane(pane_id) else { return };
        let bridge = self.bridge.clone();
        let rt = &self.rt;
        let Some(pane) = self.panes.iter_mut().find(|p| p.id == pane_id) else {
            return;
        };
        let Some(Bottom::Logs(v)) = pane.bottom.as_mut() else { return };

        if let Some(t) = v.tarea.take() {
            t.abort();
        }
        v.lineas.clear();
        v.cerrado = None;
        v.token = token;

        let req = k8s::logs::LogRequest {
            namespace: v.ns.clone(),
            pod: v.pod.clone(),
            container: v.contenedor.clone(),
            follow: v.follow,
            previous: v.previous,
            tail_lines: Some(v.tail),
            timestamps: false,
        };
        v.tarea = Some(rt.spawn(async move {
            k8s::logs::stream(client, req, token, bridge).await;
        }));
    }

    pub fn abrir_shell(&mut self, pane_id: u64, key: &str) {
        let token = self.token();
        let Some(client) = self.client_del_pane(pane_id) else { return };
        let Some(pane) = self.panes.iter().find(|p| p.id == pane_id) else {
            return;
        };
        let Some(store) = pane.store.as_ref() else { return };
        let Some(obj) = store.objeto(key) else { return };
        let Some(ns) = kube::ResourceExt::namespace(obj) else { return };
        let pod = kube::ResourceExt::name_any(obj);
        let contenedor = contenedores_de(obj).first().cloned();

        let (stdin_tx, stdin_rx) = tokio::sync::mpsc::unbounded_channel();
        let (resize_tx, resize_rx) = tokio::sync::mpsc::unbounded_channel();
        let bridge = self.bridge.clone();
        let (c, n, p2, cont) = (client, ns.clone(), pod.clone(), contenedor.clone());
        let tarea = self.rt.spawn(async move {
            k8s::exec::run(c, n, p2, cont, stdin_rx, resize_rx, token, bridge).await;
        });

        if let Some(pane) = self.pane(pane_id) {
            pane.cerrar_bottom();
            pane.bottom = Some(Bottom::Term(VistaTerm {
                ns,
                pod,
                contenedor,
                parser: vt100::Parser::new(24, 80, 2_000),
                handles: k8s::exec::TermHandles {
                    stdin: stdin_tx,
                    resize: resize_tx,
                },
                token,
                cerrado: None,
                cols: 80,
                rows: 24,
                tarea: Some(tarea),
            }));
        }
    }

    /// Harness de depuración: `KUBO_TEST_SHELL=ns/pod` abre una shell apenas
    /// carga el primer listado; `KUBO_TEST_SHELL_CMD` manda un comando. Sirve
    /// para probar el exec sin clickear.
    fn gancho_de_prueba(&mut self, pane_id: u64) {
        if let Ok(key) = std::env::var("KUBO_TEST_SHELL") {
            if !key.is_empty() {
                std::env::set_var("KUBO_TEST_SHELL", "");
                self.abrir_shell(pane_id, &key);
                if let Ok(cmd) = std::env::var("KUBO_TEST_SHELL_CMD") {
                    if let Some(pane) = self.panes.iter().find(|p| p.id == pane_id) {
                        if let Some(Bottom::Term(v)) = pane.bottom.as_ref() {
                            let _ = v.handles.stdin.send(format!("{cmd}\n").into_bytes());
                        }
                    }
                }
            }
        }

        // KUBO_TEST_PF=ns/servicio — levanta un port-forward con los valores
        // por defecto (sin alias, así no dispara el diálogo de polkit).
        if let Ok(spec) = std::env::var("KUBO_TEST_PF") {
            if !spec.is_empty() {
                std::env::set_var("KUBO_TEST_PF", "");
                std::env::set_var("KUBO_TEST_PF_AUTO", "1");
                self.pedir_forward(pane_id, &spec);
            }
        }

        // KUBO_TEST_PALETTE=texto — abre la paleta con la query puesta.
        if let Ok(q) = std::env::var("KUBO_TEST_PALETTE") {
            if !q.is_empty() {
                std::env::set_var("KUBO_TEST_PALETTE", "");
                self.abrir_palette(pane_id);
                if let Some(p) = self.palette.as_mut() {
                    p.query = q;
                    p.desde_cambio = 1.0;
                }
                return;
            }
        }

        // KUBO_TEST_IRA=Kind:ns:name — prueba la navegación "ir al recurso".
        if let Ok(spec) = std::env::var("KUBO_TEST_IRA") {
            if !spec.is_empty() {
                std::env::set_var("KUBO_TEST_IRA", "");
                let partes: Vec<&str> = spec.splitn(3, ':').collect();
                if let [kind, ns, name] = partes[..] {
                    let (kind, name) = (kind.to_string(), name.to_string());
                    let ns = Some(ns.to_string());
                    self.ir_a(pane_id, &kind, ns, &name);
                    return;
                }
            }
        }

        // KUBO_TEST_MAPA=ns/svc (o KUBO_TEST_WMAPA=ns/deploy): navega al
        // kind y abre el mapa.
        let (var, kind_buscado) = if std::env::var("KUBO_TEST_WMAPA").map(|v| !v.is_empty()).unwrap_or(false) {
            ("KUBO_TEST_WMAPA", "Deployment")
        } else {
            ("KUBO_TEST_MAPA", "Service")
        };
        if let Ok(spec) = std::env::var(var) {
            if !spec.is_empty() {
                let kind_actual = self
                    .panes
                    .iter()
                    .find(|p| p.id == pane_id)
                    .and_then(|p| p.item.as_ref())
                    .map(|i| i.res.ar.kind.clone());
                if kind_actual.as_deref() != Some(kind_buscado) {
                    let destino = self
                        .panes
                        .iter()
                        .find(|p| p.id == pane_id)
                        .and_then(|p| self.cluster_de(p))
                        .and_then(|c| {
                            c.nav
                                .iter()
                                .flat_map(|cat| cat.items.iter())
                                .find(|i| i.res.ar.kind == kind_buscado)
                                .cloned()
                        });
                    if let Some(item) = destino {
                        self.seleccionar(pane_id, item);
                    }
                    return;
                }
                std::env::set_var(var, "");
                tracing::debug!(spec, "gancho: abriendo detalle+mapa");
                self.abrir_detalle(pane_id, &spec);
                if let Some(pane) = self.pane(pane_id) {
                    if let Some(d) = pane.detalle.as_mut() {
                        d.tab = crate::app::TabDetalle::Mapa;
                    }
                }
                self.pedir_mapa(pane_id);
            }
        }
    }

    // ------------------------------------------------------------ canal -> UI

    /// Persiste qué está mirando cada panel, para volver acá al reabrir.
    fn guardar_layout(&self) {
        crate::layout::guardar(&crate::layout::Estado {
            panes: self
                .panes
                .iter()
                .map(|p| crate::layout::PaneGuardado {
                    contexto: p.contexto.clone(),
                    ns: p.ns_sel.clone(),
                    recurso: p
                        .item
                        .as_ref()
                        .map(|i| i.res.key())
                        .or_else(|| p.recurso_pendiente.clone()),
                })
                .collect(),
        });
    }

    /// Panel activo, cayendo al primero si el recordado ya se cerró.
    pub fn pane_activo(&self) -> Option<u64> {
        if self.panes.iter().any(|p| p.id == self.pane_activo) {
            Some(self.pane_activo)
        } else {
            self.panes.first().map(|p| p.id)
        }
    }

    pub fn drenar_eventos(&mut self) {
        while let Ok(ev) = self.rx.try_recv() {
            self.aplicar_evento(ev);
        }
    }

    fn aplicar_evento(&mut self, ev: K8sEvent) {
        match ev {
            K8sEvent::Connected {
                token,
                info,
                client,
            } => {
                let Some((_, cluster)) = self
                    .clusters
                    .iter_mut()
                    .find(|(_, c)| c.token == token)
                else {
                    return;
                };
                cluster.nav = crate::nav::build(&info.resources);
                cluster.info = Some(*info);
                cluster.client = Some(client);
                cluster.conn = Conn::Lista;
                let ids: Vec<u64> = self.panes.iter().map(|p| p.id).collect();
                for id in ids {
                    self.autoseleccionar(id);
                }
            }
            K8sEvent::ConnectFailed { token, error } => {
                if let Some((_, c)) = self.clusters.iter_mut().find(|(_, c)| c.token == token) {
                    c.conn = Conn::Error;
                    c.error = Some(error);
                }
            }
            K8sEvent::Backends { token, items } => {
                for pane in &mut self.panes {
                    if let Some(d) = pane.detalle.as_mut() {
                        if d.backends_token == token {
                            d.backends = items;
                            d.backends_pedidos = false;
                            return;
                        }
                    }
                }
            }
            K8sEvent::Endpoints { token, mapa } => {
                if let Some(p) = self.panes.iter_mut().find(|p| p.endpoints_token == token) {
                    p.endpoints = mapa;
                }
            }
            K8sEvent::PuertosSvc { servicio, puertos } => {
                if let Some(d) = self.dialogo_pf.as_mut() {
                    if d.servicio != servicio {
                        return;
                    }
                    // Por defecto, el mismo puerto que dentro del cluster; si es
                    // privilegiado se remapea a uno alto y la UI lo avisa.
                    d.puerto_local = puertos
                        .first()
                        .map(|p| k8s::portforward::puerto_local_sugerido(p.puerto).to_string())
                        .unwrap_or_default();
                    d.puertos = puertos;
                    d.cargando = false;
                }
                if std::env::var("KUBO_TEST_PF_AUTO").is_ok_and(|v| !v.is_empty()) {
                    std::env::set_var("KUBO_TEST_PF_AUTO", "");
                    self.abrir_forward();
                }
            }
            K8sEvent::Pf { id, msg } => {
                let Some(f) = self.forwards.iter_mut().find(|f| f.id == id) else {
                    return;
                };
                match msg {
                    k8s::PfMsg::Escuchando => {
                        f.estado = EstadoPf::Activo;
                        f.error = None;
                    }
                    k8s::PfMsg::Conexion(d) => f.conexiones = (f.conexiones + d).max(0),
                    k8s::PfMsg::Fatal(e) => {
                        f.estado = EstadoPf::Caido;
                        f.error = Some(e.clone());
                        self.toast(e, true);
                    }
                    // El listener sigue vivo: se anota en la fila y nada más.
                    k8s::PfMsg::FalloConexion(e) => f.error = Some(e),
                }
            }
            K8sEvent::Alias { id, error } => {
                if let Some(e) = error {
                    if let Some(f) = self.forwards.iter_mut().find(|f| f.id == id) {
                        // El listener ya está atado a su IP propia, así que
                        // `.localhost` no llegaría: se ofrece la IP directa.
                        f.alias = false;
                        f.host = f.bind.to_string();
                    }
                    self.toast(format!("alias no aplicado: {e}"), true);
                } else {
                    self.toast("/etc/hosts actualizado", false);
                }
            }
            K8sEvent::Version { token, version } => {
                if let Some((_, c)) = self.clusters.iter_mut().find(|(_, c)| c.token == token) {
                    if let Some(i) = c.info.as_mut() {
                        i.version = version;
                    }
                }
            }
            K8sEvent::Resources { token, resources } => {
                let Some((_, cluster)) = self.clusters.iter_mut().find(|(_, c)| c.token == token)
                else {
                    return;
                };
                cluster.nav = crate::nav::build(&resources);
                if let Some(i) = cluster.info.as_mut() {
                    i.resources = resources;
                }
                // Un panel que quedó sin vista porque el kind todavía no estaba
                // en la caché ahora sí puede abrirla.
                let ids: Vec<u64> = self.panes.iter().map(|p| p.id).collect();
                for id in ids {
                    self.autoseleccionar(id);
                }
            }
            K8sEvent::Namespaces { token, list } => {
                if let Some((_, c)) = self.clusters.iter_mut().find(|(_, c)| c.token == token) {
                    c.namespaces = list;
                }
            }
            K8sEvent::Watch { token, msg } => {
                let mut error_toast = None;
                let mut init_listo: Option<u64> = None;
                if let Some(pane) = self.panes.iter_mut().find(|p| p.watch_token == token) {
                    if let Some(store) = pane.store.as_mut() {
                        match msg {
                            WatchMsg::Init => store.init_start(),
                            WatchMsg::InitBatch(objs) => store.init_batch(objs),
                            WatchMsg::InitDone => {
                                store.init_done();
                                init_listo = Some(pane.id);
                            }
                            WatchMsg::Apply(o) => store.apply(*o),
                            WatchMsg::Delete(o) => store.delete(&o),
                            WatchMsg::Error(e) => {
                                store.set_error(e.clone());
                                error_toast = Some(e);
                            }
                        }
                    }
                }
                if let Some(e) = error_toast {
                    self.toast(format!("watch: {e}"), true);
                }
                if let Some(pane_id) = init_listo {
                    // Navegación diferida: el usuario clickeó "ir al recurso".
                    let pendiente = self
                        .pane(pane_id)
                        .and_then(|p| p.pendiente_detalle.take());
                    // KUBO_TEST_TAB=Yaml|Eventos|Mapa fija la pestaña al navegar.
                    let tab_forzada = std::env::var("KUBO_TEST_TAB").ok().filter(|s| !s.is_empty());
                    if let Some(key) = pendiente {
                        let existe = self
                            .panes
                            .iter()
                            .find(|p| p.id == pane_id)
                            .and_then(|p| p.store.as_ref())
                            .and_then(|s| s.objeto(&key))
                            .is_some();
                        if existe {
                            tracing::debug!(key, "navegación: detalle diferido abierto");
                            self.abrir_detalle(pane_id, &key);
                            if let Some(t) = tab_forzada {
                                if let Some(d) = self.pane(pane_id).and_then(|p| p.detalle.as_mut()) {
                                    d.tab = match t.as_str() {
                                        "Yaml" => TabDetalle::Yaml,
                                        "Eventos" => TabDetalle::Eventos,
                                        "Mapa" => TabDetalle::Mapa,
                                        _ => TabDetalle::Resumen,
                                    };
                                }
                            }
                        } else {
                            self.toast(format!("«{key}» no está en la vista actual"), true);
                        }
                    }
                    self.gancho_de_prueba(pane_id);
                }
            }
            K8sEvent::Yaml { token, text } => {
                for pane in &mut self.panes {
                    if let Some(d) = pane.detalle.as_mut() {
                        if d.yaml_token == token {
                            d.yaml = Some(text);
                            d.yaml_fresco = true;
                            return;
                        }
                    }
                }
            }
            K8sEvent::ObjectEvents { token, items } => {
                for pane in &mut self.panes {
                    if let Some(d) = pane.detalle.as_mut() {
                        if d.eventos_token == token {
                            d.eventos = items;
                            return;
                        }
                    }
                }
            }
            K8sEvent::Mapa { token, data } => {
                tracing::debug!(token, "mapa recibido");
                for pane in &mut self.panes {
                    if let Some(d) = pane.detalle.as_mut() {
                        if d.mapa_token == token {
                            d.mapa = Some(data);
                            return;
                        }
                    }
                }
            }
            K8sEvent::LogLine { token, line } => {
                for pane in &mut self.panes {
                    if let Some(Bottom::Logs(v)) = pane.bottom.as_mut() {
                        if v.token == token {
                            if v.lineas.len() >= MAX_LINEAS_LOG {
                                v.lineas.pop_front();
                            }
                            v.lineas.push_back(line);
                            return;
                        }
                    }
                }
            }
            K8sEvent::LogClosed { token, error } => {
                for pane in &mut self.panes {
                    if let Some(Bottom::Logs(v)) = pane.bottom.as_mut() {
                        if v.token == token {
                            v.cerrado = Some(error.unwrap_or_else(|| "stream cerrado".into()));
                            return;
                        }
                    }
                }
            }
            K8sEvent::TermData { token, bytes } => {
                for pane in &mut self.panes {
                    if let Some(Bottom::Term(v)) = pane.bottom.as_mut() {
                        if v.token == token {
                            v.parser.process(&bytes);
                            return;
                        }
                    }
                }
            }
            K8sEvent::TermClosed { token, error } => {
                for pane in &mut self.panes {
                    if let Some(Bottom::Term(v)) = pane.bottom.as_mut() {
                        if v.token == token {
                            v.cerrado = Some(error.unwrap_or_else(|| "sesión terminada".into()));
                            return;
                        }
                    }
                }
            }
            K8sEvent::Search { token, hits } => {
                if let Some(p) = self.palette.as_mut() {
                    if p.token == token {
                        p.hits = hits;
                        p.buscando = false;
                    }
                }
            }
            K8sEvent::Toast { text, error } => self.toast(text, error),
        }
    }
}

fn contenedores_de(obj: &kube::api::DynamicObject) -> Vec<String> {
    obj.data
        .get("spec")
        .and_then(|s| s.get("containers"))
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|c| c.get("name").and_then(|n| n.as_str()))
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default()
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
