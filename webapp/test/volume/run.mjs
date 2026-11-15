#!/usr/bin/env node
// The desktop player bar's level, measured where the person uses it.
//
//   DIST=<dir> node test/volume/run.mjs   (npx vite build --outDir <dir> first)
//
// The bar's level is a fader now: an 8 px track, a 22 px thumb whose centre
// rides the track's own ends, a 44 px hit box, and the value under it. The four
// things a slider gets wrong: the thumb is not centred on its track (vertically
// or at the ends), the hit area is a 4 px line, a click does not land where it
// looks, and the value cannot be read. Measured at 1920/1440/1280/1181 (the
// slider's floor is 1180) and 641, where the bar is still the desktop one and
// the level is deliberately not mounted (no room — the note in PlayerBar).
import { existsSync } from "node:fs";
import { loadPlaywright, startServer, login, checker } from "../harness.mjs";

const { chromium } = await loadPlaywright();
const { base, stop } = await startServer();
const { check, state } = checker();
const SHOTS = process.env.SHOTS;
const say = (s) => console.log(s);

const browser = await chromium.launch({
  executablePath: process.env.CHROMIUM || (existsSync("/usr/bin/chromium") ? "/usr/bin/chromium" : undefined),
  args: ["--no-sandbox", "--autoplay-policy=no-user-gesture-required"],
});

const read = () =>
  (() => {
    const wrap = document.querySelector(".vol-wrap");
    const track = document.querySelector(".vol-track");
    const thumb = document.querySelector(".vol-thumb");
    const input = document.querySelector("input.vol");
    const val = document.querySelector(".vol-val");
    const bar = document.querySelector("footer.player").getBoundingClientRect();
    const r = (e) => e.getBoundingClientRect();
    const t = track ? r(track) : null;
    const h = thumb ? r(thumb) : null;
    const i = input ? r(input) : null;
    const w = wrap ? r(wrap) : null;
    return {
      display: wrap ? getComputedStyle(wrap).display : "none",
      p: wrap ? +getComputedStyle(wrap).getPropertyValue("--p") : null,
      wrap: w && { x: w.x, y: w.y, w: w.width, h: w.height },
      track: t && { x: t.x, y: t.y, w: t.width, h: t.height, cx: t.x + t.width / 2, cy: t.y + t.height / 2 },
      thumb: h && { x: h.x, y: h.y, w: h.width, h: h.height, cx: h.x + h.width / 2, cy: h.y + h.height / 2 },
      input: i && { x: i.x, y: i.y, w: i.width, h: i.height },
      value: input?.value,
      valuetext: input?.getAttribute("aria-valuetext"),
      caption: val?.textContent.trim(),
      captionBox: val && (() => {
        const b = r(val);
        return { top: b.top, bottom: b.bottom };
      })(),
      bar: { left: bar.left, right: bar.right, top: bar.top, bottom: bar.bottom, h: bar.height },
      docOverflow: document.documentElement.scrollWidth - document.documentElement.clientWidth,
      muteVisible: !!document.querySelector(".vol-ic") && getComputedStyle(document.querySelector(".vol-ic")).display !== "none",
    };
  })();

async function open(ctx) {
  const page = await ctx.newPage();
  page.on("pageerror", (e) => say("pageerror: " + e.message));
  await page.goto(base + "/app/");
  await login(page, base);
  await page.goto(base + "/app/");
  await page.waitForSelector("footer.player", { timeout: 30000 });
  await page.waitForTimeout(500);
  return page;
}

const setVolume = async (page, v) =>
  page.evaluate((v) => {
    const i = document.querySelector("input.vol");
    i.value = String(v);
    i.dispatchEvent(new Event("input", { bubbles: true }));
  }, v);

