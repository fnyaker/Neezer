#!/usr/bin/env python3
"""Mesures d'acceptation du logo Neezer (lot LG1).

Aucun réseau, aucune dépendance hors `rsvg-convert`, ImageMagick et numpy.

  python3 mobile/brand/check.py           # le logo retenu, ses deux thèmes
  python3 mobile/brand/check.py path/to/logo.svg --day path/to/logo-papier.svg \
          --mono ...-mono.svg --glyph ...-notification.svg

Ce que le script mesure, et pourquoi (docs/agents/mobile.md, LG1) :

1. **Zone sûre 66 dp** — la couche avant d'une icône adaptative est dessinée
   dans un canevas de 108 dp dont seuls les 66 dp centraux (rayon 33 dp autour
   de 54,54) échappent à tous les masques d'OEM
   (developer.android.com/develop/ui/views/launch/icon_design_adaptive). On
   rastérise la couche avant seule à 1080 px (10 px par dp) et on vérifie que
   le pixel le plus éloigné du centre reste dans le cercle de rayon 33 dp. On
   mesure les PIXELS, jamais les coins de la boîte : un disque de 30 dp de rayon
   a une boîte de 60×60 dont les coins sont à 42 dp, alors qu'aucun de ses
   pixels ne dépasse 30 dp — tester les coins rejetterait tout logo rond.
2. **Silhouette à 16 px** — la couche monochrome est LA silhouette (les creux
   du dessin y sont transparents). On compte ses composantes connexes et on
   mesure la largeur moyenne de chacune (surface / plus grande dimension), qui
   doit rester ≥ 1 px : c'est « aucun détail sous 1 px » à la plus petite taille
   d'un écran d'accueil dense.
3. **Traits ≥ 2 dp à 24 dp** — le glyphe de notification est dessiné à 24 dp,
   en blanc sur transparent, et une ouverture 2×2 doit y conserver ≥ 90 % de la
   surface : mesuré, une bande de 1 dp tombe à 0 % (elle disparaît), une bande
   de 2 dp en garde 94 à 100 % (elle ne perd que ses bouts arrondis). Source
   AOSP, icon_design_status_bar (blanc sur transparent, 24 dp, ~1 dp de marge).
4. **Contrastes WCAG, PAR THÈME** — l'icône n'est pas une plaque noire fixe :
   elle a une variante Papier (fond crème, disque encre, lentille citron) et une
   variante Encre (fond encre, disque citron, lentille crème). Pour chacune, le
   script vérifie que la forme principale garde ≥ 3:1 avec son propre fond, que
   ce fond garde ≥ 3:1 avec le fond d'écran opposé, et que la forme garde ≥ 3:1
   avec l'autre fond d'écran : c'est ce qui fait qu'elle se lit sur un thème
   clair ET sur un thème sombre. La règle de Sillon est vérifiée ici : crème et
   citron ont la même luminance (1,05:1) et ne doivent jamais se toucher.
5. **Monochrome** — la couche monochrome est un masque : le lanceur la teinte.
   On vérifie donc son nombre de formes à 16 px et son contraste dans les deux
   modes thémés réels (teinte claire sur fond sombre, teinte sombre sur fond
   clair), pas « blanc sur blanc », qui n'existe pas.
"""

import argparse
import os
import subprocess
import sys
import tempfile
import xml.etree.ElementTree as ET

import numpy as np

SVG_NS = "{http://www.w3.org/2000/svg}"
CANVAS = 108.0        # le canevas d'une icône adaptative, en dp
SAFE_R = 33.0         # rayon de la zone sûre, en dp
CENTRE = (54.0, 54.0)
INK = "#0C0C0B"
CREAM = "#F3F0E8"
LEMON = "#D2FF3C"


# --- rendu -------------------------------------------------------------------

def _render(svg_path, size, strip_ground, out_png):
    """Rastérise `svg_path` à `size` px, sans le groupe id="ground" si demandé."""
    src = svg_path
    tmp = None
    if strip_ground:
        tree = ET.parse(svg_path)
        _drop_group(tree.getroot(), "ground")
        tmp = out_png + ".svg"
        tree.write(tmp, encoding="utf-8", xml_declaration=False)
        src = tmp
    subprocess.run(["rsvg-convert", "-w", str(size), "-h", str(size),
                    "-o", out_png, src], check=True)
    if tmp:
        os.unlink(tmp)
    return out_png


