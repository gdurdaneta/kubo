//! Estado en memoria de una vista de recursos.
//!
//! Las celdas se calculan una sola vez, al entrar el objeto, no en cada frame.
//! El orden y el filtro se recalculan solo cuando algo cambió (`dirty`).

use std::collections::BTreeMap;

use k8s_openapi::jiff::Timestamp;
use kube::api::DynamicObject;
use kube::ResourceExt;

use crate::columns::{self, Cell};

struct Entry {
    obj: DynamicObject,
    cells: Vec<Cell>,
    creado: Option<Timestamp>,
}

/// Filtro rápido sobre la columna de estado.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum FiltroEstado {
    #[default]
    Todos,
    /// Todo lo que no sea un estado sano: lo que uno busca cuando algo falla.
    Problemas,
    Valor(String),
}

/// Estados que no son un problema. El resto (ImagePullBackOff, CrashLoopBackOff,
/// Error, Evicted, Pending…) cae en `Problemas` sin tener que enumerarlos.
const ESTADOS_SANOS: &[&str] = &["Running", "Succeeded", "Completed", "Ready", "Active", "Bound"];

pub fn estado_sano(e: &str) -> bool {
    ESTADOS_SANOS.contains(&e)
}

pub struct Store {
    kind: String,
    mostrar_ns: bool,
    items: BTreeMap<String, Entry>,
    /// Buffer del listado inicial: la tabla no parpadea al resincronizar.
    buffer: Option<BTreeMap<String, Entry>>,
    /// Claves visibles ya ordenadas y filtradas.
    view: Vec<String>,
    dirty: bool,
    filtro: String,
    /// Columna de estado y qué se está filtrando en ella.
    col_estado: Option<usize>,
    filtro_estado: FiltroEstado,
    pub sort_col: usize,
    pub sort_desc: bool,
    pub cargando: bool,
    pub error: Option<String>,
}

impl Store {
    pub fn new(kind: String, mostrar_ns: bool) -> Self {
        Self {
            kind,
            mostrar_ns,
            items: BTreeMap::new(),
            buffer: None,
            view: Vec::new(),
            dirty: true,
            filtro: String::new(),
            col_estado: None,
            filtro_estado: FiltroEstado::Todos,
            sort_col: 0,
            sort_desc: false,
            cargando: true,
            error: None,
        }
    }

    fn clave(o: &DynamicObject) -> String {
        match o.namespace() {
            Some(ns) => format!("{ns}/{}", o.name_any()),
            None => o.name_any(),
        }
    }

    fn entrada(&self, o: DynamicObject) -> Entry {
        let cells = columns::row(&self.kind, &o, self.mostrar_ns);
        let creado = o.creation_timestamp().map(|t| t.0);
        Entry {
            obj: o,
            cells,
            creado,
        }
    }

    pub fn init_start(&mut self) {
        self.buffer = Some(BTreeMap::new());
        self.error = None;
    }

    pub fn init_batch(&mut self, objs: Vec<DynamicObject>) {
        for o in objs {
            self.init_apply(o);
        }
    }

    pub fn init_apply(&mut self, o: DynamicObject) {
        let k = Self::clave(&o);
        let e = self.entrada(o);
        match self.buffer.as_mut() {
            Some(b) => {
                b.insert(k, e);
            }
            // Un InitApply sin Init previo no debería pasar, pero no perdemos el dato.
            None => {
                self.items.insert(k, e);
                self.dirty = true;
            }
        }
    }

    pub fn init_done(&mut self) {
        if let Some(b) = self.buffer.take() {
            self.items = b;
        }
        self.cargando = false;
        self.dirty = true;
    }

    pub fn apply(&mut self, o: DynamicObject) {
        let k = Self::clave(&o);
        let e = self.entrada(o);
        self.items.insert(k, e);
        self.dirty = true;
    }

    pub fn delete(&mut self, o: &DynamicObject) {
        self.items.remove(&Self::clave(o));
        self.dirty = true;
    }

    pub fn set_error(&mut self, e: String) {
        self.error = Some(e);
        self.cargando = false;
    }

    pub fn total(&self) -> usize {
        self.items.len()
    }

    pub fn set_filtro(&mut self, f: &str) {
        if self.filtro != f {
            self.filtro = f.to_string();
            self.dirty = true;
        }
    }

    pub fn set_col_estado(&mut self, col: Option<usize>) {
        if self.col_estado != col {
            self.col_estado = col;
            self.dirty = true;
        }
    }

    pub fn filtro_estado(&self) -> &FiltroEstado {
        &self.filtro_estado
    }

    pub fn set_filtro_estado(&mut self, f: FiltroEstado) {
        if self.filtro_estado != f {
            self.filtro_estado = f;
            self.dirty = true;
        }
    }

    /// Valores distintos de la columna de estado, para armar el selector.
    pub fn estados(&self) -> Vec<(String, usize)> {
        let Some(col) = self.col_estado else {
            return Vec::new();
        };
        let mut cuenta: BTreeMap<&str, usize> = BTreeMap::new();
        for e in self.items.values() {
            if let Some(c) = e.cells.get(col) {
                *cuenta.entry(c.text.as_str()).or_insert(0) += 1;
            }
        }
        cuenta.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
    }

    /// ¿La fila pasa el filtro de estado?
    fn pasa_estado(&self, e: &Entry) -> bool {
        let Some(col) = self.col_estado else { return true };
        let texto = e.cells.get(col).map(|c| c.text.as_str()).unwrap_or("");
        match &self.filtro_estado {
            FiltroEstado::Todos => true,
            FiltroEstado::Problemas => !estado_sano(texto),
            FiltroEstado::Valor(v) => texto == v,
        }
    }

