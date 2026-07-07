//! Terminal Artist — turn images (and GIF frames) into terminal art.
//!
//! A single [`render`] call maps a decoded image to a string of terminal art
//! using one of the [`Renderer`] modes and an optional [`ColorMode`] of ANSI
//! escapes. Animations are just a sequence of frames: decode each and render it.
//!
//! ```no_run
//! use terminal_artist::{render, Options};
//! let img = image::open("cat.png").unwrap();
//! print!("{}", render(&img, &Options::default()));
//! ```

use image::{imageops::FilterType, DynamicImage, GenericImageView, Rgba};

pub mod glyphs;

const RESET: &str = "\x1b[0m";

/// How color from the source image is encoded into the output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorMode {
    /// No escapes — plain characters only.
    None,
    /// ANSI 256-color palette (6×6×6 cube).
    Ansi256,
    /// 24-bit truecolor (`38;2;r;g;b`).
    TrueColor,
}

/// Output style. Each renderer maps pixels to a different glyph family.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Renderer {
    /// Brightness → glyph ramp (ASCII or any custom/emoji ramp).
    Ascii,
    /// `▀` with fg/bg color — doubles vertical resolution. Always colored.
    HalfBlocks,
    /// 2×4 Braille dot matrix — highest spatial detail.
    Braille,
}

/// Rendering configuration. Sensible defaults via [`Options::default`].
#[derive(Clone, Debug)]
pub struct Options {
    /// Output width in terminal cells.
    pub width: u32,
    pub renderer: Renderer,
    pub color: ColorMode,
    /// Glyph ramp (dark → light) for [`Renderer::Ascii`].
    pub glyphs: String,
    /// Flip brightness → glyph mapping.
    pub invert: bool,
    /// Font cell height/width ratio, corrects for tall monospace cells.
    pub char_aspect: f32,
    /// Luminance cutoff for a Braille dot to turn on (0–255).
    pub threshold: u8,
    /// Stretch the brightness range to full contrast (helps flat/low-contrast photos).
    pub auto_contrast: bool,
    /// Floyd–Steinberg error diffusion — smooth gradients become halftone
    /// texture instead of flat bands. The photo-like look.
    pub dither: bool,
    /// Midtone gamma. `< 1.0` brightens midtones (sparser art on white),
    /// `> 1.0` darkens them. `1.0` = no change.
    pub gamma: f32,
    /// Downscale by darkest-pixel instead of averaging — keeps thin dark
    /// strokes solid. For cartoons, sketches, and line art.
    pub line_art: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            width: 80,
            renderer: Renderer::Ascii,
            color: ColorMode::None,
            glyphs: glyphs::resolve("standard"),
            invert: false,
            char_aspect: 0.5,
            threshold: 128,
            auto_contrast: true,
            dither: true,
            gamma: 1.0,
            line_art: false,
        }
    }
}

/// Render a decoded image to a block of terminal art (trailing newline included).
pub fn render(img: &DynamicImage, opts: &Options) -> String {
    match opts.renderer {
        Renderer::Ascii => render_ascii(img, opts),
        Renderer::HalfBlocks => render_halfblocks(img, opts),
        Renderer::Braille => render_braille(img, opts),
    }
}

fn luminance(p: Rgba<u8>) -> u8 {
    // Rec. 601 luma; alpha treated as opaque.
    (0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32) as u8
}

/// 2nd/98th brightness percentiles of an image — the range to stretch to 0–255.
/// Percentiles (not min/max) so a few stray pixels don't flatten the stretch.
fn contrast_bounds(img: &DynamicImage) -> (f32, f32) {
    let mut hist = [0u32; 256];
    let mut total = 0u32;
    for (_, _, p) in img.pixels() {
        hist[luminance(p) as usize] += 1;
        total += 1;
    }
    let cut = (total as f32 * 0.02) as u32;
    let mut lo = 0usize;
    let mut acc = 0;
    while lo < 255 && acc + hist[lo] < cut {
        acc += hist[lo];
        lo += 1;
    }
    let mut hi = 255usize;
    acc = 0;
    while hi > 0 && acc + hist[hi] < cut {
        acc += hist[hi];
        hi -= 1;
    }
    if hi <= lo {
        (0.0, 255.0)
    } else {
        (lo as f32, hi as f32)
    }
}