def _drop_group(root, group_id):
    for parent in root.iter():
        for child in list(parent):
            if child.get("id") == group_id:
                parent.remove(child)


def _rgba(png_path, size):
    raw = png_path + ".rgba"
    subprocess.run(["magick", png_path, "-depth", "8", "rgba:" + raw],
                   check=True)
    with open(raw, "rb") as f:
        data = f.read()
    os.unlink(raw)
    return np.frombuffer(data, dtype=np.uint8).reshape(size, size, 4)


def _opaque(png_path, size, threshold=128):
    return _rgba(png_path, size)[:, :, 3] >= threshold


# --- morphologie (numpy, 4-connexité) ----------------------------------------

def _components(mask, want_labels=False):
    """Étiquettes 4-connexes et leur surface, sans scipy."""
    h, w = mask.shape
    labels = np.zeros((h, w), dtype=np.int32)
    current = 0
    areas = []
    for y in range(h):
        for x in range(w):
            if mask[y, x] and labels[y, x] == 0:
                current += 1
                stack = [(y, x)]
                labels[y, x] = current
                area = 0
                while stack:
                    cy, cx = stack.pop()
                    area += 1
                    for ny, nx in ((cy - 1, cx), (cy + 1, cx),
                                   (cy, cx - 1), (cy, cx + 1)):
                        if 0 <= ny < h and 0 <= nx < w:
                            if mask[ny, nx] and labels[ny, nx] == 0:
                                labels[ny, nx] = current
                                stack.append((ny, nx))
                areas.append(area)
    if want_labels:
        return current, areas, labels
    return current, areas


def _open2x2(mask):
    """Ouverture morphologique 2×2 : ce qui a moins de 2 px d'épaisseur meurt."""
    h, w = mask.shape
    eroded = np.zeros_like(mask)
    eroded[:h - 1, :w - 1] = (mask[:h - 1, :w - 1] & mask[1:, :w - 1] &
                              mask[:h - 1, 1:] & mask[1:, 1:])
    grown = np.zeros_like(mask)
    grown[:h - 1, :w - 1] = eroded[:h - 1, :w - 1]
    grown[1:, :w - 1] |= eroded[:h - 1, :w - 1]
    grown[:h - 1, 1:] |= eroded[:h - 1, :w - 1]
    grown[1:, 1:] |= eroded[:h - 1, :w - 1]
    return grown


def _bbox_dp(png_path, size):
    mask = _opaque(png_path, size)
    ys, xs = np.nonzero(mask)
    if not len(ys):
        return None
    scale = CANVAS / size
    return (xs.min() * scale, ys.min() * scale,
            (xs.max() + 1) * scale, (ys.max() + 1) * scale)


def _safe_zone(png_path, size=1080):
    """Boîte en dp et distance du pixel le plus éloigné du centre, en dp."""
    mask = _opaque(png_path, size)
    ys, xs = np.nonzero(mask)
    if not len(ys):
        return None
    scale = CANVAS / size
    far_px = np.hypot((xs + 0.5) * scale - CENTRE[0],
                      (ys + 0.5) * scale - CENTRE[1]).max()
    box = (xs.min() * scale, ys.min() * scale,
           (xs.max() + 1) * scale, (ys.max() + 1) * scale)
    return box, far_px


# --- contraste WCAG ----------------------------------------------------------

def _srgb(c):
    c = c / 255.0
    return c / 12.92 if c <= 0.03928 else ((c + 0.055) / 1.055) ** 2.4


def luminance(hex_colour):
    h = hex_colour.lstrip("#")
    if len(h) == 3:
        h = "".join(ch * 2 for ch in h)
    r, g, b = (int(h[i:i + 2], 16) for i in (0, 2, 4))
    return 0.2126 * _srgb(r) + 0.7152 * _srgb(g) + 0.0722 * _srgb(b)


def contrast(a, b):
    la, lb = luminance(a), luminance(b)
    hi, lo = max(la, lb), min(la, lb)
    return (hi + 0.05) / (lo + 0.05)


