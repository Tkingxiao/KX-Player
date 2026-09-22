<script setup lang="ts">
/**
 * AI-16 半自动模式：导出批次 → 用户拿任意工具跑 → 导回校验 → 整篇配齐的文件才写盘。
 *
 * 这条路径**完全不碰 Provider**（`05` 第 16 条要求不配 Provider 也得能用）。切块、按序号对齐、
 * 缺项/重复/越界的判定全在 Rust（`ai_export.rs`），这里只负责问用户要文件、把账目摊开给人看。
 */
import { computed, ref, watch } from 'vue'
import { useUiStore } from '@/stores/ui'
import { api } from '@/bridge/ipc'
import { errText, toAppError } from '@/contracts/result'
import type { AiExportResult, AiImportResult } from '@/contracts/api'

const props = defineProps<{ paths: string[] }>()
const emit = defineEmits<{ (e: 'applied', done: { path: string, out: string }[]): void }>()

const ui = useUiStore()
const exporting = ref(false)
const reading = ref(false)
const applying = ref(false)
const exported = ref<AiExportResult | null>(null)
const preview = ref<AiImportResult | null>(null)
/** 合并后的译文文本：写入那一步要用它再跑一次同一个命令，不能靠用户重新选一遍文件 */
let merged = ''

const base = (p: string): string => p.split(/[\\/]/).pop() ?? p

/**
 * 列表一变序号就变（对齐只认序号），旧预览上的数字全部不能再信。
 * 传进来的必须是**整个队列**而不是「待重试的那几个」：队列行改成 done 不该让序号重排。
 */
watch(
  () => props.paths.join('|'),
  () => {
    preview.value = null
    merged = ''
  },
)

const readyCount = computed(() => preview.value?.ready ?? 0)
const canApply = computed(() => readyCount.value > 0 && !reading.value && !applying.value)
const shortFiles = computed(() => (preview.value?.files ?? []).filter((f) => !f.complete))

async function doExport(): Promise<void> {
  if (!props.paths.length || exporting.value) return
  exporting.value = true
  try {
    const r = await api.aiExportPrompts(props.paths)
    exported.value = r
    preview.value = null
    merged = ''
    const skip = r.unexportable ? `，${r.unexportable} 条切不开已跳过` : ''
    ui.toast(`已导出 ${r.exported}/${r.units} 段到 ${base(r.path)}${skip}`, 'success')
  } catch (e) {
    // 保存框被取消不是故障，别报红
    if (toAppError(e).code !== 'CANCELLED') ui.toast(errText(e), 'error')
  } finally {
    exporting.value = false
  }
}

/** 多选：一批可以拆给几台机器跑，几份译文直接拼起来 —— 对齐靠序号，拼起来的顺序无所谓 */
async function doImport(): Promise<void> {
  if (!props.paths.length || reading.value) return
  const picked = await api.openTextFiles()
  if (!picked.length) return
  reading.value = true
  try {
    const parts: string[] = []
    for (const p of picked) {
      const t = await api.readTextFile(p)
      if (t === null) throw new Error(`读不到 ${base(p)}`)
      parts.push(t)
    }
    merged = parts.join('\n')
    preview.value = await api.aiImportTranslations(props.paths, merged, false)
  } catch (e) {
    preview.value = null
    merged = ''
    ui.toast(errText(e), 'error')
  } finally {
    reading.value = false
  }
}

async function doApply(): Promise<void> {
  if (!canApply.value || !merged) return
  applying.value = true
  try {
    // 与预览同一个命令、同一份入参，只是这次让它落盘：报的账和写的东西不可能脱节
    const r = await api.aiImportTranslations(props.paths, merged, true)
    preview.value = r
    const done = r.files
      .filter((f) => f.outPath && r.written.includes(f.outPath))
      .map((f) => ({ path: f.path, out: f.outPath as string }))
    if (done.length) emit('applied', done)
    if (r.written.length) {
      ui.toast(`已写入 ${r.written.length} 个译文文件${r.missing ? `，另有 ${r.missing} 段缺译文` : ''}`, 'success')
    } else {
      ui.toast(r.errors[0] || '没有整篇配齐的文件，一个都没写', 'error')
    }
  } catch (e) {
    ui.toast(errText(e), 'error')
  } finally {
    applying.value = false
  }
}

async function copyPrompt(): Promise<void> {
  if (!exported.value) return
  const ok = await api.clipboardWriteText(exported.value.systemPrompt)
  ui.toast(ok ? '提示词已复制' : '复制失败', ok ? 'success' : 'error')
}
</script>

