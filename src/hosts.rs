//! Alias en `/etc/hosts` para consumir un servicio expuesto por su nombre.
//!
//! Escribir ahí necesita root, así que kubo arma el archivo completo y lo copia
//! con `pkexec` (diálogo gráfico del agente polkit). No se usa shell en ningún
//! punto: el nombre del servicio viene del cluster y no tiene por qué ser
//! confiable, así que se valida antes y se pasa como argumento, nunca
//! interpolado en un comando.

use std::io::Write as _;
use std::net::IpAddr;
use std::path::PathBuf;

use anyhow::{anyhow, bail, Context as _, Result};

const RUTA: &str = "/etc/hosts";

/// ¿Se puede resolver un Service por su nombre en esta plataforma?
///
/// Hace falta editar `/etc/hosts` con privilegios y que el sistema enrute todo
/// `127.0.0.0/8`. Linux cumple las dos: `pkexec` da el diálogo gráfico y el
/// rango entero es loopback sin configurar nada. macOS necesitaría un
/// `ifconfig lo0 alias` por cada servicio (solo 127.0.0.1 está enrutado de
/// fábrica) y Windows, elevación por UAC sobre otro archivo. Mientras no estén
/// implementados, el port-forward de esas plataformas escucha en 127.0.0.1.
pub const fn soportado() -> bool {
    cfg!(target_os = "linux")
}

/// Por qué no se puede, para decirlo en la UI en vez de dejar el check muerto.
pub const fn motivo_no_soportado() -> &'static str {
    if cfg!(target_os = "macos") {
        "en macOS haría falta un alias de loopback por servicio (ifconfig lo0 alias)"
    } else if cfg!(target_os = "windows") {
        "en Windows haría falta elevación por UAC para editar el archivo hosts"
    } else {
        "solo implementado en Linux"
    }
}
const INICIO: &str = "# >>> kubo port-forward >>>";
const FIN: &str = "# <<< kubo port-forward <<<";

/// ¿Es un nombre de Service válido (RFC 1035) y por lo tanto seguro de escribir?
///
/// Kubernetes ya obliga a este formato, pero acá es lo único que separa un
/// nombre de una línea inyectada en /etc/hosts.
pub fn nombre_valido(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= 63
        && n.starts_with(|c: char| c.is_ascii_lowercase())
        && n.ends_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && n.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Alias que kubo tiene puestos ahora mismo.
pub fn actuales() -> Vec<(IpAddr, String)> {
    if !soportado() {
        return Vec::new();
    }
    let Ok(txt) = std::fs::read_to_string(RUTA) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for linea in bloque(&txt).lines() {
        let mut campos = linea.split_whitespace();
        let (Some(ip), Some(nombre)) = (campos.next(), campos.next()) else {
            continue;
        };
        if let Ok(ip) = ip.parse::<IpAddr>() {
            out.push((ip, nombre.to_string()));
        }
    }
    out
}

/// Contenido del bloque de kubo dentro de un /etc/hosts.
fn bloque(txt: &str) -> String {
    let mut dentro = false;
    let mut out = String::new();
    for l in txt.lines() {
        if l.trim() == INICIO {
            dentro = true;
        } else if l.trim() == FIN {
            dentro = false;
        } else if dentro {
            out.push_str(l);
            out.push('\n');
        }
    }
    out
}

/// Reescribe el archivo dejando el bloque de kubo con `entradas`.
fn componer(original: &str, entradas: &[(IpAddr, String)]) -> String {
    let mut out = String::new();
    let mut dentro = false;
    for l in original.lines() {
        if l.trim() == INICIO {
            dentro = true;
            continue;
        }
        if l.trim() == FIN {
            dentro = false;
            continue;
        }
        if !dentro {
            out.push_str(l);
            out.push('\n');
        }
    }
    // Sin entradas no queda bloque: kubo no deja rastro cuando no hay forwards.
    if entradas.is_empty() {
        return out;
    }
    while out.ends_with("\n\n") {
        out.pop();
    }
    out.push_str(INICIO);
    out.push('\n');
    for (ip, nombre) in entradas {
        out.push_str(&format!("{ip}\t{nombre}\n"));
    }
    out.push_str(FIN);
    out.push('\n');
    out
}

fn ruta_temporal() -> Result<PathBuf> {
    // En XDG_RUNTIME_DIR (0700, solo el usuario) para que nadie más pueda
    // cambiar el archivo entre que lo escribimos y pkexec lo copia.
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("XDG_RUNTIME_DIR no está definido"))?;
    Ok(dir.join(format!("kubo-hosts-{}", std::process::id())))
}

