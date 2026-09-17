//! Paleta de comandos (Ctrl+K) y pickers de contexto/namespace.

use super::*;

impl App {
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
            parcial: false,
            token: 0,
            sel: 0,
            query_buscada: String::new(),
            desde_cambio: 0.0,
            tarea: None,
        });
    }

    /// Dispara la búsqueda si el texto se estabilizó (debounce) y tiene
    /// al menos dos caracteres: con uno solo el barrido no filtra nada.
    pub(super) fn quizas_buscar(&mut self, dt: f32, ctx: &egui::Context) {
        let Some(p) = self.palette.as_mut() else {
            return;
        };
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
        p.parcial = false;
        if let Some(tarea) = p.tarea.take() {
            tarea.abort();
        }

        let pane_id = p.pane;
        let token = self.token();
        if let Some(p) = self.palette.as_mut() {
            p.token = token;
        }
        let Some(client) = self.client_del_pane(pane_id) else {
            return;
        };
        let Some(pane) = self.panes.iter().find(|p| p.id == pane_id) else {
            return;
        };
        let ns = pane.ns_sel.clone();
        let Some(cluster) = self.cluster_de(pane) else {
            return;
        };
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
        let tarea = self.rt.spawn(async move {
            crate::k8s::search::buscar(client, recursos, query, ns, token, bridge).await;
        });
        if let Some(p) = self.palette.as_mut() {
            p.tarea = Some(tarea);
        }
    }
}
