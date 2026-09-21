/**
 * 契约层 · 事件清单：Rust `emit` 与前端 `listen` 之间唯一的名字与载荷来源。
 *
 * 命名规则 `module:action`（`02 §6.3`）。`npm run check:ipc` 比对三处集合：
 * Rust 侧 `emit("…")`、这里的键、前端 `onEvent<T>('…')` 的字面量，
 * 任一侧多出名字即失败——事件拼错不会有任何报错，只会静默失灵。
 */
import type { ConvertProgress, MpvPlayerState } from './dto'

/** `loudness:progress` 载荷（Rust 侧 `serde_json::json!` 字面量，非结构体） */
export interface LoudnessProgress {
  completed: number
  total: number
  current: string
  done: number
  failed: number
  finished: boolean
  cancelled: boolean
}

export interface EventSpecs {
  // ── 扫描 ──
  'scanner:progress': { completed: number; total: number }
  'scanner:stage': string
  'scanner:fsChanged': null
  // ── 播放内核 ──
  'player:state': MpvPlayerState
  'player:loading': null
  'player:ended': { reason: string }
  'player:error': { message: string }
  // ── 悬浮窗 ──
  'pip:shown': null
  'pip:closed': null
  'pip:restored': null
  'pip:pinned': boolean
  // ── 窗口 ──
  'window:beforeClose': null
  'window:maximizeChange': boolean
  // ── 后台任务 ──
  'convert:progress': ConvertProgress
  'loudness:progress': LoudnessProgress
}

export type EventName = keyof EventSpecs
export type EventPayload<K extends EventName> = EventSpecs[K]
