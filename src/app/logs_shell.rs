//! Logs y shell de un pod, también resueltos desde un workload.

use super::*;

impl App {
    // -------------------------------------------------------- logs y shell

    pub fn abrir_logs(&mut self, pane_id: u64, key: &str) {
        let Some(obj) = self.objeto_del_pane(pane_id, key) else {
            return;
        };
        self.abrir_logs_de(pane_id, &obj);
    }

    pub(super) fn objeto_del_pane(
        &self,
        pane_id: u64,
        key: &str,
    ) -> Option<kube::api::DynamicObject> {
        self.panes
            .iter()
            .find(|p| p.id == pane_id)?
            .store
            .as_ref()?
            .objeto(key)
            .cloned()
    }

    /// Logs y shell de un workload: se resuelve un pod por su selector y la
    /// respuesta llega como `PodResuelto`.
    pub fn resolver_pod_de(&mut self, pane_id: u64, key: &str, que: k8s::pods::QuePod) {
        let Some(obj) = self.objeto_del_pane(pane_id, key) else {
            return;
        };
        let Some(client) = self.client_del_pane(pane_id) else {
            return;
        };
        let (Some(ns), Some(selector)) = (
            kube::ResourceExt::namespace(&obj),
            k8s::pods::selector_de(&obj),
        ) else {
            self.toast("este recurso no tiene selector de pods", true);
            return;
        };
        let bridge = self.bridge.clone();
        self.rt.spawn(async move {
            k8s::pods::resolver(client, ns, selector, pane_id, que, bridge).await;
        });
    }

    pub fn abrir_logs_de(&mut self, pane_id: u64, obj: &kube::api::DynamicObject) {
        let Some(ns) = kube::ResourceExt::namespace(obj) else {
            return;
        };
        let pod = kube::ResourceExt::name_any(obj);
        let contenedores = contenedores_de(obj);
        crate::auditoria::anotar(
            &self.contexto_del_pane(pane_id),
            "logs",
            "Pod",
            &Some(ns.clone()),
            &pod,
            contenedores.first().cloned(),
            Ok(()),
        );

        if let Some(pane) = self.pane(pane_id) {
            pane.cerrar_bottom();
            let contenedor = contenedores.first().cloned();
            pane.bottom = Some(Bottom::Logs(Box::new(VistaLogs {
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
            })));
        }
        self.reiniciar_logs(pane_id);
    }

    pub fn reiniciar_logs(&mut self, pane_id: u64) {
        let token = self.token();
        let Some(client) = self.client_del_pane(pane_id) else {
            return;
        };
        let bridge = self.bridge.clone();
        let rt = &self.rt;
        let Some(pane) = self.panes.iter_mut().find(|p| p.id == pane_id) else {
            return;
        };
        let Some(Bottom::Logs(v)) = pane.bottom.as_mut() else {
            return;
        };

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
        let Some(obj) = self.objeto_del_pane(pane_id, key) else {
            return;
        };
        self.abrir_shell_de(pane_id, &obj);
    }

    pub fn abrir_shell_de(&mut self, pane_id: u64, obj: &kube::api::DynamicObject) {
        if super::solo_lectura() {
            self.toast("kubo está en modo solo lectura: sin shell", true);
            return;
        }
        let token = self.token();
        let Some(client) = self.client_del_pane(pane_id) else {
            return;
        };
        let Some(ns) = kube::ResourceExt::namespace(obj) else {
            return;
        };
        let pod = kube::ResourceExt::name_any(obj);
        let contenedor = contenedores_de(obj).first().cloned();
        // Una shell dentro de un pod es lo más sensible que hace kubo: queda
        // en la auditoría aunque no mute nada.
        crate::auditoria::anotar(
            &self.contexto_del_pane(pane_id),
            "shell",
            "Pod",
            &Some(ns.clone()),
            &pod,
            contenedor.clone(),
            Ok(()),
        );

        let (stdin_tx, stdin_rx) = tokio::sync::mpsc::unbounded_channel();
        let (resize_tx, resize_rx) = tokio::sync::mpsc::unbounded_channel();
        let bridge = self.bridge.clone();
        let (c, n, p2, cont) = (client, ns.clone(), pod.clone(), contenedor.clone());
        let tarea = self.rt.spawn(async move {
            k8s::exec::run(c, n, p2, cont, stdin_rx, resize_rx, token, bridge).await;
        });

        if let Some(pane) = self.pane(pane_id) {
            pane.cerrar_bottom();
            pane.bottom = Some(Bottom::Term(Box::new(VistaTerm {
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
            })));
        }
    }
}

/// Quita las secuencias de escape ANSI (`ESC [ … m`, `ESC ] … BEL`, etc.).
pub(super) fn sin_ansi(s: &str) -> String {
    if !s.contains('\x1b') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match chars.peek() {
            // CSI: ESC [ parámetros… byte final en 0x40..=0x7e.
            Some('[') => {
                chars.next();
                for c2 in chars.by_ref() {
                    if ('\x40'..='\x7e').contains(&c2) {
                        break;
                    }
                }
            }
            // OSC: ESC ] … hasta BEL o ESC \.
            Some(']') => {
                chars.next();
                while let Some(c2) = chars.next() {
                    if c2 == '\x07' {
                        break;
                    }
                    if c2 == '\x1b' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            // Escapes de dos bytes (ESC c, ESC =, …).
            Some(_) => {
                chars.next();
            }
            None => {}
        }
    }
    out
}

pub(super) fn contenedores_de(obj: &kube::api::DynamicObject) -> Vec<String> {
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

#[cfg(test)]
mod tests_ansi {
    #[test]
    fn quita_ansi_de_logs() {
        assert_eq!(
            super::sin_ansi("\x1b[95m[Nest]\x1b[39m 1  - \x1b[38;5;3mJob\x1b[0m"),
            "[Nest] 1  - Job"
        );
        assert_eq!(super::sin_ansi("sin color"), "sin color");
        assert_eq!(super::sin_ansi("\x1b]0;titulo\x07x"), "x");
    }
}
