<script setup lang="ts">
/** AI 翻译独立界面：Provider 配置 + 字幕/歌词批量翻译队列（.lrc/.srt/.vtt → .zh.*） */
import { ref, computed, onMounted, onBeforeUnmount } from 'vue'
import { useUiStore } from '@/stores/ui'
import { useSettingsStore } from '@/stores/settings'
import { useLibraryStore } from '@/stores/library'
import { api } from '@/bridge/ipc'
import {
  applyProgress,
  scanSubDirectories,
  scanSubtitleFiles,
  translateSubtitleFiles,
  translateTexts,
  type AiTranslateJob,
} from '@/services/aiTranslate'
import { normDir } from '@/utils/format'
import { errText } from '@/contracts/result'
import AiManualPanel from './AiManualPanel.vue'
import AiRenamePanel from './AiRenamePanel.vue'
import type { AiProgress, SubtitleScanItem } from '@/contracts/api'

const ui = useUiStore()
const settings = useSettingsStore()
const library = useLibraryStore()

const apiKey = ref('')          // 仅内存，不落盘
const testing = ref(false)
const testResult = ref('')
const modelList = ref<string[]>([])
const fetchingModels = ref(false)
const modelError = ref('')
const modelOpen = ref(false)
const modelFilter = ref('')

/** 已获取的模型按输入过滤（不区分大小写），用于下拉展示 */
const visibleModels = computed(() => {
  const q = modelFilter.value.trim().toLowerCase()
  if (!q) return modelList.value
  return modelList.value.filter((m) => m.toLowerCase().includes(q))
})

function pickModel(m: string): void {
  settings.aiModel = m
  settings.scheduleSave()
  modelOpen.value = false
}

async function fetchModels(): Promise<void> {
  if (!settings.aiBaseURL.trim()) { modelError.value = '请先填写接口地址'; return }
  fetchingModels.value = true
  modelError.value = ''
  try {
    const r = await api.aiListModels({ baseURL: settings.aiBaseURL, apiKey: apiKey.value })
    if (r.ok && r.models.length) {
      modelList.value = r.models
      if (!settings.aiModel || !r.models.includes(settings.aiModel)) {
        settings.aiModel = r.models[0]
        settings.scheduleSave()
      }
      ui.toast(`已获取 ${r.models.length} 个模型`, 'success')
      modelOpen.value = true
    } else {
      modelError.value = r.error || '未获取到模型'
    }
  } catch (e) {
    // 之前只有 try/finally：invoke 失败会被静默吞掉，按钮看起来「无效」
    modelError.value = errText(e)
  } finally {
    fetchingModels.value = false
  }
}

type FileState = 'pending' | 'running' | 'done' | 'failed' | 'skipped'
type FileRow = SubtitleScanItem & { state: FileState; error?: string; out?: string }

const files = ref<FileRow[]>([])
const scanning = ref(false)
const running = ref(false)
const doneCount = ref(0)
const totalCount = ref(0)

const targetDir = computed(() => ui.aiFolder)

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
    files.value = found.map((f) => ({ ...f, state: 'pending' as const }))
    totalCount.value = files.value.length
    doneCount.value = 0
  } catch (e) {
    // 递归扫描整体失败要吭声：静默清空会被当成「这个文件夹没有字幕文件」
    files.value = []
    ui.toast(`扫描字幕文件失败：${errText(e)}`, 'error')
  } finally {
    scanning.value = false
  }
}

async function testConnection(): Promise<void> {
  testing.value = true
  testResult.value = ''
  try {
    const r = await api.aiPing({ baseURL: settings.aiBaseURL, apiKey: apiKey.value, model: settings.aiModel })
    // 成功时把 AI-5 回显的档位信息（模型名/延迟/token）拼在后面，失败时只有原文
    const facts = [
      r.model && `模型 ${r.model}`,
      r.latencyMs != null && `延迟 ${r.latencyMs} ms`,
      r.totalTokens != null && `tokens ${r.totalTokens}`,
    ].filter(Boolean)
    testResult.value = (r.ok ? '✓ ' : '✗ ') + r.message + (facts.length ? `（${facts.join(' · ')}）` : '')
  } finally {
    testing.value = false
  }
}

let job: AiTranslateJob | null = null

/**
 * 两个任务共用的接入参数。
 * 批量大小 / 并发 / 超时**不在这里决定** —— 前端只说「本地还是外接、本地合几条」，
 * 档位参数在 Rust 的 `ai_strategy.rs` 里，两处各定一半必然漂移。
 */
