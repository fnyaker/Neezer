//! The colours of a sleeve, or of the theme — read once, in the same shape the
//! page reads them.
//!
//! A PORT, not a design: `webapp/src/lib/coverpalette.js#analyse`, the pure
//! half of the app's one cover decoder, with its numbers and its ordering
//! intact (the vertical zones, the horizontal bands, the 3×3 grid, the 4-bit
//! histogram whose bucket MEAN is the colour reported, five distinct colours at
//! most, the "vivid" accent score, and the light-surface flag). The page's
//! other half — drawing the sleeve on a 64×64 canvas and reading it back — is
//! the app's business, not this crate's: `nz-viz` is pure, so it takes RGBA
//! bytes and never touches an image.
//!
//! WHY THE ORDER MATTERS. `colors`, `dominant` and `vivid` are picked from the
//! histogram SORTED by presence, ties kept in the order the pixels arrived, and
//! `dominant` is the first colour that survived the 26-unit distinctness test.
//! A different tie-break would colour a two-colour sleeve differently from the
//! browser, so the sort here is stable and the bins keep their insertion order.
//!
//! `Palette::from_theme` is the other source the lot asks for: the Omarchy
//! theme's accent when there is no artwork (the player's strip is ONE flat
//! colour there, the theme's, and it changes with the theme without rebuilding
//! anything).

/// An opaque colour, 0..255 per channel.
pub type Rgb = [u8; 3];

/// The three vertical bands of the artwork, left to right — what a horizontal
/// gradient follows. `None` when a band has no visible pixel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Zones {
    pub left: Option<Rgb>,
    pub center: Option<Rgb>,
    pub right: Option<Rgb>,
}

/// The three horizontal bands, top to bottom.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Bands {
    pub top: Option<Rgb>,
    pub middle: Option<Rgb>,
    pub bottom: Option<Rgb>,
}

/// What the page's decoder answers with, field for field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Palette {
    /// The most PRESENT colour (what the ground tints with), never a bright
    /// minority.
    pub dominant: Rgb,
    /// The mean of every opaque pixel.
    pub average: Rgb,
    /// The most saturated well-lit colour — an accent.
    pub vivid: Rgb,
    /// Up to five distinct dominant colours, most present first.
    pub colors: Vec<Rgb>,
    pub zones: Zones,
    pub bands: Bands,
    /// The middle row's three cells: the row the eye lands on when the sleeve
    /// is centred.
    pub middles: [Option<Rgb>; 3],
    /// 3 columns × 3 rows, row-major.
    pub cells: [Option<Rgb>; 9],
    /// The average reads as a light surface (the caller picks the ink).
    pub light: bool,
}

/// Below this alpha a pixel is not part of the picture (a rounded PNG corner, a
/// transparent sleeve): averaging it in would drag every zone toward black.
const ALPHA_MIN: u8 = 200;
/// How far apart two "dominant" colours must be to be reported as distinct.
const DISTINCT: f64 = 26.0;
const MAX_COLORS: usize = 5;

fn clamp8(v: f64) -> u8 {
    if v < 0.0 {
        0
    } else if v > 255.0 {
        255
    } else {
        // `Math.round` on a non-negative value is round-half-up; `f64::round`
        // is round-half-away-from-zero, and the value is never negative here.
        v.round() as u8
    }
}

/// One accumulating region: a sum and a count, so a region with no visible
/// pixel stays `None` instead of becoming black.
#[derive(Clone, Copy, Default)]
struct Bin {
    r: f64,
    g: f64,
    b: f64,
    n: f64,
}

impl Bin {
    fn add(&mut self, r: u8, g: u8, b: u8) {
        self.r += r as f64;
        self.g += g as f64;
        self.b += b as f64;
        self.n += 1.0;
    }

    fn mean(&self) -> Option<Rgb> {
        if self.n <= 0.0 {
            None
        } else {
            Some([
                clamp8(self.r / self.n),
                clamp8(self.g / self.n),
                clamp8(self.b / self.n),
            ])
        }
    }
}