/// Rescale a luminance so `lo..hi` maps to `0..255`.
fn stretch(lum: u8, (lo, hi): (f32, f32)) -> u8 {
    (((lum as f32 - lo) / (hi - lo)).clamp(0.0, 1.0) * 255.0) as u8
}

/// Quantize a luminance plane to `levels` steps, optionally diffusing the
/// quantization error to neighbors (Floyd–Steinberg). Dithering is what turns
/// smooth gradients into halftone texture instead of flat character bands.
fn quantize_plane(mut lum: Vec<f32>, w: usize, h: usize, levels: usize, dither: bool) -> Vec<u8> {
    let max = (levels - 1).max(1) as f32;
    let mut out = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let idx = (lum[i].clamp(0.0, 255.0) / 255.0 * max).round();
            out[i] = idx as u8;
            if dither {
                let err = lum[i] - idx / max * 255.0;
                if x + 1 < w {
                    lum[i + 1] += err * 7.0 / 16.0;
                }
                if y + 1 < h {
                    if x > 0 {
                        lum[i + w - 1] += err * 3.0 / 16.0;
                    }
                    lum[i + w] += err * 5.0 / 16.0;
                    if x + 1 < w {
                        lum[i + w + 1] += err * 1.0 / 16.0;
                    }
                }
            }
        }
    }
    out
}

/// Stretched (and optionally inverted) luminance plane of an image.
fn lum_plane(img: &DynamicImage, opts: &Options) -> Vec<f32> {
    let bounds = bounds_for(img, opts);
    img.pixels()
        .map(|(_, _, p)| {
            let mut l = stretch(luminance(p), bounds) as f32;
            if opts.gamma != 1.0 {
                l = 255.0 * (l / 255.0).powf(opts.gamma);
            }
            if opts.invert {
                255.0 - l
            } else {
                l
            }
        })
        .collect()
}

/// Contrast bounds for `img`, or the identity range when auto-contrast is off.
fn bounds_for(img: &DynamicImage, opts: &Options) -> (f32, f32) {
    if opts.auto_contrast {
        contrast_bounds(img)
    } else {
        (0.0, 255.0)
    }
}

/// fg escape for a pixel under the given color mode (empty for `None`).
fn fg(p: Rgba<u8>, mode: ColorMode) -> String {
    match mode {
        ColorMode::None => String::new(),
        ColorMode::Ansi256 => format!("\x1b[38;5;{}m", ansi256(p)),
        ColorMode::TrueColor => format!("\x1b[38;2;{};{};{}m", p[0], p[1], p[2]),
    }
}

fn bg(p: Rgba<u8>, mode: ColorMode) -> String {
    match mode {
        ColorMode::None => String::new(),
        ColorMode::Ansi256 => format!("\x1b[48;5;{}m", ansi256(p)),
        ColorMode::TrueColor => format!("\x1b[48;2;{};{};{}m", p[0], p[1], p[2]),
    }
}

fn ansi256(p: Rgba<u8>) -> u8 {
    let q = |c: u8| c as u16 * 5 / 255;
    (16 + 36 * q(p[0]) + 6 * q(p[1]) + q(p[2])) as u8
}

fn resize(img: &DynamicImage, w: u32, h: u32) -> DynamicImage {
    img.resize_exact(w.max(1), h.max(1), FilterType::Triangle)
}

