/**
 * 队列统计的纯函数层（设计包第 1 期：audit B2/B3 的落地）。
 *
 * 进度与收尾统计从此只认「本次运行的范围快照」：
 *  · 取消的行是「已跳过」，不再混进「已完成」（B2：旧代码把 file-done / file-failed /
 *    cancelled 一起累加进 doneCount，界面出现 6/12 却有 6 个已跳过的对不上）；
 *  · 收尾成功率只数本次快照里的行，历史完成的不再掺进来（B3：旧代码用 files 全量过滤
 *    done，第二次只跑 3 个文件也会报「成功 30/33」）。
 * 视图只做展示，口径全在这里 —— 纯函数，可单测。
 */

/** 与 AiTranslateView.vue 的 FileState 同一口径。`exempt` = 已翻译可豁免（扫描默认跳过，可手动重译） */
export type QueueRowState = 'pending' | 'running' | 'done' | 'failed' | 'skipped' | 'exempt'

export interface QueueStatRow {
  state: QueueRowState
}

export interface QueueStat {
  total: number
  done: number
  failed: number
  skipped: number
  /** done + failed + skipped：三种行都「有了结果」，进度分母按它走 */
  finished: number
  pending: number
}

/** 有结果的行：进度条走这里，running/pending 都不算数。已翻译豁免（exempt）也算收尾过 */
export function isFinishedState(state: QueueRowState): boolean {
  return state === 'done' || state === 'failed' || state === 'skipped' || state === 'exempt'
}

export function queueStats(rows: readonly QueueStatRow[]): QueueStat {
  const stat: QueueStat = { total: rows.length, done: 0, failed: 0, skipped: 0, finished: 0, pending: 0 }
  for (const row of rows) {
    switch (row.state) {
      case 'done':
        stat.done++
        break
      case 'failed':
        stat.failed++
        break
      case 'skipped':
      case 'exempt':
        // 已翻译豁免是「这轮不用重翻」的行，与取消的跳过同行口径计入 finished
        stat.skipped++
        break
      case 'pending':
        stat.pending++
        break
      case 'running':
        break
    }
  }
  stat.finished = stat.done + stat.failed + stat.skipped
  return stat
}

export interface RunSummary {
  total: number
  done: number
  failed: number
  skipped: number
}

/** 只统计快照里的那批行：path 不在 runPaths 的（历史完成、中途扫进来的）一律不算 */
export function summarizeRun(
  runPaths: readonly string[],
  rows: readonly (QueueStatRow & { path: string })[],
): RunSummary {
  const take = new Set(runPaths)
  const stat = queueStats(rows.filter((r) => take.has(r.path)))
  return { total: stat.total, done: stat.done, failed: stat.failed, skipped: stat.skipped }
}

/**
 * 重试失败项（audit B7）：把 failed 行重置回 pending，其余行原样保留。
 * 关键约束：**不改行的顺序、不增删行** —— 导出批次的序号按整个队列编（ai_export.rs），
 * 这里只要动到序，prompts.txt 就对到别的文件上（audit B5 的另一半）。
 * 返回重排的行数；视图只负责提示，不做判断。
 */
export function resetFailed<T extends QueueStatRow & { error?: string }>(rows: T[]): number {
  let n = 0
  for (const row of rows) {
    if (row.state !== 'failed') continue
    row.state = 'pending'
    row.error = undefined
    n++
  }
  return n
}

/**
 * 只重试某一行（audit B6「只重试这一项」）：按 path 定位，只动 failed 行。
 * 与 resetFailed 同一条铁律：不改顺序、不增删行 —— 导出批次的序号绑整个队列。
 * 返回是否重排了（找不到 / 不是 failed 都返回 false，视图据此决定要不要提示）。
 */
export function resetOne<T extends QueueStatRow & { error?: string; path: string }>(rows: T[], path: string): boolean {
  const row = rows.find((r) => r.path === path)
  if (!row || row.state !== 'failed') return false
  row.state = 'pending'
  row.error = undefined
  return true
}

/**
 * 手动重译一条已豁免的行（AI-15）：扫描时默认把「已翻译」行标记为 exempt 跳过，
 * 用户想重翻某个文件时点行内「重译」= 把这行放回 pending。铁律同 resetOne：
 * 不改顺序、不增删行 —— 导出批次的序号绑整个队列，动序就全错。
 * 返回是否放回队列（找不到 / 不是 exempt 都返回 false）。
 */
export function reenableOne<T extends QueueStatRow & { path: string }>(rows: T[], path: string): boolean {
  const row = rows.find((r) => r.path === path)
  if (!row || row.state !== 'exempt') return false
  row.state = 'pending'
  return true
}