/// Deja `/etc/hosts` con exactamente estos alias en el bloque de kubo.
///
/// Bloquea mientras el usuario responde el diálogo de polkit: va en el runtime
/// de tokio, nunca en el hilo de la UI.
pub fn aplicar(entradas: &[(IpAddr, String)]) -> Result<()> {
    if !soportado() {
        bail!("resolver por nombre no está soportado acá: {}", motivo_no_soportado());
    }
    for (_, n) in entradas {
        if !nombre_valido(n) {
            bail!("nombre de servicio inválido para /etc/hosts: {n:?}");
        }
    }
    let original = std::fs::read_to_string(RUTA).context("no se pudo leer /etc/hosts")?;
    let nuevo = componer(&original, entradas);
    if nuevo == original {
        return Ok(());
    }

    let tmp = ruta_temporal()?;
    {
        let mut f = std::fs::File::create(&tmp)
            .with_context(|| format!("no se pudo escribir {}", tmp.display()))?;
        f.write_all(nuevo.as_bytes())?;
        f.sync_all()?;
    }

    // `cp` sobre un archivo existente conserva dueño y permisos del destino,
    // así que /etc/hosts sigue siendo root:root 0644.
    let salida = std::process::Command::new("pkexec")
        .arg("/bin/cp")
        .arg(&tmp)
        .arg(RUTA)
        .output();
    let _ = std::fs::remove_file(&tmp);

    let salida = salida.context("no se pudo ejecutar pkexec (¿está instalado?)")?;
    if !salida.status.success() {
        // 126 = el usuario canceló el diálogo, 127 = no se pudo autorizar.
        let code = salida.status.code().unwrap_or(-1);
        let err = String::from_utf8_lossy(&salida.stderr).trim().to_string();
        if code == 126 {
            bail!("autorización cancelada: el alias no se agregó");
        }
        bail!("pkexec falló ({code}): {err}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn ip(a: u8, b: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(127, 30, a, b))
    }

    #[test]
    fn rechaza_nombres_que_podrian_inyectar_lineas() {
        assert!(nombre_valido("agent-ops"));
        assert!(nombre_valido("billing-api-2"));
        assert!(!nombre_valido(""));
        assert!(!nombre_valido("-arranca-con-guion"));
        assert!(!nombre_valido("Mayus"));
        assert!(!nombre_valido("con espacio"));
        assert!(!nombre_valido("dos\nlineas"));
        assert!(!nombre_valido("con\ttab"));
        assert!(!nombre_valido("punto.com"));
        assert!(!nombre_valido(&"x".repeat(64)));
    }

    #[test]
    fn compone_sin_tocar_el_resto_del_archivo() {
        let original = "127.0.0.1\tlocalhost\n::1\tip6-localhost\n";
        let con = componer(original, &[(ip(14, 7), "agent-ops".into())]);
        assert!(con.contains("127.0.0.1\tlocalhost"));
        assert!(con.contains("::1\tip6-localhost"));
        assert!(con.contains("127.30.14.7\tagent-ops"));

        // Reaplicar reemplaza el bloque en vez de acumularlo.
        let dos = componer(&con, &[(ip(20, 3), "billing".into())]);
        assert!(!dos.contains("agent-ops"), "el bloque viejo debe irse");
        assert!(dos.contains("127.30.20.3\tbilling"));
        assert_eq!(dos.matches(INICIO).count(), 1);

        // Sin entradas vuelve al archivo original, sin marcadores.
        let vacio = componer(&dos, &[]);
        assert_eq!(vacio, original);
    }

    #[test]
    fn lee_de_vuelta_lo_que_escribio() {
        let txt = componer(
            "127.0.0.1\tlocalhost\n",
            &[(ip(1, 2), "uno".into()), (ip(3, 4), "dos".into())],
        );
        let leidas: Vec<_> = bloque(&txt)
            .lines()
            .map(|l| {
                let mut c = l.split_whitespace();
                (c.next().unwrap().to_string(), c.next().unwrap().to_string())
            })
            .collect();
        assert_eq!(
            leidas,
            vec![
                ("127.30.1.2".to_string(), "uno".to_string()),
                ("127.30.3.4".to_string(), "dos".to_string())
            ]
        );
    }
}
