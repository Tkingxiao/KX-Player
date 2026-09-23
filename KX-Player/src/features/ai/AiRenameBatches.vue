<script setup lang="ts">
/**
 * 改名批次账（`rename_batches`）：跑过的每一批列在这里，整批可回滚，回滚之后还能重做（`04 §7.8`）。
 *
 * 回滚的依据是**当时落库的映射**，不是重新预览 —— 盘上后来被谁动过，由后端逐条复核并把失败
 * 明细列出来。这里不静默、也不替用户猜「大概已经改回去了」。
 */
import { onMounted, ref, watch } from 'vue'
import { useUiStore } from '@/stores/ui'
import { useLibraryStore } from '@/stores/library'
import { api } from '@/bridge/ipc'
import { errText } from '@/contracts/result'
import confirmHost from '@/services/confirmHost'
import type { RenameBatch, RenameReport } from '@/contracts/api'

/** 父组件每应用一批就 +1，让这张表跟着重读 */
const props = defineProps<{ tick: number }>()

const ui = useUiStore()
const library = useLibraryStore()
const rows = ref<RenameBatch[]>([])
/** 正在跑的那一批（同一时刻只让动一批：两批交错回滚，账就说不清是谁改的了） */
const busyId = ref('')
const loaded = ref(false)
/** 最近一次回滚/重做的失败明细，挂在它所属的那一行下面 */
const detail = ref<{ id: string; rep: RenameReport } | null>(null)

const when = (ms: number): string => new Date(ms).toLocaleString('zh-CN', { hour12: false })

async function load(): Promise<void> {
  loaded.value = false
  try {
    rows.value = await api.renameBatches(20)
    loaded.value = true
  } catch (e) {
    ui.toast(errText(e), 'error')
  }
}

async function reverse(b: RenameBatch, forward: boolean): Promise<void> {
  if (busyId.value) return
  const word = forward ? '重做' : '回滚'
  const ok = await confirmHost.open({
    title: `${word}这一批（${b.itemCount} 项）`,
    message: forward
      ? '把这一批再执行一遍。目标名已经被占用的条目会跳过并逐条报出来。'
      : '把这一批改过的名字搬回原始名。改完之后曲库会自动增量重扫。',
    confirmText: word,
    danger: !forward,
  })
  if (!ok) return
  busyId.value = b.id
  try {
    const rep = forward ? await api.renameRedo(b.id) : await api.renameRollback(b.id)
    detail.value = rep.failed.length ? { id: b.id, rep } : null
    ui.toast(`${word}完成：${rep.applied}/${rep.planned}${rep.failed.length ? ` · 失败 ${rep.failed.length}` : ''}`,
      rep.failed.length ? 'error' : 'success')
    await library.rescan(true)
    await load()
  } catch (e) {
    ui.toast(errText(e), 'error')
  } finally {
    busyId.value = ''
  }
}

onMounted(load)
watch(() => props.tick, load)
</script>

<template>
  <div class="rb">
    <div class="rb-head">
      <h4 class="rb-title">改名批次</h4>
      <button class="btn-ghost" :disabled="busyId !== ''" @click="load">刷新</button>
    </div>
    <p class="rb-hint">
      这一张表读的是库里的账：<b>整批</b>可回滚，回滚过的还能重做。只有本地规则改的条目在账上，
      要问模型的那部分还没上线。
    </p>

    <div v-if="!rows.length" class="rb-empty">{{ loaded ? '还没有应用过改名。' : '读取中…' }}</div>
    <ul v-else class="rb-list">
      <li v-for="b in rows" :key="b.id" class="rb-item">
        <span class="rb-when">{{ when(b.createdAt) }}</span>
        <span class="rb-count">{{ b.itemCount }} 项</span>
        <span v-if="b.rolledBack" class="rb-flag">已回滚</span>
        <span v-else-if="!b.appliedAt" class="rb-flag warn">未跑完</span>
        <span class="rb-note" :title="b.note ?? ''">{{ b.note }}</span>
        <span class="rb-ops">
          <button
            class="btn-ghost"
            :disabled="busyId !== '' || b.rolledBack || !b.itemCount"
            :title="b.rolledBack ? '这一批已经回到改之前了' : '把这一批改过的名字搬回原名'"
            @click="reverse(b, false)"
          >回滚</button>
          <button
            class="btn-ghost"
            :disabled="busyId !== '' || !b.rolledBack"
            title="回滚是可回滚的：把这一批再执行一遍"
            @click="reverse(b, true)"
          >重做</button>
        </span>
        <ul v-if="detail?.id === b.id" class="rb-failed">
          <li v-for="(f, k) in detail.rep.failed" :key="`f-${k}`">{{ f }}</li>
        </ul>
      </li>
    </ul>
  </div>
</template>

<style scoped>
.rb { display: flex; flex-direction: column; gap: 6px; }
.rb-head { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
.rb-title { margin: 0; font-size: 12.5px; color: var(--text); }
.rb-hint { font-size: 11.5px; line-height: 1.65; color: var(--text-muted); }
.rb-empty { font-size: 11.5px; color: var(--text-muted); }

.rb-list { list-style: none; display: flex; flex-direction: column; gap: 3px; font-size: 11.5px; }
.rb-item {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: 8px;
  padding: 3px 0;
  border-top: 1px solid var(--border);
  color: var(--text-sub);
}
.rb-when { color: var(--text-muted); }
.rb-count { min-width: 4.5em; }
.rb-flag { color: var(--text-muted); }
.rb-flag.warn { color: var(--warning); }
.rb-note {
  flex: 1 1 auto;
  min-width: 0;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
  color: var(--text-muted);
}
.rb-ops { display: flex; gap: 6px; margin-left: auto; }
.rb-failed {
  flex: 1 0 100%;
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding-left: 10px;
  color: var(--danger);
  word-break: break-all;
}
</style>
