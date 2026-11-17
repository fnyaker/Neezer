# `mobile/brand` — the logo's sources

The launcher icon, its monochrome (themed) twin and the notification glyph are
generated from the files here; the app only ever references the *names*
(`ic_launcher`, `ic_launcher_round`, `ic_stat_neezer`), so swapping the artwork
never touches a manifest, a Gradle file or a Kotlin file.

**Lot LG1 owns this directory** and the three drawables in
`mobile/android/app/src/main/res/` the ownership table names.

## Three directions, and why they are three

Sillon is the design direction: the groove, ink `#0c0c0b`, lemon `#d2ff3c`,
cream `#f3f0e8`, and the glass in layers. What the mark may not be is the old
SPA mark (a lemon disc with two concentric circles and a centre dot) or anyone
else's: Spotify's three arcs, Deezer's bars and heart, Apple Music's note,
YouTube Music's play triangle in a disc, Tidal's diamonds, Qobuz's Q. No text,
no emoji, no gradient that dies in monochrome.

The brief asked for **three directions** and a person's choice, so the three are
three different *structures*, not three variations of one:

| file | name | structure |
|---|---|---|
| `directions/a-lens.svg` | La lentille | a lemon disc, its lens laid off-centre: the gorge is the groove, the cream label is the light. Nothing is concentric. |
| `directions/b-sleeve.svg` | La pochette | a lemon groove running under a cream sleeve; the disc reads through it. Four layers, like Sillon's glass ladder. |
| `directions/c-wake.svg` | Le sillage | two furrows tapering to a point, the wide one cream (the light), the short one lemon. No disc at all: the trace alone. |

Each `*-mono.svg` beside it is the **same geometry** in one colour: that is the
monochrome layer the launcher tints, so the themed icon is not a redrawing.

## What the palette forces (measured, not a taste)

- **Cream and lemon have the same luminance** — `#f3f0e8` is 1.14:1 against
  white and `#d2ff3c` 1.16:1, and they are **1.05:1 against each other**. They
  never touch: every cream shape is separated from a lemon one by at least
  5 dp of ink, or it disappears. In monochrome they merge by definition, which
  is exactly why the mono layer is the same silhouette and not a tinted copy.
- **At 16 px only two tones survive.** 1 px = 6.75 dp on the 108 dp canvas, so
  any stroke under 6.75 dp is sub-pixel at the smallest launcher size. The
  spiral that was tried first (a groove spiralling from the rim to the label)
  fragmented the disc into 1–2 px specks at 16 px — measured, five components
  instead of two. It was dropped; what remains is two shapes at 16 px in every
  direction.
- **The safe zone is a circle, not a box.** The 66 dp area no mask clips is the
  circle of radius 33 dp around (54, 54); a 60 dp disc has a 60×60 box whose
  corners are 42 dp out, so a box-corner test would reject every round mark.
  `check.py` measures the *pixels*.

## `check.py` — the acceptance, measured

```sh
python3 mobile/brand/check.py mobile/brand/directions/a-lens.svg \
        --mono mobile/brand/directions/a-lens-mono.svg
```

No network; needs `rsvg-convert`, ImageMagick and numpy. It reports, and fails
on:

1. **Safe zone** — the farthest opaque pixel of the foreground layer, in dp,
   against 33 dp (1080 px raster = 10 px per dp).
2. **Silhouette at 16 px** — connected components of the monochrome layer, and
   each component's average width (`area / longest side`), which must stay
   ≥ 1 px: that is "no detail below 1 px". Plus a 2×2 morphological opening as
   a guard against sub-pixel fuzz.
3. **Glyph at 24 dp** — same component and width test on the notification
   glyph, where 1 px = 1 dp, so "every stroke ≥ 2 dp" is the 2×2 opening.
4. **Contrast** — WCAG ratios of every fill and stroke against white, black and
   the ink ground. Measured: lemon 18.1:1 and cream 18.4:1 on ink; ink 19.6:1
   on a white wallpaper. On a black wallpaper the ink ground dissolves (1.07:1)
   and the lemon and cream carry the icon, which is what a dark adaptive icon
   does; on white the ink ground carries it.
5. **Monochrome** — the mask's shape count at 16 px, and its contrast in the two
   themed modes that actually exist (a light tint on a dark ground, a dark tint
   on a light ground: 17.2:1). White-on-white is not a mode.

## Export

`export.sh` (written once the direction is chosen) produces, offline from these
sources only:

- `drawable/ic_launcher_foreground.xml` — the light shapes, `viewport 0..108`;
- `drawable/ic_launcher_monochrome.xml` — the same geometry, one colour;
- `mipmap-anydpi-v26/ic_launcher.xml` and `ic_launcher_round.xml` — the
  `<adaptive-icon>` that points at them plus `@color/ic_launcher_background`;
- `drawable/ic_stat_neezer.xml` — the 24 dp white silhouette;
- `brand/play-store-512.png` — the 512×512, 32-bit, opaque PNG Google Play
  wants (full square: Play applies the 30 % radius and the shadow itself).

`icon.svg` stays the 108 dp source of the icon's own composition (ground +
mark); `logo.svg` is the 1024 master the other assets derive from.
