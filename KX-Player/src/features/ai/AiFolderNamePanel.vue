<!--
  文件夹名翻译工种（AI-15 复用账 + AI-17 的「AI 生成译名」）：
  扫描子目录 → 生成简体译名 → 用户勾选后才重命名。**深层优先**（先改子目录，父目录路径才仍然有效）。
  译名与「哪个目录已译过」都记账（`ai_translate_cache`，kind='folder'），重新扫描时直接复用、不再烧 token。
-->
<script setup lang="ts">
import { ref, watch } from 'vue'
import { useUiStore } from '@/stores/ui'
import { useSettingsStore } from '@/stores/settings'
import { useLibraryStore } from '@/stores/library'
import { api } from '@/bridge/ipc'
import { errText } from '@/contracts/result'
import { scanSubDirectories, translateTexts } from '@/services/aiTranslate'
import { jobOpts } from './useAiAccess'

defineProps<{ targetDir: string }>()

const ui = useUiStore()
const settings = useSettingsStore()
const library = useLibraryStore()

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

/** 条目数报给父级的分段控件角标（父级不再自己持有一份扫描状态） */
const emit = defineEmits<{ (e: 'count', n: number): void }>()
watch(dirItems, (v) => emit('count', v.length))

async function scan(): Promise<void> {
  if (!ui.aiFolder || dirScanning.value) return
  dirScanning.value = true
  try {
    // 复用已译名（AI-15）：账里记过这个目录 → 直接填译名并标为已译，不再烧 token
    const suggestions = await api.aiCacheFolderSuggestions()
    const dirs = await scanSubDirectories(ui.aiFolder)
    dirItems.value = dirs.map((d) => {
      const cached = suggestions[d.path]
      if (cached) {
        const parent = d.path.slice(0, d.path.lastIndexOf('/'))
        return { path: d.path, name: d.name, suggested: cached, newPath: `${parent}/${cached}`, state: 'done', enabled: false, error: undefined }
      }
      return { path: d.path, name: d.name, suggested: '', newPath: '', state: 'pending', enabled: true, error: undefined }
    })
  } catch (e) {
    ui.toast(`扫描文件夹失败：${errText(e)}`, 'error')
  } finally {
    dirScanning.value = false
  }
}

