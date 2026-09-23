<!--
  半自动导入的**校验账**（AI-16）：一次导入命中多少段、缺多少、重复/越界/空块各几个，
  再附上前几段「原文 → 译文」样例。纯展示 —— 判定全在 Rust（`ai_export.rs`），
  这里只把账目摊开，不改状态、不发消息（DESIGN.md「AI 工作台」§1「读数不是控件」）。
  非零的异常项才上 warning 色，零项保持灰（audit D8）。
-->
<script setup lang="ts">
import { computed } from 'vue'
import type { AiImportResult } from '@/contracts/api'

const props = defineProps<{ preview: AiImportResult }>()

/** 没配齐整篇译文的文件：这一次不动它们，但要告诉用户有几个 */
const shortCount = computed(() => (props.preview.files ?? []).filter((f) => !f.complete).length)
</script>

<template>
  <div class="audit">
    <div class="audit-counts">
      <span class="cnt">命中 <b class="tnum">{{ preview.matched }}/{{ preview.units }}</b></span>
      <span class="cnt" :class="{ warn: preview.missing > 0 }">缺 <b class="tnum">{{ preview.missing }}</b></span>
      <span class="cnt" :class="{ warn: preview.duplicates > 0 }">重复 <b class="tnum">{{ preview.duplicates }}</b></span>
      <span class="cnt" :class="{ warn: preview.outOfRange > 0 }">越界 <b class="tnum">{{ preview.outOfRange }}</b></span>
      <span class="cnt" :class="{ warn: preview.empty > 0 }">空块 <b class="tnum">{{ preview.empty }}</b></span>
      <span class="cnt" :class="{ warn: preview.unexportable > 0 }">切不开 <b class="tnum">{{ preview.unexportable }}</b></span>
    </div>
    <ul v-if="preview.samples.length" class="audit-samples">
      <li v-for="s in preview.samples" :key="s.seq">
        <span class="tnum">#{{ s.seq }}</span>
        <span class="audit-from">{{ s.from }}</span>
        <span class="audit-arrow">→</span>
        <span class="audit-to">{{ s.to }}</span>
      </li>
    </ul>
    <p v-if="shortCount" class="audit-warn">{{ shortCount }} 个文件没配齐，这一次不动它们。</p>
    <p v-for="e in preview.errors" :key="`p-${e}`" class="audit-warn">{{ e }}</p>
  </div>
</template>

<style scoped>
.audit {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.audit-counts {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  font-size: 11.5px;
}
.audit-counts .cnt {
  padding: 2px 10px;
  border-radius: 999px;
  border: 1px solid var(--border);
  color: var(--text-muted);
}
.audit-counts .cnt b { font-weight: 500; color: var(--text-sub); }
.audit-counts .cnt.warn { border-color: color-mix(in srgb, var(--warning) 45%, transparent); color: var(--warning); }
.audit-counts .cnt.warn b { color: var(--warning); }
.audit-samples {
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 3px;
  max-height: 168px;
  overflow-y: auto;
  font-size: 11.5px;
  color: var(--text-sub);
}
.audit-samples li { display: flex; align-items: baseline; gap: 6px; }
.audit-from { color: var(--text-muted); text-decoration: line-through; }
.audit-arrow { color: var(--text-muted); }
.audit-to { color: var(--text); }
.audit-warn { font-size: 11.5px; color: var(--warning); word-break: break-all; }
</style>
