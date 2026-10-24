<script setup lang="ts">
/** AI 翻译独立界面：Provider 配置 + 字幕/歌词批量翻译队列（.lrc/.srt/.vtt → .zh.*） */
import { ref, computed, onMounted, onBeforeUnmount } from 'vue'
import { useUiStore } from '@/stores/ui'
import { useSettingsStore } from '@/stores/settings'
import { useLibraryStore } from '@/stores/library'
import { api } from '@/bridge/ipc'
import {
  applyProgress,
  scanSubtitleFiles,
  translateSubtitleFiles,
  type AiTranslateJob,
} from '@/services/aiTranslate'
import { normDir } from '@/utils/format'
import { errText } from '@/contracts/result'
import { queueStats, resetFailed, resetOne, reenableOne, summarizeRun } from './queueStats'
import JobTable from './JobTable.vue'
import ActionRail from './ActionRail.vue'
import ProgressBand from './ProgressBand.vue'
import AiManualPanel from './AiManualPanel.vue'
import AiRenamePanel from './AiRenamePanel.vue'
import AiApiPane from './AiApiPane.vue'
import AiFolderNamePanel from './AiFolderNamePanel.vue'
import AiTranslatedPanel from './AiTranslatedPanel.vue'
import { jobOpts } from './useAiAccess'
import type { AiProgress, SubtitleScanItem } from '@/contracts/api'

const ui = useUiStore()
const settings = useSettingsStore()
const library = useLibraryStore()

type FileState = 'pending' | 'running' | 'done' | 'failed' | 'skipped' | 'exempt'
type FileRow = SubtitleScanItem & {
  state: FileState
  error?: string
  out?: string
  /** 本次回退原文的行数 / 总段数（file-done 带回，>0 = 完成但没翻干净） */
  fallback?: number
  segments?: number
  /** 运行中的块进度（chunk-done 心跳） */
  chunkDone?: number
  chunkTotal?: number
  /** 行内「重译」点的名：本次启动对这个文件绕开豁免账与内容缓存 */
  force?: boolean
}

const files = ref<FileRow[]>([])
const scanning = ref(false)
const running = ref(false)
// ── 选择性翻译 ──
// 勾哪些就翻哪些（与「全翻」按钮并存）；选中状态由这里持有，JobTable 只回事件。
// 重扫清空（路径换了一批，旧选中不再有意义）。
const selected = ref<Set<string>>(new Set())

function toggleOne(path: string): void {
  const next = new Set(selected.value)
  if (next.has(path)) next.delete(path)
  else next.add(path)
  selected.value = next
}

function toggleAll(on: boolean): void {
  selected.value = on ? new Set(files.value.map((f) => f.path)) : new Set()
}

/** 选中且这轮真能跑的行：等待 / 失败 / 已跳过（已翻译豁免的行内用「重译」，不进这批） */
const selectedRetryable = computed(() => files.value.filter((f) => selected.value.has(f.path) && isRetryable(f)))
/**
 * 本次运行的范围快照：进度与收尾统计只认它。
 * 旧实现用 files 全量过滤/累加，历史完成的行会混进本次成功率（audit B3），
 * cancelled 也会被累加进 doneCount（B2）。口径统一收在 queueStats.ts。
 */
let runPaths: string[] = []
const stat = computed(() => queueStats(files.value))
const finishedCount = computed(() => stat.value.finished)
const failedCount = computed(() => stat.value.failed)

/**
 * 运行中快照统计：进度带与轨道进度只认本次运行的任务集合。
 * 避免历史已完成、或免译（exempt）行使进度分母/分子在开局虚高。
 */
const runStat = computed(() => {
  if (!runPaths.length) return { total: 0, done: 0, failed: 0, skipped: 0, finished: 0, pending: 0 }
  const take = new Set(runPaths)
  return queueStats(files.value.filter((r) => take.has(r.path)))
})
const runFinishedCount = computed(() => runStat.value.finished)
const runFailedCount = computed(() => runStat.value.failed)

const targetDir = computed(() => ui.aiFolder)

