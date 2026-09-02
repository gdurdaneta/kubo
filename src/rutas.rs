//! Dónde guarda kubo sus cosas en cada sistema.
//!
//! Linux sigue la spec XDG; macOS y Windows tienen sus propias convenciones y
//! meter `~/.cache` en un Mac queda fuera de lugar.

use std::path::PathBuf;

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// Caché descartable: se puede borrar y la app solo va más lenta.
pub fn cache() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        Some(home()?.join("Library/Caches/kubo"))
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(|| home().map(|h| h.join("AppData/Local")))
            .map(|b| b.join("kubo/cache"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| home().map(|h| h.join(".cache")))
            .map(|b| b.join("kubo"))
    }
}

/// Estado de sesión: qué panel miraba qué. Se pierde y no pasa nada grave.
pub fn estado() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        Some(home()?.join("Library/Application Support/kubo"))
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(|| home().map(|h| h.join("AppData/Local")))
            .map(|b| b.join("kubo/state"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| home().map(|h| h.join(".local/state")))
            .map(|b| b.join("kubo"))
    }
}

/// Fuentes del sistema con los glifos que kubo dibuja (●▼×↻⇄✎⌨) y que las
/// fuentes por defecto de egui no traen. Se prueba en orden y se usa la
/// primera que exista.
pub const FUENTES_PROPORCIONALES: &[&str] = &[
    // Linux
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
    // macOS
    "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
    "/Library/Fonts/Arial Unicode.ttf",
    "/System/Library/Fonts/Apple Symbols.ttf",
    // Windows
    "C:\\Windows\\Fonts\\seguisym.ttf",
    "C:\\Windows\\Fonts\\arialuni.ttf",
];

pub const FUENTES_MONO: &[&str] = &[
    "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
    "/usr/share/fonts/dejavu/DejaVuSansMono.ttf",
    "/usr/share/fonts/TTF/DejaVuSansMono.ttf",
    "/System/Library/Fonts/Menlo.ttc",
    "/System/Library/Fonts/SFNSMono.ttf",
    "C:\\Windows\\Fonts\\consola.ttf",
];

/// Primera ruta de la lista que exista.
pub fn primera_existente(rutas: &[&str]) -> Option<Vec<u8>> {
    rutas.iter().find_map(|r| std::fs::read(r).ok())
}

/// Crea el directorio y deja el archivo accesible solo por su dueño.
///
/// Lo que kubo guarda no son credenciales, pero sí dice bastante: la sesión
/// lleva el nombre del contexto —que en EKS es un ARN con el ID de cuenta— y
/// la caché enumera toda la API del cluster, operadores incluidos. Con el
/// umask habitual eso quedaba legible por cualquier usuario de la máquina.
pub fn escribir_privado(destino: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = destino.parent() {
        std::fs::create_dir_all(dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        }
    }
    // Escritura atómica: un archivo a medias rompería el arranque siguiente.
    let tmp = destino.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, destino)
}
