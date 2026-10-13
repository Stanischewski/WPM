//! Programmsymbol, zur Laufzeit gezeichnet (Paket mit Pfeil nach unten).

use eframe::egui::IconData;

pub fn app_icon() -> IconData {
    const SIZE: u32 = 64;
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
            let pixel = if shaft || head || tray {
                [255, 255, 255]
            } else {
                [32, 110, 220]
            };
            rgba.extend_from_slice(&pixel);
            rgba.push((background * 255.0) as u8);
        }
    }
    IconData {
        rgba,
        width: SIZE,
        height: SIZE,
    }
}