// ── 工种切换（DESIGN.md「AI 工作台」§2）：分段控件，不重建页面 ──
type WorkKind = 'subtitle' | 'foldername' | 'rename' | 'translated'
const workKind = ref<WorkKind>('subtitle')
/** 工种二的条目数由子组件 emit 回来（父级不再持有一份扫描状态） */
const dirCount = ref(0)
/** 译文管理的条目数，同样由子组件 emit 回来 */
const translatedCount = ref(0)
const workKinds = computed(() => [
  { key: 'subtitle' as const, label: '字幕翻译', hint: '批量翻译 .lrc / .srt / .vtt，生成 .zh.* 译文文件，不覆盖原文件', count: files.value.length },
  { key: 'foldername' as const, label: '文件夹名翻译', hint: 'AI 生成目录译名，确认后才重命名', count: dirCount.value },
  { key: 'rename' as const, label: '规则改名', hint: '本地规则规范化，先算一遍再应用，整批可回滚', count: 0 },
  { key: 'translated' as const, label: '译文管理', hint: '管理已翻译的文件：重译、删除译文、清账。已翻译的不占翻译队列', count: translatedCount.value },
])
const rangeLabel = computed(() => {
  if (!targetDir.value) return '未选择文件夹'
  const tail = targetDir.value.replace(/\\/g, '/').split('/').filter(Boolean).pop() ?? targetDir.value
  return `${tail} · ${files.value.length} 个文件`
})

// ── 工序轨道与进度带（第 4 期，DESIGN.md「AI 工作台」§3.1/§4）──
const hasScope = computed(() => !!targetDir.value)
const scanned = computed(() => files.value.length > 0)
/** 接入可用 = 已选模型（Provider 的连通以「测试连接」回显为准，不阻塞阶段推导） */
const providerReady = computed(() => !!settings.aiModel)
const finished = computed(() => stat.value.finished)

const phases = computed(() => [
  { key: 'scope', label: '选择范围', state: hasScope.value ? 'done' : 'active', note: '' },
  {
    key: 'scan',
    label: '扫描',
    state: scanning.value ? 'active' : scanned.value ? 'done' : hasScope.value ? 'active' : 'pending',
    note: scanned.value ? `${files.value.length} 个文件` : '',
  },
  {
    key: 'check',
    label: '校验接入',
    state: providerReady.value ? 'done' : hasScope.value ? 'active' : 'pending',
    note: providerReady.value ? settings.aiModel : '未配置',
  },
  {
    key: 'translate',
    label: '翻译',
    state: running.value ? 'active' : stat.value.done > 0 || stat.value.failed > 0 ? 'done' : 'pending',
    note: running.value ? `${runFinishedCount.value}/${runStat.value.total}` : '',
  },
  {
    key: 'review',
    label: '核对结果',
    state: !running.value && finished.value > 0 && runSettled.value ? (stat.value.failed > 0 ? 'failed' : 'done') : 'pending',
    note: stat.value.failed > 0 ? `失败 ${stat.value.failed}` : '',
  },
] as const)

/** 队列是否收过尾：running 结束后才算，避免翻译前误亮「核对结果」 */
const runSettled = ref(false)

// 运行计时：条目每分的前端现算（不依赖新契约，设计包 §5.1）
const runStart = ref(0)
const elapsedSec = ref(0)
let runTimer: ReturnType<typeof setInterval> | null = null

function startRunTimer(): void {
  runStart.value = Date.now()
  elapsedSec.value = 0
  runTimer = setInterval(() => {
    elapsedSec.value = Math.round((Date.now() - runStart.value) / 1000)
  }, 1000)
}
function stopRunTimer(): void {
  if (runTimer) clearInterval(runTimer)
  runTimer = null
}
onBeforeUnmount(() => stopRunTimer())

/** 汇总播报文案（aria-live=polite，DESIGN.md §7）：只报里程碑，不逐行刷屏 */
const liveSummary = computed(() => {
  if (running.value) return `已完成 ${runFinishedCount.value} 项 / 共 ${runStat.value.total} 项`
  if (runSettled.value && stat.value.failed > 0) return `翻译结束，失败 ${stat.value.failed} 项`
  if (runSettled.value && stat.value.done > 0) return `翻译完成，成功 ${stat.value.done} 项`
  return ''
})

async function pickFolder(): Promise<void> {
  const picked = await api.openFolder()
  if (picked && picked.length) {
    ui.aiFolder = normDir(picked[0])
    await rescan()
  }
}

async function useLibraryFolder(path: string): Promise<void> {
  ui.aiFolder = normDir(path)
  await rescan()
}

