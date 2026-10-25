<!--
  译文管理（AI 工作台第四工种）：管理已经翻译好的歌词文件。
  数据 = 翻译账（ai_translate_cache）+ 文件系统实时核对，状态由 Rust 分好（四种），
  这里只做呈现与处置：重译（带 force 直接发任务、就地看进度）、打开译文位置、
  删除译文（连账）、清账（留文件）、清理失效账。已翻译文件不再占翻译队列的位置，
  它们的家在这里。
-->
<script setup lang="ts">
import { ref, computed, watch, onMounted, onBeforeUnmount } from 'vue'
import { useUiStore } from '@/stores/ui'
import { api } from '@/bridge/ipc'
import { errText } from '@/contracts/result'
import { translateSubtitleFiles, type AiTranslateJob } from '@/services/aiTranslate'
import { jobOpts } from './useAiAccess'
import {
  STATUS_LABEL, isTodo, filterEntries, summarize, retranslatable, fmtTranslatedAt,
  type TranslatedFilter,
} from './translated'
import type { AiTranslatedEntry } from '@/contracts/api'

const props = defineProps<{ active: boolean }>()
const emit = defineEmits<{ (e: 'count', n: number): void }>()

const ui = useUiStore()

const entries = ref<AiTranslatedEntry[]>([])
const loading = ref(false)
const query = ref('')
const filter = ref<TranslatedFilter>('all')
const selected = ref<Set<string>>(new Set())
const cleanupBusy = ref(false)
/** 危险操作的二次确认：第一次点变成「确认？」，3 秒不点就还原 */
const armed = ref('')

const summary = computed(() => summarize(entries.value))
const visible = computed(() => filterEntries(entries.value, query.value, filter.value))
const allSelected = computed(() => visible.value.length > 0 && visible.value.every((r) => selected.value.has(r.sourcePath)))
const selectedRows = computed(() => entries.value.filter((r) => selected.value.has(r.sourcePath)))

// ── 重新翻译：直接带 force 发任务，进度就地显示 ──
const run = ref<{ total: number; done: number; failed: number; current: string } | null>(null)
const job = ref<AiTranslateJob | null>(null)

async function load(): Promise<void> {
  loading.value = true
  try {
    entries.value = await api.aiTranslatedList()
    selected.value = new Set()
    emit('count', entries.value.length)
  } catch (e) {
    ui.toast(`读取翻译账失败：${errText(e)}`, 'error')
  } finally {
    loading.value = false
  }
}

watch(
  () => props.active,
  (v) => { if (v) void load() },
)
onMounted(() => { if (props.active) void load() })
onBeforeUnmount(() => job.value?.stop())

function arm(key: string, action: () => void): void {
  if (armed.value === key) {
    armed.value = ''
    action()
    return
  }
  armed.value = key
  setTimeout(() => { if (armed.value === key) armed.value = '' }, 3000)
}

function toggleSelect(path: string): void {
  const next = new Set(selected.value)
  if (next.has(path)) next.delete(path)
  else next.add(path)
  selected.value = next
}

function toggleSelectAll(): void {
  selected.value = allSelected.value ? new Set() : new Set(visible.value.map((r) => r.sourcePath))
}

async function retranslate(rows: readonly AiTranslatedEntry[]): Promise<void> {
  if (run.value) return
  const paths = retranslatable(rows)
  if (!paths.length) {
    ui.toast('选中的条目源文件都不在了，没法重译', 'error')
    return
  }
  run.value = { total: paths.length, done: 0, failed: 0, current: '' }
  job.value = translateSubtitleFiles({ ...jobOpts(), paths, forcePaths: paths }, (p) => {
    const r = run.value
    if (!r) return
    if (p.state === 'file-start') r.current = p.path
    if (p.state === 'file-done') { r.done++; r.current = '' }
    if (p.state === 'file-failed') { r.done++; r.failed++ }
  })
  try {
    await job.value?.finished
  } catch (e) {
    ui.toast(errText(e), 'error')
  } finally {
    job.value = null
    const r = run.value
    run.value = null
    await load()
    if (r) ui.toast(r.failed ? `重译完成：${r.done - r.failed} 成功，${r.failed} 失败` : `重译完成：${r.done} 个文件`, r.failed ? 'error' : 'success')
  }
}

