<script setup lang="ts">
/**
 * 改名预览的清单：一行一条「当前名 → **应用真会用的那个名字**」。
 *
 * 顺序由后端排（目录在前、文件在后，各自按名字），与**应用的顺序一致**（`04 §7.8`），
 * 所以这里不许再排一次 —— 让人看到的顺序就是将来动的顺序。
 * 同理，箭头右边也不许自己挑：有已核准译名的就显示译名，因为后端那一刻用的就是它（`04 §7.7`）。
 */
import { useUiStore } from '@/stores/ui'
import { api } from '@/bridge/ipc'
import { errText } from '@/contracts/result'
import type { RenamePreviewItem } from '@/contracts/api'
import { CONFLICT_LABEL, REASON_LABEL, VERDICT_LABEL } from './renameLabels'

defineProps<{ items: RenamePreviewItem[] }>()
const emit = defineEmits<{ (e: 'refresh'): void }>()

const ui = useUiStore()

const rowCls = (i: RenamePreviewItem): string => `v-${i.verdict}${i.conflict ? ' bad' : ''}`
const to = (i: RenamePreviewItem): string => i.suggested ?? i.target

/** 「不要这个译名」只写拒绝表，之后重算一次预览就看见它真的不再生效（`04 §7.7` 第 9 条） */
async function reject(i: RenamePreviewItem): Promise<void> {
  if (!i.suggested) return
  const name = i.suggested
  try {
    await api.renameRejectSuggestion(i.path, name)
    ui.toast(`这一条以后不再用「${name}」`)
    emit('refresh')
  } catch (e) {
    ui.toast(errText(e), 'error')
  }
}
</script>

<template>
  <ul class="rn-items">
    <li v-for="i in items" :key="i.path" :class="rowCls(i)">
      <span class="rn-kind">{{ i.kind === 'folder' ? '目录' : '文件' }}</span>
      <span class="rn-verdict">{{ VERDICT_LABEL[i.verdict] }}</span>
      <span class="rn-from" :title="i.path">{{ i.name }}</span>
      <span class="rn-arrow">→</span>
      <span class="rn-to" :title="to(i)">{{ to(i) }}</span>
      <span v-if="i.reasons.length" class="rn-reasons">{{ i.reasons.map((r) => REASON_LABEL[r]).join('、') }}</span>
      <span v-if="i.suggested" class="rn-named">已核准译名</span>
      <button
        v-if="i.suggested"
        class="rn-no"
        title="这一条不要这个译名：记进拒绝表，之后不再生成也不会应用"
        @click="reject(i)"
      >
        不要
      </button>
      <span v-else-if="i.needsAi" class="rn-ai">要模型</span>
      <span v-if="i.conflict" class="rn-conflict">{{ CONFLICT_LABEL[i.conflict] }}</span>
    </li>
  </ul>
</template>

<style scoped>
.rn-items {
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 3px;
  max-height: 260px;
  overflow-y: auto;
  font-size: 11.5px;
}
.rn-items li { display: flex; align-items: baseline; gap: 8px; color: var(--text-sub); }
.rn-kind,
.rn-verdict { flex: 0 0 auto; color: var(--text-muted); }
.rn-verdict { min-width: 5.5em; }
.rn-from { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--text-muted); }
.rn-arrow { color: var(--text-muted); }
.rn-to { color: var(--text); word-break: break-all; }
.rn-reasons { color: var(--text-muted); }
.rn-ai { color: rgb(var(--accent-rgb)); }
.rn-named { flex: 0 0 auto; color: var(--success); }
.rn-no {
  flex: 0 0 auto;
  padding: 0 6px;
  border: 1px solid var(--border);
  border-radius: 999px;
  background: none;
  color: var(--text-muted);
  font-size: 10.5px;
  cursor: pointer;
}
.rn-no:hover { color: var(--danger); border-color: var(--danger); }
.rn-conflict,
.rn-items li.bad .rn-to { color: var(--danger); }
.rn-items li.v-skip .rn-verdict { color: var(--text-muted); }
.rn-items li.v-compliant .rn-verdict { color: var(--success); }
.rn-items li.v-needs_repair .rn-verdict,
.rn-items li.v-needs_translation .rn-verdict { color: var(--warning); }
</style>
