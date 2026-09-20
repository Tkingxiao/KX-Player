/** 播放进度/响度/睡眠定时的纯逻辑（可单测；player store 消费）。 */

import { COMPLETED_RATIO } from '@/utils/collections'

/** 进度回写时的完听判定：
 *  - finished=true（播放到 EOF）且进度 ≥5% 时，按是否 ≥95% 给出 completed；
 *  - 其余情况返回 undefined（保持数据库原值，不覆盖）。
 */
export function completedOnWrite(positionSec: number, durationSec: number, finished: boolean): boolean | undefined {
  const ratio = durationSec > 0 ? positionSec / durationSec : 0
  return finished && ratio >= 0.05 ? ratio >= COMPLETED_RATIO : undefined
}

/** 响度增益（dB）：无测量值返回 0（必须清零上一首的增益）；有值按目标差值并限制 ±12。 */
export function loudnessGainDb(trackLufs: number | null | undefined, targetLufs: number): number {
  if (trackLufs == null || !isFinite(trackLufs)) return 0
  return Math.max(-12, Math.min(12, targetLufs - trackLufs))
}

export interface SleepTimerInput {
  minutes: number
  action: 'pause' | 'stop' | 'finishTrack'
  now: number
}

/** 睡眠定时参数归一化：非法分钟数视为取消。 */
export function sleepTimerPlan(input: SleepTimerInput): { active: boolean; endAt: number; action: SleepTimerInput['action'] } {
  if (!isFinite(input.minutes) || input.minutes <= 0) {
    return { active: false, endAt: 0, action: input.action }
  }
  return { active: true, endAt: input.now + input.minutes * 60000, action: input.action }
}

/** 到点行为：finishTrack 模式在仍在播放时不触发（等 ended 事件接管）。 */
export function sleepShouldFire(action: SleepTimerInput['action'], playing: boolean, remainingMs: number): boolean {
  if (remainingMs > 0) return false
  if (action === 'finishTrack' && playing) return false
  return true
}
