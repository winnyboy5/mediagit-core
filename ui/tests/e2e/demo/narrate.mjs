// Synthesize every demo caption once into OUT/tts-cache/<sha1(engine|voice|text)>.wav and write manifest.json
// ({ text: { wav, durationMs } }). demo.ts only reads the manifest; it never synthesizes.
// MG_DEMO_TTS = kitten (default; local KittenTTS HTTP, MG_DEMO_VOICE=expr-voice-3-m, MG_DEMO_SPEED=1.1) | sapi (fallback) | none
// Usage: node narrate.mjs [OUT]
import { execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'

const OUT = process.argv[2] ?? process.env.MG_DEMO_OUT ?? 'D:\\own\\saas\\qa-live\\demo'
const engine = process.env.MG_DEMO_TTS ?? 'kitten'
if (engine === 'none') process.exit(0)
if (!['sapi', 'kitten'].includes(engine)) { console.error(`narrate: unknown MG_DEMO_TTS=${engine}`); process.exit(2) }
const speed = Number(process.env.MG_DEMO_SPEED ?? 1.1)
const voice = engine === 'sapi' ? 'Microsoft Zira Desktop' : (process.env.MG_DEMO_VOICE ?? 'expr-voice-3-m')
const voiceKey = engine === 'kitten' ? `${voice}@${speed}` : voice

const dir = import.meta.dirname
const unesc = (s) => s.replace(/\\(.)/g, '$1')
const texts = new Set()
let literal = 0
for (const f of readdirSync(dir).filter((f) => /^\d\d-.*\.demo\.ts$/.test(f))) {
  const src = readFileSync(join(dir, f), 'utf8')
  for (const m of src.matchAll(/caption\(\s*'((?:[^'\\]|\\.)*)'/g)) { texts.add(unesc(m[1])); literal++ }
  // Captions picked from a table (chapter 05 compare styles): [tab, 'caption'] rows inside `const styles = [...]`.
  const tbl = src.match(/const styles[^=]*=\s*\[([\s\S]*?)\n\s*\]/)
  if (tbl) for (const m of tbl[1].matchAll(/\[\s*'[^']*',\s*'((?:[^'\\]|\\.)*)'\s*\]/g)) texts.add(unesc(m[1]))
}
console.log(`narrate: ${literal} literal caption(...) calls, ${texts.size} unique texts, engine ${engine}`)

const cache = join(OUT, 'tts-cache')
mkdirSync(cache, { recursive: true })
const psFile = join(cache, '_sapi.ps1')
writeFileSync(psFile, [
  'param([string]$TextFile,[string]$Wav,[string]$Voice)',
  'Add-Type -AssemblyName System.Speech',
  '$s = New-Object System.Speech.Synthesis.SpeechSynthesizer',
  '$s.SelectVoice($Voice)',
  '$s.SetOutputToWaveFile($Wav)',
  '$s.Speak([IO.File]::ReadAllText($TextFile))',
  '$s.Dispose()',
].join('\r\n'))

const manifest = {}
let made = 0
for (const text of texts) {
  const wav = join(cache, createHash('sha1').update(`${engine}|${voiceKey}|${text}`).digest('hex') + '.wav')
  if (!existsSync(wav)) {
    try {
      if (engine === 'sapi') {
        const tf = join(cache, '_text.txt')
        writeFileSync(tf, text, 'utf8')
        execFileSync('powershell', ['-NoProfile', '-File', psFile, tf, wav, voice], { stdio: 'pipe' })
      } else {
        const r = await fetch('http://127.0.0.1:3900/v1/audio/speech', { method: 'POST', headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ model: 'kittentts', voice, input: text, response_format: 'wav', speed }) })
        if (r.status !== 200) throw new Error(`HTTP ${r.status} ${(await r.text()).slice(0, 300)}`)
        writeFileSync(wav, Buffer.from(await r.arrayBuffer()))
      }
    } catch (e) {
      console.error(`narrate: ${engine} failed on "${text}":\n${e.stderr?.toString() || e.stdout?.toString() || e.message}`)
      process.exit(1)
    }
    made++
  }
  const dur = Number(execFileSync('ffprobe', ['-v', 'error', '-show_entries', 'format=duration', '-of', 'csv=p=0', wav], { encoding: 'utf8' }).trim())
  manifest[text] = { wav, durationMs: Math.round(dur * 1000) }
}
writeFileSync(join(cache, 'manifest.json'), JSON.stringify(manifest, null, 2))
console.log(`narrate: ${texts.size} texts, ${made} synthesized, ${texts.size - made} cached`)