fn dist(a: Rgb, b: Rgb) -> f64 {
    let (dr, dg, db) = (
        a[0] as f64 - b[0] as f64,
        a[1] as f64 - b[1] as f64,
        a[2] as f64 - b[2] as f64,
    );
    (dr * dr + dg * dg + db * db).sqrt()
}

/// The pure half of the cover decoder: `width × height` RGBA pixels in, the
/// palette out. `None` when the buffer is too short or every pixel is
/// transparent — never a black palette.
pub fn analyse(pixels: &[u8], width: usize, height: usize) -> Option<Palette> {
    let w = width.max(1);
    let h = height.max(1);
    if pixels.len() < w * h * 4 {
        return None;
    }
    let mut cols = [Bin::default(); 3];
    let mut rows = [Bin::default(); 3];
    let mut cells = [Bin::default(); 9];
    // 4 bits a channel: 4 096 buckets, which is a colour count a 64 px sleeve
    // can actually fill — and the bucket's own mean is the colour reported, so
    // the quantisation decides the grouping, never the result. `order` keeps
    // the buckets in the order they were first seen, which is the order the
    // JavaScript Map hands them back in.
    let mut hist = vec![Bin::default(); 4096];
    let mut seen = [false; 4096];
    let mut order: Vec<u16> = Vec::with_capacity(64);
    let mut total = Bin::default();

    for y in 0..h {
        let row = ((y * 3) / h).min(2);
        for x in 0..w {
            let i = (y * w + x) * 4;
            if pixels[i + 3] < ALPHA_MIN {
                continue;
            }
            let (r, g, b) = (pixels[i], pixels[i + 1], pixels[i + 2]);
            let col = ((x * 3) / w).min(2);
            cols[col].add(r, g, b);
            rows[row].add(r, g, b);
            cells[row * 3 + col].add(r, g, b);
            total.add(r, g, b);
            let key = ((r >> 4) as usize) << 8 | ((g >> 4) as usize) << 4 | ((b >> 4) as usize);
            if !seen[key] {
                seen[key] = true;
                order.push(key as u16);
            }
            hist[key].add(r, g, b);
        }
    }
    if total.n == 0.0 {
        return None;
    }

    let average = total.mean()?;

    // Most present first; a colour too close to one already kept is dropped, so
    // five near-identical darks never stand in for five different colours.
    // `sort_by` is stable, so ties stay in the order the pixels arrived.
    let mut bins: Vec<usize> = order.iter().map(|&k| k as usize).collect();
    bins.sort_by(|&a, &b| hist[b].n.total_cmp(&hist[a].n));
    let mut colors: Vec<Rgb> = Vec::with_capacity(MAX_COLORS);
    for &k in &bins {
        let Some(c) = hist[k].mean() else { continue };
        if colors.iter().any(|&kept| dist(kept, c) < DISTINCT) {
            continue;
        }
        colors.push(c);
        if colors.len() >= MAX_COLORS {
            break;
        }
    }

    // The colour a bar or a focus ring can use: saturated AND not at an extreme
    // of lightness, the rule `color.js` has always used for the header accent.
    let mut vivid = average;
    let mut best = 0.15;
    for &k in &bins {
        let Some(c) = hist[k].mean() else { continue };
        let max = c[0].max(c[1]).max(c[2]);
        let min = c[0].min(c[1]).min(c[2]);
        let sat = if max == 0 {
            0.0
        } else {
            (max - min) as f64 / max as f64
        };
        let lum = max as f64 / 255.0;
        let score = sat * (1.0 - (lum - 0.55).abs());
        if score > best {
            best = score;
            vivid = c;
        }
    }

    let dominant = colors.first().copied().unwrap_or(average);
    // A surface flag, not a WCAG pair: the caller chooses the ink. Weighted as
    // the eye weighs, on the sRGB values themselves.
    let light =
        0.2126 * average[0] as f64 + 0.7152 * average[1] as f64 + 0.0722 * average[2] as f64
            > 140.0;

    Some(Palette {
        dominant,
        average,
        vivid,
        colors,
        zones: Zones {
            left: cols[0].mean(),
            center: cols[1].mean(),
            right: cols[2].mean(),
        },
        bands: Bands {
            top: rows[0].mean(),
            middle: rows[1].mean(),
            bottom: rows[2].mean(),
        },
        middles: [cells[3].mean(), cells[4].mean(), cells[5].mean()],
        cells: [
            cells[0].mean(),
            cells[1].mean(),
            cells[2].mean(),
            cells[3].mean(),
            cells[4].mean(),
            cells[5].mean(),
            cells[6].mean(),
            cells[7].mean(),
            cells[8].mean(),
        ],
        light,
    })
}

