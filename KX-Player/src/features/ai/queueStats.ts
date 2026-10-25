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

/** 翻译启动范围：默认全部待翻译，也可以明确只跑失败项或当前勾选项。 */
export type TranslateQueueScope = 'all' | 'failed' | 'selected'

export interface QueueStatRow {
  state: QueueRowState
  /** file-done 带回的「回退原文」行数：done 行也可能没翻干净（部分未翻） */
  fallback?: number
}

/** 可再次送入翻译任务的行态。exempt/done 必须通过行内「重译」显式放回。 */
export function isRetryableState(state: QueueRowState): boolean {
  return state === 'pending' || state === 'failed' || state === 'skipped'
}

/** 按用户明确选择的范围取本次翻译队列，保持原数组顺序。 */
export function selectTranslateRows<T extends QueueStatRow & { path: string }>(
  rows: readonly T[],
  scope: TranslateQueueScope,
  selected: ReadonlySet<string> = new Set(),
): T[] {
  return rows.filter((row) => {
    if (scope === 'failed') return row.state === 'failed'
    if (scope === 'selected') return selected.has(row.path) && isRetryableState(row.state)
    return isRetryableState(row.state)
  })
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

/**
 * 进度带填充条的百分比：文件粒度做骨架，当前文件用块进度（chunk-done 心跳）内插 ——
 * 不内插的话，大文件翻译期间整条进度带凝固在原地，看起来像卡死（tick 一直在发，
 * 只是没人消费）。chunk 信息缺席（两文件的间隙 / 收尾时刻）自动退回文件粒度。
 * `done/total` 的数字仍按文件数走（设计包 §4 锁死字段口径），条与条之间由填充条说话。
 */
export function runPercent(done: number, total: number, chunkDone?: number, chunkTotal?: number): number {
  if (total <= 0) return 0
  const frac =
    chunkTotal !== undefined && chunkTotal > 0
      ? Math.min(Math.max(chunkDone ?? 0, 0), chunkTotal) / chunkTotal
      : 0
  return Math.min(100, Math.round(((done + frac) / total) * 100))
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
 * 行内重译（AI-15 + 部分未翻）：按 path 定位，两种行放得回队列 ——
 * · `exempt`：已翻译豁免（扫描默认跳过），用户点名重翻；
 * · `done` 且 `fallback > 0`：「完成」但混着原文的行，不重译就只能一直用着半成品。
 * 放回时把 `force` 置上：后端对点名的文件绕开豁免账与内容缓存，逼一次真翻译 ——
 * 否则豁免账/缓存命中只会把旧译文再抄一遍，重译名存实亡。
 * 与 resetOne 同一条铁律：不改顺序、不增删行 —— 导出批次的序号绑整个队列。
 * 返回是否放回队列（找不到 / 状态不符都返回 false，视图据此决定要不要提示）。
 */
export function reenableOne<T extends QueueStatRow & { path: string; force?: boolean }>(rows: T[], path: string): boolean {
  const row = rows.find((r) => r.path === path)
  if (!row) return false
  const partial = row.state === 'done' && (row.fallback ?? 0) > 0
  if (row.state !== 'exempt' && !partial) return false
  row.state = 'pending'
  row.fallback = undefined
  row.force = true
  return true
}
