import { describe, it, expect } from 'vitest'
import { nextIndex, prevIndex, onEnded } from '../src/utils/queue'

describe('queue modes', () => {
  it('顺序循环取模', () => {
    expect(nextIndex(0, 0, 3)).toBe(1)
    expect(nextIndex(0, 2, 3)).toBe(0)
  })

  it('随机模式不与当前重复且在范围内', () => {
    for (let i = 0; i < 200; i++) {
      const r = nextIndex(1, 2, 5)
      expect(r).not.toBe(2)
      expect(r).toBeGreaterThanOrEqual(0)
      expect(r).toBeLessThan(5)
    }
  })

  it('单曲长度随机返回自身', () => {
    expect(nextIndex(1, 0, 1)).toBe(0)
  })

  it('prev 循环回绕', () => {
    expect(prevIndex(0, 0, 3)).toBe(2)
    expect(prevIndex(0, 1, 3)).toBe(0)
  })

  it('空队列为 null', () => {
    expect(nextIndex(0, 0, 0)).toBeNull()
    expect(prevIndex(0, 0, 0)).toBeNull()
  })

  it('结束行为按模式', () => {
    expect(onEnded(0)).toBe('next')
    expect(onEnded(1)).toBe('next')
    expect(onEnded(2)).toBe('repeat')
    expect(onEnded(3)).toBe('stop')
  })
})