impl Palette {
    /// The Omarchy theme's palette, for when there is no artwork: the accent is
    /// the one colour every zone reports (the player's strip is ONE flat colour
    /// on the theme), and `surface` is what the screen is, for a caller that
    /// needs to know what the bars are drawn over.
    pub fn from_theme(accent: Rgb, surface: Rgb, light: bool) -> Palette {
        Palette {
            dominant: accent,
            average: surface,
            vivid: accent,
            colors: vec![accent],
            zones: Zones {
                left: Some(accent),
                center: Some(accent),
                right: Some(accent),
            },
            bands: Bands {
                top: Some(accent),
                middle: Some(accent),
                bottom: Some(accent),
            },
            middles: [Some(accent); 3],
            cells: [Some(accent); 9],
            light,
        }
    }

    /// The three colour anchors a row of bars walks, left to right —
    /// `palette.js#zonesOfPalette`: PER COLUMN, THE MOST SATURATED OF ITS THREE
    /// CELLS, not the column's mean. A mean mixes the top of a sleeve with its
    /// bottom, and in colour a blue over an amber average is olive: the zone
    /// ends up a colour that appears nowhere in the artwork. A column of greys
    /// keeps its mean instead ("the most saturated grey" is noise).
    ///
    /// The resampling to the scene's stops, and the hue walk between them, stay
    /// with the scene: they are the drawing, not the sleeve.
    pub fn zone_colours(&self) -> [Rgb; 3] {
        let fill = self.average;
        [0, 1, 2].map(|c| {
            let of = [self.cells[c], self.cells[3 + c], self.cells[6 + c]];
            let mut best: Option<Rgb> = None;
            let mut score_max = -1.0f64;
            let (mut sum, mut n) = ([0f64; 3], 0f64);
            for cell in of.into_iter().flatten() {
                for k in 0..3 {
                    sum[k] += cell[k] as f64;
                }
                n += 1.0;
                let max = cell[0].max(cell[1]).max(cell[2]);
                let min = cell[0].min(cell[1]).min(cell[2]);
                let sat = if max == 0 {
                    0.0
                } else {
                    (max - min) as f64 / max as f64
                };
                let score = sat * (1.0 - (max as f64 / 255.0 - 0.55).abs());
                if score > score_max {
                    score_max = score;
                    best = Some(cell);
                }
            }
            match best {
                Some(b) if score_max >= 0.06 => b,
                _ if n > 0.0 => [clamp8(sum[0] / n), clamp8(sum[1] / n), clamp8(sum[2] / n)],
                _ => fill,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Rgb = [200, 20, 20];
    const GREEN: Rgb = [20, 200, 20];
    const BLUE: Rgb = [20, 20, 200];
    const WHITE: Rgb = [255, 255, 255];

    /// A `width × height` RGBA grid from `f(x, y)`, like the page's own test
    /// helper (`webapp/test/coverpalette.test.mjs#grid`).
    fn grid(width: usize, height: usize, f: impl Fn(usize, usize) -> [u8; 4]) -> Vec<u8> {
        let mut data = vec![0u8; width * height * 4];
        for y in 0..height {
            for x in 0..width {
                let px = f(x, y);
                let i = (y * width + x) * 4;
                data[i..i + 4].copy_from_slice(&px);
            }
        }
        data
    }

    fn solid(c: Rgb) -> [u8; 4] {
        [c[0], c[1], c[2], 255]
    }

    #[test]
    fn the_three_vertical_zones_are_the_three_columns_averaged() {
        let d = grid(30, 30, |x, _| {
            solid(if x < 10 {
                RED
            } else if x < 20 {
                GREEN
            } else {
                BLUE
            })
        });
        let p = analyse(&d, 30, 30).expect("a palette");
        assert_eq!(p.zones.left, Some(RED));
        assert_eq!(p.zones.center, Some(GREEN));
        assert_eq!(
            p.zones.right,
            Some(BLUE),
            "a colour present only on the right stays on the right"
        );
        assert_eq!(p.middles, [Some(RED), Some(GREEN), Some(BLUE)]);
    }

    #[test]
    fn the_three_bands_are_the_three_rows_averaged() {
        let d = grid(30, 30, |_, y| {
            solid(if y < 10 {
                RED
            } else if y < 20 {
                GREEN
            } else {
                BLUE
            })
        });
        let p = analyse(&d, 30, 30).expect("a palette");
        assert_eq!(p.bands.top, Some(RED));
        assert_eq!(p.bands.middle, Some(GREEN));
        assert_eq!(p.bands.bottom, Some(BLUE));
    }

    #[test]
    fn dominant_is_the_colour_that_covers_the_most_not_the_brightest() {
        // A white square on a red field: the white is the loudest pixel and
        // must not become the ground's tint.
        let d = grid(40, 40, |x, y| {
            solid(if (5..15).contains(&x) && (5..15).contains(&y) {
                WHITE
            } else {
                RED
            })
        });
        let p = analyse(&d, 40, 40).expect("a palette");
        assert_eq!(p.dominant, RED);
        assert!(
            p.colors
                .iter()
                .any(|c| c[0] > 240 && c[1] > 240 && c[2] > 240),
            "the white is reported too"
        );
        assert!(p.colors.len() <= 5, "at most five colours");
    }

    #[test]
    fn a_transparent_pixel_is_not_part_of_the_picture() {
        let d = grid(30, 30, |x, y| {
            if x < 10 {
                [RED[0], RED[1], RED[2], 0]
            } else {
                solid(if y < 15 { GREEN } else { BLUE })
            }
        });
        let p = analyse(&d, 30, 30).expect("a palette");
        assert_eq!(p.zones.left, None, "a fully transparent band has no colour");
        assert_ne!(p.average, [0, 0, 0]);
    }

    #[test]
    fn an_empty_image_is_none_never_a_black_palette() {
        let d = grid(8, 8, |_, _| [0, 0, 0, 0]);
        assert_eq!(analyse(&d, 8, 8), None);
        assert_eq!(analyse(&[0, 0, 0, 0], 2, 2), None, "short buffer");
    }

    #[test]
    fn light_says_whether_the_sleeve_reads_as_a_light_surface() {
        let white = grid(8, 8, |_, _| solid(WHITE));
        let black = grid(8, 8, |_, _| solid([0, 0, 0]));
        assert!(analyse(&white, 8, 8).unwrap().light);
        assert!(!analyse(&black, 8, 8).unwrap().light);
    }

    #[test]
    fn a_zone_takes_its_saturated_cell_not_the_column_mean() {
        // Each column is a saturated colour between two grey rows: the column
        // MEAN is a muted tone that appears nowhere in the sleeve, while the
        // middle cell is the colour it actually contains.
        const GREY: Rgb = [128, 128, 128];
        let d = grid(30, 30, |x, y| {
            let base = if x < 10 {
                RED
            } else if x < 20 {
                GREEN
            } else {
                BLUE
            };
            solid(if (10..20).contains(&y) { base } else { GREY })
        });
        let p = analyse(&d, 30, 30).expect("a palette");
        assert_eq!(
            p.zones.left,
            Some([152, 92, 92]),
            "the column mean is the muted tone"
        );
        assert_eq!(p.zone_colours(), [RED, GREEN, BLUE]);
    }

    #[test]
    fn the_theme_is_one_flat_accent() {
        let p = Palette::from_theme([240, 200, 90], [18, 18, 20], false);
        assert_eq!(p.dominant, [240, 200, 90]);
        assert_eq!(p.average, [18, 18, 20]);
        assert_eq!(p.zone_colours(), [[240, 200, 90]; 3]);
        assert!(!p.light);
    }
}
