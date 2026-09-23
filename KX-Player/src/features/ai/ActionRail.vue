<!--
  工序轨道（设计包第 4 期，DESIGN.md「AI 工作台」§3.1）：五段横轨，一次只有一段进行中。
  · 状态色只取既有 role：进行中 accent、完成 success、失败 danger、跳过 text-muted；
  · 进行中的段带 aria-current="step"（DESIGN.md §7）；
  · 整条 role=list，非交互不占用 Tab 序；失败段可点（父组件给 onPhaseClick 时）。
  纯展示组件：阶段状态由父组件推导，这里不发状态。
-->
<script setup lang="ts">
export type PhaseState = 'pending' | 'active' | 'done' | 'skipped' | 'failed'
export interface Phase {
  key: string
  label: string
  state: PhaseState
  /** 段上可附一个短注（如「32 个文件」「失败 2」），只读文本 */
  note?: string
}

defineProps<{ phases: Phase[] }>()
</script>

<template>
  <ol class="rail" role="list" aria-label="工序">
    <li
      v-for="(p, i) in phases"
      :key="p.key"
      class="rail-step"
      :class="p.state"
      :aria-current="p.state === 'active' ? 'step' : undefined"
    >
      <span class="rail-dot" aria-hidden="true">{{ p.state === 'done' ? '✓' : i + 1 }}</span>
      <span class="rail-label">{{ p.label }}</span>
      <span v-if="p.note" class="rail-note tnum">{{ p.note }}</span>
      <span v-if="i < phases.length - 1" class="rail-line" aria-hidden="true" />
    </li>
  </ol>
</template>

<style scoped>
.rail {
  list-style: none;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 0;
  flex-wrap: wrap;
}
.rail-step {
  position: relative;
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 4px 10px;
  border-radius: 999px;
  font-size: 11.5px;
  color: var(--text-muted);
}
.rail-step + .rail-step::before {
  content: none;
}
.rail-dot {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  border-radius: 50%;
  border: 1px solid var(--border-strong);
  font-size: 10px;
  flex-shrink: 0;
}
.rail-step.done { color: var(--text-sub); }
.rail-step.done .rail-dot { border-color: var(--success); color: var(--success); }
.rail-step.active {
  color: rgb(var(--accent-rgb));
  background: var(--bg-selected);
}
.rail-step.active .rail-dot { border-color: rgb(var(--accent-rgb)); }
.rail-step.failed { color: var(--danger); }
.rail-step.failed .rail-dot { border-color: var(--danger); }
.rail-step.skipped .rail-dot { border-style: dashed; }
.rail-note { font-size: 10.5px; color: var(--text-muted); }
.rail-step.active .rail-note { color: rgb(var(--accent-rgb) / 0.8); }
.rail-line {
  display: none;
}
</style>