for (const [w, h] of [
  [1920, 900],
  [1440, 900],
  [1280, 900],
  [1181, 900],
]) {
  const tag = `${w}px`;
  const ctx = await browser.newContext({ viewport: { width: w, height: h } });
  const page = await open(ctx);

  const at = async (v) => {
    await setVolume(page, v);
    await page
      .waitForFunction(
        (v) => Math.abs(+getComputedStyle(document.querySelector(".vol-wrap")).getPropertyValue("--p") - v) < 0.001,
        v,
        { timeout: 2000 }
      )
      .catch(() => {});
    const m = await page.evaluate(read);
    const track = m.track;
    const thumb = m.thumb;
    const dx = Math.abs(thumb.cx - (track.x + v * track.w));
    const dy = Math.abs(thumb.cy - track.cy);
    return { m, dx, dy };
  };

  const zero = await at(0);
  const half = await at(0.5);
  const full = await at(1);
  const fourteen = await at(0.14);

  check(zero.m.display !== "none", `${tag} : le niveau est monté`, zero.m.display);
  check(
    zero.m.input && zero.m.input.h >= 44 && zero.m.input.w >= 44,
    `${tag} : zone cliquable ${zero.m.input?.w}×${zero.m.input?.h} (≥ 44)`,
    ""
  );
  check(zero.m.track && Math.abs(zero.m.track.h - 8) < 0.5, `${tag} : piste de 8 px`, `${zero.m.track?.h}`);
  check(
    zero.m.thumb && Math.abs(zero.m.thumb.w - 22) < 0.5 && Math.abs(zero.m.thumb.h - 22) < 0.5,
    `${tag} : pouce de 22 px`,
    `${zero.m.thumb?.w}×${zero.m.thumb?.h}`
  );
  check(
    [zero, half, full].every((x) => x.dy < 0.5),
    `${tag} : pouce centré verticalement sur la piste (Δ ≤ 0,5 px)`,
    `${zero.dy.toFixed(2)} / ${half.dy.toFixed(2)} / ${full.dy.toFixed(2)}`
  );
  check(
    [zero, half, full, fourteen].every((x) => x.dx < 0.5),
    `${tag} : centre du pouce sur la piste à 0 / 50 / 100 / 14 % (Δ ≤ 0,5 px)`,
    [zero, half, full, fourteen].map((x) => x.dx.toFixed(2)).join(" / ")
  );
  check(
    Math.abs(zero.m.p) < 0.001 && Math.abs(full.m.p - 1) < 0.001,
    `${tag} : les extrémités atteignent 0 et 100 %`,
    `${zero.m.p} → ${full.m.p}`
  );
  const fill = await page.evaluate(() => {
    const f = document.querySelector(".vol-fill").getBoundingClientRect();
    const t = document.querySelector(".vol-track").getBoundingClientRect();
    return f.width / t.width;
  });
  check(Math.abs(fill - 0.14) < 0.02, `${tag} : le remplissage est dans la piste (14 %)`, `${(fill * 100).toFixed(1)} %`);
  check(
    zero.m.caption === "0 %" && full.m.caption === "100 %" && fourteen.m.caption === "14 %",
    `${tag} : la valeur se lit (${fourteen.m.caption})`,
    `${zero.m.caption} / ${fourteen.m.caption} / ${full.m.caption}`
  );
  check(
    fourteen.m.valuetext === fourteen.m.caption,
    `${tag} : la valeur annoncée est la même`,
    `${fourteen.m.valuetext} / ${fourteen.m.caption}`
  );
  check(
    !!fourteen.m.captionBox && fourteen.m.captionBox.top >= fourteen.m.thumb.y + fourteen.m.thumb.h - 0.5,
    `${tag} : la valeur ne recouvre pas le pouce`,
    `valeur en haut ${fourteen.m.captionBox?.top?.toFixed(1)}, pouce bas ${(fourteen.m.thumb.y + fourteen.m.thumb.h).toFixed(1)}`
  );

  // A real click at 14 % of the TRACK (not of the box: the thumb's radius is
  // the track's own inset).
  await page.mouse.click(
    fourteen.m.track.x + 0.14 * fourteen.m.track.w,
    fourteen.m.track.y + fourteen.m.track.h / 2
  );
  await page
    .waitForFunction(() => document.querySelector("input.vol").value === "0.14", null, { timeout: 1500 })
    .catch(() => {});
  const clicked = await page.evaluate(read);
  check(
    clicked.value === "0.14" && clicked.caption === "14 %",
    `${tag} : un clic à 14 % de la piste règle 14 %`,
    `${clicked.value} / ${clicked.caption}`
  );
  await page.mouse.click(
    clicked.track.x + 0.5 * clicked.track.w,
    clicked.track.y + clicked.track.h / 2
  );
  await page
    .waitForFunction(() => document.querySelector("input.vol").value === "0.5", null, { timeout: 1500 })
    .catch(() => {});
  const middle = await page.evaluate(read);
  check(middle.value === "0.5", `${tag} : un clic au milieu règle 50 %`, middle.value);

  await page.evaluate(() => document.querySelector("input.vol").focus());
  await page.keyboard.press("ArrowLeft");
  await page.waitForTimeout(60);
  const keyed = await page.evaluate(read);
  check(keyed.value === "0.49" || keyed.value === "0.5", `${tag} : les flèches règlent aussi (${keyed.value})`, "");

  const geometry = await page.evaluate(read);
  check(geometry.docOverflow === 0, `${tag} : aucun débordement de page`, `${geometry.docOverflow}`);
  check(
    geometry.wrap.x >= geometry.bar.left && geometry.wrap.x + geometry.wrap.w <= geometry.bar.right,
    `${tag} : le niveau tient dans la barre`,
    `${geometry.wrap.x}…${geometry.wrap.x + geometry.wrap.w} / ${geometry.bar.left}…${geometry.bar.right}`
  );
  check(
    geometry.captionBox.bottom <= geometry.bar.bottom - 2,
    `${tag} : la valeur reste dans la barre`,
    `${geometry.captionBox.bottom.toFixed(1)} / ${geometry.bar.bottom}`
  );

  // Muting is still the icon's job, and the fader follows it.
  await page.locator(".vol-ic").click();
  await page
    .waitForFunction(() => document.querySelector("input.vol").value === "0", null, { timeout: 1500 })
    .catch(() => {});
  const muted = await page.evaluate(read);
  check(muted.value === "0" && muted.caption === "0 %", `${tag} : muet met le niveau à 0`, `${muted.value} / ${muted.caption}`);
  await page.locator(".vol-ic").click();
  await page
    .waitForFunction(() => document.querySelector("input.vol").value !== "0", null, { timeout: 1500 })
    .catch(() => {});
  const back = await page.evaluate(read);
  check(back.value === "0.49" || back.value === "0.5", `${tag} : rétablir rend son niveau`, back.value);

  say(
    `${tag} : piste ${geometry.track.w}×${geometry.track.h}, pouce ${geometry.thumb.w}, ` +
      `zone ${geometry.input.w}×${geometry.input.h}, Δ vertical ${half.dy.toFixed(2)} px, clic 14 % → ${clicked.value}`
  );
  if (SHOTS) await page.locator("footer.player").screenshot({ path: `${SHOTS}/volume-${w}.png` });
  await ctx.close();
}

{
  const ctx = await browser.newContext({ viewport: { width: 641, height: 900 } });
  const page = await open(ctx);
  const m = await page.evaluate(read);
  check(m.bar.h === 88, "641px : la barre desktop existe (88 px)", `${m.bar.h}`);
  check(m.display === "none", "641px : le niveau n'est pas monté (pas de place — règle ≤ 1180 px)", m.display);
  check(m.muteVisible, "641px : le son reste accessible par l'icône", "");
  check(m.docOverflow === 0, "641px : aucun débordement", `${m.docOverflow}`);
  if (SHOTS) await page.locator("footer.player").screenshot({ path: `${SHOTS}/volume-641.png` });
  await ctx.close();
}

{
  const mobile = await browser.newContext({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });
  const phone = await open(mobile);
  check(
    !(await phone.locator(".vol-wrap").isVisible()),
    "390px : pas de barre desktop sur le téléphone (la carte seulement)",
    ""
  );
  await mobile.close();
}

await browser.close();
stop();
console.log(state.failed ? `\n${state.failed} check(s) failed` : "\nall volume checks passed");
process.exit(state.failed ? 1 : 0);
