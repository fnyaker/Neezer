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
   glyph, where 1 px = 1 dp, so "every stroke ≥ 2 dp" is the 2×2 opening. A 1 dp
   band falls to **0 %** of its area; a 2 dp band keeps 94–100 % (it only loses
   its round caps), which is why the guard is 90 %. Measured on the three
   glyphs: 98, 100 and 94 %.
4. **Contrast** — WCAG ratios of every fill and stroke against white, black and
   the ink ground. Measured: lemon 18.1:1 and cream 18.4:1 on ink; ink 19.6:1
   on a white wallpaper. On a black wallpaper the ink ground dissolves (1.07:1)
   and the lemon and cream carry the icon, which is what a dark adaptive icon
   does; on white the ink ground carries it.
5. **Monochrome** — the mask's shape count at 16 px, and its contrast in the two
   themed modes that actually exist (a light tint on a dark ground, a dark tint
   on a light ground: 17.2:1). White-on-white is not a mode.

## Export

```sh
mobile/brand/export.sh            # write every generated resource
mobile/brand/export.sh --check    # fail if one of them has drifted from the sources
```

Offline, stdlib Python + `rsvg-convert` (or `resvg`); no Gradle, no network. It
writes:

- `drawable/ic_launcher_foreground.xml` — the light shapes, viewport from
  `logo.svg` (1024), drawn at 108 dp;
- `drawable/ic_launcher_monochrome.xml` — the same geometry in one colour, for
  themed icons (Android 13+);
- `mipmap-anydpi-v26/ic_launcher.xml` and `ic_launcher_round.xml` — the
  `<adaptive-icon>` pointing at `@color/ic_launcher_background` and the two
  layers;
- `drawable/ic_stat_neezer.xml` — the 24 dp white status-bar silhouette. It is
  the mark's silhouette at an **optical size**: nothing under 2 dp, so its
  thinnest band is thickened (the mark's own bands are 12 and 7 dp on the
  108 dp canvas, which at 24 dp would be 2.7 and 1.6 dp) — the shape reads, the
  hairline does not become fuzz;
- `play-store-512.png` — 512×512, 32-bit, sRGB, opaque, full square (Play
  applies the 30 % radius and the shadow itself);
- `icon-108.png` — the composed icon, for docs and a quick look.

Three rules the script enforces instead of documenting:

- **The background colour is MB0's file.** `values/colors.xml` is not LG1's: the
  script *reads* `ic_launcher_background`, refuses to run if it is not
  `#0C0C0B`, and never edits it.
- **An element the exporter does not understand is a hard error** — a silently
  dropped element is a wrong icon, and a wrong icon ships. Supported: `<path>`,
  `<circle>`, `<ellipse>`, `<rect>` (converted to `pathData`), `fill-rule`
  `evenodd` (as `android:fillType`), strokes, and `rotate(a cx cy)` as a
  `<group>`. Anything else stops the export.
- **`--check` is the drift guard**: it renders into a temp tree and compares
  byte for byte, so a source edited without re-exporting fails loudly. It was
  tested in both directions (matching, and one injected comment in a committed
  drawable).

Checked against `aapt2 compile` + `aapt2 link` for all three directions (a minimal
manifest, `--min-sdk-version 26`): every generated drawable links.

## Evidence on the emulator

The lot's emulator image (API 35 `default`, `nz35`) ships **neither** the
`com.android.theme.icon.*` overlays **nor** the themed-icons setting: `cmd
overlay list` has one `com.android.theme.font.*` entry and nothing else, and
`settings list secure` has no `themed_icons`. The launcher therefore can only
ever show its own mask. Two things are captured instead, both headless
(`-no-window -no-audio`, `adb -s emulator-<port>` only, the lot's own PID):

1. **The real launcher**: the app installed from `:app:assembleDebug`, the icon
   in the app drawer, `adb exec-out screencap`.
2. **The three masks and the themed icon**, drawn *on the device* by the
   framework's own `AdaptiveIconDrawable` over the **installed APK's real
   resources** (`AssetManager.addAssetPath` + `getIdentifier`), applying the
   AOSP mask geometry (circle, superellipse squircle, teardrop with its sharp
   corner) and the monochrome layer tinted in the two modes that exist. Run as
   `app_process` under the shell uid — no app, no signature, no Gradle:

```sh
mobile/brand/device/render-masks.sh --serial emulator-5580 \
    --apk mobile/android/app/build/outputs/apk/debug/app-debug.apk \
    --out ~/.cache/neezer-lg1/shots
```

It compiles `device/IconMasks.java` with the platform's `d8`, pushes the dex,
runs it, pulls the PNGs and writes a strip. It also renders the 24 dp status
glyph at this screen's density (420 dpi → 63 px) and four times larger, which is
where a stroke under 2 dp would show.

`mobile/tools/emu.sh` (lot MQ1) is the harness that owns the emulator lifecycle;
before it lands, the same flags are used by hand: `emulator -avd nz35 -port 5580
-no-window -no-audio -gpu swiftshader_indirect -no-boot-anim -no-snapshot-save
-readonly -memory 3072 -cores 4`.

`icon.svg` stays the 108 dp composition of the icon (ground + mark), so a human
can open one file and see what the launcher draws; `logo.svg` is the 1024 master
every other asset derives from, `logo-mono.svg` its monochrome twin and
`logo-notification.svg` the 24 dp glyph. The three files in `directions/` are the
frozen candidates the person chose from — not sources to keep in sync.
