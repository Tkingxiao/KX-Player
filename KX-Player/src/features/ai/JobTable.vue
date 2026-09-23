<!--
  队列表（设计包第 3 期，DESIGN.md「AI 工作台」§5）：替换旧斑马纹列表。
  · 行状态 = 文字标签 + 语义色，不再只用颜色表意（audit C5/D4）；
  · 失败行可展开失败明细（时间/原因原文/只重试这一项），不再用裸「!」徽标（audit B6）；
  · 表头 sticky，行高 var(--row-h)，样式对齐 TrackTable 的表范式。
  纯展示组件：行状态由父组件的队列快照驱动，这里只发事件不改状态。
-->
<script setup lang="ts">
import { ref } from 'vue'
import FailureDetail from './FailureDetail.vue'

export interface JobRow {
  path: string
  name: string
  state: 'pending' | 'running' | 'done' | 'failed' | 'skipped' | 'exempt'
  error?: string
  out?: string
}

const props = defineProps<{
  rows: JobRow[]
  /** 表头后面的说明（如「该文件夹下没有找到 .lrc / .srt / .vtt 文件」走空态，不走这里） */
  emptyText?: string
}>()

const emit = defineEmits<{ (e: 'retry-one', path: string): void; (e: 're-enable-one', path: string): void }>()

const STATE_LABEL: Record<JobRow['state'], string> = {
  pending: '等待',
  running: '翻译中',
  done: '完成',
  failed: '失败',
  skipped: '已跳过',
  exempt: '已翻译',
}

/** 展开的失败明细（同一时刻至多一行：长队列里多行展开会互相顶开布局） */
const expanded = ref('')

function toggle(row: JobRow): void {
  expanded.value = expanded.value === row.path ? '' : row.path
}

function retryOne(row: JobRow): void {
  emit('retry-one', row.path)
  expanded.value = ''
}

const base = (p: string): string => p.split(/[\\/]/).pop() ?? p
</script>

<template>
  <div class="job-table" role="table" aria-label="翻译队列">
    <div class="jt-head" role="row">
      <span class="jt-col jt-state" role="columnheader">状态</span>
      <span class="jt-col jt-name" role="columnheader">文件</span>
      <span class="jt-col jt-out" role="columnheader">译文文件</span>
      <span class="jt-col jt-op" role="columnheader"><span class="visually-hidden">操作</span></span>
    </div>

    <div
      v-for="f in rows"
      :key="f.path"
      class="jt-row"
      :class="f.state"
      role="row"
    >
      <div class="jt-cell jt-state" role="cell">
        <span class="jt-state-text">{{ STATE_LABEL[f.state] }}</span>
      </div>
      <div class="jt-cell jt-name" role="cell">
        <button
          v-if="f.state === 'failed'"
          class="jt-fail-toggle"
          :aria-expanded="expanded === f.path"
          :aria-controls="'fd-' + base(f.path)"
          :title="expanded === f.path ? '收起失败原因' : '展开失败原因'"
          @click="toggle(f)"
        >{{ f.name }}<span class="jt-caret" aria-hidden="true">{{ expanded === f.path ? '▾' : '▸' }}</span></button>
        <span v-else class="jt-name-text" :title="f.path">{{ f.name }}</span>
      </div>
      <div class="jt-cell jt-out" role="cell">
        <span v-if="f.out" class="jt-out-text" :title="f.out">→ {{ base(f.out) }}</span>
      </div>
      <div class="jt-cell jt-op" role="cell">
        <button
          v-if="f.state === 'failed' && expanded !== f.path"
          class="jt-retry"
          title="只重试这一项"
          @click="retryOne(f)"
        >重试</button>
        <button
          v-else-if="f.state === 'exempt'"
          class="jt-retry"
          title="该文件已有 .zh.* 译文（默认跳过），点它重新翻译"
          @click="emit('re-enable-one', f.path)"
        >重译</button>
      </div>

      <FailureDetail
        v-if="f.state === 'failed' && expanded === f.path"
        :id="'fd-' + base(f.path)"
        :path="f.path"
        :name="f.name"
        :error="f.error"
        @retry="retryOne(f)"
      />
    </div>

    <div v-if="!rows.length && emptyText" class="jt-empty">{{ emptyText }}</div>
  </div>
</template>

<style scoped>
.job-table { display: flex; flex-direction: column; min-width: 0; }
.jt-head,
.jt-row {
  display: grid;
  grid-template-columns: 72px minmax(0, 1fr) minmax(0, 280px) 64px;
  gap: 12px;
  align-items: center;
  padding: 0 10px;
}
.jt-head {
  height: 32px;
  font-size: 11px;
  color: var(--text-muted);
  border-bottom: 1px solid var(--border);
  user-select: none;
  position: sticky;
  top: 0;
  background: var(--bg-titlebar);
  z-index: 1;
}
.jt-row {
  min-height: var(--row-h);
  border-radius: var(--radius-sm);
  transition: background var(--dur-fast) var(--ease);
}
.jt-row:hover { background: var(--bg-hover); }
.jt-cell { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 12.5px; }

.jt-state-text { font-size: 11.5px; color: var(--text-muted); }
.jt-row.done .jt-state-text { color: var(--success); }
.jt-row.failed .jt-state-text { color: var(--danger); }
.jt-row.running .jt-state-text { color: rgb(var(--accent-rgb)); }
/* 已翻译（豁免，扫描默认跳过）：与 done 同色但更收敛，提示「这轮不用重翻」 */
.jt-row.exempt .jt-state-text { color: var(--success); opacity: 0.75; }
/* 已跳过与等待同为 muted，但加上删除线以外的区分：跳过用更暗的字 + 倾斜不做，保持克制 */

.jt-name-text { display: block; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.jt-fail-toggle {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 12.5px;
  color: var(--danger);
  text-align: left;
}
.jt-fail-toggle:hover { text-decoration: underline; text-underline-offset: 3px; }
.jt-caret { flex-shrink: 0; font-size: 10px; }

.jt-out-text { font-size: 11.5px; color: var(--success); }
.jt-retry {
  padding: 3px 10px;
  border: 1px solid var(--border);
  border-radius: 999px;
  font-size: 11px;
  color: var(--text-sub);
}
.jt-retry:hover { color: var(--text); border-color: var(--border-strong); }

.jt-empty { padding: 24px; text-align: center; font-size: 12px; color: var(--text-muted); }

.visually-hidden {
  position: absolute;
  width: 1px;
  height: 1px;
  overflow: hidden;
  clip: rect(0 0 0 0);
}
</style>
