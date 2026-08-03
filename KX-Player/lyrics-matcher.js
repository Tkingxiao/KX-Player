// === Lyrics Matcher ===
// Selects the most likely .lrc/.vtt/.srt file from a directory listing.

function extractTrackSuffix(name) {
  const m = name.match(/((?:[_\-](?:DAY|Disc|CD|Track|trk|part|vol|No|No\.|P)\w*)*[_\-](?:trk|track|Disc|CD|DAY|part|vol|No\.?|P)\w*[_\w]*?)$/i)
  if (m) return m[1]
  const m2 = name.match(/([_\-]\d{2,4})$/)
  if (m2) return m2[1]
  return ''
}

export function findBestLyricsMatch(audioNameWithoutExt, audioExt, dirEntries) {
  const lrcExts = new Set(['.lrc', '.vtt', '.srt'])
  const candidates = []
  for (const entry of dirEntries) {
    const lower = entry.toLowerCase()
    const dotIdx = lower.lastIndexOf('.')
    if (dotIdx < 0) continue
    const entryExt = lower.slice(dotIdx)
    const entryName = entry.slice(0, dotIdx)
    let isLyricsFile = false
    if (lrcExts.has(entryExt)) {
      isLyricsFile = true
    } else if (lrcExts.has(lower.slice(lower.lastIndexOf('.', dotIdx - 1)))) {
      isLyricsFile = true
    }
    if (!isLyricsFile) continue
    candidates.push({ filename: entry, nameWithoutExt: entryName, ext: entryExt })
  }
  if (candidates.length === 0) return null

  for (const c of candidates) {
    if (c.nameWithoutExt === audioNameWithoutExt || c.nameWithoutExt === audioNameWithoutExt + audioExt) return c
  }

  const audioSuffix = extractTrackSuffix(audioNameWithoutExt)
  if (audioSuffix) {
    let bestMatch = null
    let bestScore = 0
    for (const c of candidates) {
      if (c.nameWithoutExt.endsWith(audioSuffix)) {
        const score = audioSuffix.length
        if (score > bestScore) { bestScore = score; bestMatch = c }
      }
    }
    if (bestMatch) return bestMatch
  }

  let bestMatch = null
  let bestCommonLen = 0
  for (const c of candidates) {
    let commonLen = 0
    const a = audioNameWithoutExt.toLowerCase()
    const b = c.nameWithoutExt.toLowerCase()
    for (let i = 1; i <= Math.min(a.length, b.length); i++) {
      if (a[a.length - i] === b[b.length - i]) commonLen = i
      else break
    }
    if (commonLen >= 5 && commonLen > bestCommonLen) {
      const suffixStart = audioNameWithoutExt.length - commonLen
      if (suffixStart > 0 && /[_\-]/.test(audioNameWithoutExt[suffixStart])) {
        bestCommonLen = commonLen
        bestMatch = c
      }
    }
  }
  if (bestMatch) return bestMatch

  if (candidates.length === 1) {
    const audioExts = new Set(['.mp3','.flac','.wav','.ogg','.m4a','.aac','.wma','.opus','.ape','.wv','.aiff','.mp4','.mkv'])
    const audioFiles = dirEntries.filter(e => audioExts.has(e.slice(e.lastIndexOf('.')).toLowerCase()))
    if (audioFiles.length === 1) return candidates[0]
  }

  return null
}