async function rescan(): Promise<void> {
  if (!ui.aiFolder) return
  scanning.value = true
  try {
    const found = await scanSubtitleFiles(ui.aiFolder)
    // 已翻译（账命中 + 译文还在）的不进队列：它们的家在「译文管理」标签，
    // 重译 / 删除译文 / 清账都在那边处置 —— 队列只留真正要翻的源文件
    files.value = found
      .filter((f) => !f.translated)
      .map((f) => ({ ...f, state: 'pending' as FileState }))
    runPaths = []
    selected.value = new Set()
  } catch (e) {
    // 递归扫描整体失败要吭声：静默清空会被当成「这个文件夹没有字幕文件」
    files.value = []
    ui.toast(`扫描字幕文件失败：${errText(e)}`, 'error')
  } finally {
    scanning.value = false
  }
}

let job: AiTranslateJob | null = null

/** 取消过的行留在队列里，「开始翻译」要能把它们重新排队，否则停止后这个文件夹就再也点不动了。
 * 已翻译豁免（exempt）不在其列 —— AI-15 要求「扫描时默认跳过」，「开始翻译」默认不重翻；要重译单点行内「重译」。 */
const isRetryable = (f: FileRow): boolean => f.state === 'pending' || f.state === 'failed' || f.state === 'skipped'

/**
 * 「全翻」与「只翻选中」共用同一条启动流水：queue 是本次运行的范围快照，
 * 统计 / 进度 / force 点名都只认它（audit B3）。
 */
async function startTranslate(scope?: 'selected'): Promise<void> {
  if (running.value) return
  const pool = scope === 'selected' ? selectedRetryable.value : files.value.filter(isRetryable)
  if (!pool.length) return
  const queue = pool
  running.value = true
  runSettled.value = false
  startRunTimer()
  // 快照先落，再启动：统计只认这批行，后面扫进来的、历史完成的都不掺（audit B3）
  runPaths = queue.map((f) => f.path)
  // 行对象与 paths 同序：Rust 事件的 fileIndex 就是这里的下标
  const paths = queue.map((f) => f.path)
  // 行内「重译」点过名的文件要真翻译：后端对它们绕开豁免账与内容缓存
  const forcePaths = queue.filter((f) => f.force).map((f) => f.path)
  // 队列被提前掐断时的原因（端点失联），收尾事件带来
  let aborted = ''
  job = translateSubtitleFiles({ ...jobOpts(), paths, forcePaths }, (p: AiProgress) => {
    applyProgress(queue, p)
    if (p.state === 'queue-finished' && p.error) aborted = p.error
  })
  try {
    await job.finished
  } catch (e) {
    ui.toast(errText(e), 'error')
  } finally {
    running.value = false
    stopRunTimer()
    runSettled.value = true
    job = null
    // force 是「这一轮」的点名：跑完就清，下一轮不再绕开省钱闸门
    for (const r of queue) r.force = false
  }
  const summary = summarizeRun(runPaths, files.value)
  if (aborted) {
    ui.toast(`AI 翻译已中断：${aborted}`, 'error')
    return
  }
  const take = new Set(runPaths)
  const partial = files.value.filter((r) => take.has(r.path) && r.state === 'done' && (r.fallback ?? 0) > 0).length
  const base = `AI 翻译完成：成功 ${summary.done}/${summary.total}`
  if (partial) {
    ui.toast(`${base}，其中 ${partial} 个文件部分未翻（行内有「重译」）`, 'error')
  } else {
    ui.toast(base, summary.done ? 'success' : 'error')
  }
}

function stopTranslate(): void {
  job?.stop()
}

function clearFinished(): void {
  files.value = files.value.filter((f) => f.state === 'pending' || f.state === 'running')
  runPaths = []
  // 清掉的行从选中集里一并摘除，别让「翻译选中」对着空号
  const alive = new Set(files.value.map((f) => f.path))
  selected.value = new Set([...selected.value].filter((p) => alive.has(p)))
}

/**
 * 半自动批次的序号是按**整个队列**编出来的（`ai_export.rs`），所以这里不能传「待重试的那几个」：
 * 队列行少一条，后面所有序号就全体前移，用户手上那份 prompts.txt 就对到别的文件上去了。
 */
const manualPaths = computed(() => files.value.map((f) => f.path))

/** 手工导入写盘成功的行标成完成 —— 否则队列一直显示「还差 N 个」，看着像没生效 */
function onManualApplied(done: { path: string, out: string }[]): void {
  for (const d of done) {
    const row = files.value.find((f) => f.path === d.path)
    if (!row) continue
    row.state = 'done'
    row.out = d.out
    row.error = undefined
  }
}

