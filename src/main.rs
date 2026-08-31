//! kubo — cliente de escritorio para Kubernetes.
//!
//! El hilo de UI nunca hace I/O: todo el tráfico contra el API server vive en
//! un runtime de tokio y llega por canal.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod hosts;
mod layout;
mod columns;
mod k8s;
mod nav;
mod store;
mod theme;
mod ui;

fn main() -> eframe::Result {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .init();

    let opciones = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("kubo")
            .with_app_id("kubo")
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
