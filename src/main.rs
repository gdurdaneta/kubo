//! kubo — cliente de escritorio para Kubernetes.
//!
//! El hilo de UI nunca hace I/O: todo el tráfico contra el API server vive en
//! un runtime de tokio y llega por canal.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod hosts;
mod layout;
mod rutas;
mod columns;
mod k8s;
mod nav;
mod store;
mod theme;
mod ui;

/// Ícono de la ventana, para la barra de tareas y el alt-tab.
///
/// Va incrustado en el binario: buscarlo en disco obligaría a instalarlo en
/// alguna ruta del sistema y los binarios sueltos no pasan por ahí.
fn icono() -> egui::IconData {
    const PNG: &[u8] = include_bytes!("../assets/iconset/kubo-256.png");
    let vacio = egui::IconData {
        rgba: Vec::new(),
        width: 0,
        height: 0,
    };
    // png 0.18 pide BufRead + Seek: un Cursor sobre el slice alcanza.
    let decodificador = png::Decoder::new(std::io::Cursor::new(PNG));
    let Ok(mut lector) = decodificador.read_info() else {
        return vacio;
    };
    let mut buf = vec![0; lector.output_buffer_size().unwrap_or(0)];
    let Ok(info) = lector.next_frame(&mut buf) else {
        return vacio;
    };
    buf.truncate(info.buffer_size());
    egui::IconData {
        rgba: buf,
        width: info.width,
        height: info.height,
    }
}

fn main() -> eframe::Result {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .init();

    let opciones = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("kubo")
            .with_app_id("kubo")
            .with_icon(icono())
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([900.0, 560.0]),
        ..Default::default()
    };

    eframe::run_native(
        "kubo",
        opciones,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