async function removeTranslated(rows: readonly AiTranslatedEntry[]): Promise<void> {
  const paths = rows.map((r) => r.sourcePath)
  if (!paths.length) return
  try {
    const report = await api.aiTranslatedRemove(paths, true)
    if (report.errors.length) ui.toast(report.errors[0], 'error')
    else ui.toast(`已删除 ${report.removed} 个译文文件`)
    await load()
  } catch (e) {
    ui.toast(errText(e), 'error')
  }
}

async function clearLedger(rows: readonly AiTranslatedEntry[]): Promise<void> {
  const paths = rows.map((r) => r.sourcePath)
  if (!paths.length) return
  try {
    const report = await api.aiTranslatedRemove(paths, false)
    if (report.errors.length) ui.toast(report.errors[0], 'error')
    else ui.toast(`已清除 ${report.removed} 条翻译账`)
    await load()
  } catch (e) {
    ui.toast(errText(e), 'error')
  }
}

/** 删除原文：只对「源还在」的选中条目生效；译文与账都保留，条目转为「源已删」。
 *  真删的是用户的原始文件 —— 按钮只在有选中且其中还有源文件时解锁，另有二次确认。 */
const sourceDeletable = computed(() => selectedRows.value.filter((r) => r.status !== 'source_gone'))

async function removeSource(): Promise<void> {
  const paths = sourceDeletable.value.map((r) => r.sourcePath)
  if (!paths.length) return
  try {
    const report = await api.aiTranslatedRemoveSource(paths)
    if (report.errors.length) ui.toast(report.errors[0], 'error')
    else ui.toast(`已删除 ${report.removed} 个原文文件（译文与记录保留）`)
    await load()
  } catch (e) {
    ui.toast(errText(e), 'error')
  }
}

async function cleanupGone(): Promise<void> {
  cleanupBusy.value = true
  try {
    const n = await api.aiTranslatedCleanup()
    ui.toast(n ? `已清理 ${n} 条失效账` : '没有失效账可清理')
    await load()
  } catch (e) {
    ui.toast(errText(e), 'error')
  } finally {
    cleanupBusy.value = false
  }
}

async function openOut(entry: AiTranslatedEntry): Promise<void> {
  // 打不开要让人看见：false = 资源管理器没起来，reject = 命令层报错——都提示，不再点了没反应
  try {
    if (!(await api.showItemInFolder(entry.outPath))) {
      ui.toast(`资源管理器打不开这份译文：${entry.outPath}`, 'error')
    }
  } catch (e) {
    ui.toast(errText(e), 'error')
  }
}
</script>

