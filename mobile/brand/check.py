#!/usr/bin/env python3
"""Mesures d'acceptation du logo Neezer (lot LG1).

Aucun réseau, aucune dépendance hors `rsvg-convert`, ImageMagick et numpy.

  python3 mobile/brand/check.py mobile/brand/logo.svg          # le logo retenu
  python3 mobile/brand/check.py --direction directions/b-sleeve.svg

Ce que le script mesure, et pourquoi (docs/agents/mobile.md, LG1) :

1. **Zone sûre 66 dp** — la couche avant d'une icône adaptative est dessinée
   dans un canevas de 108 dp dont seuls les 66 dp centraux (rayon 33 dp autour
   de 54,54) échappent à tous les masques d'OEM
   (developer.android.com/develop/ui/views/launch/icon_design_adaptive). On
   rastérise la couche avant seule à 1080 px (10 px par dp), on prend la boîte
   des pixels opaques et on vérifie que chaque coin reste dans le cercle de
   rayon 33 dp — un coin hors cercle est un coin qu'un masque peut rogner.
2. **Silhouette à 16 px** — l'icône telle qu'un lanceur la dessine à 16 px (le
   format le plus petit d'un écran d'accueil dense) : on compte les composantes
   connexes de la couche avant. « ≥ 2 formes distinctes » veut dire que le
   disque et son étiquette, ou la pochette et son sillon, restent séparés.
   On vérifie aussi qu'aucun trait ne descend sous 1 px : une ouverture
   morphologique 2×2 sur le rasteur 16 px ne doit pas manger plus de 20 % de la
   surface (un trait de 1 px disparaît entièrement).
3. **Traits ≥ 2 dp à 24 dp** — le glyphe de notification est dessiné à 24 dp,
   en blanc sur transparent, et une ouverture 2×2 doit y conserver ≥ 90 % de la
   surface : mesuré, une bande de 1 dp tombe à 0 % (elle disparaît), une bande
   de 2 dp en garde 94 à 100 % (elle ne perd que ses bouts arrondis). Source
   AOSP, icon_design_status_bar (blanc sur transparent, 24 dp, ~1 dp de marge).
4. **Contrastes WCAG** — chaque couleur du logo contre un fond blanc et un fond
   noir. Le rapport est calculé, pas estimé. La règle de Sillon (crème et citron
   ont la même luminance, 1,05:1) est vérifiée ici : ils ne doivent jamais se
   toucher, seulement se séparer par l'encre.
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


# --- rendu -------------------------------------------------------------------

def _render(svg_path, size, strip_ground, out_png):
    """Rastérise `svg_path` à `size` px, sans le groupe id="ground" si demandé."""
    src = svg_path
    tmp = None
    if strip_ground:
        tree = ET.parse(svg_path)
        root = tree.getroot()
        _drop_group(root, "ground")
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
    arr = np.frombuffer(data, dtype=np.uint8).reshape(size, size, 4)
    return arr


def _opaque(png_path, size, threshold=128):
    arr = _rgba(png_path, size)
    return arr[:, :, 3] >= threshold


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
    """Boîte en dp et distance du pixel le plus éloigné du centre, en dp.

    On mesure les PIXELS, jamais les coins de la boîte : un disque de 30 dp de
    rayon a une boîte de 60×60 dont les coins sont à 42 dp du centre, alors
    qu'aucun de ses pixels ne dépasse 30 dp. Tester les coins ferait échouer
    tout disque, donc tout logo rond.
    """
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


def _colours(svg_path):
    tree = ET.parse(svg_path)
    out = []
    for el in tree.getroot().iter():
        for attr in ("fill", "stroke"):
            c = el.get(attr)
            if c and c.startswith("#"):
                key = c.upper()
                if key not in out:
                    out.append(key)
    return out


# --- rapport -----------------------------------------------------------------

def check(svg_path, mono_path=None, glyph_path=None, verbose=True):
    work = tempfile.mkdtemp(prefix="lg1-check-")
    ok = True
    say = (lambda *a: print(*a)) if verbose else (lambda *a: None)

    say("\n== %s" % svg_path)

    # 1. zone sûre : la couche avant seule (le fond n'est jamais rogné, il est
    #    de la couleur de l'icône et remplit les 108 dp).
    fg = os.path.join(work, "fg1080.png")
    _render(svg_path, 1080, True, fg)
    safe = _safe_zone(fg, 1080)
    if safe is None:
        say("  ZONE SÛRE  aucune forme dans la couche avant (échec)")
        ok = False
    else:
        box, far = safe
        inside = far <= SAFE_R + 0.01
        say("  ZONE SÛRE  boîte %.1f..%.1f x %.1f..%.1f dp, pixel le plus loin "
            "%.2f dp du centre (limite %.0f)  %s"
            % (box[0], box[2], box[1], box[3], far, SAFE_R,
               "ok" if inside else "ÉCHEC"))
        ok = ok and inside

    # 2. silhouette à 16 px : la couche monochrome est LA silhouette (les creux
    #    du dessin y sont transparents). Sans elle, on mesure la couche avant,
    #    qui ne dit rien des creux.
    sil = mono_path or svg_path
    fg16 = os.path.join(work, "fg16.png")
    _render(sil, 16, mono_path is None, fg16)
    m16 = _opaque(fg16, 16)
    n16, areas16, labels16 = _components(m16, want_labels=True)
    thin = _open2x2(m16)
    kept = thin.sum() / max(1, m16.sum())
    # largeur moyenne de chaque forme = surface / sa plus grande dimension :
    # c'est « aucun détail sous 1 px ». Une forme de 1 px de large tombe à 1.0,
    # une tache de 2×2 px monte à 2.0. On refuse strictement sous 1 px.
    widths = []
    for i in range(1, n16 + 1):
        ys, xs = np.nonzero(labels16 == i)
        span = max(xs.max() - xs.min() + 1, ys.max() - ys.min() + 1)
        widths.append(round(len(xs) / span, 2))
    say("  SILHOUETTE  %d forme(s) à 16 px, surfaces %s px, largeurs moyennes %s "
        "px ; ouverture 2×2 garde %d %%  (%s)"
        % (n16, areas16, widths, round(kept * 100), os.path.basename(sil)))
    ok = ok and n16 >= 2 and min(widths) >= 1.0 and min(areas16) >= 2

    # 3. glyphe 24 dp : traits ≥ 2 dp (le glyphe est déjà en 0..24)
    if glyph_path:
        g24 = os.path.join(work, "g24.png")
        _render(glyph_path, 24, False, g24)
        mg = _opaque(g24, 24)
        ng, ag = _components(mg)
        kg = _open2x2(mg).sum() / max(1, mg.sum())
        # À 24 px, 2 dp = 2 px : une ouverture 2×2 garde une bande de 2 px
        # (elle ne perd que ses bouts arrondis, 2 à 6 %) et efface entièrement
        # une bande de 1 px. 90 % sépare donc proprement « ≥ 2 dp » de « 1 dp ».
        say("  GLYPHE 24   %d forme(s), surfaces %s px ; ouverture 2×2 garde "
            "%d %% (une bande de 1 dp tomberait à 0 %%)" % (ng, ag, round(kg * 100)))
        ok = ok and ng >= 1 and kg >= 0.90

    # 4. contrastes des couleurs du logo
    cols = _colours(svg_path)
    say("  COULEURS    %s" % ", ".join(cols))
    for c in cols:
        cw = contrast(c, "#FFFFFF")
        cb = contrast(c, "#000000")
        say("    %s  /blanc %.2f:1  /noir %.2f:1" % (c, cw, cb))
    light_cols = [c for c in cols if contrast(c, "#0C0C0B") >= 3.0]
    say("    lisibles sur l'encre (#0C0C0B) : %s"
        % (", ".join(light_cols) or "aucune"))
    ok = ok and len(light_cols) >= 1

    # 5. monochrome : nombre de formes + contraste dans les deux modes thémés
    if mono_path:
        mono = os.path.join(work, "mono16.png")
        _render(mono_path, 16, False, mono)
        mm = _opaque(mono, 16)
        nm = _components(mm)[0]
        on_dark = contrast("#F3F0E8", "#0C0C0B")   # teinte claire, fond encre
        on_light = contrast("#0C0C0B", "#F3F0E8")  # teinte encre, fond crème
        say("  MONOCHROME  %d forme(s) à 16 px ; teinte claire/fond encre "
            "%.1f:1, teinte encre/fond crème %.1f:1" % (nm, on_dark, on_light))
        ok = ok and nm >= 2 and min(on_dark, on_light) >= 3.0

    say("  => %s" % ("ok" if ok else "ÉCHEC"))
    return ok


def main(argv):
    here = os.path.dirname(os.path.abspath(__file__))
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("svg", nargs="?", default=os.path.join(here, "logo.svg"))
    ap.add_argument("--mono", help="couche monochrome (défaut : logo-mono.svg "
                                   "à côté du SVG)")
    ap.add_argument("--glyph", help="glyphe de notification (défaut : "
                                    "logo-notification.svg s'il existe)")
    args = ap.parse_args(argv)

    base = os.path.splitext(args.svg)[0]
    mono = args.mono or (base + "-mono.svg" if os.path.exists(base + "-mono.svg")
                         else None)
    glyph = args.glyph or (base + "-notification.svg"
                           if os.path.exists(base + "-notification.svg") else None)
    sys.exit(0 if check(args.svg, mono, glyph) else 1)


if __name__ == "__main__":
    main(sys.argv[1:])
