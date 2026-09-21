/**
 * 守卫「嵌套对象整对象透传」这条契约。
 *
 * 起因：`playerSetSubStyle` 曾在 bridge 里逐字段白名单转发，结果 P0-24 的
 * backColor / backOpacity 与 P0-25 的 assOverride 被**静默吃掉** —— 后端 model.rs
 * 实现齐全，前端永远传不到，UI 上表现为「改了没反应」。
 * `check:ipc` 只对账顶层参数键名，TS 又因为 SubStyle 字段全可选而查不出来，
 * 所以这里用「传入对象 == 传出对象」的整对象断言把这类丢失钉死。
 */
import { describe, it, expect, vi } from 'vitest'

const { calls } = vi.hoisted(() => ({
  calls: [] as Array<{ cmd: string; args: unknown }>,
}))

vi.mock('@tauri-apps/api/core', () => ({
  invoke: async (cmd: string, args: unknown) => {
    calls.push({ cmd, args })
    return undefined
  },
  convertFileSrc: (p: string) => p,
}))

import { api } from '../src/bridge/ipc'

describe('playerSetSubStyle 整对象透传', () => {
  it('10 个字段一个都不许丢（含 backColor / backOpacity / assOverride）', async () => {
    const style = {
      font: 'Noto Sans SC',
      fontSize: 30,
      color: '#FFFFFF',
      borderColor: '#000000',
      borderSize: 2,
      shadowOffset: 1,
      pos: 95, // dto.ts:147 是 number（mpv sub-pos 百分比），不是字符串
      backColor: '#112233',
      backOpacity: 55,
      assOverride: 'scale' as const,
    }
    await api.playerSetSubStyle(style)
    const call = calls.find((c) => c.cmd === 'player_set_sub_style')
    expect(call).toBeTruthy()
    expect((call?.args as { style: unknown }).style).toEqual(style)
    // 反向：白名单式的字段裁剪会让键数少于 10
    expect(Object.keys((call?.args as { style: object }).style)).toHaveLength(10)
  })
})