def _layers(svg_path):
    """Les couleurs du fond et des formes, séparément."""
    tree = ET.parse(svg_path)
    ground, marks, colours = [], [], []
    for el in tree.getroot().iter():
        for attr in ("fill", "stroke"):
            c = el.get(attr)
            if c and c.startswith("#"):
                c = c.upper()
                if c not in colours:
                    colours.append(c)
                (ground if _in_ground(tree, el) else marks).append(c)
    # une couleur qui n'est que dans le fond
    return colours, [c for c in colours if c not in marks]


def _in_ground(tree, target):
    for parent in tree.getroot().iter():
        if parent.get("id") == "ground":
            if any(child is target for child in parent.iter()):
                return True
    return False


# --- rapport -----------------------------------------------------------------

def _variant(svg_path, work, tag):
    ok = True
    say = print
    colours, ground = _layers(svg_path)
    marks = [c for c in colours if c not in ground]
    fg = os.path.join(work, tag + "-fg.png")
    _render(svg_path, 1080, True, fg)
    safe = _safe_zone(fg, 1080)
    if safe is None:
        say("  [%s] ZONE SÛRE  aucune forme dans la couche avant (échec)" % tag)
        return False
    box, far = safe
    inside = far <= SAFE_R + 0.01
    say("  [%s] ZONE SÛRE  boîte %.1f..%.1f x %.1f..%.1f dp, pixel le plus "
        "loin %.2f dp (limite %.0f)  %s"
        % (tag, box[0], box[2], box[1], box[3], far, SAFE_R,
           "ok" if inside else "ÉCHEC"))
    ok = ok and inside

    g = ground[0] if ground else None
    if g is None:
        say("  [%s] CONTRASTES impossibles : aucun fond dans le SVG" % tag)
        return False
    say("  [%s] COULEURS   fond %s, formes %s" % (tag, g, ", ".join(marks)))

    # La règle exacte : CHAQUE BORD QUI SE TOUCHE doit porter ≥ 3:1. On la mesure
    # sur le rendu, pas sur une liste de couleurs — en Papier la gorge citron
    # touche le disque encre et non le fond crème, et une forme peut n'être
    # voisine que d'une autre forme.
    pairs = _touching_pairs(svg_path)
    if not pairs:
        say("      aucun bord mesurable (rendu vide ?)")
        return False
    worst = min(p[2] for p in pairs)
    for a, b, c in pairs:
        say("      %s | %s  %.2f:1  %s" % (a, b, c, "ok" if c >= 3.0 else "ÉCHEC"))
    ok = ok and worst >= 3.0
    say("      pire bord : %.2f:1" % worst)

    # L'icône ne doit jamais disparaître : sur CHAQUE fond d'écran, au moins une
    # de ses formes tient ≥ 3:1 — le fond sur un fond d'écran opposé, ou la
    # forme sur un fond d'écran de même ton.
    for wp, name in (("#FFFFFF", "blanc"), ("#000000", "noir")):
        readers = [(c, contrast(c, wp)) for c in [g] + marks]
        best = max(readers, key=lambda r: r[1])
        good = best[1] >= 3.0
        say("      sur un fond d'écran %-5s : %s tient %.2f:1  %s"
            % (name, best[0], best[1], "ok" if good else "ÉCHEC"))
        ok = ok and good
    return ok


def _touching_pairs(svg_path, size=1080, reach=2):
    """Les couples de couleurs qui se touchent, et leur contraste WCAG."""
    tree = ET.parse(svg_path)
    colours = []
    for el in tree.getroot().iter():
        for attr in ("fill", "stroke"):
            c = el.get(attr)
            if c and c.startswith("#") and c.upper() not in colours:
                colours.append(c.upper())
    png = os.path.join(tempfile.mkdtemp(prefix="lg1-adj-"), "c.png")
    _render(svg_path, size, False, png)
    arr = _rgba(png, size)
    rgb = arr[:, :, :3].astype(np.int16)
    alpha = arr[:, :, 3] >= 200

    def mask(hex_colour):
        h = hex_colour.lstrip("#")
        target = np.array([int(h[i:i + 2], 16) for i in (0, 2, 4)], dtype=np.int16)
        return alpha & (np.abs(rgb - target).max(axis=2) <= 6)

    masks = {c: mask(c) for c in colours}
    pairs = []
    for i, a in enumerate(colours):
        if not masks[a].any():
            continue
        grown = masks[a].copy()
        for _ in range(reach):
            nxt = grown.copy()
            nxt[1:, :] |= grown[:-1, :]
            nxt[:-1, :] |= grown[1:, :]
            nxt[:, 1:] |= grown[:, :-1]
            nxt[:, :-1] |= grown[:, 1:]
            grown = nxt
        for b in colours[i + 1:]:
            if masks[b].any() and (grown & masks[b]).any():
                pairs.append((a, b, contrast(a, b)))
    return pairs


