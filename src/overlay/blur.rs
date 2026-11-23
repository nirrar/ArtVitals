//! Rozmyte tlo pod wyskakujacym oknem.
//!
//! egui nie ma filtra rozmycia, wiec przy otwarciu okna kopiujemy aktualny wyglad okna
//! konfiguracji z ekranu (jeszcze bez wyskakujacego okna - patrz `win32::capture_window`),
//! zmniejszamy go 4x, rozmywamy kilkoma przebiegami filtra pudelkowego (~rozmycie Gaussa)
//! i wyswietlamy jako teksture pod oknem. Tlo jest "zamrozone" na czas otwarcia okna.

use eframe::egui::{self, Color32, ColorImage, Rect, TextureHandle, TextureOptions, pos2};

const DOWNSCALE: usize = 4;
const BLUR_RADIUS: usize = 3;
const BLUR_PASSES: usize = 3;

#[derive(Default)]
pub struct BackdropBlur {
    texture: Option<TextureHandle>,
    captured: bool,
}

impl BackdropBlur {
    /// Wywolywane w pierwszej klatce po otwarciu okna - na ekranie jest jeszcze poprzednia klatka bez okna.
    pub fn capture(&mut self, ctx: &egui::Context, window_title: &str) {
        self.captured = true;
        self.texture = super::win32::capture_window(window_title)
            .map(|image| ctx.load_texture("settings_backdrop", blur_image(&image), TextureOptions::LINEAR));
        ctx.request_repaint();
    }

    pub fn clear(&mut self) {
        self.texture = None;
        self.captured = false;
    }

    pub fn is_captured(&self) -> bool {
        self.captured
    }

    /// Rysuje rozmyte tlo (lub samo przyciemnienie, gdy zrzut sie nie udal) na warstwie pod oknem.
    pub fn paint(&self, ctx: &egui::Context, tint: Color32) {
        let layer = egui::LayerId::new(egui::Order::Middle, egui::Id::unique("settings_backdrop_layer"));
        let painter = ctx.layer_painter(layer);
        let screen = ctx.content_rect();
        if let Some(texture) = &self.texture {
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            painter.image(texture.id(), screen, uv, Color32::WHITE);
        }
        painter.rect_filled(screen, 0.0, tint);
    }
}

/// Zmniejszenie + wieloprzebiegowe rozmycie pudelkowe.
pub fn blur_image(image: &ColorImage) -> ColorImage {
    let [w, h] = image.size;
    let (sw, sh) = ((w / DOWNSCALE).max(1), (h / DOWNSCALE).max(1));
    let mut pixels = vec![[0f32; 4]; sw * sh];
    for y in 0..sh {
        for x in 0..sw {
            let mut acc = [0f32; 4];
            let mut n: f32 = 0.0;
            for dy in 0..DOWNSCALE {
                for dx in 0..DOWNSCALE {
                    let (px, py) = (x * DOWNSCALE + dx, y * DOWNSCALE + dy);
                    if px < w && py < h {
                        let c = image.pixels[py * w + px];
                        acc[0] += c.r() as f32;
                        acc[1] += c.g() as f32;
                        acc[2] += c.b() as f32;
                        acc[3] += c.a() as f32;
                        n += 1.0;
                    }
                }
            }
            pixels[y * sw + x] = acc.map(|v| v / n.max(1.0));
        }
    }
    let mut scratch = pixels.clone();
    for _ in 0..BLUR_PASSES {
        box_blur_pass(&pixels, &mut scratch, sw, sh, true);
        box_blur_pass(&scratch, &mut pixels, sw, sh, false);
    }
    let out = pixels
        .iter()
        .map(|p| Color32::from_rgb(p[0] as u8, p[1] as u8, p[2] as u8))
        .collect();
    ColorImage::new([sw, sh], out)
}

fn box_blur_pass(src: &[[f32; 4]], dst: &mut [[f32; 4]], w: usize, h: usize, horizontal: bool) {
    let r = BLUR_RADIUS as isize;
    let (len, lines) = if horizontal { (w, h) } else { (h, w) };
    let index = |line: usize, i: usize| if horizontal { line * w + i } else { i * w + line };
    for line in 0..lines {
        for i in 0..len {
            let mut acc = [0f32; 4];
            let mut n: f32 = 0.0;
            for k in -r..=r {
                let j = (i as isize + k).clamp(0, len as isize - 1) as usize;
                let p = src[index(line, j)];
                for c in 0..4 {
                    acc[c] += p[c];
                }
                n += 1.0;
            }
            dst[index(line, i)] = acc.map(|v| v / n);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blur_downscales_and_keeps_uniform_color() {
        let image = ColorImage::new([40, 20], vec![Color32::from_rgb(100, 150, 200); 40 * 20]);
        let blurred = blur_image(&image);
        assert_eq!(blurred.size, [10, 5]);
        assert!(blurred.pixels.iter().all(|c| c.r() == 100 && c.g() == 150 && c.b() == 200));
    }
}