/// Downscale by keeping the *darkest* source pixel per cell. Averaging washes
/// a thin black stroke out to grey; min-pooling keeps every cell the stroke
/// touches fully dark — the difference between a cartoon face surviving or not.
fn min_pool(img: &DynamicImage, w: u32, h: u32) -> DynamicImage {
    let (iw, ih) = img.dimensions();
    let (w, h) = (w.max(1), h.max(1));
    let mut out = image::RgbaImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let (x0, x1) = (x * iw / w, ((x + 1) * iw / w).clamp(x * iw / w + 1, iw));
            let (y0, y1) = (y * ih / h, ((y + 1) * ih / h).clamp(y * ih / h + 1, ih));
            let mut best = img.get_pixel(x0, y0);
            let mut best_l = luminance(best);
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let p = img.get_pixel(sx, sy);
                    let l = luminance(p);
                    if l < best_l {
                        best_l = l;
                        best = p;
                    }
                }
            }
            out.put_pixel(x, y, best);
        }
    }
    DynamicImage::ImageRgba8(out)
}

/// Pick the downscaler that matches the source: averaging for photos,
/// darkest-pixel for line art.
fn downscale(img: &DynamicImage, w: u32, h: u32, opts: &Options) -> DynamicImage {
    if opts.line_art {
        min_pool(img, w, h)
    } else {
        resize(img, w, h)
    }
}

fn render_ascii(img: &DynamicImage, opts: &Options) -> String {
    let ramp: Vec<char> = opts.glyphs.chars().collect();
    let (iw, ih) = img.dimensions();
    let h = ((opts.width as f32) * (ih as f32 / iw as f32) * opts.char_aspect).round() as u32;
    let small = downscale(img, opts.width, h, opts);
    let (w, hh) = (small.width() as usize, small.height() as usize);
    let idxs = quantize_plane(lum_plane(&small, opts), w, hh, ramp.len(), opts.dither);

    let mut out = String::new();
    for y in 0..hh {
        for x in 0..w {
            let idx = (idxs[y * w + x] as usize).min(ramp.len() - 1);
            out.push_str(&fg(small.get_pixel(x as u32, y as u32), opts.color));
            out.push(ramp[idx]);
        }
        if opts.color != ColorMode::None {
            out.push_str(RESET);
        }
        out.push('\n');
    }
    out
}

fn render_halfblocks(img: &DynamicImage, opts: &Options) -> String {
    // Each cell stacks two vertical pixels: top = fg, bottom = bg.
    let color = if opts.color == ColorMode::None {
        ColorMode::TrueColor // half-blocks are meaningless without color
    } else {
        opts.color
    };
    let (iw, ih) = img.dimensions();
    let rows = ((opts.width as f32) * (ih as f32 / iw as f32) / 2.0).round().max(1.0) as u32;
    let small = resize(img, opts.width, rows * 2);

    let mut out = String::new();
    for r in 0..rows {
        for x in 0..small.width() {
            let top = small.get_pixel(x, r * 2);
            let bot = small.get_pixel(x, r * 2 + 1);
            out.push_str(&fg(top, color));
            out.push_str(&bg(bot, color));
            out.push('▀');
        }
        out.push_str(RESET);
        out.push('\n');
    }
    out
}

// Braille dot bit for a (col, row) within the 2×4 cell. Unicode order:
//   (0,0)=0x01 (1,0)=0x08
//   (0,1)=0x02 (1,1)=0x10
//   (0,2)=0x04 (1,2)=0x20
//   (0,3)=0x40 (1,3)=0x80
const BRAILLE_BITS: [[u8; 2]; 4] = [
    [0x01, 0x08],
    [0x02, 0x10],
    [0x04, 0x20],
    [0x40, 0x80],
];

