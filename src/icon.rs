//! Programmsymbol, zur Laufzeit gezeichnet (Paket mit Pfeil nach unten).

use eframe::egui::IconData;

const SIZE: u32 = 64;

pub fn app_icon() -> IconData {
    IconData {
        rgba: pixels(false),
        width: SIZE,
        height: SIZE,
    }
}

/// Symbol für den Infobereich; `badge` ergänzt einen orangen Punkt
/// („Updates verfügbar“). Liefert RGBA-Daten und die Kantenlänge.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn tray_icon(badge: bool) -> (Vec<u8>, u32) {
    (pixels(badge), SIZE)
}

fn pixels(badge: bool) -> Vec<u8> {
    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (x, y) = (x as f32 + 0.5, y as f32 + 0.5);
            // Abgerundetes Quadrat als Hintergrund.
            let (cx, cy) = (x.clamp(14.0, 50.0), y.clamp(14.0, 50.0));
            let corner = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
            let background = (12.0 - corner + 0.5).clamp(0.0, 1.0);
            // Pfeil: Schaft und Spitze.
            let shaft = (27.0..37.0).contains(&x) && (12.0..32.0).contains(&y);
            let head = (30.0..46.0).contains(&y) && (x - 32.0).abs() <= 46.0 - y;
            let tray = (49.0..54.0).contains(&y) && (14.0..50.0).contains(&x);
            let mut pixel = if shaft || head || tray {
                [255, 255, 255]
            } else {
                [32, 110, 220]
            };
            let mut alpha = background;
            if badge {
                // Punkt unten rechts mit weißem Rand, damit er auf jedem
                // Taskleisten-Hintergrund und auch in 16 × 16 Pixeln erkennbar ist.
                let distance = ((x - 47.0).powi(2) + (y - 47.0).powi(2)).sqrt();
                if distance <= 17.0 {
                    pixel = if distance <= 13.0 {
                        [245, 130, 32]
                    } else {
                        [255, 255, 255]
                    };
                    alpha = (17.0 - distance + 0.5).clamp(0.0, 1.0).max(background);
                }
            }
            rgba.extend_from_slice(&pixel);
            rgba.push((alpha * 255.0) as u8);
        }
    }
    rgba
}
