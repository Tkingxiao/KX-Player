import { readFileSync } from 'node:fs'
const lines = readFileSync(process.argv[2] ?? process.env.TEMP + '\\kx-nofont-debug.log', 'utf-8').split(/\r?\n/)
const timed = []
for (const l of lines) {
  const m = l.match(/^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3})Z?\s+(.+)$/)
  if (m) timed.push([new Date(m[1]).getTime(), m[2], l])
}
timed.sort((a, b) => a[0] - b[0])
let prev = timed[0][0]
let prevMsg = timed[0][2].slice(0, 160)
for (const [t, msg, raw] of timed) {
  const gap = t - prev
  if (gap > 1000) {
    console.log(`\n===== +${(gap / 1000).toFixed(1)}s silent gap ending ${new Date(t).toISOString().slice(11, 23)} =====`)
    console.log('LAST EVENT BEFORE GAP:')
    console.log('  ' + prevMsg)
    console.log('FIRST EVENT AFTER GAP:')
    console.log('  ' + raw.slice(0, 160))
  }
  prev = t
  prevMsg = raw.slice(0, 160)
}
