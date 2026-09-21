/**
 * IPC 错误信封的 TS 侧（`02 §6.1`；事实源是 Rust `src-tauri/src/error.rs`）。
 *
 * 注意语义：Rust 的 `Err` 经 Tauri 走的是 **reject**，不是 resolve 一个 `{ ok: false }`。
 * `bridge/ipc.ts` 里 48 处 `.catch(fallback)` 依赖这个语义，所以 `Result<T>` 在这里是
 * 「线上形状的文档 + 想要非抛错写法时的返回值」，不是每个命令的实际 resolve 值。
 */

/** 与 Rust `AppErrorCode` 逐项对齐（SCREAMING_SNAKE_CASE）。 */
export type AppErrorCode =
  | 'NOT_FOUND'
  | 'PERMISSION_DENIED'
  | 'IO_FAILED'
  | 'DB_FAILED'
  | 'DECODE_FAILED'
  | 'UNSUPPORTED_FORMAT'
  | 'MPV_INIT_FAILED'
  | 'MPV_EMBED_FAILED'
  | 'REMUX_FAILED'
  | 'SUBTITLE_PARSE_FAILED'
  | 'ENCODING_UNKNOWN'
  | 'BUSY'
  | 'INVALID_ARGUMENT'
  | 'CANCELLED'
  | 'INTERNAL'

/** Rust `IpcError` 的 JSON 形状（`serde(rename_all = "camelCase")`，`detail` 缺省时整键缺席）。 */
export interface IpcErrorPayload {
  code: AppErrorCode
  message: string
  detail?: string
}

export type Result<T> = { ok: true; data: T } | { ok: false; error: IpcErrorPayload }

const CODES: readonly AppErrorCode[] = [
  'NOT_FOUND',
  'PERMISSION_DENIED',
  'IO_FAILED',
  'DB_FAILED',
  'DECODE_FAILED',
  'UNSUPPORTED_FORMAT',
  'MPV_INIT_FAILED',
  'MPV_EMBED_FAILED',
  'REMUX_FAILED',
  'SUBTITLE_PARSE_FAILED',
  'ENCODING_UNKNOWN',
  'BUSY',
  'INVALID_ARGUMENT',
  'CANCELLED',
  'INTERNAL',
]

/** 后端加了新码但忘了同步这里时，宁可退成 `INTERNAL` 也不要留下一个不存在的字面量。 */
export function asAppErrorCode(v: unknown): AppErrorCode {
  return typeof v === 'string' && (CODES as readonly string[]).includes(v) ? (v as AppErrorCode) : 'INTERNAL'
}

/**
 * 归一化后的 IPC 错误。`message` 面向用户（可直接进 Toast），
 * `detail` 只进日志与自检导出，默认不展示。
 */
export class AppError extends Error {
  readonly code: AppErrorCode
  readonly detail?: string

  constructor(code: AppErrorCode, message: string, detail?: string) {
    super(message)
    this.name = 'AppError'
    this.code = code
    this.detail = detail
  }
}

/**
 * 把任意 reject 值转成 `AppError`。
 *
 * 除信封载荷外还要接住两类历史形状：Tauri 自身的字符串错误（`Err(String)` 时代与
 * `tauri::Error`），以及插件抛的 `Error`。字符串里不带码，一律按 `INTERNAL` 处理 ——
 * 从中文文案里猜码会把「已存在同名标签」猜成五花八门的码，宁可不猜。
 */
export function toAppError(raw: unknown): AppError {
  if (raw instanceof AppError) return raw
  if (typeof raw === 'string') {
    return new AppError('INTERNAL', raw)
  }
  if (raw instanceof Error) {
    // 非 IPC 来源（前端自身或插件抛错）：保留原文，别假装它有码
    return new AppError('INTERNAL', raw.message || String(raw), raw.stack)
  }
  const p = raw as Partial<IpcErrorPayload> | null
  if (p && typeof p.message === 'string') {
    return new AppError(asAppErrorCode(p.code), p.message, typeof p.detail === 'string' ? p.detail : undefined)
  }
  return new AppError('INTERNAL', String(raw))
}

/** 展示用文本：只要用户能看懂的那半句，不带 `[CODE]` 前缀。 */
export function errText(e: unknown): string {
  return toAppError(e).message
}
