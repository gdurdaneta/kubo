//! Mete el ícono en el .exe de Windows. En el resto de los sistemas no hace
//! nada: macOS lo toma del bundle .app y Linux del .desktop o del propio
//! ejecutable en runtime.
fn main() {
    println!("cargo:rerun-if-changed=assets/kubo.ico");
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/kubo.ico");
        if let Err(e) = res.compile() {
            // Sin ícono el binario sirve igual: no vale romper el build.
            println!("cargo:warning=no se pudo incrustar el ícono: {e}");
        }
    }
}
