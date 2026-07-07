//! `tartist` — render an image or GIF animation as terminal art.

use std::fs::File;
use std::io::{BufReader, Write};
use std::path::PathBuf;
use std::thread::sleep;
use std::time::Duration;

use clap::{Parser, ValueEnum};
use image::codecs::gif::GifDecoder;
use image::{AnimationDecoder, DynamicImage};
use terminal_artist::{glyphs, render, ColorMode, Options, Renderer};

#[derive(Parser)]
#[command(name = "tartist", about = "Render images and GIFs as terminal art.")]
struct Cli {
    /// Input image or GIF file.
    input: PathBuf,

    /// Write to a file instead of the terminal (static; first frame for GIFs).
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Output width in terminal cells.
    #[arg(short, long, default_value_t = 80)]
    width: u32,

    /// Output style.
    #[arg(short, long, value_enum, default_value_t = Style::Ascii)]
    renderer: Style,

    /// Color mode.
    #[arg(short, long, value_enum, default_value_t = Color::None)]
    color: Color,

    /// Glyph ramp for the ascii renderer: a preset name or a custom "dark→light" string.
    #[arg(short, long, default_value = "standard")]
    glyphs: String,

    /// Font cell height/width ratio. Defaults to 0.5 (1.0 for the emoji "moon" ramp).
    #[arg(short = 'a', long)]
    char_aspect: Option<f32>,

    /// Braille dot on/off luminance cutoff (0–255).
    #[arg(short, long, default_value_t = 128)]
    threshold: u8,

    /// Invert brightness→glyph mapping.
    #[arg(long)]
    invert: bool,

    /// Disable auto-contrast (brightness stretch).
    #[arg(long)]
    no_contrast: bool,

    /// Disable Floyd–Steinberg dithering (crisp bands instead of halftone texture).
    #[arg(long)]
    no_dither: bool,

    /// Midtone gamma: <1 brightens (sparser art), >1 darkens.
    #[arg(long, default_value_t = 1.0)]
    gamma: f32,

    /// Force darkest-pixel downscaling (auto-detected for flat-color art by default).
    #[arg(long, conflicts_with = "no_lineart")]
    lineart: bool,

    /// Force averaging downscale even for flat-color art.
    #[arg(long)]
    no_lineart: bool,

    /// Render only the first frame of a GIF.
    #[arg(long)]
    once: bool,

    /// Number of animation loops (0 = forever).
    #[arg(short, long, default_value_t = 0)]
    loops: u32,
}

#[derive(Clone, Copy, ValueEnum)]
enum Style {
    Ascii,
    Half,
    Braille,
}

#[derive(Clone, Copy, ValueEnum)]
enum Color {
    None,
    Ansi256,
    Truecolor,
}

fn main() {
    if let Err(e) = run(Cli::parse()) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    let svg = matches!(cli.output.as_ref(), Some(p) if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("svg")));
    let opts = Options {
        width: cli.width,
        renderer: match cli.renderer {
            Style::Ascii => Renderer::Ascii,
            Style::Half => Renderer::HalfBlocks,
            Style::Braille => Renderer::Braille,
        },
        color: match cli.color {
            Color::None => ColorMode::None,
            Color::Ansi256 => ColorMode::Ansi256,
            Color::Truecolor => ColorMode::TrueColor,
        },
        // SVG cells are exactly 8.4×16 px → aspect 0.525 keeps proportions
        // faithful; terminal cells vary, 0.5 is the common ratio.
        char_aspect: cli.char_aspect.unwrap_or(match () {
            _ if cli.glyphs == "moon" => 1.0,
            _ if svg => 8.4 / 16.0,
            _ => 0.5,
        }),
        glyphs: glyphs::resolve(&cli.glyphs),
        invert: cli.invert,
        threshold: cli.threshold,
        auto_contrast: !cli.no_contrast,
        dither: !cli.no_dither,
        gamma: cli.gamma,
        line_art: match (cli.lineart, cli.no_lineart) {
            (true, _) => Some(true),
            (_, true) => Some(false),
            _ => None, // auto-detect
        },
    };

    let is_gif = cli
        .input
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("gif"));

    // GIF playback (skipped for --once, file output, or non-gif inputs).
    if is_gif && !cli.once && cli.output.is_none() {
        let frames = load_gif_frames(&cli.input)?;
        if frames.len() > 1 {
            play(&frames, &opts, cli.loops);
            return Ok(());
        }
    }

    let img = image::open(&cli.input)?;
    // SVG carries color as per-glyph fills; use truecolor so parsing is exact.
    let render_opts = if svg && opts.color != ColorMode::None {
        Options { color: ColorMode::TrueColor, ..opts }
    } else {
        opts
    };
    let art = render(&img, &render_opts);

    match cli.output {
        Some(path) => {
            let bytes = if svg { to_svg(&art) } else { art };
            File::create(&path)?.write_all(bytes.as_bytes())?;
            eprintln!("wrote {}", path.display());
        }
        None => print!("{art}"),
    }
    Ok(())
}

