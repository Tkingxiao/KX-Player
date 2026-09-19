<script setup lang="ts">
/** 工具页：音频格式转换 / 视频提取音轨（ffmpeg 批处理，逐文件状态与总进度）。 */
import { ref, computed, onMounted } from 'vue'
import { api } from '@/bridge/ipc'
import { useUiStore } from '@/stores/ui'
import { useLibraryStore } from '@/stores/library'
import { fmtFSize } from '@/utils/format'
import type { Track } from '@/contracts/api'

const ui = useUiStore()
const library = useLibraryStore()

type TaskKind = 'convert' | 'extract'
type TaskState = 'pending' | 'running' | 'done' | 'failed' | 'cancelled'

interface Task {
  id: number
  path: string
  name: string
  kind: TaskKind
  format: string
  state: TaskState
  error?: string
}

const mode = ref<TaskKind>('convert')
const format = ref('mp3')
const tasks = ref<Task[]>([])
const running = ref(false)
const fileInput = ref<HTMLInputElement | null>(null)
const dropActive = ref(false)

const FORMATS = [
  { value: 'mp3', label: 'MP3' },
  { value: 'aac', label: 'AAC' },
  { value: 'flac', label: 'FLAC' },
  { value: 'wav', label: 'WAV' },
  { value: 'ogg', label: 'OGG' },
  { value: 'm4a', label: 'M4A' },
]

let taskSeq = 0

const AUDIO_EXTS = new Set(['mp3', 'flac', 'wav', 'ogg', 'm4a', 'aac', 'wma', 'opus', 'ape', 'wv', 'aiff'])
const VIDEO_EXTS = new Set(['mp4', 'mkv', 'avi', 'mov', 'webm', 'flv', 'wmv'])

function extOf(p: string): string {
  const m = p.toLowerCase().match(/\.([a-z0-9]+)$/)
  return m ? m[1] : ''
}

function addPaths(paths: string[]): void {
  for (const p of paths) {
    const ext = extOf(p)
    const kind: TaskKind = VIDEO_EXTS.has(ext) ? 'extract' : 'convert'
    if (mode.value === 'extract' && !VIDEO_EXTS.has(ext)) continue
    if (mode.value === 'convert' && !AUDIO_EXTS.has(ext)) continue
    tasks.value.push({
      id: ++taskSeq,
      path: p,
      name: p.replace(/\\/g, '/').split('/').pop() || p,
      kind,
      format: format.value,
      state: 'pending',
    })
  }
}

function onPick(): void {
  fileInput.value?.click()
}

async function onFiles(e: Event): Promise<void> {
  const input = e.target as HTMLInputElement
  if (input.files) addPaths([...input.files].map((f) => (f as File & { path: string }).path).filter(Boolean))
  input.value = ''
}

function onDrop(e: DragEvent): void {
  dropActive.value = false
  const files = e.dataTransfer?.files
  if (files) addPaths([...files].map((f) => (f as File & { path: string }).path).filter(Boolean))
}

function outputName(path: string, fmt: string): string {
  const p = path.replace(/\\/g, '/')
  const dir = p.slice(0, p.lastIndexOf('/'))
  const base = p.slice(p.lastIndexOf('/') + 1).replace(/\.[^/.]+$/, '')
  const sep = path.includes('\\') ? '\\' : '/'
  // 冲突加序号，不静默覆盖
  let candidate = `${dir}${sep}${base}.${fmt}`
  return candidate
}

async function runAll(): Promise<void> {
  if (running.value || !tasks.value.length) return
  running.value = true
  const pending = tasks.value.filter((t) => t.state === 'pending' || t.state === 'failed')
  let done = 0
  for (const t of pending) {
    if (!running.value) { t.state = 'cancelled'; continue }
    t.state = 'running'
    const out = outputName(t.path, t.format)
    const args = t.kind === 'extract'
      ? ['-y', '-i', t.path, '-vn', out]
      : ['-y', '-i', t.path, out]
    try {
      const result = await api.ffmpegExec(args)
      if (result.code === 0) {
        t.state = 'done'
      } else {
        t.state = 'failed'
        t.error = (result.stderr || 'ffmpeg 失败').split('\n').slice(-3).join('\n')
      }
    } catch (err) {
      t.state = 'failed'
      t.error = String(err)
    }
    done++
    ui.toast(`转换进度 ${done}/${pending.length}`, 'info', 10)
  }
  running.value = false
  const ok = tasks.value.filter((t) => t.state === 'done').length
  ui.toast(`转换完成：成功 ${ok}/${tasks.value.length}`, ok === tasks.value.length ? 'success' : 'error')
}

function clearFinished(): void {
  tasks.value = tasks.value.filter((t) => t.state === 'pending' || t.state === 'running')
}

const hasDone = computed(() => tasks.value.some((t) => t.state === 'done' || t.state === 'failed'))

// 从曲库添加
function addFromLibrary(): void {
  const picker = document.createElement('input')
  picker.type = 'file'
  picker.multiple = true
  picker.accept = mode.value === 'extract' ? 'video/*' : 'audio/*'
  picker.onchange = () => {
    if (picker.files) addPaths([...picker.files].map((f) => (f as File & { path: string }).path).filter(Boolean))
  }
  picker.click()
}

onMounted(() => {
  void library
  void ui
})

