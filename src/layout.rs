//! Persistencia de lo que estabas mirando: contexto, namespace y recurso de
//! cada panel.
//!
//! Sin esto kubo arranca siempre en el `current-context` del kubeconfig, que
//! no tiene por qué ser el cluster con el que estabas trabajando. Es estado de
//! sesión, no configuración, así que va a `XDG_STATE_HOME`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneGuardado {
    pub contexto: Option<String>,
    pub ns: Option<String>,
    /// Clave `grupo/version/Kind` del recurso seleccionado.
    pub recurso: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Estado {
    pub panes: Vec<PaneGuardado>,
}

fn ruta() -> Option<PathBuf> {
    Some(crate::rutas::estado()?.join("sesion.json"))
}

pub fn cargar() -> Estado {
    let Some(p) = ruta() else {
        return Estado::default();
    };
    std::fs::read(p)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Guarda el estado. Los errores no se propagan: no poder recordar el layout no
/// justifica molestar al usuario.
pub fn guardar(e: &Estado) {
    let Some(p) = ruta() else { return };
    if let Some(dir) = p.parent() {
        if std::fs::create_dir_all(dir).is_err() {
            return;
        }
    }
    let Ok(bytes) = serde_json::to_vec_pretty(e) else {
        return;
    };
    let tmp = p.with_extension("tmp");
    if std::fs::write(&tmp, bytes).is_ok() {
        let _ = std::fs::rename(&tmp, &p);
    }
}