/** 只重试失败项：把 failed 行重置回 pending，其余不动（audit B7；顺序不变，见 queueStats.resetFailed） */
function retryFailed(): void {
  if (running.value) return
  const n = resetFailed(files.value)
  if (n) ui.toast(`已把 ${n} 个失败文件重新排队`, 'info')
}

/** 行内只重试一项（audit B6）：resetOne 找不到/不是 failed 时静默（按钮本就只在 failed 行出现） */
function retryOne(path: string): void {
  if (running.value) return
  resetOne(files.value, path)
}

/** 行内重译一项已豁免的文件（AI-15）：exempt → pending，重排进「开始翻译」的队列 */
function reEnableOne(path: string): void {
  if (running.value) return
  if (reenableOne(files.value, path)) ui.toast('已把该文件放回翻译队列', 'info')
}

/** 进度带上的当前项：最后一个 running 行（事件只带来下标，用行状态反推最稳）；
 *  它的块进度同时喂给进度带做填充内插，大文件期间条子也在走 */
const runningRow = computed(() => files.value.find((f) => f.state === 'running'))
const currentItem = computed(() => runningRow.value?.name ?? '')

/** 进度带上点「失败 N」：JobTable 的展开态是内部状态，本期先提示到账（第 5 期随 LedgerDrawer 收口） */
function openFailures(): void {
  ui.toast(`队列里有 ${failedCount.value} 个失败项，点击失败行可看原因`, 'info')
}

const libraryRoots = computed(() => library.folderPaths)

onMounted(() => {
  if (!ui.aiFolder && libraryRoots.value.length) {
    ui.aiFolder = normDir(libraryRoots.value[0])
    void rescan()
  }
})
// 离开页面就停：AI 队列没有跨页面的任务状态，留它在后台跑会变成「看不见但仍在烧 token」
onBeforeUnmount(() => { job?.stop() })

// Ctrl+1/2/3 切工种（DESIGN.md「AI 工作台」§7）。
// 放视图本地而不是全局 useKeyboardShortcuts：全局把带 Ctrl 的键直接 return（33 行），
// 且这组键只在 AI 视图有意义 —— 本地监听挂载/卸载即得，不碰全局语义。
function onKeydown(e: KeyboardEvent): void {
  if (!(e.ctrlKey || e.metaKey) || e.altKey) return
  const idx = ['1', '2', '3', '4'].indexOf(e.key)
  if (idx < 0) return
  const kinds = workKinds.value
  if (idx >= kinds.length) return
  e.preventDefault()
  workKind.value = kinds[idx].key
}
onMounted(() => window.addEventListener('keydown', onKeydown))
onBeforeUnmount(() => window.removeEventListener('keydown', onKeydown))
</script>

