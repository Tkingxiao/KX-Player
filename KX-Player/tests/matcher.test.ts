import { describe, it, expect } from 'vitest'
import { findBestLyricsMatch } from '../src/utils/parsers/matcher'
import { fmtTime, fmtCountdown, fmtFSize, normDir, fileUrl, trackName } from '../src/utils/format'

describe('歌词匹配器', () => {
  it('精确同名优先', () => {
    const entries = ['a.lrc', 'b.lrc']
    expect(findBestLyricsMatch('a', '.mp3', entries)?.filename).toBe('a.lrc')
  })

  it('同名含音频扩展名（xxx.mp3.lrc）', () => {
    expect(findBestLyricsMatch('a', '.mp3', ['a.mp3.lrc', 'b.lrc'])?.filename).toBe('a.mp3.lrc')
  })

  it('轨道后缀匹配（CD/Track）', () => {
    const entries = ['作品名_CD1.lrc', '作品名_CD2.lrc', '其他.lrc']
    expect(findBestLyricsMatch('作品名_CD2', '.flac', entries)?.filename).toBe('作品名_CD2.lrc')
  })

  it('末尾公共子串 ≥5', () => {
    const entries = ['RJ123_耳かき_トラック02_lotion.lrc']
    expect(findBestLyricsMatch('RJ123_耳かき_トラック03_lotion', '.wav', entries)?.filename).toBe('RJ123_耳かき_トラック02_lotion.lrc')
  })

  it('唯一歌词+唯一音频兜底', () => {
    expect(findBestLyricsMatch('完全不同名', '.mp3', ['唯一歌词.lrc', '唯一音频.mp3'])?.filename).toBe('唯一歌词.lrc')
  })

  it('无候选返回 null', () => {
    expect(findBestLyricsMatch('a', '.mp3', ['b.txt'])).toBeNull()
  })
})

describe('格式化', () => {
  it('fmtTime', () => {
    expect(fmtTime(0)).toBe('00:00')
    expect(fmtTime(65)).toBe('01:05')
    expect(fmtTime(3600)).toBe('1:00:00')
    expect(fmtTime(NaN)).toBe('00:00')
  })
  it('fmtCountdown', () => {
    expect(fmtCountdown(90_000)).toBe('01:30')
    expect(fmtCountdown(0)).toBe('00:00')
  })
  it('fmtFSize', () => {
    expect(fmtFSize(512)).toBe('512 B')
    expect(fmtFSize(2048)).toBe('2.0 KB')
  })
  it('normDir 与 fileUrl', () => {
    expect(normDir('C:\\a\\b\\')).toBe('C:/a/b')
    expect(fileUrl('C:\\a b\\x.mp3')).toBe('file:///C:/a b/x.mp3')
  })
  it('trackName 优先 name', () => {
    expect(trackName({ name: '', path: 'C:\\x\\y.flac' })).toBe('y')
    expect(trackName({ name: '标题', path: 'C:\\x\\y.flac' })).toBe('标题')
  })
})
