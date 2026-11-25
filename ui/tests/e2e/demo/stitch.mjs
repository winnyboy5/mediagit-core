// Stitch the chapter clips into one mp4 with a title card before each chapter, intro/outro cards, silent card audio and an SRT.
// Reads OUT/NN-slug.json (written by demo.ts stop()): { id, title, clip }.
// Usage: node stitch.mjs [OUT]   (default D:\own\saas\qa-live\demo)
import { execFileSync } from 'node:child_process'
import { copyFileSync, existsSync, readdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'

const OUT = process.argv[2] ?? process.env.MG_DEMO_OUT ?? 'D:\\own\\saas\\qa-live\\demo'
const NL = String.fromCharCode(10)
const run =(cmd, args) => execFileSync(cmd, args, { cwd: OUT, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] })

copyFileSync('C:/Windows/Fonts/segoeui.ttf', join(OUT, 'segoeui.ttf')) // relative fontfile avoids drive-colon escaping
copyFileSync('C:/Windows/Fonts/seguisb.ttf', join(OUT, 'seguisb.ttf'))
const chapters = readdirSync(OUT)
  .filter((f) => /^\d\d-.*\.json$/.test(f))
  .map((f) => JSON.parse(readFileSync(join(OUT, f), 'utf8')))
  .filter((c) => existsSync(join(OUT, c.clip)))
  .sort((a, b) => a.id.localeCompare(b.id))
if (!chapters.length) { console.error('stitch: no chapter clips found'); process.exit(1) }

/** A dark title card with silent 48 kHz stereo audio: optional small muted line, a large centred title, optional muted subtitle. */
function card(file, { small, title, sub, dur }) {
  const base = file.replace(/\.mp4$/, '')
  const txt = (k, v) => { writeFileSync(join(OUT, `${base}-${k}.txt`), v); return `${base}-${k}.txt` }
  const dt = (f, t, size, color, y) => `drawtext=fontfile=${f}:textfile=${t}:fontcolor=${color}:fontsize=${size}:x=(w-text_w)/2:y=${y}`
  const vf = [
    small && dt('segoeui.ttf', txt('small', small), 30, '0x9a9aaa', '(h/2)-120'),
    dt('seguisb.ttf', txt('title', title), 64, 'white', '(h-text_h)/2'),
    sub && dt('segoeui.ttf', txt('sub', sub), 32, '0x9a9aaa', '(h/2)+70'),
    `fade=t=in:st=0:d=0.4,fade=t=out:st=${dur - 0.4}:d=0.4`,
  ].filter(Boolean).join(',')
  run('ffmpeg', ['-y', '-loglevel', 'error', '-f', 'lavfi', '-i', `color=c=0x14141c:s=1920x1080:r=30:d=${dur}`,
    '-f', 'lavfi', '-i', 'anullsrc=r=48000:cl=stereo', '-t', String(dur), '-vf', vf, '-pix_fmt', 'yuv420p', '-r', '30',
    '-c:v', 'libx264', '-crf', '18', '-c:a', 'aac', '-b:a', '192k', '-ar', '48000', '-ac', '2', file])
}

const srtTime = (ms) => { const t = Math.round(ms); const p = (n, w = 2) => String(n).padStart(w, '0'); return `${p(Math.floor(t / 3600000))}:${p(Math.floor(t / 60000) % 60)}:${p(Math.floor(t / 1000) % 60)},${p(t % 1000, 3)}` }

let bad = 0
let at = 0 // ms into the final video
const srt = []
const list = []
const add = (f, ms) => { list.push(`file '${f}'`); at += ms }
card('card-intro.mp4', { title: 'MediaGit', sub: 'Version control for big creative files', dur: 3 })
add('card-intro.mp4', 3000)
for (const c of chapters) {
  const p = JSON.parse(run('ffprobe', ['-v', 'error', '-show_entries', 'stream=codec_type,width,height,r_frame_rate,sample_rate,channels:format=duration', '-of', 'json', c.clip]))
  const s = p.streams.find((x) => x.codec_type === 'video')
  const au = p.streams.find((x) => x.codec_type === 'audio')
  const dur = Number(p.format.duration)
  const ok = s.width === 1920 && s.height === 1080 && s.r_frame_rate === '30/1' && au?.sample_rate === '48000' && au?.channels === 2
  console.log(`${c.id} ${c.title}: ${s.width}x${s.height} ${s.r_frame_rate} ${dur.toFixed(1)}s audio ${au ? `${au.sample_rate}Hz ${au.channels}ch` : 'MISSING'} ${ok ? 'ok' : 'BAD'}`)
  if (!ok) bad++
  const cardFile = `card-${c.id}.mp4`
  card(cardFile, { small: `MediaGit · Chapter ${Number(c.id)}`, title: c.title, dur: 2.5 })
  add(cardFile, 2500)
  for (const k of c.captions ?? []) {
    const st = at + k.startMs
    srt.push([srt.length + 1, `${srtTime(st)} --> ${srtTime(Math.min(at + k.endMs, at + dur * 1000))}`, k.text, ''].join(NL))
  }
  add(c.clip, dur * 1000)
}
card('card-outro.mp4', { title: 'MediaGit', sub: 'Every version of every file, safe', dur: 3 })
add('card-outro.mp4', 3000)
if (bad) { console.error(`stitch: ${bad} clip(s) are not 1920x1080 at 30 fps with 48 kHz stereo audio`); process.exit(1) }
writeFileSync(join(OUT, 'stitch-list.txt'), list.join(NL) + NL)
writeFileSync(join(OUT, 'mediagit-demo.srt'), srt.join(NL))
run('ffmpeg', ['-y', '-loglevel', 'error', '-f', 'concat', '-safe', '0', '-i', 'stitch-list.txt', '-c', 'copy', 'mediagit-demo.mp4'])
const d = JSON.parse(run('ffprobe', ['-v', 'error', '-show_entries', 'format=duration', '-of', 'json', 'mediagit-demo.mp4']))
console.log(`stitch: ${chapters.length} chapter(s), mediagit-demo.mp4 ${Number(d.format.duration).toFixed(1)}s, ${srt.length} subtitle(s)`)