function jobOpts() {
  return {
    baseURL: settings.aiBaseURL,
    apiKey: apiKey.value,
    model: settings.aiModel,
    modelKind: settings.aiModelKind,
    mergeLines: settings.aiMergeLines,
  }
}

/** 切档位要存下来：它决定 Rust 侧用哪一档参数 */
function setModelKind(k: 'remote' | 'local'): void {
  settings.aiModelKind = k
  settings.scheduleSave()
}

/** 输入框能填任意数，这里先按选项范围收住（Rust 侧还会再钳一次，两边都得有） */
function clampMerge(): void {
  const n = Math.round(Number(settings.aiMergeLines))
  settings.aiMergeLines = isFinite(n) ? Math.min(10, Math.max(2, n)) : 4
  settings.scheduleSave()
}

/** 取消过的行留在队列里，「开始翻译」要能把它们重新排队，否则停止后这个文件夹就再也点不动了 */
const isRetryable = (f: FileRow): boolean => f.state === 'pending' || f.state === 'failed' || f.state === 'skipped'

async function startTranslate(): Promise<void> {
  if (running.value) return
  const queue = files.value.filter(isRetryable)
  if (!queue.length) return
  running.value = true
  totalCount.value = queue.length
  doneCount.value = 0
  // 行对象与 paths 同序：Rust 事件的 fileIndex 就是这里的下标
  const paths = queue.map((f) => f.path)
  // 队列被提前掐断时的原因（端点失联），收尾事件带来
  let aborted = ''
  job = translateSubtitleFiles({ ...jobOpts(), paths }, (p: AiProgress) => {
    applyProgress(queue, p)
    if (p.state === 'file-done' || p.state === 'file-failed' || p.state === 'cancelled') doneCount.value++
    if (p.state === 'queue-finished' && p.error) aborted = p.error
  })
  try {
    await job.finished
  } catch (e) {
    ui.toast(errText(e), 'error')
  } finally {
    running.value = false
    job = null
  }
  const ok = files.value.filter((f) => f.state === 'done').length
  if (aborted) {
    ui.toast(`AI 翻译已中断：${aborted}`, 'error')
    return
  }
  ui.toast(`AI 翻译完成：成功 ${ok}/${files.value.length}`, ok ? 'success' : 'error')
}

function stopTranslate(): void {
  job?.stop()
}

function clearFinished(): void {
  files.value = files.value.filter((f) => f.state === 'pending' || f.state === 'running')
  totalCount.value = files.value.length
  doneCount.value = 0
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
  doneCount.value = files.value.filter((f) => f.state === 'done').length
}

const stateLabel = {
  pending: '等待',
  running: '翻译中',
  done: '完成',
  failed: '失败',
  skipped: '已跳过',
} as const

// ── 文件夹名翻译 ──
interface DirRenameItem {
  path: string
  name: string
  suggested: string
  newPath: string
  state: 'pending' | 'done' | 'conflict' | 'failed' | 'skipped'
  error?: string
  enabled: boolean
}

const dirItems = ref<DirRenameItem[]>([])
const dirScanning = ref(false)
const dirSuggesting = ref(false)
const dirApplying = ref(false)
const dirDone = ref(0)
const dirTotal = ref(0)

async function scanDirNames(): Promise<void> {
  if (!ui.aiFolder || dirScanning.value) return
  dirScanning.value = true
  try {
    const dirs = await scanSubDirectories(ui.aiFolder)
    dirItems.value = dirs.map((d) => ({
      path: d.path, name: d.name, suggested: '', newPath: '',
      state: 'pending' as const, enabled: true,
    }))
  } catch (e) {
    ui.toast(`扫描文件夹失败：${errText(e)}`, 'error')
  } finally {
    dirScanning.value = false
  }
}

