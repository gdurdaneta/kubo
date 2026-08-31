//! Streaming de logs de un contenedor, línea a línea.

use futures::{AsyncBufReadExt, StreamExt};
use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, LogParams};
use kube::Client;

use super::{K8sEvent, UiBridge};

pub struct LogRequest {
    pub namespace: String,
    pub pod: String,
    pub container: Option<String>,
    pub follow: bool,
    pub previous: bool,
    pub tail_lines: Option<i64>,
    pub timestamps: bool,
}

/// Abre el stream y empuja cada línea a la UI hasta que se aborte la tarea.
pub async fn stream(client: Client, req: LogRequest, token: u64, bridge: UiBridge) {
    // El subrecurso /log solo está definido para el tipo Pod, no para
    // DynamicObject: acá sí conviene el tipo concreto.
    let api: Api<Pod> = Api::namespaced(client, &req.namespace);

    let lp = LogParams {
        container: req.container.clone(),
        follow: req.follow,
        previous: req.previous,
        tail_lines: req.tail_lines,
        timestamps: req.timestamps,
        ..Default::default()
    };

    let reader = match api.log_stream(&req.pod, &lp).await {
        Ok(r) => r,
        Err(e) => {
            bridge.send(K8sEvent::LogClosed {
                token,
                error: Some(e.to_string()),
            });
            return;
        }
    };

    // `log_stream` devuelve el AsyncBufRead de `futures`, no el de tokio: sus
    // `lines()` son un Stream, no un lector con `next_line()`.
    let mut lines = reader.lines();
    while let Some(item) = lines.next().await {
        match item {
            Ok(line) => bridge.send(K8sEvent::LogLine { token, line }),
            Err(e) => {
                bridge.send(K8sEvent::LogClosed {
                    token,
                    error: Some(e.to_string()),
                });
                return;
            }
        }
    }
    bridge.send(K8sEvent::LogClosed { token, error: None });
}
