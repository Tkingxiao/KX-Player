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

async function scan(): Promise<void> {
  const dir = props.targetDir || ui.aiFolder
  if (!dir || dirScanning.value) return
  dirScanning.value = true
  try {
    previewResult.value = await api.renamePreview([dir])
  } catch (e) {
    ui.toast(`扫描文件夹失败：${errText(e)}`, 'error')
  } finally {
    dirScanning.value = false
  }
}

async function suggest(): Promise<void> {
  const dir = props.targetDir || ui.aiFolder
  if (!dir || dirSuggesting.value) return
  dirSuggesting.value = true
  try {
    const o = jobOpts()
    const rep = await api.renameSuggestNames([dir], o.baseURL, o.apiKey, o.model, o.modelKind)
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
        <button class="btn-ghost" :disabled="!targetDir || dirScanning" @click="scan">
          {{ dirScanning ? '扫描中…' : `扫描文件夹${previewResult?.items.length ? '（重新）' : ''}` }}
        </button>
        <button
          class="btn-primary"
          :disabled="dirSuggesting || !previewResult?.items.length || !settings.aiModel"
          :title="!settings.aiModel ? '先在左列配置模型服务' : !previewResult?.items.length ? '先扫描文件夹' : ''"
          @click="suggest"
        >{{ dirSuggesting ? '生成译名中…' : '生成译名' }}</button>
        <button
          class="btn-primary"
          :disabled="dirApplying || !applicableCount"
          :title="applicableCount ? '应用改名并记账，整批可回滚' : '没有可应用的改名项'"
          @click="apply"
        >应用改名 ({{ applicableCount }})</button>
      </div>
    </div>
    <AiRenameItems v-if="previewResult?.items.length" :items="previewResult.items" @refresh="scan" />
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
.ai-files-actions { display: flex; align-items: center; gap: 10px; }
.ai-empty {
  padding: 30px;
  text-align: center;
  color: var(--text-muted);
  font-size: 13px;
}
</style>
