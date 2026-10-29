//! Das Programmsymbol (Paket mit Pfeil nach unten) als Pixel, in beliebiger
//! Größe. Nur Standardbibliothek: `build.rs` bindet diese Datei ebenfalls ein
//! und erzeugt daraus `wpm.ico` für die EXE.

/// Kantenlänge der Vorlage, in der die Formen beschrieben sind.
const GRID: f32 = 64.0;
/// Abtastpunkte je Pixel und Richtung, für glatte Kanten in jeder Größe.
const SAMPLES: u32 = 8;

const BLUE: [u8; 3] = [32, 110, 220];
const WHITE: [u8; 3] = [255, 255, 255];
const ORANGE: [u8; 3] = [245, 130, 32];

/// Farbe an einer Stelle der 64 × 64 großen Vorlage; `None` = durchsichtig.
fn color_at(x: f32, y: f32, badge: bool) -> Option<[u8; 3]> {
    if badge {
        // Punkt unten rechts mit weißem Rand, damit er auf jedem
        // Taskleisten-Hintergrund und auch in 16 × 16 Pixeln erkennbar ist.
        let distance = ((x - 47.0).powi(2) + (y - 47.0).powi(2)).sqrt();
        if distance <= 13.0 {
            return Some(ORANGE);
        }
        if distance <= 17.0 {
            return Some(WHITE);
        }
    }
    // Abgerundetes Quadrat als Hintergrund.
    let (cx, cy) = (x.clamp(14.0, 50.0), y.clamp(14.0, 50.0));
    if (x - cx).powi(2) + (y - cy).powi(2) > 12.0 * 12.0 {
        return None;
    }
    // Pfeil (Schaft und Spitze) und die Ablage darunter.
    let shaft = (27.0..37.0).contains(&x) && (12.0..32.0).contains(&y);
    let head = (30.0..46.0).contains(&y) && (x - 32.0).abs() <= 46.0 - y;
    let tray = (49.0..54.0).contains(&y) && (14.0..50.0).contains(&x);
    Some(if shaft || head || tray { WHITE } else { BLUE })
}

/// RGBA-Pixel (nicht vormultipliziert), zeilenweise von oben; `badge`
/// ergänzt den orangen Punkt („Updates verfügbar“).
pub fn rgba(size: u32, badge: bool) -> Vec<u8> {
    let scale = GRID / size as f32;
    let total = SAMPLES * SAMPLES;
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    for py in 0..size {
        for px in 0..size {
            let mut sum = [0u32; 3];
            let mut covered = 0;
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let x = (px as f32 + (sx as f32 + 0.5) / SAMPLES as f32) * scale;
                    let y = (py as f32 + (sy as f32 + 0.5) / SAMPLES as f32) * scale;
                    if let Some(color) = color_at(x, y, badge) {
                        for (sum, channel) in sum.iter_mut().zip(color) {
                            *sum += u32::from(channel);
                        }
                        covered += 1;
                    }
                }
            }
            if covered == 0 {
                pixels.extend_from_slice(&[0, 0, 0, 0]);
            } else {
                pixels.extend(sum.map(|s| ((s + covered / 2) / covered) as u8));
                pixels.push(((covered * 255 + total / 2) / total) as u8);
            }
        }
    }
    pixels
}

