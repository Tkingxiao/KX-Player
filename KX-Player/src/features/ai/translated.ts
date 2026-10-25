/**
 * 译文管理页的纯函数层：状态口径、筛选与统计都收在这里，视图只做展示 —— 可单测。
 *
 * 状态由 Rust 侧按「账 + 文件系统现状」分好（ai_translated::list），这里只管怎么呈现：
 *  · ok           正常：源、译文都在且源没改过
 *  · changed      源已修改：译文已过时，值得重译
 *  · out_missing  译文丢失：账还在、文件没了
 *  · source_gone  源已删除：译文往往才是留下的那份，去留由人决定
 */
import type { AiTranslatedEntry, AiTranslatedStatus } from '@/contracts/dto'

export const STATUS_LABEL: Record<AiTranslatedStatus, string> = {
  ok: '正常',
  changed: '源已修改',
  out_missing: '译文丢失',
  source_gone: '源已删除',
}

/** 待处理 = 有事可做（重译 / 清账）的条目；正常的不打扰人 */
export function isTodo(status: AiTranslatedStatus): boolean {
  return status !== 'ok'
}

export type TranslatedFilter = 'all' | 'todo' | 'ok'

export function filterEntries(
  rows: readonly AiTranslatedEntry[],
  query: string,
  filter: TranslatedFilter,
): AiTranslatedEntry[] {
  const q = query.trim().toLowerCase()
  return rows.filter((r) => {
    if (filter === 'todo' && !isTodo(r.status)) return false
    if (filter === 'ok' && r.status !== 'ok') return false
    if (q && !r.sourceName.toLowerCase().includes(q) && !r.sourcePath.toLowerCase().includes(q)) return false
    return true
  })
}

export interface TranslatedSummary {
  total: number
  ok: number
  todo: number
}

export function summarize(rows: readonly AiTranslatedEntry[]): TranslatedSummary {
  let ok = 0
  for (const r of rows) if (r.status === 'ok') ok++
  return { total: rows.length, ok, todo: rows.length - ok }
}

/** 这些行能不能「重新翻译」：源文件还在的才有得翻（source_gone 没源可读） */
export function retranslatable(rows: readonly AiTranslatedEntry[]): string[] {
  return rows.filter((r) => r.status !== 'source_gone').map((r) => r.sourcePath)
}

/** Windows 路径下判断源文件和译文是否落在同一目录；只用于界面提示，不参与后端授权。 */
export function sameParentPath(sourcePath: string, outPath: string): boolean {
  const parent = (p: string): string => {
    const normalized = p.replace(/\\/g, '/')
    const index = normalized.lastIndexOf('/')
    return (index < 0 ? '' : normalized.slice(0, index)).toLowerCase()
  }
  return parent(sourcePath) === parent(outPath)
}

export interface TranslatedBatchSummary {
  total: number
  todo: number
  retranslatable: number
  outputPresent: number
  sourceDeletable: number
}

/** 批量操作条的影响预览：把可执行数量说清楚，再让用户确认危险动作。 */
export function summarizeBatch(rows: readonly AiTranslatedEntry[]): TranslatedBatchSummary {
  return {
    total: rows.length,
    todo: rows.filter((r) => isTodo(r.status)).length,
    retranslatable: rows.filter((r) => r.status !== 'source_gone').length,
    outputPresent: rows.filter((r) => r.outExists).length,
    sourceDeletable: rows.filter((r) => r.status !== 'source_gone').length,
  }
}

/** 翻译时刻的短格式（同年省年份）：MM-DD HH:mm / YYYY-MM-DD HH:mm */
export function fmtTranslatedAt(ms: number): string {
  const d = new Date(ms)
  if (isNaN(d.getTime())) return '—'
  const pad = (n: number): string => String(n).padStart(2, '0')
  const md = `${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`
  const now = new Date()
  return d.getFullYear() === now.getFullYear() ? md : `${d.getFullYear()}-${md}`
}
