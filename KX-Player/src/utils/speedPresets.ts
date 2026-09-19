/** 倍速预设（与队列逻辑解耦，供键盘与 UI 共用）。 */

export const SPEED_MIN = 0.5
export const SPEED_MAX = 2.0
export const SPEED_STEP = 0.05

export function clampSpeed(v: number): number {
  return Math.max(SPEED_MIN, Math.min(SPEED_MAX, Math.round(v * 100) / 100))
}

export function stepSpeed(current: number, dir: 1 | -1): number {
  return clampSpeed(current + dir * SPEED_STEP)
}

export const SPEED_PRESETS = [0.75, 1, 1.25, 1.5, 2]
