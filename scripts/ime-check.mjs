// an IME composition in the bar and the notes pad, through Chromium's own IME path (CDP)
// usage: node scripts/ime-check.mjs shot.png  (a hub with the web app on 127.0.0.1:8090,
// `anend-hub --mode app --local`; Playwright's chromium; CHROMIUM=path picks the browser)
import { chromium } from 'playwright';
const browser = await chromium.launch(process.env.CHROMIUM ? { executablePath: process.env.CHROMIUM } : {});
const page = await browser.newPage({ viewport: { width: 1280, height: 820 } });
await page.goto('http://127.0.0.1:8090/app/');
await page.waitForTimeout(2500);
const cdp = await page.context().newCDPSession(page);
const out = {};
// the memory view's bar: compose にほん, press Enter while composing (it picks the candidate,
// it must not search), then commit 日本
await page.keyboard.press('Control+1');
await page.waitForTimeout(800);
await page.click('#bar');
const status0 = await page.evaluate(() => document.querySelector('.top .status')?.textContent || '');
await cdp.send('Input.imeSetComposition', { text: 'にほん', selectionStart: 3, selectionEnd: 3 });
await page.waitForTimeout(200);
out.composing = await page.evaluate(() => document.getElementById('bar').value);
await cdp.send('Input.dispatchKeyEvent', { type: 'rawKeyDown', key: 'Enter', code: 'Enter', windowsVirtualKeyCode: 229, nativeVirtualKeyCode: 229 });
await cdp.send('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Enter', code: 'Enter', windowsVirtualKeyCode: 229 });
await page.waitForTimeout(600);
out.status_after_ime_enter = await page.evaluate(() => document.querySelector('.top .status')?.textContent || '');
await cdp.send('Input.insertText', { text: '日本' });
await page.waitForTimeout(400);
out.committed = await page.evaluate(() => document.getElementById('bar').value);
out.status_before = status0;
// then a real Enter searches
await page.keyboard.press('Enter');
await page.waitForTimeout(1200);
out.status_after_enter = await page.evaluate(() => document.querySelector('.top .status')?.textContent || '');
// the notes pad
await page.keyboard.press('Control+5');
await page.waitForTimeout(800);
await page.mouse.click(600, 300);
await page.keyboard.press('Control+n');
await page.waitForTimeout(800);
await page.keyboard.type('IME');
await page.keyboard.press('Enter');
await cdp.send('Input.imeSetComposition', { text: 'かんじ', selectionStart: 3, selectionEnd: 3 });
await page.waitForTimeout(200);
await cdp.send('Input.insertText', { text: '漢字' });
await page.waitForTimeout(500);
out.pad = await page.evaluate(() => document.getElementById('note-pad').value);
await page.screenshot({ path: process.argv[2] });
console.log(JSON.stringify(out, null, 1));
await browser.close();
