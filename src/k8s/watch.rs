//! Watch genérico sobre `DynamicObject`: sirve igual para Pods que para un CRD.

use futures::StreamExt;
use kube::api::{Api, DynamicObject};
use kube::discovery::ApiResource;
use kube::runtime::watcher::{self, Event};
use kube::Client;

use super::{K8sEvent, UiBridge, WatchMsg};

/// Objetos por página en el listado inicial.
const TAM_PAGINA: u32 = 2_000;
/// Objetos por mensaje hacia la UI durante el listado inicial.
const TAM_LOTE: usize = 500;

/// Ámbito de la vista actual: cluster entero o un namespace concreto.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    AllNamespaces,
    Namespace(String),
}

/// Arranca el watch y bombea eventos hasta que se aborte la tarea.
///
/// `token` identifica la vista: la UI descarta lo que llegue de watches viejos,
/// así un cambio rápido de recurso no mezcla filas.
pub async fn run(
    client: Client,
    ar: ApiResource,
    namespaced: bool,
    target: Target,
    token: u64,
    bridge: UiBridge,
) {
    let api: Api<DynamicObject> = match (&target, namespaced) {
        (Target::Namespace(ns), true) => Api::namespaced_with(client, ns, &ar),
        _ => Api::all_with(client, &ar),
    };

    // Menos páginas = menos round trips contra el API server, que es lo que
    // domina el tiempo de carga en un cluster remoto.
    let cfg = watcher::Config::default().page_size(TAM_PAGINA);
    tracing::info!(kind = %ar.kind, ?target, token, "watch: arrancando");
    let t0 = std::time::Instant::now();
    let mut stream = watcher::watcher(api, cfg).boxed();
    let mut vistos = 0usize;
    let mut lote: Vec<DynamicObject> = Vec::with_capacity(TAM_LOTE);

    let mut backoff_notified = false;
    while let Some(item) = stream.next().await {
        let msg = match item {
            Ok(Event::Init) => {
                lote.clear();
                WatchMsg::Init
            }
            Ok(Event::InitApply(o)) => {
                vistos += 1;
                lote.push(o);
                if lote.len() < TAM_LOTE {
                    continue;
                }
                WatchMsg::InitBatch(std::mem::replace(
                    &mut lote,
                    Vec::with_capacity(TAM_LOTE),
                ))
            }
            Ok(Event::InitDone) => {
                backoff_notified = false;
                if !lote.is_empty() {
                    bridge.send(K8sEvent::Watch {
                        token,
                        msg: WatchMsg::InitBatch(std::mem::take(&mut lote)),
                    });
                }
                tracing::info!(
                    kind = %ar.kind, vistos, ms = t0.elapsed().as_millis(),
                    "watch: listado inicial completo"
                );
                WatchMsg::InitDone
            }
            Ok(Event::Apply(o)) => WatchMsg::Apply(Box::new(o)),
            Ok(Event::Delete(o)) => WatchMsg::Delete(Box::new(o)),
            Err(e) => {
                // El watcher reintenta solo; no inundamos la UI con el mismo error.
                if backoff_notified {
                    continue;
                }
                backoff_notified = true;
                tracing::warn!(kind = %ar.kind, error = %e, "watch: error");
                WatchMsg::Error(e.to_string())
            }
        };
        bridge.send(K8sEvent::Watch { token, msg });
    }
}
