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

/// `--version` y `--help` salen sin abrir ventana.
///
/// Además de ser lo que uno espera de un binario, permite comprobar en CI que
/// el ejecutable arranca en Windows y macOS —que se resuelven sus librerías y
/// no le falta ningún símbolo— sin necesidad de un escritorio.
fn atajo_de_linea_de_comandos() -> bool {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version" | "-V") => {
            println!("kubo {}", env!("CARGO_PKG_VERSION"));
            true
        }
        Some("--help" | "-h") => {
            println!(
                "kubo {} — cliente de escritorio para Kubernetes\n\
                 \n\
                 Uso: kubo [opciones]\n\
                 \n\
                 Sin argumentos abre la ventana. Los clusters salen de tu\n\
                 kubeconfig (KUBECONFIG o ~/.kube/config).\n\
                 \n\
                 Opciones:\n\
                 \x20 -V, --version   Versión y salir\n\
                 \x20 -h, --help      Esta ayuda\n\
                 \n\
                 Dentro de la app, F1 muestra los atajos de teclado.",
                env!("CARGO_PKG_VERSION")
            );
            true
        }
        _ => false,
    }
}

/// Tamaño inicial de la ventana. `KUBO_TEST_SIZE=1046x894` lo fija, para
/// reproducir problemas de layout a un ancho concreto sin pelearse con el
/// gestor de ventanas.
fn tamano_inicial() -> [f32; 2] {
    std::env::var("KUBO_TEST_SIZE")
        .ok()
        .and_then(|v| {
            let (a, b) = v.split_once(['x', 'X'])?;
            Some([a.trim().parse().ok()?, b.trim().parse().ok()?])
        })
        .unwrap_or([1440.0, 900.0])
}

fn main() -> eframe::Result {
    if atajo_de_linea_de_comandos() {
        return Ok(());
    }

    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .init();

    let opciones = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("kubo")
            .with_app_id("kubo")
            .with_icon(icono())
            .with_inner_size(tamano_inicial())
            .with_min_inner_size([900.0, 560.0]),
        ..Default::default()
    };

    eframe::run_native(
        "kubo",
        opciones,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
