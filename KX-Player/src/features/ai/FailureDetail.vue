<!--
  失败明细（设计包第 3 期，DESIGN.md「AI 工作台」§5）：
  时间 / 原因原文（不截断、可复制）/ 只重试这一项。
  原因原文透传 Rust，不翻译不截断 —— 翻错了比看原文更误事。
-->
<script setup lang="ts">
import { computed, ref } from 'vue'

const props = defineProps<{
  id?: string
  path: string
  name: string
  error?: string
}>()

const emit = defineEmits<{ (e: 'retry'): void }>()

const copied = ref(false)
const when = computed(() => new Date().toLocaleString('zh-CN', { hour12: false }))

async function copyReason(): Promise<void> {
  if (!props.error) return
  try {
    await navigator.clipboard.writeText(props.error)
    copied.value = true
    setTimeout(() => (copied.value = false), 1500)
  } catch {
    /* 剪贴板不可用就静默：面板内原因原文始终可读可手选 */
  }
}
</script>

<template>
  <div :id="id" class="fail-detail" role="region" :aria-label="'失败原因：' + name">
    <div class="fd-grid">
      <span class="fd-label">时间</span>
      <span class="fd-value tnum">{{ when }}</span>

      <span class="fd-label">文件</span>
      <span class="fd-value" :title="path">{{ path }}</span>

      <span class="fd-label">原因</span>
      <span class="fd-value fd-error">{{ error || '没有返回具体原因' }}</span>
    </div>
    <div class="fd-ops">
      <button v-if="error" class="fd-btn" @click="copyReason">{{ copied ? '已复制' : '复制原因' }}</button>
      <button class="fd-btn fd-retry" @click="emit('retry')">只重试这一项</button>
    </div>
  </div>
</template>

<style scoped>
.fail-detail {
  grid-column: 1 / -1;
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin: 4px 0 8px;
  padding: 10px 12px;
  border: 1px solid color-mix(in srgb, var(--danger) 30%, transparent);
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--danger) 6%, transparent);
}
.fd-grid {
  display: grid;
  grid-template-columns: 40px minmax(0, 1fr);
  gap: 4px 10px;
  font-size: 11.5px;
}
.fd-label { color: var(--text-muted); }
.fd-value { color: var(--text-sub); word-break: break-all; }
.fd-error { color: var(--text); }
.fd-ops { display: flex; gap: 8px; }
.fd-btn {
  padding: 3px 12px;
  border: 1px solid var(--border);
  border-radius: 999px;
  font-size: 11px;
  color: var(--text-sub);
}
.fd-btn:hover { color: var(--text); border-color: var(--border-strong); }
.fd-retry { color: rgb(var(--accent-rgb)); border-color: color-mix(in srgb, var(--accent-rgb) 40%, transparent); }
</style>
