//! Shell interactiva dentro de un contenedor: exec con PTY sobre WebSocket.

use futures::SinkExt;
use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, AttachParams, TerminalSize};
use kube::Client;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

use super::{K8sEvent, UiBridge};

pub struct TermHandles {
    pub stdin: mpsc::UnboundedSender<Vec<u8>>,
    pub resize: mpsc::UnboundedSender<(u16, u16)>,
}

/// Abre la sesión y bombea bytes en ambos sentidos hasta que el proceso
/// remoto termina o la tarea se aborta.
pub async fn run(
    client: Client,
    ns: String,
    pod: String,
    container: Option<String>,
    mut stdin_rx: mpsc::UnboundedReceiver<Vec<u8>>,
    mut resize_rx: mpsc::UnboundedReceiver<(u16, u16)>,
    token: u64,
    bridge: UiBridge,
) {
    let api: Api<Pod> = Api::namespaced(client, &ns);
    let mut ap = AttachParams::interactive_tty();
    ap.container = container;

    // bash si existe; si no, sh. Con exec para no dejar un sh padre colgado.
    let cmd = ["sh", "-c", "command -v bash >/dev/null 2>&1 && exec bash || exec sh"];

    let mut attached = match api.exec(&pod, cmd, &ap).await {
        Ok(a) => a,
        Err(e) => {
            bridge.send(K8sEvent::TermClosed {
                token,
                error: Some(e.to_string()),
            });
            return;
        }
    };

    let mut stdout = attached.stdout().expect("stdout pedido en AttachParams");
    let mut stdin = attached.stdin().expect("stdin pedido en AttachParams");
    let mut resize_tx = attached.terminal_size().expect("tty pedido en AttachParams");

    let bridge_out = bridge.clone();
    let lector = tokio::spawn(async move {
        let mut buf = [0u8; 8192];
        loop {
            match stdout.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => bridge_out.send(K8sEvent::TermData {
                    token,
                    bytes: buf[..n].to_vec(),
                }),
            }
        }
    });

    let escritor = tokio::spawn(async move {
        loop {
            tokio::select! {
                bytes = stdin_rx.recv() => match bytes {
                    Some(b) => {
                        if stdin.write_all(&b).await.is_err() {
                            break;
                        }
                        let _ = stdin.flush().await;
                    }
                    None => break,
                },
                size = resize_rx.recv() => match size {
                    Some((cols, rows)) => {
                        let _ = resize_tx
                            .send(TerminalSize { width: cols, height: rows })
                            .await;
                    }
                    None => break,
                },
            }
        }
    });

    // El lector termina cuando el proceso remoto cierra su lado del stream.
    let _ = lector.await;
    escritor.abort();
    let _ = attached.join().await;
    bridge.send(K8sEvent::TermClosed { token, error: None });
}
