// Ad-hoc DOM probe over CDP: node probe.mjs <hash route> <css selector>
import { chromium } from '@playwright/test'
const [route, sel] = process.argv.slice(2)
const b = await chromium.connectOverCDP('http://127.0.0.1:9333')
const page = b.contexts()[0].pages()[0]
if (route) { await page.evaluate((p) => { window.location.hash = '#/' + p }, route); await page.waitForTimeout(2000) }
const out = await page.evaluate((s) => [...document.querySelectorAll(s)].slice(0, 4).map((e) => e.outerHTML.replace(/<svg[\s\S]*?<\/svg>/g, '').slice(0, 700)), sel)
console.log(out.join('\n---\n') || '(nothing matches)')
await b.close()
