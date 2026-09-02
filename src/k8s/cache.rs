//! Caché en disco del discovery, con la misma idea que `~/.kube/cache/discovery`
//! de kubectl.
//!
//! Enumerar la API de un cluster cuesta un par de segundos contra un server
//! remoto y el resultado casi nunca cambia entre sesiones. Guardarlo permite
//! pintar la navegación al instante y refrescar por detrás.

use std::hash::{Hash as _, Hasher as _};
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use super::Discovered;

/// Pasado este tiempo la copia se sigue usando para arrancar, pero el refresco
/// de fondo es el que manda.
const VIGENCIA: Duration = Duration::from_secs(60 * 60 * 24);

fn ruta(server: &str) -> Option<PathBuf> {
    let base = crate::rutas::cache()?;
    // El URL del server no sirve como nombre de archivo; el hash sí, y además
    // no deja rastro del endpoint en el nombre.
    let mut h = std::collections::hash_map::DefaultHasher::new();
    server.hash(&mut h);
    Some(base.join("discovery").join(format!("{:016x}.json", h.finish())))
}

/// Copia guardada, si existe y se puede parsear. `true` en el segundo campo
/// cuando ya venció y conviene avisar que se está refrescando.
pub fn leer(server: &str) -> Option<(Vec<Discovered>, bool)> {
    let p = ruta(server)?;
    let meta = std::fs::metadata(&p).ok()?;
    let vencida = meta
        .modified()
        .ok()
        .and_then(|m| SystemTime::now().duration_since(m).ok())
        .map(|edad| edad > VIGENCIA)
        .unwrap_or(true);
    let bytes = std::fs::read(&p).ok()?;
    let recursos: Vec<Discovered> = serde_json::from_slice(&bytes).ok()?;
    if recursos.is_empty() {
        return None;
    }
    Some((recursos, vencida))
}

/// Guarda la copia. Los errores no se propagan: sin caché la app anda igual,
/// solo más lenta.
pub fn escribir(server: &str, recursos: &[Discovered]) {
    let Some(p) = ruta(server) else { return };
    let Ok(bytes) = serde_json::to_vec(recursos) else {
        return;
    };
    let _ = crate::rutas::escribir_privado(&p, &bytes);
}