function sanitizeName(name: string): string {
  return name.replace(/[\\/:*?"<>|]/g, ' ').replace(/\s+/g, ' ').trim()
}

async function suggestDirNames(): Promise<void> {
  if (!dirItems.value.length || dirSuggesting.value) return
  dirSuggesting.value = true
  try {
    // 文件夹名很短，一批 10 条比字幕更划算 —— 显式覆盖档位里的批量大小；
    // 并发不再写死 2：本地模型两路并发只是互相抢内存，交给档位决定。
    const translated = await translateTexts({ ...jobOpts(), texts: dirItems.value.map((d) => d.name), chunkSize: 10 })
    if (!translated) {
      ui.toast('译名生成已取消', 'error')
      return
    }
    dirItems.value = dirItems.value.map((d, i) => {
      const suggested = sanitizeName(translated[i] ?? d.name) || d.name
      const parent = d.path.slice(0, d.path.lastIndexOf('/'))
      return { ...d, suggested, newPath: `${parent}/${suggested}` }
    })
  } catch (e) {
    ui.toast(errText(e), 'error')
  } finally {
    dirSuggesting.value = false
  }
}

async function applyDirRenames(): Promise<void> {
  if (dirApplying.value) return
  const targets = dirItems.value.filter((d) => d.enabled && d.state === 'pending' && d.suggested && d.newPath !== d.path)
  if (!targets.length) return
  dirApplying.value = true
  dirDone.value = 0
  dirTotal.value = targets.length
  // 深层优先：先重命名子目录，父目录路径仍有效
  const ordered = [...targets].sort((a, b) => b.path.split('/').length - a.path.split('/').length)
  for (const item of ordered) {
    const r = await api.renameDir(item.path, item.newPath)
    item.state = r.ok ? 'done' : (r.error?.includes('已存在') ? 'conflict' : 'failed')
    if (!r.ok) item.error = r.error
    dirDone.value++
  }
  dirApplying.value = false
  const ok = dirItems.value.filter((d) => d.state === 'done').length
  ui.toast(`文件夹重命名完成：成功 ${ok}/${targets.length}`, ok ? 'success' : 'error')
  // 目录名变了，曲库需要重新扫描
  void library.rescan(true)
}

const dirStateLabel = {
  pending: '待应用',
  done: '完成',
  conflict: '冲突',
  failed: '失败',
  skipped: '跳过',
} as const

const libraryRoots = computed(() => library.folderPaths)

onMounted(() => {
  if (!ui.aiFolder && libraryRoots.value.length) {
    ui.aiFolder = normDir(libraryRoots.value[0])
    void rescan()
  }
})
// 离开页面就停：AI 队列没有跨页面的任务状态，留它在后台跑会变成「看不见但仍在烧 token」
onBeforeUnmount(() => { job?.stop() })
</script>

<template>
  <div class="ai-view">
    <div class="ai-head">
      <h1 class="ai-title">AI 翻译</h1>
      <span class="ai-sub">批量翻译字幕/歌词（.lrc / .srt / .vtt → 生成 .zh.* 译文文件，不覆盖原文件）</span>
    </div>

    <!-- Provider 配置 -->
    <section class="ai-card">
      <h3 class="section-title">模型服务（OpenAI 兼容接口）</h3>
      <div class="ai-grid">
        <label class="ai-field">
          <span>接口地址</span>
          <input v-model="settings.aiBaseURL" @change="settings.scheduleSave()" />
        </label>
        <label class="ai-field">
          <span>模型</span>
          <div class="ai-model-row">
            <div class="ai-model-combo">
              <input
                v-model="settings.aiModel"
                :placeholder="modelList.length ? '选择或输入模型' : '例如 gpt-4o-mini'"
                @change="settings.scheduleSave()"
                @focus="modelOpen = modelList.length > 0"
              />
              <button
                v-if="modelList.length"
                class="ai-model-toggle"
                title="展开模型列表"
                aria-label="展开模型列表"
                @click="modelOpen = !modelOpen"
              >
                <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2.4"><polyline points="6,9 12,15 18,9" /></svg>
              </button>
            </div>
            <button
              class="btn-ghost ai-model-btn"
              :disabled="fetchingModels || !settings.aiBaseURL.trim()"
              title="从接口获取可用模型列表"
              @click="fetchModels"
            >{{ fetchingModels ? '获取中…' : '获取模型' }}</button>
          </div>

          <!-- 模型下拉：完整列出，可滚动、可过滤（替代原生 datalist 的截断显示） -->
          <Transition name="pop-scale">
            <div v-if="modelOpen && modelList.length" class="ai-model-menu">
              <div class="ai-model-menu-head">
                <input
                  v-model="modelFilter"
                  class="ai-model-search"
                  type="text"
                  placeholder="过滤模型…"
                  aria-label="过滤模型"
                />
                <span class="ai-model-count tnum">{{ visibleModels.length }}/{{ modelList.length }}</span>
              </div>
              <div class="ai-model-list">
                <button
                  v-for="m in visibleModels"
                  :key="m"
                  class="ai-model-item"
                  :class="{ active: m === settings.aiModel }"
                  :title="m"
                  @click="pickModel(m)"
                >
                  <span class="ai-model-name">{{ m }}</span>
                  <svg v-if="m === settings.aiModel" viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2.6"><polyline points="20,6 9,17 4,12" /></svg>
                </button>
                <div v-if="!visibleModels.length" class="ai-model-empty">没有匹配的模型</div>
              </div>
            </div>
          </Transition>

          <span v-if="modelError" class="ai-field-error">{{ modelError }}</span>
        </label>
        <label class="ai-field">
          <span>API Key（仅本次运行记忆，不写入磁盘）</span>
          <input v-model="apiKey" type="password" autocomplete="off" />
        </label>
      </div>
      <div class="ai-strategy">
        <span class="ai-strategy-label">模型类型</span>
        <div class="ai-seg">
          <button
            class="btn-ghost"
            :class="{ on: settings.aiModelKind === 'remote' }"
            title="云端 OpenAI 兼容接口：一批多送几条、多路并发、超时较短"
            @click="setModelKind('remote')"
          >外接模型</button>
          <button
            class="btn-ghost"
            :class="{ on: settings.aiModelKind === 'local' }"
            title="本机 Ollama 等：一次只发一路、给足加载权重的时间、编号解析失败自动拆开重试"
            @click="setModelKind('local')"
          >本地模型</button>
        </div>
        <label v-if="settings.aiModelKind === 'local'" class="ai-merge">
          <span>一次合并</span>
          <input
            v-model.number="settings.aiMergeLines"
            type="number"
            min="2"
            max="10"
            step="1"
            @change="clampMerge()"
          />
          <span>条发送</span>
        </label>
        <!-- 只描述策略、不复述数字：具体参数的事实源在 Rust `ai_strategy.rs`，这里抄一遍就会漂 -->
        <span class="ai-strategy-hint">
          {{ settings.aiModelKind === 'local' ? '本地档：单路请求、长超时、失败自动拆半重试' : '外接档：多路并发、每批多送几条、超时较短' }}
        </span>
      </div>
      <div class="ai-test-row">
        <button class="btn-ghost" :disabled="testing || !settings.aiModel" @click="testConnection">
          {{ testing ? '测试中…' : '测试连接' }}
        </button>
        <span class="ai-test-result" :class="{ ok: testResult.startsWith('✓') }">{{ testResult }}</span>
      </div>
    </section>

    <!-- 目标文件夹 -->
    <section class="ai-card">
      <h3 class="section-title">目标文件夹</h3>
      <div class="ai-target-row">
        <span class="ai-target-path" :title="targetDir">{{ targetDir || '未选择文件夹' }}</span>
        <button class="btn-ghost" @click="pickFolder">选择文件夹…</button>
        <button class="btn-ghost" :disabled="!targetDir || scanning" @click="rescan">
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

    <!-- 文件夹名翻译 -->
    <section class="ai-card">
      <div class="ai-files-head">
        <h3 class="section-title">文件夹名翻译（AI 生成译名 → 确认后重命名，不经过确认不改动磁盘）</h3>
        <div class="ai-files-actions">
          <span v-if="dirApplying" class="tnum ai-progress">{{ dirDone }}/{{ dirTotal }}</span>
          <button class="btn-ghost" :disabled="!targetDir || dirScanning" @click="scanDirNames">
            {{ dirScanning ? '扫描中…' : `扫描文件夹${dirItems.length ? '（重新）' : ''}` }}
          </button>
          <button
            class="btn-primary"
            :disabled="dirSuggesting || !dirItems.length || !settings.aiModel"
            @click="suggestDirNames"
          >{{ dirSuggesting ? '生成译名中…' : '生成译名' }}</button>
          <button
            class="btn-primary"
            :disabled="dirApplying || !dirItems.some((d) => d.enabled && d.state === 'pending' && d.suggested)"
            @click="applyDirRenames"
          >应用重命名</button>
        </div>
      </div>
      <div v-if="dirItems.length" class="ai-files">
        <div v-for="d in dirItems" :key="d.path" class="ai-file" :class="'dir-' + d.state">
          <input v-model="d.enabled" type="checkbox" :disabled="d.state !== 'pending'" class="ai-dir-check" />
          <span class="ai-file-state">{{ dirStateLabel[d.state] }}</span>
          <span class="ai-file-name" :title="d.path">{{ d.name }}</span>
          <span class="ai-dir-arrow">→</span>
          <span class="ai-dir-suggested" :title="d.newPath">{{ d.suggested || '（未生成）' }}</span>
          <span v-if="d.error" class="ai-file-error" :title="d.error">!</span>
        </div>
      </div>
      <div v-else class="ai-empty">
        {{ targetDir ? '点击「扫描文件夹」列出子目录，再用 AI 生成简体译名' : '请先选择目标文件夹' }}
      </div>
    </section>

    <!-- 改名主干（AI-6/7/10）的 dry-run：只算不改，应用与回滚下一批才有 -->
    <section class="ai-card">
      <AiRenamePanel :target-dir="targetDir" />
    </section>

    <!-- 文件队列 -->
    <section class="ai-card ai-files-card">
      <div class="ai-files-head">
        <h3 class="section-title">翻译队列 · {{ files.length }} 个文件</h3>
        <div class="ai-files-actions">
          <span v-if="running" class="tnum ai-progress">{{ doneCount }}/{{ totalCount }}</span>
          <button class="btn-ghost" :disabled="!files.length" @click="clearFinished">清除已完成</button>
          <button v-if="running" class="btn-ghost danger-ghost" @click="stopTranslate">停止</button>
          <button class="btn-primary" :disabled="running || !files.length || !settings.aiModel" @click="startTranslate">
            {{ running ? '翻译中…' : `开始翻译（${files.filter(isRetryable).length}）` }}
          </button>
        </div>
      </div>
      <div class="ai-files">
        <div v-for="f in files" :key="f.path" class="ai-file" :class="f.state">
          <span class="ai-file-state">{{ stateLabel[f.state] }}</span>
          <span class="ai-file-name" :title="f.path">{{ f.name }}</span>
          <span v-if="f.out" class="ai-file-out" :title="f.out">→ {{ f.out.split('/').pop() }}</span>
          <span v-else-if="f.error" class="ai-file-error" :title="f.error">!</span>
        </div>
        <div v-if="!files.length && !scanning" class="ai-empty">
          {{ targetDir ? '该文件夹下没有找到 .lrc / .srt / .vtt 文件' : '请先选择目标文件夹' }}
        </div>
      </div>
      <!-- 没有 Provider 也要能用：导出批次 → 外部工具跑 → 导回校验（AI-16，`05` 第 16 条） -->
      <AiManualPanel :paths="manualPaths" @applied="onManualApplied" />
    </section>
  </div>
</template>

<style scoped>
.ai-view {
  height: 100%;
  overflow-y: auto;
  padding: 22px 26px;
  display: flex;
  flex-direction: column;
  gap: 14px;
}
.ai-head { display: flex; align-items: baseline; gap: 12px; flex-wrap: wrap; }
.ai-title { font-size: 20px; font-weight: 650; }
.ai-sub { font-size: 11.5px; color: var(--text-muted); }

.ai-card {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 14px 18px;
}
.ai-card > .section-title { display: block; margin-bottom: 10px; }

.ai-grid {
  display: grid;
  grid-template-columns: 1.4fr 1fr 1.2fr;
  gap: 12px;
}
.ai-field {
  display: flex;
  flex-direction: column;
  gap: 4px;
  font-size: 11.5px;
  color: var(--text-muted);
  min-width: 0;
}
.ai-field input {
  padding: 7px 10px;
  border-radius: var(--radius-sm);
  border: 1px solid var(--border);
  background: var(--bg-input);
  font-size: 12px;
}
.ai-field input:focus { border-color: rgb(var(--accent-rgb)); }
.ai-test-row {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-top: 12px;
}
/* 档位选择：与测试连接行分开，它是「这批怎么发」的参数，不是连接参数 */
.ai-strategy {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
  margin-top: 12px;
  font-size: 11.5px;
  color: var(--text-muted);
}
.ai-seg { display: flex; gap: 6px; }
.ai-seg .btn-ghost { padding: 5px 12px; font-size: 11.5px; }
.ai-seg .btn-ghost.on {
  border-color: rgb(var(--accent-rgb));
  color: rgb(var(--accent-rgb));
  background: var(--bg-selected);
}
.ai-merge { display: flex; align-items: center; gap: 6px; }
.ai-merge input {
  width: 52px;
  padding: 5px 8px;
  border-radius: var(--radius-sm);
  border: 1px solid var(--border);
  background: var(--bg-input);
  font-size: 12px;
  color: var(--text);
}
.ai-strategy-hint { font-size: 11px; color: var(--text-sub); }
.ai-test-result {
  font-size: 11.5px;
  color: var(--text-muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.ai-test-result.ok { color: var(--success); }

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

.ai-files-card { display: flex; flex-direction: column; min-height: 0; }
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
.ai-files {
  margin-top: 10px;
  display: flex;
  flex-direction: column;
  gap: 4px;
  max-height: 40vh;
  overflow-y: auto;
}
.ai-file {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 7px 10px;
  border-radius: var(--radius-sm);
  font-size: 12px;
}
.ai-file:nth-child(odd) { background: var(--bg-hover); }
.ai-file.done .ai-file-state { color: var(--success); }
.ai-file.failed { background: color-mix(in srgb, var(--danger) 8%, transparent); }
.ai-file.failed .ai-file-state { color: var(--danger); }
.ai-file.running .ai-file-state { color: rgb(var(--accent-rgb)); }
.ai-file-state { width: 44px; flex-shrink: 0; color: var(--text-muted); }
.ai-file-name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.ai-file-out {
  flex-shrink: 0;
  font-size: 11px;
  color: var(--success);
  max-width: 220px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.ai-file-error {
  width: 16px;
  height: 16px;
  border-radius: 50%;
  background: color-mix(in srgb, var(--danger) 20%, transparent);
  color: var(--danger);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-size: 10px;
  cursor: help;
  flex-shrink: 0;
}
.ai-empty {
  padding: 24px;
  text-align: center;
  font-size: 12px;
  color: var(--text-muted);
}
.danger-ghost:hover { color: var(--danger); border-color: color-mix(in srgb, var(--danger) 50%, transparent); }

.ai-model-row {
  display: flex;
  gap: 6px;
  align-items: center;
}
/* 输入框 + 展开箭头组合（占满剩余宽度） */
.ai-model-combo {
  position: relative;
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: center;
}
.ai-model-combo input {
  flex: 1;
  min-width: 0;
  padding-right: 26px;
}
.ai-model-toggle {
  position: absolute;
  right: 4px;
  top: 50%;
  transform: translateY(-50%);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  border-radius: 4px;
  color: var(--text-muted);
}
.ai-model-toggle:hover { background: var(--bg-hover); color: var(--text); }
.ai-model-btn { flex-shrink: 0; padding: 6px 10px; font-size: 11px; }
.ai-field-error { font-size: 10.5px; color: var(--danger); }

/* ── 模型下拉列表：完整展示、可滚动、可过滤 ── */
.ai-field { position: relative; }
.ai-model-menu {
  position: absolute;
  top: calc(100% + 6px);
  left: 0;
  right: 0;
  z-index: 60;
  border-radius: var(--radius);
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  box-shadow: var(--shadow-pop);
  overflow: hidden;
}
.ai-model-menu-head {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px;
  border-bottom: 1px solid var(--border);
}
.ai-model-search {
  flex: 1;
  min-width: 0;
  padding: 5px 8px !important;
  border-radius: var(--radius-sm);
  border: 1px solid var(--border);
  background: var(--bg-input);
  font-size: 12px;
}
.ai-model-count {
  font-size: 10.5px;
  color: var(--text-muted);
  flex-shrink: 0;
}
/* 固定最大高度并滚动，保证长列表每一项都能看到 */
.ai-model-list {
  max-height: 260px;
  overflow-y: auto;
  padding: 5px;
}
.ai-model-item {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  padding: 7px 9px;
  border-radius: var(--radius-sm);
  font-size: 12px;
  color: var(--text-sub);
  text-align: left;
}
.ai-model-item:hover { background: var(--bg-hover); color: var(--text); }
.ai-model-item.active {
  color: rgb(var(--accent-rgb));
  background: var(--bg-selected);
  font-weight: 500;
}
.ai-model-name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.ai-model-empty {
  padding: 14px 10px;
  text-align: center;
  font-size: 11.5px;
  color: var(--text-muted);
}

.ai-dir-check { flex-shrink: 0; accent-color: rgb(var(--accent-rgb)); }
.ai-dir-arrow { flex-shrink: 0; color: var(--text-muted); font-size: 11px; }
.ai-dir-suggested {
  flex-shrink: 0;
  max-width: 40%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 12px;
  color: rgb(var(--accent-rgb));
}
.ai-file.dir-done .ai-file-state { color: var(--success); }
.ai-file.dir-conflict .ai-file-state { color: var(--warning); }
.ai-file.dir-failed .ai-file-state { color: var(--danger); }
</style>