<template>
  <div class="manual">
    <div class="manual-row">
      <button class="btn-ghost" :disabled="!paths.length || exporting" @click="doExport">
        {{ exporting ? '导出中…' : '导出批次' }}
      </button>
      <button class="btn-ghost" :disabled="!paths.length || reading" @click="doImport">
        {{ reading ? '校验中…' : '导入译文' }}
      </button>
      <button class="btn-primary" :disabled="!canApply" @click="doApply">
        {{ applying ? '写入中…' : `写入 ${readyCount} 篇` }}
      </button>
      <button v-if="exported" class="btn-ghost" @click="copyPrompt">复制提示词</button>
    </div>

    <p class="manual-hint">
      不配 Provider 也能走完：把导出的 <code>prompts.txt</code> 交给任何工具（每段以 <code>### 序号 ###</code> 开头，
      序号下面写译文），跑完再导回来。<b>整篇配上译文的文件才写盘</b>，缺的那几篇把空着的序号再跑一遍、合并导入即可。
    </p>

    <p v-if="exported" class="manual-line">
      已写出 {{ exported.path }} · {{ exported.exported }}/{{ exported.units }} 段 · {{ exported.files }} 个文件
    </p>
    <p v-for="e in exported?.errors ?? []" :key="`x-${e}`" class="manual-warn">{{ e }}</p>

    <template v-if="preview">
      <div class="manual-counts">
        <span>块 {{ preview.blocks }}</span>
        <span>命中 {{ preview.matched }}/{{ preview.units }}</span>
        <span :class="{ bad: preview.missing > 0 }">缺 {{ preview.missing }}</span>
        <span :class="{ bad: preview.duplicates > 0 }">重复 {{ preview.duplicates }}</span>
        <span :class="{ bad: preview.outOfRange > 0 }">越界 {{ preview.outOfRange }}</span>
        <span :class="{ bad: preview.empty > 0 }">空块 {{ preview.empty }}</span>
        <span :class="{ bad: preview.unexportable > 0 }">切不开 {{ preview.unexportable }}</span>
      </div>
      <ul class="manual-files">
        <li v-for="f in preview.files" :key="f.path">
          <span :class="f.complete ? 'ok' : 'bad'">{{ f.complete ? '齐' : '缺' }}</span>
          <span class="manual-name" :title="f.path">{{ f.name }}</span>
          <span class="tnum">{{ f.matched }}/{{ f.total }}</span>
          <span v-if="f.outPath" class="manual-out">→ {{ base(f.outPath) }}</span>
        </li>
      </ul>
      <ul v-if="preview.samples.length" class="manual-samples">
        <li v-for="s in preview.samples" :key="s.seq">
          <span class="tnum">#{{ s.seq }}</span>
          <span class="manual-from">{{ s.from }}</span>
          <span class="manual-arrow">→</span>
          <span class="manual-to">{{ s.to }}</span>
        </li>
      </ul>
      <p v-if="shortFiles.length" class="manual-warn">{{ shortFiles.length }} 个文件没配齐，这一次不动它们。</p>
      <p v-for="e in preview.errors" :key="`p-${e}`" class="manual-warn">{{ e }}</p>
    </template>
  </div>
</template>

<style scoped>
.manual {
  display: flex;
  flex-direction: column;
  gap: 7px;
  margin-top: 10px;
  padding-top: 10px;
  border-top: 1px dashed var(--border);
}
.manual-row { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; }
.manual-hint { font-size: 11.5px; line-height: 1.65; color: var(--text-muted); }
.manual-hint code { font-size: 11px; color: var(--text-sub); }
.manual-line { font-size: 11.5px; color: var(--text-sub); word-break: break-all; }
.manual-warn { font-size: 11.5px; color: var(--warning); word-break: break-all; }

.manual-counts { display: flex; flex-wrap: wrap; gap: 6px 14px; font-size: 11.5px; color: var(--text-sub); }
.manual-counts .bad { color: var(--danger); }

.manual-files,
.manual-samples {
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 3px;
  max-height: 168px;
  overflow-y: auto;
  font-size: 11.5px;
  color: var(--text-sub);
}
.manual-files li,
.manual-samples li { display: flex; align-items: baseline; gap: 8px; }
.manual-files .ok { color: var(--success); }
.manual-files .bad { color: var(--danger); }
.manual-name { flex: 0 1 auto; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.manual-out { color: var(--text-muted); }

.manual-samples li { gap: 6px; }
.manual-from { color: var(--text-muted); text-decoration: line-through; }
.manual-arrow { color: var(--text-muted); }
.manual-to { color: var(--text); }
</style>