function sanitizeName(name: string): string {
  return name.replace(/[\\/:*?"<>|]/g, ' ').replace(/\s+/g, ' ').trim()
}

async function suggest(): Promise<void> {
  if (!dirItems.value.length || dirSuggesting.value) return
  dirSuggesting.value = true
  try {
    // 只翻还没译过的：已复用账里的译名（state='done'）或已有未应用的译名（suggested 非空）都不再烧
    const need = dirItems.value.filter((d) => d.state === 'pending' && !d.suggested)
    if (!need.length) {
      ui.toast('没有需要生成译名的文件夹', 'info')
      return
    }
    // 文件夹名很短，一批 10 条比字幕更划算 —— 显式覆盖档位里的批量大小；
    // 并发不再写死 2：本地模型两路并发只是互相抢内存，交给档位决定。
    const translated = await translateTexts({ ...jobOpts(), texts: need.map((d) => d.name), chunkSize: 10 })
    if (!translated) {
      ui.toast('译名生成已取消', 'error')
      return
    }
    need.forEach((d, i) => {
      const name = sanitizeName(translated[i] ?? d.name) || d.name
      const parent = d.path.slice(0, d.path.lastIndexOf('/'))
      d.suggested = name
      d.newPath = `${parent}/${name}`
    })
  } catch (e) {
    ui.toast(errText(e), 'error')
  } finally {
    dirSuggesting.value = false
  }
}

async function apply(): Promise<void> {
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
    // 记「新路径 → 译名」：重命名后重新扫描，这个目录就能在账里命中并复用，不重复烧 token（AI-15）
    else void api.aiCacheRecordFolder(item.newPath, item.suggested)
    dirDone.value++
  }
  dirApplying.value = false
  const ok = dirItems.value.filter((d) => d.state === 'done').length
  ui.toast(`文件夹重命名完成：成功 ${ok}/${targets.length}`, ok ? 'success' : 'error')
  // 目录名变了，曲库需要重新扫描
  void library.rescan(true)
}

const stateLabel = {
  pending: '待应用',
  done: '完成',
  conflict: '冲突',
  failed: '失败',
  skipped: '跳过',
} as const

const canApply = () => dirItems.value.some((d) => d.enabled && d.state === 'pending' && d.suggested)
</script>

<template>
  <!-- 单根：多根会让组件变成 fragment，过渡/属性继承都会出问题（同 AiTranslateView 的白屏教训） -->
  <div class="dir-pane">
    <div class="ai-files-head">
      <h3 class="pane-title section-title">文件夹名翻译 · AI 生成译名，确认后才重命名</h3>
      <div class="ai-files-actions">
        <span v-if="dirApplying" class="tnum ai-progress">{{ dirDone }}/{{ dirTotal }}</span>
        <button class="btn-ghost" :disabled="!targetDir || dirScanning" @click="scan">
          {{ dirScanning ? '扫描中…' : `扫描文件夹${dirItems.length ? '（重新）' : ''}` }}
        </button>
        <button
          class="btn-primary"
          :disabled="dirSuggesting || !dirItems.length || !settings.aiModel"
          :title="!settings.aiModel ? '先在左列配置模型服务' : !dirItems.length ? '先扫描文件夹' : ''"
          @click="suggest"
        >{{ dirSuggesting ? '生成译名中…' : '生成译名' }}</button>
        <button
          class="btn-primary"
          :disabled="dirApplying || !canApply()"
          title="把带译名的目录改名，深层优先"
          @click="apply"
        >应用改名</button>
      </div>
    </div>
    <div v-if="dirItems.length" class="ai-files">
      <div v-for="d in dirItems" :key="d.path" class="ai-file" :class="'dir-' + d.state">
        <input v-model="d.enabled" type="checkbox" :disabled="d.state !== 'pending'" class="ai-dir-check" aria-label="包含这一项" />
        <span class="ai-file-state">{{ stateLabel[d.state] }}</span>
        <span class="ai-file-name" :title="d.path">{{ d.name }}</span>
        <span class="ai-dir-arrow">→</span>
        <span class="ai-dir-suggested" :title="d.newPath">{{ d.suggested || '（未生成）' }}</span>
        <span v-if="d.error" class="ai-file-error" :title="d.error">!</span>
      </div>
    </div>
    <div v-else class="ai-empty">
      {{ targetDir ? '点「扫描文件夹」列出子目录，再用 AI 生成简体译名' : '请先在左列选择目标文件夹' }}
    </div>
  </div>
</template>

<style scoped>
/* 间距沿用父级 `.wb-work` 的 10px 节奏：套了一层单根之后，这层就是它唯一的 flex 子项 */
.dir-pane { display: flex; flex-direction: column; gap: 10px; min-width: 0; }

/* 头部与父级「翻译队列」同一形状：同样的间距与标题压线（父级的 scoped 规则选不进子组件） */
.ai-files-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
}
.pane-title { margin-bottom: 0 !important; }
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
.ai-file-state { width: 44px; flex-shrink: 0; color: var(--text-muted); }
.ai-file-name {
  flex: 1;
  min-width: 0;
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
.ai-file.dir-done .ai-file-state { color: var(--success); }
.ai-file.dir-conflict .ai-file-state { color: var(--warning); }
.ai-file.dir-failed .ai-file-state { color: var(--danger); }
.ai-dir-check { flex-shrink: 0; accent-color: rgb(var(--accent-rgb)); }
.ai-dir-arrow { flex-shrink: 0; color: var(--text-muted); font-size: 11px; }
.ai-dir-suggested {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-sub);
}
.ai-empty {
  padding: 24px;
  text-align: center;
  font-size: 12px;
  color: var(--text-muted);
}
</style>