def check(svg_path, day_path=None, mono_path=None, glyph_path=None, verbose=True):
    work = tempfile.mkdtemp(prefix="lg1-check-")
    ok = True
    say = (lambda *a: print(*a)) if verbose else (lambda *a: None)

    say("\n== %s" % svg_path)
    ok = _variant(svg_path, work, "Encre") and ok
    if day_path:
        say("\n== %s" % day_path)
        ok = _variant(day_path, work, "Papier") and ok

    # 2. silhouette à 16 px : la couche monochrome est LA silhouette
    sil = mono_path or svg_path
    fg16 = os.path.join(work, "fg16.png")
    _render(sil, 16, mono_path is None, fg16)
    m16 = _opaque(fg16, 16)
    n16, areas16, labels16 = _components(m16, want_labels=True)
    thin = _open2x2(m16)
    kept = thin.sum() / max(1, m16.sum())
    widths = []
    for i in range(1, n16 + 1):
        ys, xs = np.nonzero(labels16 == i)
        span = max(xs.max() - xs.min() + 1, ys.max() - ys.min() + 1)
        widths.append(round(float(len(xs)) / float(span), 2))
    say("\n  SILHOUETTE  %d forme(s) à 16 px, surfaces %s px, largeurs moyennes "
        "%s px ; ouverture 2×2 garde %d %%  (%s)"
        % (n16, areas16, widths, round(kept * 100), os.path.basename(sil)))
    ok = ok and n16 >= 2 and min(widths) >= 1.0 and min(areas16) >= 2

    # 3. glyphe 24 dp : traits ≥ 2 dp
    if glyph_path:
        g24 = os.path.join(work, "g24.png")
        _render(glyph_path, 24, False, g24)
        mg = _opaque(g24, 24)
        ng, ag = _components(mg)
        kg = _open2x2(mg).sum() / max(1, mg.sum())
        say("  GLYPHE 24   %d forme(s), surfaces %s px ; ouverture 2×2 garde "
            "%d %% (une bande de 1 dp tomberait à 0 %%)" % (ng, ag, round(kg * 100)))
        ok = ok and ng >= 1 and kg >= 0.90

    # 5. monochrome
    if mono_path:
        mono = os.path.join(work, "mono16.png")
        _render(mono_path, 16, False, mono)
        mm = _opaque(mono, 16)
        nm = _components(mm)[0]
        on_dark = contrast(CREAM, INK)
        on_light = contrast(INK, CREAM)
        say("  MONOCHROME  %d forme(s) à 16 px ; teinte claire/fond encre "
            "%.1f:1, teinte encre/fond crème %.1f:1" % (nm, on_dark, on_light))
        ok = ok and nm >= 2 and min(on_dark, on_light) >= 3.0

    say("  => %s" % ("ok" if ok else "ÉCHEC"))
    return ok


def main(argv):
    here = os.path.dirname(os.path.abspath(__file__))
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("svg", nargs="?", default=os.path.join(here, "logo.svg"))
    ap.add_argument("--day", help="la variante Papier (défaut : logo-papier.svg "
                                  "à côté du SVG)")
    ap.add_argument("--mono", help="couche monochrome (défaut : logo-mono.svg "
                                   "à côté du SVG)")
    ap.add_argument("--glyph", help="glyphe de notification (défaut : "
                                    "logo-notification.svg s'il existe)")
    args = ap.parse_args(argv)

    base = os.path.splitext(args.svg)[0]
    day = args.day or (base + "-papier.svg" if os.path.exists(base + "-papier.svg")
                       else None)
    mono = args.mono or (base + "-mono.svg" if os.path.exists(base + "-mono.svg")
                         else None)
    glyph = args.glyph or (base + "-notification.svg"
                           if os.path.exists(base + "-notification.svg") else None)
    sys.exit(0 if check(args.svg, day, mono, glyph) else 1)


if __name__ == "__main__":
    main(sys.argv[1:])
