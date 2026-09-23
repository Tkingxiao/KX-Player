/**
 * AI 翻译的任务客户端（B3）。
 *
 * 扫描、切块、逐块翻译、回写全部在 Rust 任务里跑（`src-tauri/src/ai_job.rs`），
 * 这里只负责「启动 + 收 ai:progress + 能取消」。
 * 提示词与「编号. 译文」解析的**唯一实现**在 `ai_prompt.rs` / `subtitles.rs`：
 * 这两层原来只在前端有一份，B3 把逐块循环搬到 Rust 后若留在前端就要每块过一次 IPC ——
 * 等于没搬；若在 Rust 里再抄一份就成了两份。所以是「搬走 + 只留 Rust 那一份」。
 */
import { api, onEvent } from '@/bridge/ipc'
import type { AiJobOptions, AiProgress, SubtitleScanItem } from '@/contracts/api'

/** 递归收集 .lrc/.srt/.vtt（深度 4、上限 500 —— 缺省值在 Rust 侧，与旧前端一致） */
export function scanSubtitleFiles(dir: string): Promise<SubtitleScanItem[]> {
  return api.subtitleScanDir(dir, 'subtitle')
}

/** 递归收集子目录（「文件夹名翻译」的队列，上限 300；根目录自身不算） */
export function scanSubDirectories(dir: string): Promise<SubtitleScanItem[]> {
  return api.subtitleScanDir(dir, 'dir')
}

export interface AiTranslateJob {
  /** 收到 queue-finished 即 resolve；启动命令被拒则 reject */
  finished: Promise<void>
  /** 块级取消：在飞的那一块跑完就停，后续块与后续文件不再开始 */
  stop: () => void
}

/** 队列行在翻译期间的最小状态面（视图侧再叠加展示字段） */
export interface TranslateRow {
  state: 'pending' | 'running' | 'done' | 'failed' | 'skipped' | 'exempt'
  error?: string
  out?: string
}

/**
 * 把一条 `ai:progress` 落到对应的队列行上。纯函数：文件任务的整条状态迁移都在这里，可单测。
 * rows 必须与发给 Rust 的 paths 同序（事件的 fileIndex 就是那个下标）。
 */
export function applyProgress(rows: TranslateRow[], p: AiProgress): void {
  const row = rows[p.fileIndex]
  if (!row) return
  switch (p.state) {
    case 'file-start':
      row.state = 'running'
      break
    case 'file-done':
      row.state = 'done'
      row.out = p.outPath ?? ''
      break
    case 'file-failed':
      row.state = 'failed'
      row.error = p.error ?? '翻译失败'
      break
    case 'cancelled':
      row.state = 'skipped'
      break
    default:
      // chunk-done / queue-finished 不改单行状态，进度由调用方自己累计
      break
  }
}

/**
 * 跑一个 AI 批量任务，进度事件按 taskId 认领。
 *
 * 先订阅再启动，且把「还不知道 taskId」的事件暂存下来：Rust 线程一 spawn 就发第一个事件，
 * 它完全可能比 `launch()` 的 Promise 更早回到 JS —— 直接按 taskId 过滤会丢掉队首那条 file-start。
 */
function startJob(launch: () => Promise<number>, onProgress: (p: AiProgress) => void): AiTranslateJob {
  let taskId = 0
  let settled = false
  let stopWanted = false
  let early: AiProgress[] = []
  let unwatch: (() => void) | null = null

  let resolveDone!: () => void
  let rejectDone!: (e: unknown) => void
  const finished = new Promise<void>((res, rej) => {
    resolveDone = res
    rejectDone = rej
  })

  const close = (): void => {
    settled = true
    if (unwatch) unwatch()
    unwatch = null
  }
  const take = (p: AiProgress): void => {
    onProgress(p)
    if (p.state === 'queue-finished') {
      close()
      resolveDone()
    }
  }

  unwatch = onEvent<AiProgress>('ai:progress', (p) => {
    if (settled) return
    if (taskId === 0) {
      early.push(p)
      return
    }
    if (p.taskId === taskId) take(p)
  })

  launch().then(
    (id) => {
      taskId = id
      const buffered = early
      early = []
      if (stopWanted) void api.aiTaskCancel(id)
      for (const p of buffered) if (!settled && p.taskId === id) take(p)
    },
    (e) => {
      close()
      rejectDone(e)
    },
  )

  return {
    finished,
    stop: () => {
      stopWanted = true
      if (taskId) void api.aiTaskCancel(taskId)
    },
  }
}

/** 启动字幕翻译队列：每个文件一个任务单元，产物是同目录的 .zh.* */
export function translateSubtitleFiles(
  opts: AiJobOptions & { paths: string[] },
  onProgress: (p: AiProgress) => void,
): AiTranslateJob {
  return startJob(() => api.aiTranslateStart(opts), onProgress)
}

/**
 * 一批纯文本的翻译（AI 页给文件夹名生成译名）。
 * 返回 null = 任务被取消，调用方别把原文当译名用。
 */
export async function translateTexts(opts: AiJobOptions & { texts: string[] }): Promise<string[] | null> {
  let translated: string[] | null = null
  const job = startJob(
    () => api.aiTextsStart(opts),
    (p) => {
      if (p.translated) translated = p.translated
    },
  )
  await job.finished
  return translated
}
