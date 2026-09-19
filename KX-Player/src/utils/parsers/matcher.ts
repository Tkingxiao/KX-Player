/** 歌词文件匹配：从目录条目中挑选最可能对应的 .lrc/.vtt/.srt（移植自旧 lyrics-matcher.js）。 */

function extractTrackSuffix(name: string): string {
  const m = name.match(/((?:[_\-](?:DAY|Disc|CD|Track|trk|part|vol|No|No\.|P)\w*)*[_\-](?:trk|track|Disc|CD|DAY|part|vol|No\.?|P)\w*[_\w]*?)$/i)
  if (m) return m[1]
  const m2 = name.match(/([_\-]\d{2,4})$/)
  return m2 ? m2[1] : ''
}

export interface LyricsCandidate {
  filename: string
  nameWithoutExt: string
  ext: string
}

export function findBestLyricsMatch(
  audioNameWithoutExt: string,
  audioExt: string,
  dirEntries: string[],
): LyricsCandidate | null {
  const lrcExts = new Set(['.lrc', '.vtt', '.srt'])
  const candidates: LyricsCandidate[] = []
  for (const entry of dirEntries) {
    const lower = entry.toLowerCase()
    const dotIdx = lower.lastIndexOf('.')
    if (dotIdx < 0) continue
    const entryExt = lower.slice(dotIdx)
    const entryName = entry.slice(0, dotIdx)
    let isLyricsFile = false
    if (lrcExts.has(entryExt)) {
      isLyricsFile = true
    } else if (dotIdx > 0 && lrcExts.has(lower.slice(lower.lastIndexOf('.', dotIdx - 1)))) {
      isLyricsFile = true
    }
    if (!isLyricsFile) continue
    candidates.push({ filename: entry, nameWithoutExt: entryName, ext: entryExt })
  }
  if (candidates.length === 0) return null

  // 1. 精确同名
  for (const c of candidates) {
    if (c.nameWithoutExt === audioNameWithoutExt || c.nameWithoutExt === audioNameWithoutExt + audioExt) return c
  }

  // 2. 轨道后缀（CD/Track/trk/Disc…）
  const audioSuffix = extractTrackSuffix(audioNameWithoutExt)
  if (audioSuffix) {
    let best: LyricsCandidate | null = null
    let bestScore = 0
    for (const c of candidates) {
      if (c.nameWithoutExt.endsWith(audioSuffix)) {
        if (audioSuffix.length > bestScore) { bestScore = audioSuffix.length; best = c }
      }
    }
    if (best) return best
  }

  // 3. 文件名末尾公共子串 ≥5 字符且以分隔符起始
  let best: LyricsCandidate | null = null
  let bestCommonLen = 0
  const a = audioNameWithoutExt.toLowerCase()
  for (const c of candidates) {
    let commonLen = 0
    const b = c.nameWithoutExt.toLowerCase()
    for (let i = 1; i <= Math.min(a.length, b.length); i++) {
      if (a[a.length - i] === b[b.length - i]) commonLen = i
      else break
    }
    if (commonLen >= 5 && commonLen > bestCommonLen) {
      const suffixStart = audioNameWithoutExt.length - commonLen
      if (suffixStart > 0 && /[_\-]/.test(audioNameWithoutExt[suffixStart])) {
        bestCommonLen = commonLen
        best = c
      }
    }
  }
  if (best) return best

  // 4. 目录里唯一歌词 + 唯一音频时兜底
  if (candidates.length === 1) {
    const audioExts = new Set(['.mp3', '.flac', '.wav', '.ogg', '.m4a', '.aac', '.wma', '.opus', '.ape', '.wv', '.aiff', '.mp4', '.mkv'])
    const audioFiles = dirEntries.filter((en) => {
      const d = en.lastIndexOf('.')
      return d >= 0 && audioExts.has(en.slice(d).toLowerCase())
    })
    if (audioFiles.length === 1) return candidates[0]
  }

  return null
}
