<script setup lang="ts">
/** 工具页：音频格式转换 / 视频提取音轨。
 *  后端任务队列（convert_run + convert:progress 事件）；取消真实终止后续任务；
 *  编码参数保持源声道数与采样率（ASMR 双耳录音红线）；同名输出自动加序号。
 */
import { ref, computed, onMounted, onBeforeUnmount } from 'vue'
import { api, onEvent } from '@/bridge/ipc'
import { useUiStore } from '@/stores/ui'
import type { ConvertItem, ConvertProgress, FfmpegInfo } from '@/contracts/api'

const ui = useUiStore()

type TaskState = 'pending' | 'running' | 'done' | 'failed' | 'cancelled'

interface Task {
  id: number
  path: string
  name: string
  kind: 'convert' | 'extract'
  format: string
  outPath: string
  state: TaskState
  error?: string
}

const mode = ref<'convert' | 'extract'>('convert')
const format = ref('mp3')
const tasks = ref<Task[]>([])
const taskId = ref(0) // 后端队列句柄；0 = 未运行
const fileInput = ref<HTMLInputElement | null>(null)
const dropActive = ref(false)
const ffmpeg = ref<FfmpegInfo>({ available: true, path: null, version: null })
const ffmpegChecked = ref(false)

const running = computed(() => taskId.value > 0)

const FORMATS = [
  { value: 'mp3', label: 'MP3' },
  { value: 'aac', label: 'AAC' },
  { value: 'flac', label: 'FLAC' },
  { value: 'wav', label: 'WAV' },
  { value: 'ogg', label: 'OGG' },
  { value: 'm4a', label: 'M4A' },
]

let taskSeq = 0
let unlisten: (() => void) | null = null

const AUDIO_EXTS = new Set(['mp3', 'flac', 'wav', 'ogg', 'm4a', 'aac', 'wma', 'opus', 'ape', 'wv', 'aiff'])
const VIDEO_EXTS = new Set(['mp4', 'mkv', 'avi', 'mov', 'webm', 'flv', 'wmv'])

function extOf(p: string): string {
  const m = p.toLowerCase().match(/\.([a-z0-9]+)$/)
  return m ? m[1] : ''
}