fn render_braille(img: &DynamicImage, opts: &Options) -> String {
    let (iw, ih) = img.dimensions();
    // Cells are 2 dots wide × 4 tall; dots are ~square when rows = width*aspect/2.
    let cols = opts.width;
    let rows = ((cols as f32) * (ih as f32 / iw as f32) / 2.0).round().max(1.0) as u32;
    let small = downscale(img, cols * 2, rows * 4, opts);
    let (pw, ph) = (small.width() as usize, small.height() as usize);
    // Dithered: quantize dots to on/off with error diffusion (photo-friendly).
    // Undithered: plain threshold cut (crisp, for line art).
    let dots: Vec<bool> = if opts.dither {
        quantize_plane(lum_plane(&small, opts), pw, ph, 2, true)
            .into_iter()
            .map(|v| v == 1)
            .collect()
    } else {
        lum_plane(&small, opts)
            .into_iter()
            .map(|l| l >= opts.threshold as f32)
            .collect()
    };

    let mut out = String::new();
    for cr in 0..rows {
        for cc in 0..cols {
            let mut mask = 0u8;
            let mut rsum = 0u32;
            let mut gsum = 0u32;
            let mut bsum = 0u32;
            let mut on = 0u32;
            for dy in 0..4u32 {
                for dx in 0..2u32 {
                    let (x, y) = (cc * 2 + dx, cr * 4 + dy);
                    let px = small.get_pixel(x, y);
                    if dots[y as usize * pw + x as usize] {
                        mask |= BRAILLE_BITS[dy as usize][dx as usize];
                        rsum += px[0] as u32;
                        gsum += px[1] as u32;
                        bsum += px[2] as u32;
                        on += 1;
                    }
                }
            }
            if opts.color != ColorMode::None && on > 0 {
                let avg = Rgba([
                    (rsum / on) as u8,
                    (gsum / on) as u8,
                    (bsum / on) as u8,
                    255,
                ]);
                out.push_str(&fg(avg, opts.color));
            }
            // 0x2800 is the blank Braille cell; add the dot mask.
            out.push(char::from_u32(0x2800 + mask as u32).unwrap());
        }
        if opts.color != ColorMode::None {
            out.push_str(RESET);
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, RgbaImage};

    fn split(s: &str) -> Vec<String> {
        s.lines().map(str::to_string).collect()
    }

    // A 4×4 image: left half black, right half white.
    fn half_img() -> DynamicImage {
        let mut im = RgbaImage::new(4, 4);
        for y in 0..4 {
            for x in 0..4 {
                let v = if x < 2 { 0 } else { 255 };
                im.put_pixel(x, y, Rgba([v, v, v, 255]));
            }
        }
        DynamicImage::ImageRgba8(im)
    }

    #[test]
    fn ascii_maps_dark_to_first_glyph() {
        let opts = Options { width: 4, char_aspect: 1.0, ..Default::default() };
        let lines = split(&render(&half_img(), &opts));
        // ramp " .:-=+*#%@": black->space, white->@
        assert!(lines[0].starts_with(' '));
        assert!(lines[0].ends_with('@'));
    }

    #[test]
    fn ascii_invert_flips() {
        let opts = Options { width: 4, char_aspect: 1.0, invert: true, ..Default::default() };
        let lines = split(&render(&half_img(), &opts));
        assert!(lines[0].starts_with('@'));
        assert!(lines[0].ends_with(' '));
    }

    #[test]
    fn braille_all_on_is_full_cell() {
        // Solid white, threshold below 255 → every dot on → U+28FF.
        let mut im = RgbaImage::new(2, 4);
        for p in im.pixels_mut() {
            *p = Rgba([255, 255, 255, 255]);
        }
        let opts = Options {
            width: 1,
            renderer: Renderer::Braille,
            threshold: 128,
            ..Default::default()
        };
        let out = render(&DynamicImage::ImageRgba8(im), &opts);
        assert!(out.starts_with('⣿')); // U+28FF, all 8 dots
    }

    #[test]
    fn halfblocks_forces_color() {
        let opts = Options {
            width: 2,
            renderer: Renderer::HalfBlocks,
            color: ColorMode::None,
            ..Default::default()
        };
        let out = render(&half_img(), &opts);
        assert!(out.contains('▀'));
        assert!(out.contains("\x1b[38;2;")); // coerced to truecolor
    }
}