<template>
  <div class="ai-view">
    <!-- 模板根级不能放注释：Transition mode="out-in" 要求单根元素，
         根级多一个注释节点就会让组件变片段、过渡钩子失效，
         离开本视图后新视图的 enter 永不执行（白屏回归的根因，见转接文档）。
         第 2 期版面：身份栏 + 左列(范围/接入) + 右区(作业面，唯一滚动容器) -->
    <header class="wb-head" role="toolbar" aria-label="AI 工作台">
      <h1 class="wb-title">AI 工作台</h1>
      <div class="wb-tabs" role="tablist" aria-label="工种">
        <button
          v-for="t in workKinds"
          :key="t.key"
          class="wb-tab"
          role="tab"
          :class="{ on: workKind === t.key }"
          :aria-selected="workKind === t.key"
          :title="t.hint"
          @click="workKind = t.key"
        >{{ t.label }}<span v-if="t.count > 0" class="tnum wb-tab-count">{{ t.count }}</span></button>
      </div>
      <span class="wb-range" :title="targetDir">{{ rangeLabel }}</span>
    </header>

    <div class="wb-body">
      <!-- 左列：范围 + 接入。设置一次、整场不变，所以不随作业区滚动 -->
      <aside class="wb-side" aria-label="范围与接入">
        <section class="wb-side-block">
          <h3 class="section-title">目标文件夹</h3>
          <div class="ai-target-row">
            <span class="ai-target-path" :title="targetDir">{{ targetDir || '未选择文件夹' }}</span>
            <button class="btn-ghost" @click="pickFolder">选择文件夹…</button>
            <button class="btn-ghost" :disabled="!targetDir || scanning" :title="!targetDir ? '先选一个文件夹' : ''" @click="rescan">
              {{ scanning ? '扫描中…' : '重新扫描' }}
            </button>
          </div>
          <div v-if="libraryRoots.length" class="ai-quick">
            <span class="ai-quick-label">快速选择：</span>
            <button v-for="p in libraryRoots" :key="p" class="ai-quick-chip" :title="p" @click="useLibraryFolder(p)">
              {{ p.replace(/\\/g, '/').split('/').filter(Boolean).pop() }}
            </button>
          </div>
        </section>

        <AiApiPane />
      </aside>

      <!-- 右区：唯一滚动容器。作业面按工种切换，不重建页面 -->
      <div class="wb-stage" role="region" aria-label="作业区">
        <!-- 字幕翻译 -->
        <section v-show="workKind === 'subtitle'" class="wb-work" :aria-hidden="workKind !== 'subtitle'">
          <ActionRail :phases="[...phases]" />
          <ProgressBand
            v-if="running"
            :done="runFinishedCount"
            :total="runStat.total"
            :elapsed-sec="elapsedSec"
            :current-item="currentItem"
            :failed="runFailedCount"
            :chunk-done="runningRow?.chunkDone"
            :chunk-total="runningRow?.chunkTotal"
            @open-failures="openFailures"
          />
          <!-- 汇总播报：只报里程碑（DESIGN.md §7），视觉上隐藏 -->
          <span class="visually-hidden" aria-live="polite">{{ liveSummary }}</span>
          <div class="ai-files-head">
            <h3 class="section-title">翻译队列 · {{ files.length }} 个文件</h3>
            <div class="ai-files-actions">
              <span v-if="running" class="tnum ai-progress">{{ runFinishedCount }}/{{ runStat.total }}</span>
              <span v-if="failedCount" class="tnum ai-failed-count">失败 {{ failedCount }}</span>
              <button v-if="failedCount && !running" class="btn-ghost" title="把失败文件重新排队，其余不动" @click="retryFailed">重试失败项</button>
              <button class="btn-ghost" :disabled="!files.length" @click="clearFinished">清除已完成</button>
              <button v-if="running" class="btn-ghost danger-ghost" @click="stopTranslate">停止</button>
              <button
                v-if="selected.size && !running"
                class="btn-primary"
                :disabled="!selectedRetryable.length || !settings.aiModel"
                :title="selectedRetryable.length
                  ? `只翻译选中的 ${selectedRetryable.length} 个文件（选中共 ${selected.size} 个）`
                  : '选中的文件里没有可翻译的：已翻译的行内用「重译」'
                "
                @click="startTranslate('selected')"
              >
                {{ `翻译选中（${selectedRetryable.length}）` }}
              </button>
              <button
                class="btn-primary"
                :disabled="running || !files.filter(isRetryable).length || !settings.aiModel"
                :title="!settings.aiModel ? '先在左列配置模型服务' : !files.filter(isRetryable).length ? '没有等待中的文件' : ''"
                @click="startTranslate()"
              >
                {{ running ? '翻译中…' : `开始翻译（${files.filter(isRetryable).length}）` }}
              </button>
            </div>
          </div>
          <JobTable
            :rows="files"
            :empty-text="targetDir ? '该文件夹下没有找到 .lrc / .srt / .vtt 文件' : '请先在左列选择目标文件夹'"
            selectable
            :selected="selected"
            @toggle-one="toggleOne"
            @toggle-all="toggleAll"
            @retry-one="retryOne"
            @re-enable-one="reEnableOne"
          />
          <!-- 没有 Provider 也要能用：导出批次 → 外部工具跑 → 导回校验（AI-16，`05` 第 16 条） -->
          <AiManualPanel :paths="manualPaths" @applied="onManualApplied" />
        </section>

        <!-- 文件夹名翻译工种 -->
        <section v-show="workKind === 'foldername'" class="wb-work" :aria-hidden="workKind !== 'foldername'">
          <AiFolderNamePanel :target-dir="targetDir" @count="dirCount = $event" />
        </section>

        <!-- 规则改名工种：dry-run 只算不改，应用与回滚下一批才有 -->
        <section v-show="workKind === 'rename'" class="wb-work" :aria-hidden="workKind !== 'rename'">
          <AiRenamePanel :target-dir="targetDir" />
        </section>

        <!-- 译文管理工种：已翻译文件的盘点与处置（active 时才拉数据，切走不打扰） -->
        <section v-show="workKind === 'translated'" class="wb-work" :aria-hidden="workKind !== 'translated'">
          <AiTranslatedPanel :active="workKind === 'translated'" @count="translatedCount = $event" />
        </section>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* ── 三区骨架（DESIGN.md「AI 工作台」§2）── */
