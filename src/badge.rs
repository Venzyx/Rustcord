//! Runtime-rendered taskbar overlay badge and tray icon variants.

use tauri::image::Image;

const RED: [u8; 3] = [0xF2, 0x3F, 0x43];

// 3x5 pixel glyphs, one row per u8 (low 3 bits, MSB = left column).
const GLYPHS: [(char, [u8; 5]); 11] = [
    ('0', [0b111, 0b101, 0b101, 0b101, 0b111]),
    ('1', [0b010, 0b110, 0b010, 0b010, 0b111]),
    ('2', [0b111, 0b001, 0b111, 0b100, 0b111]),
    ('3', [0b111, 0b001, 0b011, 0b001, 0b111]),
    ('4', [0b101, 0b101, 0b111, 0b001, 0b001]),
    ('5', [0b111, 0b100, 0b111, 0b001, 0b111]),
    ('6', [0b111, 0b100, 0b111, 0b101, 0b111]),
    ('7', [0b111, 0b001, 0b010, 0b010, 0b010]),
    ('8', [0b111, 0b101, 0b111, 0b101, 0b111]),
    ('9', [0b111, 0b101, 0b111, 0b001, 0b111]),
    ('+', [0b000, 0b010, 0b111, 0b010, 0b000]),
];

fn glyph(c: char) -> [u8; 5] {
    GLYPHS.iter().find(|(g, _)| *g == c).map(|(_, b)| *b).unwrap_or([0; 5])
}

/// Anti-aliased filled circle blended over an RGBA buffer.
fn fill_circle(buf: &mut [u8], size: u32, cx: f32, cy: f32, r: f32, rgb: [u8; 3]) {
    const SS: u32 = 4;
    for y in 0..size {
        for x in 0..size {
            let mut cover = 0u32;
            for sy in 0..SS {
                for sx in 0..SS {
                    let px = x as f32 + (sx as f32 + 0.5) / SS as f32 - cx;
                    let py = y as f32 + (sy as f32 + 0.5) / SS as f32 - cy;
                    if px * px + py * py <= r * r {
                        cover += 1;
                    }
                }
            }
            if cover == 0 {
                continue;
            }
            let a = cover as f32 / (SS * SS) as f32;
            let i = ((y * size + x) * 4) as usize;
            let dst_a = buf[i + 3] as f32 / 255.0;
            let out_a = a + dst_a * (1.0 - a);
            for c in 0..3 {
                let d = buf[i + c] as f32 * dst_a * (1.0 - a);
                buf[i + c] = ((rgb[c] as f32 * a + d) / out_a).round() as u8;
            }
            buf[i + 3] = (out_a * 255.0).round() as u8;
        }
    }
}

/// 32x32 red badge with the mention count ("9+" above nine).
pub fn overlay(mentions: u32) -> Image<'static> {
    const N: u32 = 32;
    let mut buf = vec![0u8; (N * N * 4) as usize];
    fill_circle(&mut buf, N, 16.0, 16.0, 15.5, RED);
    let text: Vec<char> = if mentions > 9 { vec!['9', '+'] } else { mentions.to_string().chars().collect() };
    let scale: u32 = if text.len() == 1 { 4 } else { 3 };
    let width = text.len() as u32 * 3 * scale + (text.len() as u32 - 1) * scale;
    let (ox, oy) = ((N - width) / 2, (N - 5 * scale) / 2);
    for (k, c) in text.iter().enumerate() {
        let rows = glyph(*c);
        let gx = ox + k as u32 * 4 * scale;
        for (ry, row) in rows.iter().enumerate() {
            for col in 0..3 {
                if row & (0b100 >> col) == 0 {
                    continue;
                }
                for dy in 0..scale {
                    for dx in 0..scale {
                        let x = gx + col * scale + dx;
                        let y = oy + ry as u32 * scale + dy;
                        let i = ((y * N + x) * 4) as usize;
                        buf[i..i + 4].copy_from_slice(&[255, 255, 255, 255]);
                    }
                }
            }
        }
    }
    Image::new_owned(buf, N, N)
}

/// Base tray icon with a red dot in the bottom-right corner.
pub fn tray_with_dot(base: &Image<'_>) -> Image<'static> {
    let (w, h) = (base.width(), base.height());
    let mut buf = base.rgba().to_vec();
    let size = w.min(h);
    let r = size as f32 * 0.22;
    // Clear a ring around the dot so it reads on any taskbar color.
    let (cx, cy) = (w as f32 - r - 0.5, h as f32 - r - 0.5);
    for y in 0..h {
        for x in 0..w {
            let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
            if dx * dx + dy * dy <= (r + size as f32 * 0.06).powi(2) {
                let i = ((y * w + x) * 4) as usize;
                buf[i + 3] = 0;
            }
        }
    }
    fill_circle(&mut buf, w, cx, cy, r, RED);
    Image::new_owned(buf, w, h)
}