<template>
  <section class="tr-panel" aria-label="译文管理">
    <!-- 头部：统计 + 筛选 + 搜索 + 全局操作 -->
    <div class="tr-head">
      <span class="tr-stat tnum">已翻译 {{ summary.total }}</span>
      <span v-if="summary.todo" class="tr-stat tnum tr-stat-todo">待处理 {{ summary.todo }}</span>
      <div class="tr-seg" role="group" aria-label="状态筛选">
        <button
          v-for="f in ([['all', '全部'], ['todo', '待处理'], ['ok', '正常']] as const)"
          :key="f[0]"
          class="btn-ghost tr-seg-btn"
          :class="{ on: filter === f[0] }"
          @click="filter = f[0]"
        >{{ f[1] }}</button>
      </div>
      <input v-model="query" class="tr-search" type="search" placeholder="搜索文件名…" aria-label="搜索已翻译文件" />
      <button class="btn-ghost" :disabled="loading" @click="load">{{ loading ? '刷新中…' : '刷新' }}</button>
      <button class="btn-ghost" :disabled="cleanupBusy || loading" title="删除「源和译文都不在了」的纯垃圾账" @click="cleanupGone">
        清理失效账
      </button>
    </div>

    <!-- 重译进度带 -->
    <div v-if="run" class="tr-run" role="status">
      <span class="tnum">{{ run.done }}/{{ run.total }}</span>
      <div class="tr-run-track" aria-hidden="true"><div class="tr-run-fill" :style="{ width: (run.total ? (run.done / run.total) * 100 : 0) + '%' }" /></div>
      <span class="tr-run-current" :title="run.current">{{ run.current || '…' }}</span>
      <button class="btn-ghost danger-ghost" @click="job?.stop(); ui.toast('已停止，正在飞的块会跑完', 'info')">停止</button>
    </div>

    <!-- 批量操作条 -->
    <div v-if="selected.size" class="tr-batch">
      <span class="tnum">已选 {{ selected.size }}</span>
      <button class="btn-ghost" :disabled="!!run" title="对源还在的选中条目强制重译" @click="retranslate(selectedRows)">重新翻译</button>
      <button
        class="btn-ghost danger-ghost"
        :disabled="!!run"
        @click="arm('batch-remove', () => removeTranslated(selectedRows))"
      >{{ armed === 'batch-remove' ? '确认删除？' : '删除译文' }}</button>
      <button
        class="btn-ghost"
        :disabled="!!run"
        @click="arm('batch-clear', () => clearLedger(selectedRows))"
      >{{ armed === 'batch-clear' ? '确认清账？' : '清账（留文件）' }}</button>
      <button
        class="btn-ghost danger-ghost"
        :disabled="!!run || !sourceDeletable.length"
        :title="sourceDeletable.length ? `删除 ${sourceDeletable.length} 个选中条目的原文文件（译文与记录保留）` : '选中的条目源文件都不在了'"
        @click="arm('batch-remove-source', () => removeSource())"
      >{{ armed === 'batch-remove-source' ? '确认删除原文？' : '删除原文' }}</button>
      <button class="btn-ghost" @click="selected = new Set()">取消选择</button>
    </div>

    <!-- 列表 -->
    <div class="tr-table" role="table" aria-label="已翻译文件">
      <div class="tr-row tr-head-row" role="row">
        <input
          class="tr-check"
          type="checkbox"
          :checked="allSelected"
          :disabled="!visible.length"
          aria-label="全选本页"
          @change="toggleSelectAll"
        />
        <span role="columnheader">状态</span>
        <span role="columnheader">文件</span>
        <span role="columnheader">翻译于</span>
        <span class="visually-hidden" role="columnheader">操作</span>
      </div>
      <div v-for="r in visible" :key="r.sourcePath" class="tr-row" :class="'st-' + r.status" role="row">
        <input
          class="tr-check"
          type="checkbox"
          :checked="selected.has(r.sourcePath)"
          :aria-label="`选择 ${r.sourceName}`"
          @change="toggleSelect(r.sourcePath)"
        />
        <span class="tr-status" :title="STATUS_LABEL[r.status]">{{ STATUS_LABEL[r.status] }}</span>
        <span class="tr-name" :title="r.sourcePath">
          <span class="tr-name-text">{{ r.sourceName }}</span>
          <span v-if="r.outExists" class="tr-out" :title="r.outPath">→ {{ r.outPath.split(/[\\/]/).pop() }}</span>
          <span v-else class="tr-out tr-out-gone">译文文件已不在</span>
        </span>
        <span class="tr-time tnum">{{ fmtTranslatedAt(r.translatedAt) }}</span>
        <span class="tr-ops">
          <button v-if="r.status === 'changed'" class="tr-op" :disabled="!!run" @click="retranslate([r])">重译</button>
          <button v-if="r.status === 'out_missing'" class="tr-op" :disabled="!!run" @click="retranslate([r])">重译</button>
          <button v-if="r.outExists" class="tr-op" @click="openOut(r)">打开译文</button>
          <button
            v-if="r.outExists"
            class="tr-op tr-danger"
            @click="arm('rm-' + r.sourcePath, () => removeTranslated([r]))"
          >{{ armed === 'rm-' + r.sourcePath ? '确认？' : '删除译文' }}</button>
          <button
            class="tr-op"
            @click="arm('cl-' + r.sourcePath, () => clearLedger([r]))"
          >{{ armed === 'cl-' + r.sourcePath ? '确认？' : '清账' }}</button>
        </span>
      </div>
      <div v-if="!visible.length" class="tr-empty">
        {{ entries.length ? '没有匹配的条目' : '还没有翻译记录：去「字幕翻译」跑一批，或点「刷新」' }}
      </div>
    </div>
    <p class="tr-note">清账只删除翻译记录、保留译文文件；删除译文会连 .zh 文件一起删；删除原文只删源文件本身（须精确命中翻译账），译文与记录保留、条目转为「源已删」。重译会把新译文写回同一位置并更新这里的记录。</p>
  </section>
