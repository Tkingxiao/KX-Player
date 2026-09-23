<!--
  进度带（设计包第 4 期，DESIGN.md「AI 工作台」§4）：
  字段顺序锁死：已完成/总数 · 条目每分 · 当前项(截断) · 失败数。
  · 条目每分 = 已完成数 ÷ 已跑秒数 × 60，前端现算（不依赖新契约）；
  · 缓存命中率/平均耗时**不显示**：AiProgress 没有字段，不许假数据填位（设计包 §5.1）；
  · 仅进行中渲染，静止时整个组件不存在（不留骨架位、不做装饰）；
  · 左缘 accent 光泽 = 页面级签名（DESIGN.md §8），prefers-reduced-motion 降静态。
-->
<script setup lang="ts">
import { computed } from 'vue'

const props = defineProps<{
  done: number
  total: number
  /** 本次已跑秒数（父组件计时；0 时条目每分显示 —） */
  elapsedSec: number
  currentItem?: string
  failed: number
}>()

const emit = defineEmits<{ (e: 'open-failures'): void }>()

const perMinute = computed(() => {
  if (props.elapsedSec <= 0 || props.done <= 0) return null
  return Math.max(1, Math.round((props.done / props.elapsedSec) * 60))
})

const percent = computed(() => (props.total > 0 ? Math.round((props.done / props.total) * 100) : 0))
</script>

<template>
  <div class="pband" role="status">
    <span class="pband-sheen" aria-hidden="true" />
    <div class="pband-track" aria-hidden="true">
      <div class="pband-fill" :style="{ width: percent + '%' }" />
    </div>
    <div class="pband-metrics">
      <span class="tnum">{{ done }}/{{ total }}</span>
      <span class="pband-sep" aria-hidden="true">·</span>
      <span class="tnum">{{ perMinute === null ? '—' : perMinute + ' 条每分' }}</span>
      <span class="pband-sep" aria-hidden="true">·</span>
      <span v-if="currentItem" class="pband-current" :title="currentItem">{{ currentItem }}</span>
      <span v-else class="pband-current pband-idle">—</span>
      <span class="pband-sep" aria-hidden="true">·</span>
      <button
        v-if="failed > 0"
        class="pband-failed tnum"
        title="查看失败明细"
        @click="emit('open-failures')"
      >失败 {{ failed }}</button>
      <span v-else class="tnum pband-nofail">失败 0</span>
    </div>
  </div>
</template>

<style scoped>
.pband {
  position: relative;
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 10px 14px;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  /* 工作台面板已是 --bg-card：进度带抬一档到 elevated 才分得清层级（浅色主题下同理） */
  background: var(--bg-elevated);
  overflow: hidden;
}
/* 页面级签名：左缘 accent 光泽。只用 opacity 动画；reduced-motion 下降为静态 */
.pband-sheen {
  position: absolute;
  left: 0;
  top: 0;
  bottom: 0;
  width: 2px;
  background: rgb(var(--accent-rgb) / 0.85);
  animation: sheen-breathe 1.6s var(--ease) infinite;
}
@keyframes sheen-breathe {
  0%, 100% { opacity: 0.45; }
  50% { opacity: 1; }
}
@media (prefers-reduced-motion: reduce) {
  .pband-sheen { animation: none; opacity: 0.85; }
}

.pband-track {
  height: 3px;
  border-radius: 999px;
  background: var(--bg-input);
  overflow: hidden;
}
.pband-fill {
  height: 100%;
  border-radius: 999px;
  background: rgb(var(--accent-rgb) / 0.75);
  transition: width var(--dur) var(--ease);
}

.pband-metrics {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 11.5px;
  color: var(--text-sub);
  min-width: 0;
}
.pband-sep { color: var(--text-muted); }
.pband-current {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-muted);
}
.pband-idle { flex: 0 0 auto; }
.pband-failed {
  padding: 1px 10px;
  border-radius: 999px;
  border: 1px solid color-mix(in srgb, var(--danger) 45%, transparent);
  color: var(--danger);
  font-size: 11px;
}
.pband-failed:hover { background: color-mix(in srgb, var(--danger) 10%, transparent); }
.pband-nofail { color: var(--text-muted); }
</style>
