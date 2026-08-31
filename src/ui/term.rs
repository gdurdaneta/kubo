//! Terminal embebida: renderiza la pantalla de vt100 y traduce el teclado a
//! bytes para el PTY remoto.

use egui::text::LayoutJob;
use egui::{Color32, EventFilter, FontId, TextFormat};

use super::Accion;
use crate::app::{App, Bottom};
use crate::theme;

const FONT: f32 = 13.0;

pub fn dibujar(app: &mut App, ui: &mut egui::Ui, id: u64, accion: &mut Accion) {
    let Some(pane) = app.panes.iter_mut().find(|p| p.id == id) else {
        return;
    };
    let Some(Bottom::Term(v)) = pane.bottom.as_mut() else { return };

    ui.horizontal(|ui| {
        ui.colored_label(theme::ACENTO, "❯");
        ui.label(egui::RichText::new(&v.pod).strong());
        ui.colored_label(theme::TEXTO_TENUE, &v.ns);
        if let Some(c) = &v.contenedor {
            ui.colored_label(theme::TEXTO_TENUE, c);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("×").on_hover_text("Cerrar shell").clicked() {
                *accion = Accion::CerrarBottom(id);
            }
            if let Some(motivo) = &v.cerrado {
                ui.colored_label(theme::WARN, motivo);
            }
        });
    });
    ui.add_space(2.0);

    let font = FontId::monospace(FONT);
    let (ancho_char, alto_fila) = ui.fonts_mut(|f| {
        (f.glyph_width(&font, 'M'), f.row_height(&font))
    });

    let disponible = ui.available_size();
    let cols = ((disponible.x - 4.0) / ancho_char).floor().max(20.0) as u16;
    let rows = ((disponible.y - 4.0) / alto_fila).floor().max(5.0) as u16;
    if (cols, rows) != (v.cols, v.rows) {
        v.cols = cols;
        v.rows = rows;
        v.parser.screen_mut().set_size(rows, cols);
        let _ = v.handles.resize.send((cols, rows));
    }

    // Zona interactiva que cubre todo el terminal.
    let (rect, resp) = ui.allocate_exact_size(disponible, egui::Sense::click());
    if resp.clicked() {
        resp.request_focus();
    }
    let focus = resp.has_focus();
    if focus {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                resp.id,
                EventFilter {
                    tab: true,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: true,
                },
            )
        });
        capturar_teclado(ui, v);
    }

    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, Color32::from_rgb(0x10, 0x12, 0x16));
    let screen = v.parser.screen();
    let origen = rect.min + egui::vec2(2.0, 2.0);

    for fila in 0..rows.min(screen.size().0) {
        let mut job = LayoutJob::default();
        for col in 0..cols.min(screen.size().1) {
            let Some(cell) = screen.cell(fila, col) else { continue };
            let mut fg = color_vt(cell.fgcolor(), theme::TEXTO);
            let mut bg = color_vt(cell.bgcolor(), Color32::TRANSPARENT);
            if cell.inverse() {
                std::mem::swap(&mut fg, &mut bg);
                if bg == Color32::TRANSPARENT {
                    bg = theme::TEXTO;
                }
                if fg == Color32::TRANSPARENT {
                    fg = Color32::from_rgb(0x10, 0x12, 0x16);
                }
            }
            let contenido = cell.contents();
            let texto = if contenido.is_empty() { " " } else { contenido };
            job.append(
                texto,
                0.0,
                TextFormat {
                    font_id: font.clone(),
                    color: fg,
                    background: bg,
                    italics: cell.italic(),
                    ..Default::default()
                },
            );
        }
        let galley = painter.layout_job(job);
        painter.galley(
            origen + egui::vec2(0.0, fila as f32 * alto_fila),
            galley,
            theme::TEXTO,
        );
    }

    // Cursor (bloque). Con foco: sólido; sin foco: solo borde.
    if !screen.hide_cursor() {
        let (cf, cc) = screen.cursor_position();
        let pos = origen + egui::vec2(cc as f32 * ancho_char, cf as f32 * alto_fila);
        let r = egui::Rect::from_min_size(pos, egui::vec2(ancho_char, alto_fila));
        if focus {
            painter.rect_filled(r, 0.0, theme::ACENTO.linear_multiply(0.7));
        } else {
            painter.rect_stroke(
                r,
                0.0,
                egui::Stroke::new(1.0, theme::ACENTO),
                egui::StrokeKind::Inside,
            );
        }
    }

    if !focus {
        // Aviso sutil para que quede claro que hay que clickear.
        painter.text(
            rect.right_top() + egui::vec2(-6.0, 4.0),
            egui::Align2::RIGHT_TOP,
            "click para escribir",
            FontId::proportional(10.5),
            theme::TEXTO_TENUE,
        );
    }
}

