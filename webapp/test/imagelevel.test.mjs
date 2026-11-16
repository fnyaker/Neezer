// The level of a picture, normalised for what the glass shows: a sleeve's
// brightness must land in a band, not at a constant. Too bright and the page
// behind the pane is a wash; too dark and it is a hole. Between the two bands
// the art is left ALONE — the point is a controlled level, not a flat one.
import test from "node:test";
import assert from "node:assert/strict";
import { levelFactor, LEVEL_LOW, LEVEL_HIGH, LEVEL_MIN, LEVEL_MAX } from "../src/lib/color.js";

const grey = (v) => [v, v, v];
const luma = (rgb) => 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];

test("une image claire est baissée, une sombre relevée, le milieu ne bouge pas", () => {
  assert.equal(levelFactor(grey(LEVEL_LOW)), 1);
  assert.equal(levelFactor(grey(LEVEL_HIGH)), 1);
  for (const v of [LEVEL_LOW, 80, 100, LEVEL_HIGH]) {
    assert.equal(levelFactor(grey(v)), 1, `${v} devrait rester intact`);
  }
  for (const v of [190, 220, 255]) {
    const f = levelFactor(grey(v));
    assert.ok(f < 1, `${v} devrait être baissé (${f})`);
    assert.ok(f >= LEVEL_MIN, `${v} : facteur ${f} sous le plancher`);
  }
  for (const v of [0, 8, 20, 40, 55]) {
    const f = levelFactor(grey(v));
    assert.ok(f > 1, `${v} devrait être relevé (${f})`);
    assert.ok(f <= LEVEL_MAX, `${v} : facteur ${f} au-dessus du plafond`);
  }
});

test("le résultat reste dans une bande, sans devenir constant", () => {
  // The level AFTER the factor, over the 10..255 range (a multiply cannot lift
  // true black, and that is honest): the extremes are pulled toward the band,
  // and the span that is left is what keeps the mood.
  const after = [];
  for (let v = 10; v <= 255; v += 5) after.push(v * levelFactor(grey(v)));
  const min = Math.min(...after);
  const max = Math.max(...after);
  // A 25:1 raw span comes out as ~10:1 (15 → 166): controlled, not flat.
  assert.ok(max / min <= 12, `bande trop large : ${min.toFixed(0)}–${max.toFixed(0)}`);
  assert.ok(max / min >= 6, `bande trop plate : ${min.toFixed(0)}–${max.toFixed(0)}`);
  assert.ok(max <= 170, `le haut sort de la bande : ${max.toFixed(0)}`);
  assert.ok(min >= 12, `le bas sort de la bande : ${min.toFixed(0)}`);
});

test("la mesure suit la luma, pas le maximum d'un canal", () => {
  // A saturated red is bright in luma (0.2126·255) and a blue is not (0.0722):
  // the factor must read the eye, or a red sleeve would be flattened while a
  // blue one stayed a hole.
  const red = levelFactor([255, 0, 0]);
  const blue = levelFactor([0, 0, 255]);
  assert.ok(blue > red, `bleu ${blue} devrait être relevé plus que rouge ${red}`);
  assert.ok(levelFactor([0, 0, 0]) === LEVEL_MAX, "le noir doit être au plafond");
  assert.equal(levelFactor(null), 1, "une image illisible ne change rien");
  assert.equal(levelFactor([120, 120, 120]), 1, "le gris moyen est dans la bande");
  assert.ok(Math.abs(luma([30, 30, 30]) - 30) < 0.01);
});