/// Render ASCII-art text (with optional ANSI color) as a monospaced SVG — a
/// scalable image that never wraps and opens in any browser. Uncolored glyphs
/// are black on white; ANSI truecolor becomes per-glyph fills.
fn to_svg(art: &str) -> String {
    let (cw, lh, fs) = (8.4_f32, 16.0_f32, 14.0_f32);
    let rows: Vec<Vec<(Option<[u8; 3]>, String)>> = art.lines().map(parse_ansi_row).collect();
    let cols = rows
        .iter()
        .map(|r| r.iter().map(|(_, t)| t.chars().count()).sum::<usize>())
        .max()
        .unwrap_or(0);
    let w = ((cols as f32) * cw).ceil().max(1.0) as u32;
    let h = (rows.len() as f32 * lh).ceil() as u32 + 8;
    let mut s = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\">\n\
         <rect width=\"100%\" height=\"100%\" fill=\"white\"/>\n\
         <g font-family=\"monospace\" font-size=\"{fs}\" fill=\"black\" xml:space=\"preserve\">\n"
    );
    for (i, runs) in rows.iter().enumerate() {
        let y = (i as f32 + 1.0) * lh;
        let mut col = 0usize;
        for (fill, text) in runs {
            let x = (col as f32 * cw) as u32;
            let esc = text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
            match fill {
                Some([r, g, b]) => s.push_str(&format!(
                    "<text x=\"{x}\" y=\"{y}\" fill=\"#{r:02x}{g:02x}{b:02x}\">{esc}</text>\n"
                )),
                None => s.push_str(&format!("<text x=\"{x}\" y=\"{y}\">{esc}</text>\n")),
            }
            col += text.chars().count();
        }
    }
    s.push_str("</g>\n</svg>\n");
    s
}

/// Split one line into runs of `(fill, text)`, consuming ANSI SGR escapes.
/// Only truecolor (`38;2;r;g;b`) and reset (`0`) are recognized — matching
/// what this tool emits; anything else clears the color.
fn parse_ansi_row(line: &str) -> Vec<(Option<[u8; 3]>, String)> {
    let mut runs: Vec<(Option<[u8; 3]>, String)> = Vec::new();
    let mut cur: Option<[u8; 3]> = None;
    let mut it = line.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\x1b' {
            if it.peek() == Some(&'[') {
                it.next();
            }
            let mut code = String::new();
            for d in it.by_ref() {
                if d == 'm' {
                    break;
                }
                code.push(d);
            }
            cur = parse_fg(&code);
        } else if matches!(runs.last(), Some((f, _)) if *f == cur) {
            runs.last_mut().unwrap().1.push(c);
        } else {
            runs.push((cur, c.to_string()));
        }
    }
    runs
}

fn parse_fg(code: &str) -> Option<[u8; 3]> {
    let p: Vec<&str> = code.split(';').collect();
    if p.len() >= 5 && p[0] == "38" && p[1] == "2" {
        Some([p[2].parse().ok()?, p[3].parse().ok()?, p[4].parse().ok()?])
    } else {
        None
    }
}

/// Decode every GIF frame with its display delay.
fn load_gif_frames(
    path: &PathBuf,
) -> Result<Vec<(DynamicImage, Duration)>, Box<dyn std::error::Error>> {
    let decoder = GifDecoder::new(BufReader::new(File::open(path)?))?;
    let mut out = Vec::new();
    for frame in decoder.into_frames() {
        let frame = frame?;
        let (num, den) = frame.delay().numer_denom_ms();
        let ms = num.checked_div(den).unwrap_or(100) as u64;
        out.push((
            DynamicImage::ImageRgba8(frame.into_buffer()),
            Duration::from_millis(ms.max(20)),
        ));
    }
    Ok(out)
}

/// Play frames in-place, clearing between each. `loops == 0` loops forever.
fn play(frames: &[(DynamicImage, Duration)], opts: &Options, loops: u32) {
    let rendered: Vec<(String, Duration)> =
        frames.iter().map(|(f, d)| (render(f, opts), *d)).collect();
    let mut out = std::io::stdout().lock();
    let mut n = 0;
    loop {
        for (art, delay) in &rendered {
            // Clear screen + cursor home, then draw.
            let _ = write!(out, "\x1b[2J\x1b[H{art}");
            let _ = out.flush();
            sleep(*delay);
        }
        n += 1;
        if loops != 0 && n >= loops {
            break;
        }
    }
}