/// ICO-Datei mit dem Symbol in den angegebenen Größen (je 32 Bit mit
/// Alphakanal; 256 steht im Verzeichnis als 0).
#[cfg_attr(not(test), allow(dead_code))]
pub fn ico(sizes: &[u32]) -> Vec<u8> {
    let images: Vec<Vec<u8>> = sizes.iter().map(|&size| bitmap(size)).collect();
    let mut file = Vec::new();
    file.extend(0u16.to_le_bytes()); // reserviert
    file.extend(1u16.to_le_bytes()); // Typ: Symbol
    file.extend((sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (&size, image) in sizes.iter().zip(&images) {
        let edge = if size >= 256 { 0 } else { size as u8 };
        file.extend([edge, edge, 0, 0]); // Breite, Höhe, Palette, reserviert
        file.extend(1u16.to_le_bytes()); // Ebenen
        file.extend(32u16.to_le_bytes()); // Bit je Pixel
        file.extend((image.len() as u32).to_le_bytes());
        file.extend(offset.to_le_bytes());
        offset += image.len() as u32;
    }
    for image in images {
        file.extend(image);
    }
    file
}

/// Ein Bild der ICO-Datei: BITMAPINFOHEADER, Farben (BGRA, von unten nach
/// oben) und die 1-Bit-Maske (gesetzt = durchsichtig).
#[cfg_attr(not(test), allow(dead_code))]
fn bitmap(size: u32) -> Vec<u8> {
    let rgba = rgba(size, false);
    let mask_row = size.div_ceil(32) * 4;
    let mut image = Vec::new();
    image.extend(40u32.to_le_bytes());
    image.extend(size.to_le_bytes());
    // Höhe von Farben und Maske zusammen.
    image.extend((2 * size).to_le_bytes());
    image.extend(1u16.to_le_bytes());
    image.extend(32u16.to_le_bytes());
    image.extend(0u32.to_le_bytes()); // BI_RGB
    image.extend((size * size * 4 + mask_row * size).to_le_bytes());
    image.extend([0; 16]); // Auflösung und Farbtabelle: ungenutzt
    for y in (0..size).rev() {
        for x in 0..size {
            let i = ((y * size + x) * 4) as usize;
            image.extend([rgba[i + 2], rgba[i + 1], rgba[i], rgba[i + 3]]);
        }
    }
    for y in (0..size).rev() {
        let mut row = vec![0u8; mask_row as usize];
        for x in 0..size {
            if rgba[((y * size + x) * 4 + 3) as usize] == 0 {
                row[(x / 8) as usize] |= 0x80 >> (x % 8);
            }
        }
        image.extend(row);
    }
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(rgba: &[u8], size: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * size + x) * 4) as usize;
        [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
    }

    #[test]
    fn icon_has_transparent_corners_and_an_opaque_body() {
        for size in [16, 32, 64, 256] {
            let rgba = rgba(size, false);
            assert_eq!(rgba.len(), (size * size * 4) as usize);
            assert_eq!(pixel(&rgba, size, 0, 0)[3], 0, "Ecke bei {size}");
            // Oben links neben dem Pfeil: blauer Hintergrund.
            assert_eq!(
                pixel(&rgba, size, size * 5 / 16, size * 5 / 16),
                [32, 110, 220, 255]
            );
            // Schaft des Pfeils: weiß.
            assert_eq!(
                pixel(&rgba, size, size / 2, size * 3 / 8),
                [255, 255, 255, 255]
            );
        }
        // Der Punkt für verfügbare Updates sitzt unten rechts.
        let badge = rgba(64, true);
        assert_eq!(pixel(&badge, 64, 47, 47), [245, 130, 32, 255]);
        assert_eq!(pixel(&rgba(64, false), 64, 47, 47)[3], 255);
    }

    #[test]
    fn ico_directory_points_to_valid_bitmaps() {
        let sizes = [16, 48, 256];
        let file = ico(&sizes);
        let u16_at = |i: usize| u16::from_le_bytes([file[i], file[i + 1]]);
        let u32_at = |i: usize| u32::from_le_bytes(file[i..i + 4].try_into().unwrap());
        assert_eq!((u16_at(0), u16_at(2), u16_at(4)), (0, 1, 3));
        let mut end = 6 + 16 * sizes.len();
        for (n, &size) in sizes.iter().enumerate() {
            let entry = 6 + 16 * n;
            let edge = if size == 256 { 0 } else { size as u8 };
            assert_eq!(&file[entry..entry + 2], &[edge, edge]);
            assert_eq!(u16_at(entry + 6), 32);
            let (length, offset) = (u32_at(entry + 8) as usize, u32_at(entry + 12) as usize);
            assert_eq!(offset, end, "Bilder liegen lückenlos hintereinander");
            end += length;
            // BITMAPINFOHEADER: 40 Byte, Breite, doppelte Höhe, 32 Bit.
            assert_eq!(u32_at(offset), 40);
            assert_eq!((u32_at(offset + 4), u32_at(offset + 8)), (size, 2 * size));
            assert_eq!(u16_at(offset + 14), 32);
            let mask_row = size.div_ceil(32) * 4;
            assert_eq!(length as u32, 40 + size * size * 4 + mask_row * size);
            // Erste Maskenzeile (unterste Bildzeile): Ecke links unten durchsichtig.
            let mask = offset + 40 + (size * size * 4) as usize;
            assert_eq!(file[mask] & 0x80, 0x80);
        }
        assert_eq!(end, file.len());
    }
}