const stateLabel = {
  pending: '等待',
  running: '转换中',
  done: '完成',
  failed: '失败',
  cancelled: '已取消',
} as Record<TaskState, string>
</script>

<template>
  <div
    class="convert-view"
    :class="{ drop: dropActive }"
    @dragover.prevent="dropActive = true"
    @dragleave="dropActive = false"
    @drop.prevent="onDrop"
  >
    <div class="cv-head">
      <h1 class="cv-title">格式转换</h1>
      <div class="cv-modes">
        <button class="cv-mode" :class="{ active: mode === 'convert' }" @click="mode = 'convert'">音频格式转换</button>
        <button class="cv-mode" :class="{ active: mode === 'extract' }" @click="mode = 'extract'">视频提取音轨</button>
      </div>
    </div>

    <div class="cv-dropzone" @click="onPick">
      <svg viewBox="0 0 24 24" width="34" height="34" fill="none" stroke="currentColor" stroke-width="1.4"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" /><polyline points="7,10 12,15 17,10" /><line x1="12" y1="15" x2="12" y2="3" /></svg>
      <p>拖入{{ mode === 'extract' ? '视频' : '音频' }}文件，或点击选择</p>
      <p class="cv-drop-sub">输出到源目录，同名文件自动追加序号，不会覆盖</p>
      <input ref="fileInput" type="file" multiple hidden @change="onFiles" />
    </div>

    <div class="cv-format-row">
      <span class="section-title">输出格式</span>
      <div class="cv-formats">
        <button
          v-for="f in FORMATS"
          :key="f.value"
          class="cv-format"
          :class="{ active: format === f.value }"
          @click="format = f.value"
        >{{ f.label }}</button>
      </div>
      <div class="cv-actions">
        <button class="btn-ghost" :disabled="!hasDone" @click="clearFinished">清除已完成</button>
        <button class="btn-primary" :disabled="!tasks.length || running" @click="runAll">
          {{ running ? '转换中…' : `开始转换（${tasks.filter((t) => t.state === 'pending').length}）` }}
        </button>
      </div>
    </div>

    <div v-if="tasks.length" class="cv-tasks">
      <div v-for="t in tasks" :key="t.id" class="cv-task" :class="t.state">
        <span class="cv-task-state">{{ stateLabel[t.state] }}</span>
        <span class="cv-task-name" :title="t.path">{{ t.name }}</span>
        <span class="cv-task-format">→ {{ t.format.toUpperCase() }}</span>
        <span v-if="t.error" class="cv-task-error" :title="t.error">!</span>
      </div>
    </div>
  </div>
</template>

<style scoped>
.convert-view {
  height: 100%;
  overflow-y: auto;
  padding: 24px 28px;
  display: flex;
  flex-direction: column;
  gap: 16px;
}
.convert-view.drop {
  outline: 2px dashed rgb(var(--accent-rgb));
  outline-offset: -8px;
}
.cv-head {
  display: flex;
  align-items: center;
  gap: 18px;
}
.cv-title { font-size: 20px; font-weight: 650; }
.cv-modes { display: flex; gap: 6px; }
.cv-mode {
  padding: 6px 14px;
  border-radius: 999px;
  border: 1px solid var(--border);
  font-size: 12px;
  color: var(--text-sub);
}
.cv-mode.active {
  border-color: rgb(var(--accent-rgb));
  color: rgb(var(--accent-rgb));
  background: var(--bg-selected);
}
.cv-dropzone {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  padding: 44px 20px;
  border: 1.5px dashed var(--border-strong);
  border-radius: var(--radius-lg);
  color: var(--text-muted);
  cursor: pointer;
  transition: border-color var(--dur-fast) var(--ease), background var(--dur-fast) var(--ease);
}
.cv-dropzone:hover {
  border-color: rgb(var(--accent-rgb));
  background: var(--bg-hover);
}
.cv-drop-sub { font-size: 11px; opacity: 0.7; }
.cv-format-row {
  display: flex;
  align-items: center;
  gap: 14px;
  flex-wrap: wrap;
}
.cv-formats { display: flex; gap: 6px; }
.cv-format {
  padding: 5px 12px;
  border-radius: 999px;
  border: 1px solid var(--border);
  font-size: 12px;
  color: var(--text-sub);
}
.cv-format.active {
  border-color: rgb(var(--accent-rgb));
  color: rgb(var(--accent-rgb));
  background: var(--bg-selected);
}
.cv-actions {
  margin-left: auto;
  display: flex;
  gap: 10px;
}
.cv-tasks {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.cv-task {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 9px 14px;
  border-radius: var(--radius);
  background: var(--bg-card);
  border: 1px solid var(--border);
  font-size: 12px;
}
.cv-task.running { border-color: rgb(var(--accent-rgb)); }
.cv-task.done .cv-task-state { color: #40c878; }
.cv-task.failed { border-color: rgba(230, 58, 46, 0.4); }
.cv-task.failed .cv-task-state { color: #e6685f; }
.cv-task-state { width: 44px; flex-shrink: 0; color: var(--text-muted); }
.cv-task-name {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.cv-task-format { color: var(--text-muted); flex-shrink: 0; }
.cv-task-error {
  width: 16px;
  height: 16px;
  border-radius: 50%;
  background: rgba(230, 58, 46, 0.2);
  color: #e6685f;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-size: 10px;
  cursor: help;
  flex-shrink: 0;
}
</style>
