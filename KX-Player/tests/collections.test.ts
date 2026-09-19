import { describe, it, expect } from 'vitest'
import { bucketOf, matchSmart, progressInfo, COMPLETED_RATIO } from '../src/utils/collections'
import type { Track } from '../src/contracts/api'

function mkTrack(partial: Partial<Track>): Track {
  return {
    id: 't1', name: 'n', path: '/a/n.mp3', duration: 0, artist: '', album: '',
    format: 'mp3', isVideo: false, coverPath: null, coverData: null, lyricsPath: null,
    fileMtime: 0, fileSize: 0, metaTitle: null, metaArtist: null, genre: null,
    bitrate: null, sampleRate: null,
    ...partial,
  }
}

describe('时长分档边界', () => {
  const sec = 1
  it('边界值落在正确档位', () => {
    expect(bucketOf(0)).toBeNull()
    expect(bucketOf(599 * sec)).toBe('lt10m')
    expect(bucketOf(600 * sec)).toBe('to30m')     // 10min → 10–30
    expect(bucketOf(1799 * sec)).toBe('to30m')
    expect(bucketOf(1800 * sec)).toBe('to60m')
    expect(bucketOf(3600 * sec)).toBe('to2h')
    expect(bucketOf(7200 * sec)).toBe('gt2h')
  })
})

describe('完听阈值', () => {
  it('≥95% 视为听完', () => {
    const t = mkTrack({ duration: 100 })
    const at = (sec: number) => progressInfo(t, { trackId: 't1', positionMs: sec * 1000, completed: false, playCount: 1, playedAt: 1, lastSpeed: null })
    expect(at(94.9).completed).toBe(false)
    expect(at(95).completed).toBe(true)
    expect(progressInfo(t, { trackId: 't1', positionMs: 0, completed: true, playCount: 1, playedAt: 1, lastSpeed: null }).completed).toBe(true)
    expect(COMPLETED_RATIO).toBe(0.95)
  })
})

describe('智能集合判定', () => {
  const track = mkTrack({ duration: 1000, isVideo: false, lyricsPath: null })
  const favIds = new Set(['t1'])

  it('未听完：有进度且未完听', () => {
    const p = { trackId: 't1', positionMs: 60_000, completed: false, playCount: 1, playedAt: 5, lastSpeed: null }
    expect(matchSmart('unfinished', track, p, favIds)).toBe(true)
    expect(matchSmart('completed', track, p, favIds)).toBe(false)
  })

  it('收藏集合', () => {
    expect(matchSmart('favorites', track, undefined, favIds)).toBe(true)
    expect(matchSmart('favorites', track, undefined, new Set())).toBe(false)
  })

  it('音视频分类', () => {
    expect(matchSmart('audio', track, undefined, favIds)).toBe(true)
    expect(matchSmart('video', track, undefined, favIds)).toBe(false)
    expect(matchSmart('video', mkTrack({ isVideo: true }), undefined, favIds)).toBe(true)
  })

  it('有歌词', () => {
    expect(matchSmart('withSubs', mkTrack({ lyricsPath: '/a.lrc' }), undefined, favIds)).toBe(true)
    expect(matchSmart('withSubs', track, undefined, favIds)).toBe(false)
  })
})
