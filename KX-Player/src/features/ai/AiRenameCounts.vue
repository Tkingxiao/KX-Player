<script setup lang="ts">
/**
 * 改名预览的账目条：一共扫了多少条、各档多少、其中多少条**真会**被这次「应用」动到。
 *
 * 单独一格的原因不是行数好看，是这几个数各有一个上游：`counts` 来自后端全量统计（清单截断了
 * 也仍是全量），`applicable` 却是从**列出的条目**算出来的 —— 混在一个组件里就容易拿截断后的
 * 条目去数全量的账（后端 `queue()` 按全量计划，两边一说两种话）。
 */
import type { RenameCounts, RenameVerdict } from '@/contracts/api'
import { VERDICT_LABEL } from './renameLabels'

defineProps<{ counts: RenameCounts; aiCount: number; applicable: number; more: string }>()

const VERDICT_ORDER: RenameVerdict[] = ['skip', 'normalize_only', 'needs_repair', 'needs_translation', 'compliant']
const verdictKey: Record<RenameVerdict, keyof RenameCounts> = {
  skip: 'skip',
  normalize_only: 'normalizeOnly',
  needs_repair: 'needsRepair',
  needs_translation: 'needsTranslation',
  compliant: 'compliant',
}
</script>

<template>
  <div class="rn-counts">
    <span>共 {{ counts.scanned }} 条</span>
    <span v-for="v in VERDICT_ORDER" :key="v" :class="`v-${v}`">{{ VERDICT_LABEL[v] }} {{ counts[verdictKey[v]] }}</span>
    <span>有改动 {{ counts.changed }}</span>
    <span :class="{ bad: counts.conflicts > 0 }">冲突 {{ counts.conflicts }}</span>
    <span>要送模型 {{ aiCount }}</span>
    <span class="good">可应用 {{ applicable }}{{ more }}</span>
  </div>
</template>

<style scoped>
.rn-counts { display: flex; flex-wrap: wrap; gap: 6px 14px; font-size: 11.5px; color: var(--text-sub); }
.rn-counts .bad { color: var(--danger); }
.rn-counts .good { color: var(--success); }
.rn-counts .v-skip { color: var(--text-muted); }
.rn-counts .v-compliant { color: var(--success); }
.rn-counts .v-needs_repair,
.rn-counts .v-needs_translation { color: var(--warning); }
</style>