/* 工作台自带面板底：不压壁纸。浅色壁纸上文字若直接落在壁纸上不可读（用户反馈），
   铺 --bg-card 后两主题都成立；凡有此底的盒子不进 .on-bg 自适应墨色（base.css 约定）。 */
.ai-view {
  height: 100%;
  display: flex;
  flex-direction: column;
  min-height: 0;
  background: var(--bg-card);
  border-radius: var(--radius);
}
.wb-head {
  display: flex;
  align-items: center;
  gap: 16px;
  height: var(--toolbar-h);
  padding: 0 20px;
  flex-shrink: 0;
  border-bottom: 1px solid var(--border);
  background: var(--bg-titlebar);
}
.wb-title { font-size: 13px; font-weight: 500; flex-shrink: 0; }
.wb-tabs { display: flex; gap: 4px; }
.wb-tab {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 5px 12px;
  border-radius: var(--radius-sm);
  font-size: 12px;
  color: var(--text-sub);
}
.wb-tab:hover { background: var(--bg-hover); color: var(--text); }
.wb-tab.on { background: var(--bg-selected); color: rgb(var(--accent-rgb)); font-weight: 500; }
.wb-tab-count {
  font-size: 10.5px;
  color: var(--text-muted);
  background: var(--bg-input);
  border-radius: 999px;
  padding: 0 6px;
}
.wb-tab.on .wb-tab-count { color: rgb(var(--accent-rgb)); }
.wb-range {
  margin-left: auto;
  font-size: 11.5px;
  color: var(--text-muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 320px;
}

.wb-body {
  flex: 1;
  min-height: 0;
  display: grid;
  grid-template-columns: 320px minmax(0, 1fr);
  gap: 0;
}
.wb-side {
  border-right: 1px solid var(--border);
  padding: 16px;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 20px;
  background: var(--bg-input);
}
.wb-side-block > .section-title { display: block; margin-bottom: 10px; }

/* 右区：唯一滚动容器 */
.wb-stage {
  min-height: 0;
  overflow-y: auto;
  padding: 16px 20px 24px;
  display: flex;
  flex-direction: column;
  gap: 16px;
}
.wb-work { display: flex; flex-direction: column; gap: 10px; min-width: 0; }

@media (max-width: 1023px) {
  .wb-body { grid-template-columns: minmax(0, 1fr); }
  .wb-side {
    border-right: 0;
    border-bottom: 1px solid var(--border);
    flex-direction: row;
    flex-wrap: wrap;
    gap: 12px;
    padding: 12px 20px;
  }
  .wb-side-block { flex: 1 1 320px; }
  .wb-range { display: none; }
}

/* ── 汇总播报：视觉隐藏（aria-live 用）── */
.visually-hidden {
  position: absolute;
  width: 1px;
  height: 1px;
  overflow: hidden;
  clip: rect(0 0 0 0);
}

.ai-target-row {
  display: flex;
  align-items: center;
  gap: 10px;
}
.ai-target-path {
  flex: 1;
  min-width: 0;
  font-size: 12px;
  color: var(--text-sub);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  direction: rtl;
  text-align: left;
}
.ai-quick {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-top: 10px;
  flex-wrap: wrap;
}
.ai-quick-label { font-size: 11px; color: var(--text-muted); }
.ai-quick-chip {
  padding: 3px 12px;
  border-radius: 999px;
  border: 1px solid var(--border);
  font-size: 11.5px;
  color: var(--text-sub);
}
.ai-quick-chip:hover { border-color: rgb(var(--accent-rgb)); color: rgb(var(--accent-rgb)); }

.ai-files-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
}
.ai-files-head .section-title { margin-bottom: 0 !important; }
.ai-files-actions { display: flex; align-items: center; gap: 10px; }
.ai-progress { font-size: 12px; color: rgb(var(--accent-rgb)); }
.ai-failed-count { font-size: 12px; color: var(--danger); }
.danger-ghost:hover { color: var(--danger); border-color: color-mix(in srgb, var(--danger) 50%, transparent); }

</style>