    pub fn set_sort(&mut self, col: usize) {
        if self.sort_col == col {
            self.sort_desc = !self.sort_desc;
        } else {
            self.sort_col = col;
            self.sort_desc = false;
        }
        self.dirty = true;
    }

    /// Recalcula la vista si hace falta. Se llama una vez por frame.
    /// ¿Quedó orden/filtro por recalcular? La UI lo consulta al cerrar el
    /// frame para pedir otro: si no, el cambio se vería recién al siguiente
    /// evento de entrada.
    pub fn sucio(&self) -> bool {
        self.dirty
    }

    pub fn refrescar(&mut self) {
        if !self.dirty {
            return;
        }
        let f = self.filtro.to_lowercase();
        let mut claves: Vec<&String> = self
            .items
            .iter()
            .filter(|(k, e)| {
                self.pasa_estado(e)
                    && (f.is_empty()
                        || k.to_lowercase().contains(&f)
                        || e.cells.iter().any(|c| c.text.to_lowercase().contains(&f)))
            })
            .map(|(k, _)| k)
            .collect();

        let col = self.sort_col;
        let items = &self.items;
        let n_cols = items.values().next().map(|e| e.cells.len()).unwrap_or(0);
        if col >= n_cols && n_cols > 0 {
            // Última columna: edad. Se ordena por instante de creación, no por
            // el texto ("2m" contra "10m" no compara bien como string).
            claves.sort_by(|a, b| {
                let va = items.get(*a).and_then(|e| e.creado);
                let vb = items.get(*b).and_then(|e| e.creado);
                vb.cmp(&va).then_with(|| a.cmp(b))
            });
        } else {
            claves.sort_by(|a, b| {
                let va = items.get(*a).and_then(|e| e.cells.get(col)).map(|c| c.text.as_str()).unwrap_or("");
                let vb = items.get(*b).and_then(|e| e.cells.get(col)).map(|c| c.text.as_str()).unwrap_or("");
                comparar(va, vb).then_with(|| a.cmp(b))
            });
        }
        if self.sort_desc {
            claves.reverse();
        }

        self.view = claves.into_iter().cloned().collect();
        self.dirty = false;
    }

    pub fn visibles(&self) -> usize {
        self.view.len()
    }

    pub fn fila(&self, i: usize) -> Option<(&str, &[Cell], Option<Timestamp>)> {
        let k = self.view.get(i)?;
        let e = self.items.get(k)?;
        Some((k.as_str(), &e.cells, e.creado))
    }

    pub fn objeto(&self, key: &str) -> Option<&DynamicObject> {
        self.items.get(key).map(|e| &e.obj)
    }
}

/// Compara alfabéticamente salvo cuando ambos valores son duraciones o
/// números: ahí "2m" tiene que ir antes que "10m", y "9" antes que "10".
fn comparar(a: &str, b: &str) -> std::cmp::Ordering {
    if let (Some(x), Some(y)) = (a_segundos(a), a_segundos(b)) {
        return x.cmp(&y);
    }
    if let (Ok(x), Ok(y)) = (a.parse::<f64>(), b.parse::<f64>()) {
        return x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal);
    }
    a.to_lowercase().cmp(&b.to_lowercase())
}

/// "3d" -> 259200. Devuelve None si no tiene forma de duración.
fn a_segundos(s: &str) -> Option<i64> {
    let (num, unidad) = s.split_at(s.len().checked_sub(1)?);
    let n: i64 = num.parse().ok()?;
    let mult = match unidad {
        "s" => 1,
        "m" => 60,
        "h" => 3_600,
        "d" => 86_400,
        "a" => 86_400 * 365,
        _ => return None,
    };
    Some(n * mult)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Genera `n` pods sintéticos con la forma que usan las columnas.
    fn pods(n: usize) -> Vec<DynamicObject> {
        (0..n)
            .map(|i| {
                let v = serde_json::json!({
                    "apiVersion": "v1",
                    "kind": "Pod",
                    "metadata": {
                        "name": format!("api-worker-{i:05}-7d9f8b6c4x"),
                        "namespace": format!("ns-{}", i % 40),
                        "creationTimestamp": "2026-05-08T15:46:15Z",
                        "uid": format!("{i:08}-0000-0000-0000-000000000000"),
                    },
                    "spec": { "containers": [{ "name": "app" }] },
                    "status": {
                        "phase": "Running",
                        "containerStatuses": [{
                            "name": "app", "ready": true, "restartCount": i % 7,
                            "state": { "running": { "startedAt": "2026-05-08T15:46:20Z" } }
                        }],
                    },
                });
                serde_json::from_value(v).unwrap()
            })
            .collect()
    }

    fn cargado(n: usize) -> Store {
        let mut s = Store::new("Pod".into(), true);
        s.init_start();
        for o in pods(n) {
            s.init_apply(o);
        }
        s.init_done();
        s
    }

    #[test]
    #[ignore = "medición manual: cargo test --release -- --ignored --nocapture"]
    fn perf_refrescar() {
        for n in [1_000usize, 5_000, 20_000] {
            let t = std::time::Instant::now();
            let mut s = cargado(n);
            let carga = t.elapsed();

            let t = std::time::Instant::now();
            s.refrescar();
            let orden = t.elapsed();

            let t = std::time::Instant::now();
            s.set_filtro("worker-1234");
            s.refrescar();
            let filtro = t.elapsed();

            let t = std::time::Instant::now();
            s.set_filtro("");
            s.set_sort(1);
            s.refrescar();
            let resort = t.elapsed();

            println!(
                "n={n:>6}  carga={carga:>10.2?}  orden={orden:>10.2?}  \
                 filtro={filtro:>10.2?}  resort={resort:>10.2?}  visibles={}",
                s.visibles()
            );
        }
    }
}