</template>

<style scoped>
.tr-panel { display: flex; flex-direction: column; gap: 10px; min-width: 0; }
.tr-head { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
.tr-stat { font-size: 12px; color: var(--text-sub); }
.tr-stat-todo { color: var(--danger); }

.tr-seg { display: flex; gap: 4px; }
.tr-seg-btn { padding: 4px 10px; font-size: 11.5px; }
.tr-seg-btn.on { border-color: rgb(var(--accent-rgb)); color: rgb(var(--accent-rgb)); background: var(--bg-selected); }
.tr-search {
  flex: 0 1 200px;
  min-width: 120px;
  padding: 5px 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-input);
  font-size: 12px;
}
.tr-search:focus { border-color: rgb(var(--accent-rgb)); }

.tr-run {
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: 11.5px;
  color: var(--text-sub);
}
.tr-run-track { flex: 1; height: 3px; border-radius: 999px; background: var(--bg-input); overflow: hidden; }
.tr-run-fill { height: 100%; background: rgb(var(--accent-rgb) / 0.75); transition: width var(--dur) var(--ease); }
.tr-run-current { flex: 0 1 240px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--text-muted); }
.danger-ghost:hover { color: var(--danger); border-color: color-mix(in srgb, var(--danger) 50%, transparent); }

.tr-batch {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 10px;
  border: 1px solid rgb(var(--accent-rgb) / 0.4);
  border-radius: var(--radius-sm);
  background: var(--bg-selected);
  font-size: 11.5px;
  color: var(--text-sub);
}

.tr-table { display: flex; flex-direction: column; min-width: 0; }
.tr-row {
  display: grid;
  grid-template-columns: 28px 72px minmax(0, 1fr) 96px auto;
  gap: 10px;
  align-items: center;
  padding: 0 8px;
  min-height: 34px;
  border-radius: var(--radius-sm);
}
.tr-row:hover { background: var(--bg-hover); }
.tr-head-row {
  min-height: 30px;
  font-size: 11px;
  color: var(--text-muted);
  border-bottom: 1px solid var(--border);
  user-select: none;
  position: sticky;
  top: 0;
  background: var(--bg-card);
  z-index: 1;
}
.tr-check { accent-color: rgb(var(--accent-rgb)); }
.tr-status { font-size: 11px; color: var(--text-muted); }
.tr-row.st-changed .tr-status { color: var(--danger); }
.tr-row.st-out_missing .tr-status { color: var(--danger); }
.tr-row.st-source_gone .tr-status { color: var(--text-muted); font-style: italic; }
.tr-name { min-width: 0; display: flex; align-items: baseline; gap: 8px; font-size: 12.5px; color: var(--text); }
.tr-name-text { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.tr-out {
  flex-shrink: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 11px;
  color: var(--success);
}
.tr-out-gone { color: var(--text-muted); }
.tr-time { font-size: 11px; color: var(--text-muted); }
.tr-ops { display: flex; gap: 4px; justify-content: flex-end; }
.tr-op {
  padding: 2px 8px;
  border: 1px solid var(--border);
  border-radius: 999px;
  font-size: 10.5px;
  color: var(--text-sub);
  white-space: nowrap;
}
.tr-op:hover { color: var(--text); border-color: var(--border-strong); }
.tr-op.tr-danger:hover { color: var(--danger); border-color: color-mix(in srgb, var(--danger) 50%, transparent); }

.tr-empty { padding: 32px; text-align: center; font-size: 12px; color: var(--text-muted); }
.tr-note { font-size: 10.5px; color: var(--text-sub); line-height: 1.6; margin: 0; }

.visually-hidden {
  position: absolute;
  width: 1px;
  height: 1px;
  overflow: hidden;
  clip: rect(0 0 0 0);
}
</style>
