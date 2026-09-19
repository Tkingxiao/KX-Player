import { describe, it, expect } from 'vitest'
import { parseLrc, parseVttOrSrt, activeLyricIndex } from '../src/utils/parsers/lyrics'

describe('parseLrc', () => {
  it('解析标准 LRC', () => {
    const lrc = `[00:12.50]第一行\n[01:02.30]第二行`
    const lines = parseLrc(lrc)
    expect(lines).toHaveLength(2)
    expect(lines[0]).toEqual({ time: 12.5, text: '第一行' })
    expect(lines[1]).toEqual({ time: 62.3, text: '第二行' })
  })

  it('一行多时间戳展开', () => {
    const lines = parseLrc('[00:01.00][00:05.00]重复段')
    expect(lines).toHaveLength(2)
    expect(lines[0].time).toBe(1)
    expect(lines[1].time).toBe(5)
    expect(lines[0].text).toBe('重复段')
  })

  it('两位毫秒补齐为三位', () => {
    const lines = parseLrc('[00:03.25]x')
    expect(lines[0].time).toBeCloseTo(3.25, 5)
  })

  it('跳过无文本行并按时间排序', () => {
    const lines = parseLrc('[01:00.00]b\n[00:30.00]a\n[ar:歌手]\n[00:10.00]')
    expect(lines.map((l) => l.text)).toEqual(['a', 'b'])
  })
})

describe('parseVttOrSrt', () => {
  it('解析 SRT（逗号毫秒，跳过序号）', () => {
    const srt = `1\n00:00:21,813 --> 00:00:24,100\n第一句\n\n2\n00:00:25,000 --> 00:00:27,000\n第二句\n`
    const lines = parseVttOrSrt(srt, 'srt')
    expect(lines).toHaveLength(2)
    expect(lines[0].time).toBeCloseTo(21.813, 3)
    expect(lines[0].text).toBe('第一句')
  })

  it('解析 WebVTT（句点毫秒，跳过头部）', () => {
    const vtt = `WEBVTT\n\n00:00.500 --> 00:02.000\n你好\n\n00:00:03.000 --> 00:00:05.000\n世界\n`
    const lines = parseVttOrSrt(vtt, 'vtt')
    expect(lines).toHaveLength(2)
    expect(lines[0].time).toBeCloseTo(0.5, 3)
  })

  it('跳过 NOTE 块与内联标签', () => {
    const vtt = `WEBVTT\n\nNOTE 这是一段注释\n跨行注释\n\n00:00:01.000 --> 00:00:02.000\n<c.ja>带标签</c>\n`
    const lines = parseVttOrSrt(vtt, 'vtt')
    expect(lines).toHaveLength(1)
    expect(lines[0].text).toBe('带标签')
  })
})

describe('activeLyricIndex', () => {
  const lines = [
    { time: 0, text: 'a' },
    { time: 10, text: 'b' },
    { time: 20, text: 'c' },
  ]
  it('首行之前返回 0（time<=t 含 0）', () => {
    expect(activeLyricIndex(lines, 0)).toBe(0)
  })
  it('边界与中间', () => {
    expect(activeLyricIndex(lines, 9.9)).toBe(0)
    expect(activeLyricIndex(lines, 10)).toBe(1)
    expect(activeLyricIndex(lines, 25)).toBe(2)
  })
  it('空歌词返回 -1', () => {
    expect(activeLyricIndex([], 5)).toBe(-1)
  })
})
