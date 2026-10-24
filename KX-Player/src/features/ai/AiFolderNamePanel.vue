<!--
  文件夹名翻译工种（全面对齐改名主干与译名账本）：
  通过 rename_preview 预检 → rename_suggest_names 生成译名 → rename_apply 事务改名与批次记账。
-->
<script setup lang="ts">
import { ref, watch, computed } from 'vue'
import { useUiStore } from '@/stores/ui'
import { useSettingsStore } from '@/stores/settings'
import { useLibraryStore } from '@/stores/library'
import { api } from '@/bridge/ipc'
import { errText } from '@/contracts/result'
import { jobOpts } from './useAiAccess'
import { isApplicable } from './renameLabels'
import AiRenameItems from './AiRenameItems.vue'
import type { RenamePreview } from '@/contracts/api'

const props = defineProps<{ targetDir: string }>()

const ui = useUiStore()
const settings = useSettingsStore()
const library = useLibraryStore()

const previewResult = ref<RenamePreview | null>(null)
const dirScanning = ref(false)
const dirSuggesting = ref(false)
const dirApplying = ref(false)

const emit = defineEmits<{ (e: 'count', n: number): void }>()
watch(previewResult, (v) => emit('count', v?.items.length ?? 0))

const applicableCount = computed(() => previewResult.value?.items.filter(isApplicable).length ?? 0)

// ── 选择性生成 ──
// 勾哪些条目就只给哪些条目问模型（rename_suggest_names 的 onlyPaths）；全翻按钮保持原样。
// 选中状态跟着这份预览走：重扫（条目换了一批）就清空，避免拿旧路径去命中新列表。
const selected = ref<Set<string>>(new Set())
const allSelected = computed(
  () => !!previewResult.value?.items.length && previewResult.value.items.every((i) => selected.value.has(i.path)),
)

function toggleOne(path: string): void {
  const next = new Set(selected.value)
  if (next.has(path)) next.delete(path)
  else next.add(path)
  selected.value = next
}

function toggleAll(): void {
  const items = previewResult.value?.items ?? []
  selected.value = allSelected.value ? new Set() : new Set(items.map((i) => i.path))
}

async function scan(): Promise<void> {
  const dir = props.targetDir || ui.aiFolder
  if (!dir || dirScanning.value) return
  dirScanning.value = true
  try {
    previewResult.value = await api.renamePreview([dir])
    selected.value = new Set()
  } catch (e) {
    ui.toast(`扫描文件夹失败：${errText(e)}`, 'error')
  } finally {
    dirScanning.value = false
  }
}

async function suggest(onlySelected = false): Promise<void> {
  const dir = props.targetDir || ui.aiFolder
  if (!dir || dirSuggesting.value) return
  // 只给选中的生成时，选中的条目路径作为过滤范围传下去（plan 精确按路径命中）
  if (onlySelected && !selected.value.size) return
  const only = onlySelected ? [...selected.value] : null
  dirSuggesting.value = true
  try {
    const o = jobOpts()
    const rep = await api.renameSuggestNames([dir], o.baseURL, o.apiKey, o.model, o.modelKind, o.apiStyle, only)
    ui.toast(`生成完成：新生成 ${rep.generated}，复用 ${rep.cached}，没拿到 ${rep.failed.length + rep.review.length}`)
    previewResult.value = await api.renamePreview([dir])
  } catch (e) {
    ui.toast(errText(e), 'error')
  } finally {
    dirSuggesting.value = false
  }
}

async function apply(): Promise<void> {
  const dir = props.targetDir || ui.aiFolder
  if (!dir || dirApplying.value || !applicableCount.value) return
  dirApplying.value = true
  try {
    const rep = await api.renameApply([dir])
    ui.toast(`改名完成：实际应用 ${rep.applied}，跳过 ${rep.skipped}`)
    await library.rescan(true)
    previewResult.value = await api.renamePreview([dir])
    // 改名之后条目路径全变了，选中集里留着的都是旧路径
    selected.value = new Set()
  } catch (e) {
    ui.toast(errText(e), 'error')
  } finally {
    dirApplying.value = false
  }
}
</script>

<template>
  <div class="dir-pane">
    <div class="ai-files-head">
      <h3 class="pane-title section-title">文件夹名翻译 · 主干批次保障，确认后才重命名</h3>
      <div class="ai-files-actions">
        <label
          v-if="previewResult?.items.length"
          class="dir-select-all"
          title="全选 / 取消全选本列表（生成译名只问选中的条目）"
        >
          <input type="checkbox" :checked="allSelected" @change="toggleAll" />
          <span>全选</span>
        </label>
        <button class="btn-ghost" :disabled="!targetDir || dirScanning" @click="scan">
          {{ dirScanning ? '扫描中…' : `扫描文件夹${previewResult?.items.length ? '（重新）' : ''}` }}
        </button>
        <button
          v-if="selected.size"
          class="btn-primary"
          :disabled="dirSuggesting || !settings.aiModel"
          :title="!settings.aiModel ? '先在左列配置模型服务' : `只为选中的 ${selected.size} 个条目问模型`"
          @click="suggest(true)"
        >{{ dirSuggesting ? '生成译名中…' : `生成译名（选中 ${selected.size}）` }}</button>
        <button
          class="btn-primary"
          :disabled="dirSuggesting || !previewResult?.items.length || !settings.aiModel"
          :title="!settings.aiModel ? '先在左列配置模型服务' : !previewResult?.items.length ? '先扫描文件夹' : ''"
          @click="suggest(false)"
        >{{ dirSuggesting ? '生成译名中…' : '生成译名（全部）' }}</button>
        <button
          class="btn-primary"
          :disabled="dirApplying || !applicableCount"
          :title="applicableCount ? '应用改名并记账，整批可回滚' : '没有可应用的改名项'"
          @click="apply"
        >应用改名 ({{ applicableCount }})</button>
      </div>
    </div>
    <AiRenameItems
      v-if="previewResult?.items.length"
      :items="previewResult.items"
      selectable
      :selected="selected"
      @toggle-one="toggleOne"
      @refresh="scan"
    />
    <div v-else class="ai-empty">
      {{ targetDir ? '点「扫描文件夹」列出子目录，再用 AI 生成简体译名' : '请先在左列选择目标文件夹' }}
    </div>
  </div>
</template>

<style scoped>
.dir-pane { display: flex; flex-direction: column; gap: 10px; min-width: 0; }
.ai-files-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
}
.pane-title { margin-bottom: 0 !important; }
.ai-files-actions { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
.dir-select-all {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-size: 11.5px;
  color: var(--text-sub);
  user-select: none;
}
.dir-select-all input { accent-color: rgb(var(--accent-rgb)); }
.ai-empty {
  padding: 30px;
  text-align: center;
  color: var(--text-muted);
  font-size: 13px;
}
</style>
