import { describe, it, expect } from 'vitest'
import { completedOnWrite, loudnessGainDb, sleepTimerPlan, sleepShouldFire } from '../src/utils/playback'

describe('completedOnWrite', () => {
  it('未结束时不给出 completed（保持原值）', () => {
    expect(completedOnWrite(100, 200, false)).toBeUndefined()
    expect(completedOnWrite(199, 200, false)).toBeUndefined()
  })

  it('结束时 ≥95% 记为完听', () => {
    expect(completedOnWrite(190, 200, true)).toBe(true)
    expect(completedOnWrite(200, 200, true)).toBe(true)
  })

  it('结束时 <95% 但 ≥5% 记为未完听', () => {
    expect(completedOnWrite(100, 200, true)).toBe(false)
    expect(completedOnWrite(10, 200, true)).toBe(false)
  })

  it('结束但进度 <5% 不覆盖（可能是误触下一首）', () => {
    expect(completedOnWrite(5, 200, true)).toBeUndefined()
  })

  it('未知时长（duration=0）时不判定', () => {
    expect(completedOnWrite(100, 0, true)).toBeUndefined()
  })
})

describe('loudnessGainDb', () => {
  it('无测量值返回 0（清零上一首增益）', () => {
    expect(loudnessGainDb(null, -23)).toBe(0)
    expect(loudnessGainDb(undefined, -23)).toBe(0)
    expect(loudnessGainDb(NaN, -23)).toBe(0)
  })

  it('按目标差值计算', () => {
    expect(loudnessGainDb(-28, -23)).toBe(5)
    expect(loudnessGainDb(-18, -23)).toBe(-5)
  })

  it('限制在 ±12 dB', () => {
    expect(loudnessGainDb(-50, -23)).toBe(12)
    expect(loudnessGainDb(-5, -23)).toBe(-12)
  })
})

describe('sleepTimerPlan', () => {
  it('正分钟数生成激活计划', () => {
    const p = sleepTimerPlan({ minutes: 30, action: 'pause', now: 1000 })
    expect(p.active).toBe(true)
    expect(p.endAt).toBe(1000 + 30 * 60000)
    expect(p.action).toBe('pause')
  })

  it('零/负/非法分钟数视为取消', () => {
    expect(sleepTimerPlan({ minutes: 0, action: 'pause', now: 0 }).active).toBe(false)
    expect(sleepTimerPlan({ minutes: -5, action: 'stop', now: 0 }).active).toBe(false)
    expect(sleepTimerPlan({ minutes: NaN, action: 'pause', now: 0 }).active).toBe(false)
  })
})

describe('sleepShouldFire', () => {
  it('未到点不触发', () => {
    expect(sleepShouldFire('pause', true, 5000)).toBe(false)
  })

  it('到点触发 pause/stop', () => {
    expect(sleepShouldFire('pause', true, 0)).toBe(true)
    expect(sleepShouldFire('stop', true, 0)).toBe(true)
  })

  it('finishTrack 在播放中不触发（等 ended）', () => {
    expect(sleepShouldFire('finishTrack', true, 0)).toBe(false)
    expect(sleepShouldFire('finishTrack', false, 0)).toBe(true)
  })
})
