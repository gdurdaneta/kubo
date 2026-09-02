//! Registro local de las mutaciones que kubo hace sobre un cluster.
//!
//! El API server tiene su propio audit log, pero no siempre se tiene acceso a
//! él ni es cómodo de cruzar. Esto deja del lado del cliente qué se tocó,
//! cuándo y en qué cluster: es lo que convierte un "creo que no fui yo" en un
//! hecho verificable.
//!
//! Formato JSONL, una acción por línea, en append. Nunca se reescribe ni se
//! borra: si el archivo se pierde, se pierde el registro, pero kubo no lo
//! trunca por su cuenta.

use std::io::Write as _;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entrada {
    /// Instante en UTC, ISO 8601.
    pub ts: String,
    pub contexto: String,
    /// borrar | escalar | reiniciar | aplicar
    pub verbo: String,
    pub kind: String,
    pub ns: Option<String>,
    pub name: String,
    /// Detalle de la acción (a cuántas réplicas, por ejemplo).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detalle: Option<String>,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub fn ruta() -> Option<PathBuf> {
    Some(crate::rutas::estado()?.join("acciones.jsonl"))
}

/// Agrega una línea. Los errores no se propagan: no poder auditar no es razón
/// para cancelar una acción que el usuario ya confirmó, pero sí queda en el
/// log de la app.
pub fn registrar(e: &Entrada) {
    let Some(p) = ruta() else { return };
    registrar_en(&p, e);
}

/// Igual que `registrar` pero contra una ruta concreta. Separado para poder
/// probarlo sin depender de variables de entorno, que son globales al proceso
/// y hacen que los tests se pisen entre sí.
fn registrar_en(p: &std::path::Path, e: &Entrada) {
    if let Some(dir) = p.parent() {
        if std::fs::create_dir_all(dir).is_err() {
            return;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        }
    }
    let Ok(mut linea) = serde_json::to_string(e) else {
        return;
    };
    linea.push('\n');

    let mut opts = std::fs::OpenOptions::new();
    opts.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        opts.mode(0o600);
    }
    match opts.open(p).and_then(|mut f| f.write_all(linea.as_bytes())) {
        Ok(()) => tracing::info!(
            verbo = %e.verbo, kind = %e.kind, name = %e.name, ok = e.ok,
            "auditoría: registrada"
        ),
        Err(err) => tracing::warn!(error = %err, "auditoría: no se pudo escribir"),
    }
}

/// Ayuda para armar la entrada desde el resultado de una acción.
pub fn anotar(
    contexto: &str,
    verbo: &str,
    kind: &str,
    ns: &Option<String>,
    name: &str,
    detalle: Option<String>,
    resultado: Result<(), String>,
) {
    registrar(&Entrada {
        ts: k8s_openapi::jiff::Timestamp::now().to_string(),
        contexto: contexto.to_string(),
        verbo: verbo.to_string(),
        kind: kind.to_string(),
        ns: ns.clone(),
        name: name.to_string(),
        detalle,
        ok: resultado.is_ok(),
        error: resultado.err(),
    });
}

/// Últimas `n` entradas, de la más nueva a la más vieja.
pub fn ultimas(n: usize) -> Vec<Entrada> {
    let Some(p) = ruta() else { return Vec::new() };
    ultimas_de(&p, n)
}

fn ultimas_de(p: &std::path::Path, n: usize) -> Vec<Entrada> {
    let Ok(txt) = std::fs::read_to_string(p) else {
        return Vec::new();
    };
    let mut v: Vec<Entrada> = txt
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    v.reverse();
    v.truncate(n);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entrada(name: &str, ok: bool) -> Entrada {
        Entrada {
            ts: "2026-09-01T23:00:00Z".into(),
            contexto: "arn:aws:eks:us-east-2:0:cluster/staging".into(),
            verbo: "borrar".into(),
            kind: "Deployment".into(),
            ns: Some("default".into()),
            name: name.into(),
            detalle: None,
            ok,
            error: (!ok).then(|| "403 Forbidden".to_string()),
        }
    }

    /// Archivo propio por test: nada de variables de entorno, que son globales
    /// al proceso y hacen que los tests en paralelo se pisen.
    fn temporal(nombre: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("kubo-audit-{nombre}/acciones.jsonl"));
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
        p
    }

    #[test]
    fn registra_en_append_y_lee_de_la_mas_nueva_a_la_mas_vieja() {
        let p = temporal("orden");
        registrar_en(&p, &entrada("uno", true));
        registrar_en(&p, &entrada("dos", false));
        registrar_en(&p, &entrada("tres", true));

        let v = ultimas_de(&p, 10);
        assert_eq!(v.len(), 3, "las tres líneas tienen que estar");
        assert_eq!(v[0].name, "tres", "la más nueva va primero");
        assert_eq!(v[2].name, "uno");
        assert!(!v[1].ok);
        assert_eq!(v[1].error.as_deref(), Some("403 Forbidden"));
        // El contexto es lo que hace útil el registro: sin él no se sabe sobre
        // qué cluster se actuó.
        assert!(v[0].contexto.contains("staging"));
        assert_eq!(ultimas_de(&p, 2).len(), 2, "el tope se respeta");
    }

    #[test]
    fn el_archivo_no_queda_legible_por_otros() {
        let p = temporal("permisos");
        registrar_en(&p, &entrada("x", true));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let modo = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
            assert_eq!(modo, 0o600, "el registro dice qué se tocó y dónde");
        }
    }

    #[test]
    fn una_linea_corrupta_no_se_lleva_puesto_el_resto() {
        let p = temporal("corrupta");
        registrar_en(&p, &entrada("buena", true));
        let mut txt = std::fs::read_to_string(&p).unwrap();
        txt.push_str("{esto no es json}\n");
        std::fs::write(&p, txt).unwrap();
        registrar_en(&p, &entrada("otra", true));

        assert_eq!(ultimas_de(&p, 10).len(), 2, "se saltea la línea rota y sigue");
    }
}
