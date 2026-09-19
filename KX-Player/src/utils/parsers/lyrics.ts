/** LRC 歌词解析（移植自旧 script.js，补充小时位与正/负时间容错）。 */

export interface LyricLine {
  time: number
  text: string
}

export function parseLrc(content: string): LyricLine[] {
  const lines = content.split(/\r?\n/)
  const timeRegex = /\[(\d{1,2}):(\d{2})(?:\.(\d{1,3}))?\]/g
  const textRegex = /\[\d{1,2}:\d{2}(?:\.\d{1,3})?\]/g
  const seen = new Map<number, string>()
  for (const line of lines) {
    const timestamps = [...line.matchAll(timeRegex)]
    if (timestamps.length === 0) continue
    const text = line.replace(textRegex, '').trim()
    if (!text) continue
    for (const ts of timestamps) {
      const min = parseInt(ts[1], 10)
      const sec = parseInt(ts[2], 10)
      const ms = ts[3] ? parseInt(ts[3].padEnd(3, '0'), 10) : 0
      const time = min * 60 + sec + ms / 1000
      if (!seen.has(time)) seen.set(time, text)
    }
  }
  return [...seen.entries()].map(([time, text]) => ({ time, text })).sort((a, b) => a.time - b.time)
}

/**
 * WebVTT / SRT 解析。
 * VTT 用句点毫秒（00:00:21.813），SRT 用逗号（00:00:21,813）——两者都接受。
 * 跳过 cue 序号（纯数字行）与 NOTE/STYLE 块。
 */
export function parseVttOrSrt(content: string, format: 'vtt' | 'srt'): LyricLine[] {
  const lines = content.split(/\r?\n/)
  // 同时支持 HH:MM:SS.mmm 与 VTT 短格式 MM:SS.mmm；句点/逗号毫秒都接受
  const timestampRegex = /(?:(\d{1,2}):)?(\d{1,2}):(\d{2})[.,](\d{1,3})\s*-->\s*(?:(?:\d{1,2}):)?(?:\d{1,2}):\d{2}[.,]\d{1,3}/
  const seen = new Map<number, string>()
  let i = 0
  if (format === 'vtt' && lines[0] && lines[0].trim().toUpperCase().startsWith('WEBVTT')) i = 1
  for (; i < lines.length; i++) {
    const line = lines[i]
    if (/^\s*(NOTE|STYLE|REGION)\b/.test(line)) {
      // 跳过整个块（到空行为止）
      i++
      while (i < lines.length && lines[i].trim()) i++
      i--
      continue
    }
    const tsMatch = line.match(timestampRegex)
    if (!tsMatch) continue
    const h = tsMatch[1] ? parseInt(tsMatch[1], 10) : 0
    const m = parseInt(tsMatch[2], 10)
    const s = parseInt(tsMatch[3], 10)
    const ms = parseInt(tsMatch[4].padEnd(3, '0'), 10)
    const time = h * 3600 + m * 60 + s + ms / 1000
    let text = ''
    for (let j = i + 1; j < lines.length; j++) {
      const nextLine = lines[j].trim()
      if (!nextLine) break
      if (/^\d+$/.test(nextLine)) continue
      // 去掉 VTT 内联标签 <c.xxx> / <00:00:00.000>
      text = nextLine.replace(/<[^>]+>/g, '').trim()
      break
    }
    if (text && !seen.has(time)) seen.set(time, text)
  }
  return [...seen.entries()].map(([time, text]) => ({ time, text })).sort((a, b) => a.time - b.time)
}

/** 二分定位当前应高亮的行：返回最后一个 time <= t 的下标，没有则 -1。 */
export function activeLyricIndex(lyrics: LyricLine[], t: number): number {
  if (!lyrics.length) return -1
  let lo = 0
  let hi = lyrics.length - 1
  let ans = -1
  while (lo <= hi) {
    const mid = (lo + hi) >> 1
    if (lyrics[mid].time <= t) { ans = mid; lo = mid + 1 } else { hi = mid - 1 }
  }
  return ans
}
