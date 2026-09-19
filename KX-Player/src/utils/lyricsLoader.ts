/** 歌词加载：同名 .lrc → 目录模糊匹配 → .vtt/.srt 兜底（移植自旧 loadLrcForTrack）。 */
import { api } from '@/bridge/ipc'
import { parseLrc, parseVttOrSrt, type LyricLine } from '@/utils/parsers/lyrics'
import { findBestLyricsMatch } from '@/utils/parsers/matcher'

export interface LoadedLyrics {
  lines: LyricLine[]
  source: string | null
  format: 'lrc' | 'vtt' | 'srt' | null
}

export function splitPath(p: string): { dir: string; sep: string; baseName: string; nameWithoutExt: string; ext: string } {
  const sep = p.includes('\\') ? '\\' : '/'
  const lastSep = p.lastIndexOf(sep)
  const dir = lastSep >= 0 ? p.substring(0, lastSep) : ''
  const baseName = lastSep >= 0 ? p.substring(lastSep + 1) : p
  const lastDot = baseName.lastIndexOf('.')
  const nameWithoutExt = lastDot >= 0 ? baseName.substring(0, lastDot) : baseName
  const ext = lastDot >= 0 ? baseName.substring(lastDot).toLowerCase() : ''
  return { dir, sep, baseName, nameWithoutExt, ext }
}

export async function loadLyricsForTrack(path: string): Promise<LoadedLyrics> {
  const empty: LoadedLyrics = { lines: [], source: null, format: null }
  try {
    const { dir, sep, baseName, nameWithoutExt, ext } = splitPath(path)
    let content: string | null = null
    let format: 'lrc' | 'vtt' | 'srt' | null = null
    let source: string | null = null

    // 1. 精确同名 .lrc（快速路径）
    const lrcPath = dir + sep + nameWithoutExt + '.lrc'
    if (await api.fileExists(lrcPath)) {
      content = await api.readTextFile(lrcPath)
      if (content) { format = 'lrc'; source = lrcPath }
    }

    // 2. 目录模糊匹配
    if (!content) {
      const entries = await api.listDir(dir)
      const names = (entries || []).map((en) => (typeof en === 'string' ? en : en.name)).filter((n): n is string => !!n)
      if (names.length) {
        const match = findBestLyricsMatch(nameWithoutExt, ext, names)
        if (match) {
          const matchPath = dir + sep + match.filename
          const c = await api.readTextFile(matchPath)
          if (c) {
            content = c
            source = matchPath
            const me = match.ext.toLowerCase()
            format = me === '.vtt' ? 'vtt' : me === '.srt' ? 'srt' : 'lrc'
          }
        }
      }
    }

    // 3. 同名 .vtt / .srt（含双扩展名「xxx.mp3.vtt」）
    if (!content) {
      for (const subExt of ['vtt', 'srt'] as const) {
        for (const candidate of [baseName + '.' + subExt, nameWithoutExt + '.' + subExt]) {
          const p = dir + sep + candidate
          if (await api.fileExists(p)) {
            const c = await api.readTextFile(p)
            if (c) { content = c; format = subExt; source = p; break }
          }
        }
        if (content) break
      }
    }

    if (!content || !format) return empty
    const lines = format === 'lrc' ? parseLrc(content) : parseVttOrSrt(content, format)
    return { lines, source, format }
  } catch {
    return empty
  }
}
