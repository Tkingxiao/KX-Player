/** 智能集合 / 时长分档 / 完听阈值（纯函数，可单测）。 */
import type { Track } from '@/contracts/api'
import type { PlayProgress } from '@/contracts/api'

/** 完听阈值：进度 ≥95% 视为听完 */
export const COMPLETED_RATIO = 0.95

export type SmartCollection =
  | 'all' | 'unfinished' | 'recent' | 'recentAdded' | 'completed'
  | 'favorites' | 'video' | 'audio' | 'withSubs'

export const SMART_LABELS: Record<SmartCollection, string> = {
  all: '全部音乐',
  unfinished: '未听完',
  recent: '最近播放',
  recentAdded: '最近加入',
  completed: '已听完',
  favorites: '收藏',
  video: '视频',
  audio: '仅音频',
  withSubs: '有字幕',
}

export type DurationBucket = 'lt10m' | 'to30m' | 'to60m' | 'to2h' | 'gt2h'

export const DURATION_BUCKETS: { key: DurationBucket; label: string; min: number; max: number }[] = [
  { key: 'lt10m', label: '< 10 分钟', min: 0, max: 10 * 60 },
  { key: 'to30m', label: '10 – 30 分钟', min: 10 * 60, max: 30 * 60 },
  { key: 'to60m', label: '30 – 60 分钟', min: 30 * 60, max: 60 * 60 },
  { key: 'to2h', label: '1 – 2 小时', min: 60 * 60, max: 2 * 60 * 60 },
  { key: 'gt2h', label: '> 2 小时', min: 2 * 60 * 60, max: Infinity },
]

export function bucketOf(durationSec: number): DurationBucket | null {
  if (!durationSec || durationSec <= 0) return null
  for (const b of DURATION_BUCKETS) {
    if (durationSec >= b.min && durationSec < b.max) return b.key
  }
  return null
}

export interface TrackProgressInfo {
  positionRatio: number
  completed: boolean
  playedAt: number
  playCount: number
}

/** 播放进度信息：库内时长优先，未知时长按 0 处理。 */
export function progressInfo(t: Track, p: PlayProgress | undefined): TrackProgressInfo {
  if (!p) return { positionRatio: 0, completed: false, playedAt: 0, playCount: 0 }
  const dur = t.duration > 0 ? t.duration : 0
  const posSec = p.positionMs / 1000
  const ratio = dur > 0 ? posSec / dur : 0
  const completed = p.completed || (dur > 0 && ratio >= COMPLETED_RATIO)
  return { positionRatio: Math.min(1, Math.max(0, ratio)), completed, playedAt: p.playedAt, playCount: p.playCount }
}

/** 智能集合判定 */
export function matchSmart(
  key: SmartCollection,
  t: Track,
  p: PlayProgress | undefined,
  favoriteIds: Set<string>,
): boolean {
  switch (key) {
    case 'all': return true
    case 'unfinished': {
      const info = progressInfo(t, p)
      return !info.completed && (info.positionRatio > 0.01 || (info.playedAt > 0 && !info.completed))
    }
    case 'completed': return progressInfo(t, p).completed
    case 'video': return t.isVideo
    case 'audio': return !t.isVideo
    case 'withSubs': return !!t.lyricsPath
    case 'favorites': return favoriteIds.has(t.id)
    case 'recent': return (p?.playedAt ?? 0) > 0
    case 'recentAdded': return true // 排序阶段处理
  }
}

export type SortField = 'name' | 'artist' | 'duration' | 'mtime' | 'playedAt' | 'trackCount'
export type SortDir = 'asc' | 'desc'

const collator = new Intl.Collator('zh-Hans-CN', { numeric: true, sensitivity: 'base' })

export function cmpBy(field: SortField, dir: SortDir): (a: Track, b: Track) => number {
  const sign = dir === 'asc' ? 1 : -1
  return (a, b) => {
    switch (field) {
      case 'name': return sign * collator.compare(a.name || '', b.name || '')
      case 'artist': return sign * collator.compare(a.artist || '', b.artist || '')
      case 'duration': return sign * ((a.duration || 0) - (b.duration || 0))
      case 'mtime': return sign * ((a.fileMtime || 0) - (b.fileMtime || 0))
      case 'playedAt': return sign * 0 // 调用方按进度表处理
      case 'trackCount': return sign * 0
    }
  }
}
