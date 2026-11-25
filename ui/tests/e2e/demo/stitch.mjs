// Stitch the chapter clips into one mp4 with a 2 s title card before each chapter.
// Reads OUT/NN-slug.json (written by demo.ts stop()): { id, title, clip }.
// Usage: node stitch.mjs [OUT]   (default D:\own\saas\qa-live\demo)
import { execFileSync } from 'node:child_process'
import { copyFileSync, existsSync, readdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'

const OUT = process.argv[2] ?? process.env.MG_DEMO_OUT ?? 'D:\\own\\saas\\qa-live\\demo'
const run = (cmd, args) => execFileSync(cmd, args, { cwd: OUT, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] })

copyFileSync('C:/Windows/Fonts/segoeui.ttf', join(OUT, 'segoeui.ttf')) // relative fontfile avoids drive-colon escaping
const chapters = readdirSync(OUT)
  .filter((f) => /^\d\d-.*\.json$/.test(f))
  .map((f) => JSON.parse(readFileSync(join(OUT, f), 'utf8')))
  .filter((c) => existsSync(join(OUT, c.clip)))
  .sort((a, b) => a.id.localeCompare(b.id))
if (!chapters.length) { console.error('stitch: no chapter clips found'); process.exit(1) }

let bad = 0
const list = []
for (const c of chapters) {
  const p = JSON.parse(run('ffprobe', ['-v', 'error', '-select_streams', 'v:0', '-show_entries', 'stream=width,height,r_frame_rate:format=duration', '-of', 'json', c.clip]))
  const s = p.streams[0]
  const ok = s.width === 1920 && s.height === 1080 && s.r_frame_rate === '30/1'
  console.log(`${c.id} ${c.title}: ${s.width}x${s.height} ${s.r_frame_rate} ${Number(p.format.duration).toFixed(1)}s ${ok ? 'ok' : 'BAD'}`)
  if (!ok) bad++
  const card = `card-${c.id}.mp4`
  writeFileSync(join(OUT, `card-${c.id}.txt`), `Chapter ${Number(c.id)}\n${c.title}`)
  run('ffmpeg', ['-y', '-loglevel', 'error', '-f', 'lavfi', '-i', 'color=c=0x14141c:s=1920x1080:r=30:d=2',
    '-vf', `drawtext=fontfile=segoeui.ttf:textfile=card-${c.id}.txt:fontcolor=white:fontsize=64:line_spacing=24:x=(w-text_w)/2:y=(h-text_h)/2`,
    '-c:v', 'libx264', '-crf', '18', '-pix_fmt', 'yuv420p', '-r', '30', card])
  list.push(`file '${card}'`, `file '${c.clip}'`)
}
if (bad) { console.error(`stitch: ${bad} clip(s) are not 1920x1080 at 30 fps`); process.exit(1) }
writeFileSync(join(OUT, 'stitch-list.txt'), list.join('\n') + '\n')
run('ffmpeg', ['-y', '-loglevel', 'error', '-f', 'concat', '-safe', '0', '-i', 'stitch-list.txt', '-c', 'copy', 'mediagit-demo.mp4'])
const d = JSON.parse(run('ffprobe', ['-v', 'error', '-show_entries', 'format=duration', '-of', 'json', 'mediagit-demo.mp4']))
console.log(`stitch: ${chapters.length} chapter(s), mediagit-demo.mp4 ${Number(d.format.duration).toFixed(1)}s`)