/// Traduce los eventos de egui a bytes de terminal.
fn capturar_teclado(ui: &mut egui::Ui, v: &mut crate::app::VistaTerm) {
    use egui::{Event, Key};
    let eventos = ui.input(|i| i.events.clone());
    let mut bytes: Vec<u8> = Vec::new();

    for ev in eventos {
        match ev {
            Event::Text(t) => bytes.extend_from_slice(t.as_bytes()),
            Event::Paste(t) => bytes.extend_from_slice(t.as_bytes()),
            Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => {
                // Ctrl+letra no genera Event::Text: se traduce a mano.
                if modifiers.ctrl && !modifiers.alt {
                    if let Some(c) = letra(key) {
                        bytes.push(c & 0x1f);
                        continue;
                    }
                }
                let seq: &[u8] = match key {
                    Key::Enter => b"\r",
                    Key::Backspace => b"\x7f",
                    Key::Tab => b"\t",
                    Key::Escape => b"\x1b",
                    Key::ArrowUp => b"\x1b[A",
                    Key::ArrowDown => b"\x1b[B",
                    Key::ArrowRight => b"\x1b[C",
                    Key::ArrowLeft => b"\x1b[D",
                    Key::Home => b"\x1b[H",
                    Key::End => b"\x1b[F",
                    Key::PageUp => b"\x1b[5~",
                    Key::PageDown => b"\x1b[6~",
                    Key::Delete => b"\x1b[3~",
                    Key::Insert => b"\x1b[2~",
                    _ => b"",
                };
                bytes.extend_from_slice(seq);
            }
            _ => {}
        }
    }

    if !bytes.is_empty() {
        let _ = v.handles.stdin.send(bytes);
    }
}

fn letra(key: egui::Key) -> Option<u8> {
    use egui::Key::*;
    Some(match key {
        A => b'a', B => b'b', C => b'c', D => b'd', E => b'e', F => b'f',
        G => b'g', H => b'h', I => b'i', J => b'j', K => b'k', L => b'l',
        M => b'm', N => b'n', O => b'o', P => b'p', Q => b'q', R => b'r',
        S => b's', T => b't', U => b'u', V => b'v', W => b'w', X => b'x',
        Y => b'y', Z => b'z',
        _ => return None,
    })
}

/// Colores vt100 → egui, con la paleta estándar de xterm.
fn color_vt(c: vt100::Color, defecto: Color32) -> Color32 {
    match c {
        vt100::Color::Default => defecto,
        vt100::Color::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
        vt100::Color::Idx(i) => idx_color(i),
    }
}

fn idx_color(i: u8) -> Color32 {
    const BASE: [(u8, u8, u8); 16] = [
        (0x1c, 0x1f, 0x26), (0xe5, 0x63, 0x63), (0x4c, 0xc3, 0x8a), (0xe0, 0xa6, 0x3a),
        (0x3d, 0x90, 0xf0), (0xb0, 0x7f, 0xd8), (0x3e, 0xc5, 0xc7), (0xdc, 0xe1, 0xe8),
        (0x55, 0x5b, 0x66), (0xf0, 0x87, 0x87), (0x7d, 0xd8, 0xa8), (0xf0, 0xc6, 0x74),
        (0x74, 0xb2, 0xf5), (0xd0, 0xa5, 0xe8), (0x7f, 0xdb, 0xdd), (0xff, 0xff, 0xff),
    ];
    match i {
        0..=15 => {
            let (r, g, b) = BASE[i as usize];
            Color32::from_rgb(r, g, b)
        }
        16..=231 => {
            // Cubo 6×6×6.
            let i = i - 16;
            let esc = |n: u8| if n == 0 { 0 } else { 55 + n * 40 };
            Color32::from_rgb(esc(i / 36), esc((i % 36) / 6), esc(i % 6))
        }
        _ => {
            // Escala de grises.
            let v = 8 + (i - 232) * 10;
            Color32::from_rgb(v, v, v)
        }
    }
}
