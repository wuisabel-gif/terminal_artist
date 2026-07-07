# How it works

Every renderer is the same five-stage pipeline; only the last stage differs.

```
decode → downscale → luminance → tone mapping → quantize (dither) → glyphs + color
         (avg | min-pool)        (contrast, gamma, invert)
```

## 1. Downscale — and why there are two ways to do it

A terminal cell covers a block of source pixels, so the image is first shrunk
to the output grid. **How** you shrink decides what survives:

- **Averaging** (triangle filter) — each cell is the mean of its pixels.
  Correct for photos: smooth tones stay smooth.
- **Min-pool** (`--lineart`) — each cell keeps its *darkest* pixel.
  Correct for cartoons and sketches: a 2-px black stroke inside a 10-px cell
  averages out to ~80% grey and vanishes, but min-pooling keeps every cell the
  stroke touches fully dark. This is the difference between Charlie Brown
  having a face or not.

The mode is auto-detected. Flat-color art concentrates its luminance histogram
into a few spikes; photos spread smoothly. We bucket luminance into 32 coarse
bins and check whether the top 4 bins hold over 75% of pixels:

| image | top-4 bin share | detected as |
|---|---|---|
| Charlie Brown (cartoon) | 0.92 | line art → min-pool |
| puppy photo | 0.54 | photo → averaging |
| cosmos flower photo | 0.34 | photo → averaging |

Height is `width × (img_h/img_w) × char_aspect`. Terminal cells are roughly
twice as tall as wide, so `char_aspect` defaults to 0.5 to keep circles round.

## 2. Luminance

Each pixel becomes a brightness via Rec. 601 luma, weighted for human
perception (green counts most):

```
Y = 0.299 R + 0.587 G + 0.114 B
```

## 3. Tone mapping

Three adjustments, in order:

- **Auto-contrast** — stretch the 2nd–98th luminance percentiles to the full
  0–255 range. Percentiles rather than min/max so a few stray dark or bright
  pixels can't flatten the stretch. This is why a cream puppy on pale grass
  still separates instead of collapsing into one grey band.
- **Gamma** (`--gamma`) — `Y' = 255·(Y/255)^γ`. γ < 1 brightens midtones
  (backgrounds fall to blank, art gets sparser — the "stipple on white" look);
  γ > 1 darkens.
- **Invert** (`--invert`) — `Y' = 255 − Y`. The glyph ramp maps dark→sparse by
  default, which is right for dark terminals (bright areas get dense, glowing
  characters). On paper or a white README, ink works the other way, so you
  invert.

## 4. Quantize with Floyd–Steinberg dithering

A ramp of *k* glyphs gives only *k* brightness levels. Rounding each cell to
the nearest level produces flat bands with visible contours. Instead, each
cell's rounding error is diffused to its unprocessed neighbours:

```
        cell  7/16
 3/16   5/16  1/16
```

Neighbouring cells then over- or under-shoot in compensation, so a smooth
gradient becomes a halftone-like stipple whose local *density* matches the
original tone — the photographic texture in the gallery images. Disable with
`--no-dither` for crisp bands (sometimes better for logos and line art).

## 5. Glyphs and color

- **ASCII** — the quantized level indexes a ramp ordered dark→light
  (` .:-=+*#%@`, or the 70-level Bourke ramp for `-g detailed`, or any custom
  string — including emoji).
- **Braille** — each cell is a 2×4 dot matrix. Dots quantize on/off
  individually (with dithering) and set bits in the Unicode Braille block:
  `char = U+2800 + mask`. Eight sub-cell pixels per character — the highest
  spatial resolution text can carry.
- **Half-blocks** — each cell stacks two pixels using `▀`: the top pixel is
  the foreground color, the bottom the background. Double vertical resolution,
  full color, no brightness mapping at all.
- **Color** — optional ANSI escapes per glyph: truecolor (`38;2;r;g;b`) or the
  ANSI-256 6×6×6 cube (`16 + 36r' + 6g' + b'` with each channel quantized to
  0–5). Color is what saves images whose subject and background have similar
  luminance — the pink-on-green flower is unreadable in grayscale and obvious
  in color.

## SVG export

`-o out.svg` wraps the glyph grid in a monospaced `<text>` grid (one row per
line, per-run `fill` colors parsed back out of the ANSI escapes). Text in a
code block wraps and shreds the picture; an SVG never does, scales losslessly,
and opens anywhere.
