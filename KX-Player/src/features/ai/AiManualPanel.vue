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
import ActionRail from './ActionRail.vue'
import ManualAudit from './ManualAudit.vue'
import type { AiExportResult, AiImportResult } from '@/contracts/api'

const props = defineProps<{ paths: string[] }>()
const emit = defineEmits<{ (e: 'applied', done: { path: string, out: string }[]): void }>()

const ui = useUiStore()
const exporting = ref(false)
const reading = ref(false)
const applying = ref(false)
const exported = ref<AiExportResult | null>(null)
const preview = ref<AiImportResult | null>(null)
/**
 * 导出那一刻的队列快照：批次的序号是按它编的（ai_export.rs）。
 * 之后队列一动（清除/重扫/重试），这份批次就对不上号，必须明示作废，
 * 而不是让导入报「越界 N 段」让用户猜（audit B5）。
 */
const exportedPaths = ref<string[]>([])
const stale = computed(
  () => exported.value !== null && exportedPaths.value.join('|') !== props.paths.join('|'),
)
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
const canApply = computed(() => readyCount.value > 0 && !stale.value && !reading.value && !applying.value)

async function doExport(): Promise<void> {
  if (!props.paths.length || exporting.value) return
  exporting.value = true
  try {
    const r = await api.aiExportPrompts(props.paths)
    exported.value = r
    exportedPaths.value = [...props.paths]
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

type StepState = 'pending' | 'active' | 'done' | 'skipped'
/** 三步条状态（第 5 期）：导出 → 导入 → 写入。stale 时不亮任何一步（批次已作废） */
const steps = computed(() => {
  const done0 = exported.value !== null && !stale.value
  const done1 = preview.value !== null && !stale.value
  const s1: StepState = stale.value ? 'skipped' : done0 ? 'done' : 'pending'
  const s2: StepState = stale.value ? 'skipped' : done1 ? 'done' : done0 ? 'active' : 'pending'
  const s3: StepState = applying.value ? 'active' : done1 && readyCount.value > 0 ? 'active' : 'pending'
  return [
    { key: 'export', label: '导出批次', state: s1, note: done0 ? `${exported.value?.exported ?? 0} 段` : '' },
    { key: 'import', label: '导入译文', state: s2, note: done1 ? `${preview.value?.matched ?? 0} 命中` : '' },
    { key: 'apply', label: '写入', state: s3, note: done1 ? `${readyCount.value} 篇可写` : '' },
  ]
})

async function copyPrompt(): Promise<void> {
  if (!exported.value) return
  const ok = await api.clipboardWriteText(exported.value.systemPrompt)
  ui.toast(ok ? '提示词已复制' : '复制失败', ok ? 'success' : 'error')
}
</script>

<template>
  <div class="manual">
    <!-- 三步条复用工作台的工序轨道（DESIGN.md「AI 工作台」§3.1）：不再自己画一份同款胶囊 -->
    <ActionRail :phases="steps" />

    <div class="manual-row">
      <button class="btn-ghost" :disabled="!paths.length || exporting" @click="doExport">
        {{ exporting ? '导出中…' : '导出批次' }}
      </button>
      <button
        class="btn-ghost"
        :disabled="!paths.length || reading || stale"
        :title="stale ? '批次已作废，先重新导出' : ''"
        @click="doImport"
      >
        {{ reading ? '校验中…' : '导入译文' }}
      </button>
      <button class="btn-primary" :disabled="!canApply" :title="stale ? '批次已作废，先重新导出' : ''" @click="doApply">
        {{ applying ? '写入中…' : `写入 ${readyCount} 篇` }}
      </button>
      <button v-if="exported && !stale" class="btn-ghost" @click="copyPrompt">复制提示词</button>
    </div>

    <details class="manual-how">
      <summary>怎么用</summary>
      <p class="manual-hint">
        把导出的 <code>prompts.txt</code> 交给任何工具（每段以 <code>### 序号 ###</code> 开头，序号下面写译文），
        跑完再导回来。整篇配上译文的文件才写盘；缺的那几篇把空着的序号再跑一遍、合并导入即可。
      </p>
    </details>

    <p v-if="stale" class="manual-stale">
      队列已变动（清除或重新扫描），这份批次的序号对不上现在的列表了。
      <button class="btn-ghost manual-stale-btn" @click="doExport">重新导出批次</button>
    </p>
    <p v-if="exported && !stale" class="manual-line">
      已写出 {{ exported.path }} · {{ exported.exported }}/{{ exported.units }} 段 · {{ exported.files }} 个文件
    </p>
    <p v-for="e in exported?.errors ?? []" :key="`x-${e}`" class="manual-warn">{{ e }}</p>

    <ManualAudit v-if="preview" :preview="preview" />
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
.manual-stale {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
  padding: 6px 10px;
  border: 1px solid color-mix(in srgb, var(--warning) 45%, transparent);
  border-radius: var(--radius-sm);
  font-size: 11.5px;
  color: var(--warning);
}
.manual-stale-btn { padding: 2px 10px; font-size: 11px; }

.manual-how { font-size: 11.5px; color: var(--text-muted); }
.manual-how summary {
  cursor: pointer;
  width: fit-content;
  text-decoration: underline;
  text-underline-offset: 3px;
  color: var(--text-sub);
}
.manual-how[open] summary { color: rgb(var(--accent-rgb)); }
</style>