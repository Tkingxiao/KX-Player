/** 时间/大小/文件名格式化（纯展示函数）。 */

export function fmtTime(t: number | null | undefined): string {
  if (!t || !isFinite(t) || t < 0) return '00:00'
  const total = Math.floor(t)
  const h = Math.floor(total / 3600)
  const m = Math.floor((total % 3600) / 60)
  const s = total % 60
  const mm = String(m).padStart(2, '0')
  const ss = String(s).padStart(2, '0')
  return h > 0 ? `${h}:${mm}:${ss}` : `${mm}:${ss}`
}

/** 睡眠定时倒计时（mm:ss） */
export function fmtCountdown(ms: number): string {
  const total = Math.max(0, Math.ceil(ms / 1000))
  const m = Math.floor(total / 60)
  const s = total % 60
  return `${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`
}

export function fmtFSize(bytes: number | null | undefined): string {
  if (!bytes || !isFinite(bytes)) return ''
  const u = ['B', 'KB', 'MB', 'GB']
  let b = bytes
  let i = 0
  while (b >= 1024 && i < u.length - 1) { b /= 1024; i++ }
  return b.toFixed(i > 0 ? 1 : 0) + ' ' + u[i]
}

/** 音频质量标签，如 "FLAC 44.1kHz" */
export function fmtQuality(t: { format: string; sampleRate: number | null; bitrate: number | null }): string {
  if (!t) return ''
  const parts: string[] = [t.format.toUpperCase()]
  if (t.sampleRate) parts.push((t.sampleRate / 1000).toFixed(1).replace(/\.0$/, '') + 'kHz')
  else if (t.bitrate) parts.push(Math.round(t.bitrate / 1000) + 'kbps')
  return parts.join(' ')
}

export function trackName(t: { name: string; path: string }): string {
  if (t.name) return t.name
  const p = String(t.path || '').replace(/\\/g, '/').split('/').pop() || ''
  return p.replace(/\.[^/.]+$/, '') || p
}

export function trackArtist(t: { metaArtist: string | null; artist: string }): string {
  return t.metaArtist || t.artist || '佚名'
}

/** 归一化目录路径（正斜杠、无尾斜杠） */
export function normDir(p: string): string {
  return String(p || '').replace(/\\/g, '/').replace(/\/+$/, '')
}

export function trackDir(t: { path: string }): string | null {
  const p = normDir(t?.path || '')
  const i = p.lastIndexOf('/')
  return i > 0 ? p.slice(0, i) : null
}

export function fileUrl(p: string): string {
  return 'file:///' + String(p).replace(/\\/g, '/').replace(/^\/+/, '')
}
