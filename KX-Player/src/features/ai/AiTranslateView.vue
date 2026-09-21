<script setup lang="ts">
/** AI 翻译独立界面：Provider 配置 + 字幕/歌词批量翻译队列（.lrc/.srt/.vtt → .zh.*） */
import { ref, computed, onMounted, onBeforeUnmount } from 'vue'
import { useUiStore } from '@/stores/ui'
import { useSettingsStore } from '@/stores/settings'
import { useLibraryStore } from '@/stores/library'
import { api } from '@/bridge/ipc'
import { scanSubtitleFiles, translateFile, translateSegments, type SubtitleFile } from '@/services/aiTranslate'
import { normDir } from '@/utils/format'
import { errText } from '@/contracts/result'

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

const files = ref<(SubtitleFile & { state: 'pending' | 'running' | 'done' | 'failed' | 'skipped'; error?: string; out?: string })[]>([])
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

let cancelFlag = false

async function startTranslate(): Promise<void> {
  if (running.value) return
  const pending = files.value.filter((f) => f.state === 'pending' || f.state === 'failed')
  if (!pending.length) return
  running.value = true
  cancelFlag = false
  totalCount.value = pending.length
  doneCount.value = 0
  for (const f of pending) {
    if (cancelFlag) { f.state = 'skipped'; continue }
    f.state = 'running'
    try {
      const r = await translateFile(f, {
        baseURL: settings.aiBaseURL,
        apiKey: apiKey.value,
        model: settings.aiModel,
        onProgress: () => { /* 块级进度可扩展 */ },
      })
      f.state = 'done'
      f.out = r.outPath
    } catch (e) {
      f.state = 'failed'
      f.error = errText(e)
    }
    doneCount.value++
  }
  running.value = false
  const ok = files.value.filter((f) => f.state === 'done').length
  ui.toast(`AI 翻译完成：成功 ${ok}/${files.value.length}`, ok ? 'success' : 'error')
}

function stopTranslate(): void {
  cancelFlag = true
}

function clearFinished(): void {
  files.value = files.value.filter((f) => f.state === 'pending' || f.state === 'running')
  totalCount.value = files.value.length
  doneCount.value = 0
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

async function scanDirs(dir: string, depth: number): Promise<{ path: string; name: string }[]> {
  const out: { path: string; name: string }[] = []
  async function walk(d: string, dep: number): Promise<void> {
    if (dep > 3 || out.length >= 300) return
    let entries: import('@/contracts/api').DirEntry[] = []
    try { entries = await api.listDir(d) } catch { return }
    for (const entry of entries) {
      if (typeof entry === 'string') continue
      if (!entry.isDirectory || !entry.name) continue
      const p = d + '/' + entry.name
      out.push({ path: p, name: entry.name })
      await walk(p, dep + 1)
      if (out.length >= 300) return
    }
  }
  await walk(dir, depth)
  return out
}

async function scanDirNames(): Promise<void> {
  if (!ui.aiFolder || dirScanning.value) return
  dirScanning.value = true
  try {
    const dirs = await scanDirs(ui.aiFolder, 1)
    dirItems.value = dirs.map((d) => ({
      path: d.path, name: d.name, suggested: '', newPath: '',
      state: 'pending' as const, enabled: true,
    }))
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
    const names = dirItems.value.map((d) => d.name)
    const translated = await translateSegments(names, {
      baseURL: settings.aiBaseURL,
      apiKey: apiKey.value,
      model: settings.aiModel,
      chunkSize: 10,
      concurrency: 2,
    })
    dirItems.value = dirItems.value.map((d, i) => {
      const suggested = sanitizeName(translated[i] ?? d.name) || d.name
      const parent = d.path.slice(0, d.path.lastIndexOf('/'))
      return { ...d, suggested, newPath: `${parent}/${suggested}` }
    })
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
onBeforeUnmount(() => { cancelFlag = true })
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

    <!-- 文件队列 -->
    <section class="ai-card ai-files-card">
      <div class="ai-files-head">
        <h3 class="section-title">翻译队列 · {{ files.length }} 个文件</h3>
        <div class="ai-files-actions">
          <span v-if="running" class="tnum ai-progress">{{ doneCount }}/{{ totalCount }}</span>
          <button class="btn-ghost" :disabled="!files.length" @click="clearFinished">清除已完成</button>
          <button v-if="running" class="btn-ghost danger-ghost" @click="stopTranslate">停止</button>
          <button class="btn-primary" :disabled="running || !files.length || !settings.aiModel" @click="startTranslate">
            {{ running ? '翻译中…' : `开始翻译（${files.filter((f) => f.state === 'pending' || f.state === 'failed').length}）` }}
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
.ai-file.failed { background: rgba(230, 58, 46, 0.08); }
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
  background: rgba(230, 58, 46, 0.2);
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