function addPaths(paths: string[]): void {
  let added = 0
  for (const p of paths) {
    const ext = extOf(p)
    if (mode.value === 'extract' && !VIDEO_EXTS.has(ext)) continue
    if (mode.value === 'convert' && !AUDIO_EXTS.has(ext)) continue
    // 去重：同一路径不重复入队
    if (tasks.value.some((t) => t.path === p && t.state === 'pending')) continue
    tasks.value.push({
      id: ++taskSeq,
      path: p,
      name: p.replace(/\\/g, '/').split('/').pop() || p,
      kind: mode.value,
      format: format.value,
      outPath: '',
      state: 'pending',
    })
    added++
  }
  if (!added && paths.length) ui.toast('没有符合当前模式的文件', 'info')
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

/** 输出路径：同目录 + 同名加序号（红线：不静默覆盖）。Rust 侧 file_exists 同步探测。 */
async function uniqueOutputPath(path: string, fmt: string): Promise<string> {
  const p = path.replace(/\\/g, '/')
  const dir = p.slice(0, p.lastIndexOf('/'))
  const base = p.slice(p.lastIndexOf('/') + 1).replace(/\.[^/.]+$/, '')
  let i = 0
  let candidate = `${dir}/${base}.${fmt}`
  while (await api.fileExists(candidate)) {
    i++
    candidate = `${dir}/${base}_${i}.${fmt}`
  }
  return candidate
}

async function runAll(): Promise<void> {
  if (running.value) return
  if (!ffmpeg.value.available) {
    ui.toast('未找到 ffmpeg，无法转换。请安装 ffmpeg 或将 ffmpeg.exe 放到程序目录。', 'error', 5000)
    return
  }
  const pending = tasks.value.filter((t) => t.state === 'pending' || t.state === 'failed')
  if (!pending.length) return
  // 先解析全部输出路径（冲突加序号），再整体提交后端队列
  const items: ConvertItem[] = []
  for (const t of pending) {
    t.outPath = await uniqueOutputPath(t.path, t.format)
    items.push({ id: t.id, path: t.path, outPath: t.outPath, kind: t.kind, format: t.format })
  }
  const id = await api.convertRun(items)
  if (!id) {
    ui.toast('转换队列启动失败', 'error')
    return
  }
  taskId.value = id
}

function cancelAll(): void {
  if (taskId.value) void api.convertCancel(taskId.value)
}

function clearFinished(): void {
  tasks.value = tasks.value.filter((t) => t.state === 'pending' || t.state === 'running')
}

const hasDone = computed(() => tasks.value.some((t) => t.state === 'done' || t.state === 'failed' || t.state === 'cancelled'))
const pendingCount = computed(() => tasks.value.filter((t) => t.state === 'pending' || t.state === 'failed').length)
const doneCount = computed(() => tasks.value.filter((t) => t.state === 'done').length)

onMounted(async () => {
  ffmpeg.value = await api.ffmpegProbe()
  ffmpegChecked.value = true
  unlisten = onEvent<ConvertProgress>('convert:progress', (p) => {
    if (p.state === 'queue-finished') {
      taskId.value = 0
      const ok = tasks.value.filter((t) => t.state === 'done').length
      ui.toast(`转换完成：成功 ${ok}/${tasks.value.length}`, ok === tasks.value.length ? 'success' : 'error', 4000)
      return
    }
    if (p.taskId !== taskId.value) return
    const t = tasks.value.find((x) => x.id === p.itemId)
    if (!t) return
    if (p.state === 'running' || p.state === 'done' || p.state === 'failed' || p.state === 'cancelled') {
      t.state = p.state
      t.error = p.error
    }
  })
})

onBeforeUnmount(() => {
  unlisten?.()
  // 离开页面不自动取消：转换在后台继续，事件仍会写日志
})

const stateLabel: Record<TaskState, string> = {
  pending: '等待',
  running: '转换中',
  done: '完成',
  failed: '失败',
  cancelled: '已取消',
}
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

    <!-- ffmpeg 缺失降级提示 -->
    <div v-if="ffmpegChecked && !ffmpeg.available" class="cv-warning">
      <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2"><path d="M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" /><line x1="12" y1="9" x2="12" y2="13" /><line x1="12" y1="17" x2="12.01" y2="17" /></svg>
      <div>
        <strong>未找到 ffmpeg</strong>，转换功能不可用。播放不受影响。<br />
        <span class="cv-warning-sub">安装 ffmpeg 并加入 PATH，或将 ffmpeg.exe 放到程序目录后重启。</span>
      </div>
    </div>

    <div class="cv-dropzone" @click="onPick">
      <svg viewBox="0 0 24 24" width="34" height="34" fill="none" stroke="currentColor" stroke-width="1.4"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" /><polyline points="7,10 12,15 17,10" /><line x1="12" y1="15" x2="12" y2="3" /></svg>
      <p>拖入{{ mode === 'extract' ? '视频' : '音频' }}文件，或点击选择</p>
      <p class="cv-drop-sub">输出到源目录，同名自动追加序号；保持源声道数与采样率</p>
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
        <button class="btn-ghost" :disabled="!hasDone || running" @click="clearFinished">清除已完成</button>
        <button v-if="running" class="btn-ghost cv-cancel" @click="cancelAll">取消</button>
        <button class="btn-primary" :disabled="!pendingCount || running || !ffmpeg.available" @click="runAll">
          {{ running ? `转换中（${doneCount}/${tasks.length}）` : `开始转换（${pendingCount}）` }}
        </button>
      </div>
    </div>

    <div v-if="tasks.length" class="cv-tasks">
      <div v-for="t in tasks" :key="t.id" class="cv-task" :class="t.state">
        <span class="cv-task-state">{{ stateLabel[t.state] }}</span>
        <span class="cv-task-name" :title="t.path">{{ t.name }}</span>
        <span class="cv-task-out" :title="t.outPath">{{ t.outPath ? '→ ' + (t.outPath.replace(/\\/g, '/').split('/').pop() || '') : '' }}</span>
        <span class="cv-task-format">{{ t.format.toUpperCase() }}</span>
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
.cv-warning {
  display: flex;
  gap: 12px;
  align-items: flex-start;
  padding: 12px 16px;
  border-radius: var(--radius);
  border: 1px solid var(--warning);
  background: color-mix(in srgb, var(--warning) 10%, transparent);
  color: var(--warning);
  font-size: 12px;
  line-height: 1.6;
}
.cv-warning strong { color: var(--text); }
.cv-warning-sub { color: var(--text-sub); font-size: 11px; }
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
.cv-cancel { color: var(--danger); border-color: var(--danger); }
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
.cv-task.done .cv-task-state { color: var(--success); }
.cv-task.failed { border-color: color-mix(in srgb, var(--danger) 40%, transparent); }
.cv-task.failed .cv-task-state { color: var(--danger); }
.cv-task.cancelled .cv-task-state { color: var(--text-muted); }
.cv-task-state { width: 44px; flex-shrink: 0; color: var(--text-muted); }
.cv-task-name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.cv-task-out {
  flex-shrink: 0;
  max-width: 30%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-muted);
  font-size: 11px;
}
.cv-task-format { color: var(--text-muted); flex-shrink: 0; }
.cv-task-error {
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
</style>
