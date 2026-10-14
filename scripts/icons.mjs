// Rasterizes the shared icon set (assets/icons/*.svg, 24x24 line icons drawn with
// currentColor) into white-on-transparent PNGs in assets/icons/png/, which the desktop
// app embeds and tints with its palette. Re-run after adding or changing an icon:
//
//   node scripts/icons.mjs [size]        (default 48 px)
//
// Needs Playwright with a Chromium; PLAYWRIGHT (the module) and CHROMIUM (the browser
// binary) override where they're looked for. Also writes assets/icons/sheet.png, every
// icon on one sheet, for looking them over.
import { readdirSync, readFileSync, mkdirSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const pw = await import(process.env.PLAYWRIGHT || "/opt/node22/lib/node_modules/playwright/index.mjs");
const chromium = pw.chromium || pw.default.chromium;
const size = Number(process.argv[2] || 48);
const dir = join(root, "assets/icons");
const out = join(dir, "png");
mkdirSync(out, { recursive: true });

const names = readdirSync(dir).filter((f) => f.endsWith(".svg")).map((f) => f.slice(0, -4)).sort();
const browser = await chromium.launch({ executablePath: process.env.CHROMIUM || "/opt/pw-browsers/chromium" });
const page = await browser.newPage({ deviceScaleFactor: 1 });
await page.setContent(`<html><body style="margin:0;background:transparent;color:#fff"></body></html>`);
for (const name of names) {
  const svg = readFileSync(join(dir, name + ".svg"), "utf8")
    .replace(/width="24"/, `width="${size}"`)
    .replace(/height="24"/, `height="${size}"`);
  await page.evaluate((svg) => {
    document.body.innerHTML = `<div id="i" style="width:fit-content;line-height:0">${svg}</div>`;
  }, svg);
  await page.locator("#i").screenshot({ path: join(out, name + ".png"), omitBackground: true });
}
// a contact sheet on the app's sand, to look the set over
const cells = names
  .map((n) => `<div style="display:flex;flex-direction:column;align-items:center;gap:4px;width:84px;font:11px sans-serif">
     <div style="line-height:0">${readFileSync(join(dir, n + ".svg"), "utf8")}</div>${n}</div>`)
  .join("");
await page.setViewportSize({ width: 940, height: 200 });
await page.setContent(`<html><body style="margin:0"><div id="s" style="display:flex;flex-wrap:wrap;gap:10px;padding:14px;width:910px;background:#c2ab7f;color:#262626">${cells}</div></body></html>`);
await page.locator("#s").screenshot({ path: join(dir, "sheet.png") });
await browser.close();
console.log(`${names.length} icons -> ${out} (${size}px)`);
