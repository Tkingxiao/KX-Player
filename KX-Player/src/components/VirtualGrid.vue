<script setup lang="ts">
/** 虚拟化卡片网格：按容器宽度计算列数，虚拟化「卡片行」。 */
import { ref, computed, onMounted, onBeforeUnmount, watch } from 'vue'

const props = withDefaults(defineProps<{
  count: number
  cardWidth: number
  cardHeight: number
  gap?: number
}>(), { gap: 12 })

const emit = defineEmits<{
  (e: 'visible', start: number, end: number): void
}>()

const viewport = ref<HTMLElement | null>(null)
const scrollTop = ref(0)
const viewportH = ref(600)
const cols = ref(4)
let rafId = 0

const rowH = computed(() => props.cardHeight + props.gap)
const rowCount = computed(() => Math.ceil(props.count / cols.value))
const totalH = computed(() => rowCount.value * rowH.value)
const buffer = 2

const startRow = computed(() => Math.max(0, Math.floor(scrollTop.value / rowH.value) - buffer))
const endRow = computed(() => Math.min(rowCount.value, Math.ceil((scrollTop.value + viewportH.value) / rowH.value) + buffer))
const offsetY = computed(() => startRow.value * rowH.value)

const rows = computed(() => {
  const out: number[][] = []
  for (let r = startRow.value; r < endRow.value; r++) {
    const idxs: number[] = []
    for (let c = 0; c < cols.value; c++) {
      const i = r * cols.value + c
      if (i < props.count) idxs.push(i)
    }
    out.push(idxs)
  }
  return out
})

let resizeObserver: ResizeObserver | null = null
let rafId2 = 0

function recalcCols(): void {
  const el = viewport.value
  if (!el) return
  viewportH.value = el.clientHeight
  const c = Math.max(1, Math.floor((el.clientWidth + props.gap) / (props.cardWidth + props.gap)))
  if (c !== cols.value) cols.value = c
}

/** rAF 节流重算：侧边栏/检查器开合、窗口缩放时列数即时跟随（避免旧宽度下错误列数残留） */
function scheduleRecalc(): void {
  if (rafId2) return
  rafId2 = requestAnimationFrame(() => {
    rafId2 = 0
    recalcCols()
  })
}

function onScroll(): void {
  if (rafId) return
  rafId = requestAnimationFrame(() => {
    rafId = 0
    const el = viewport.value
    if (!el) return
    scrollTop.value = el.scrollTop
  })
}

onMounted(() => {
  const el = viewport.value
  if (!el) return
  recalcCols()
  resizeObserver = new ResizeObserver(scheduleRecalc)
  resizeObserver.observe(el)
})

// 卡片基准尺寸变化（如设置里切换封面大小）：列数必须重算，否则 1fr 单元格宽度异常
watch([() => props.cardWidth, () => props.gap], scheduleRecalc)

onBeforeUnmount(() => {
  resizeObserver?.disconnect()
  resizeObserver = null
  if (rafId2) cancelAnimationFrame(rafId2)
  if (rafId) cancelAnimationFrame(rafId)
})

watch([startRow, endRow], ([s]) => {
  emit('visible', s * cols.value, endRow.value * cols.value)
})

function scrollToTop(top: number): void {
  const el = viewport.value
  if (el) el.scrollTop = top
}

defineExpose({ scrollToTop, cols })
</script>

<template>
  <div ref="viewport" class="vgrid-viewport" @scroll.passive="onScroll">
    <div class="vgrid-spacer" :style="{ height: totalH + 'px' }">
      <div class="vgrid-window" :style="{ transform: `translateY(${offsetY}px)` }">
        <div
          v-for="(row, ri) in rows"
          :key="startRow + ri"
          class="vgrid-row"
          :style="{
            height: cardHeight + 'px',
            marginBottom: gap + 'px',
            columnGap: gap + 'px',
            gridTemplateColumns: `repeat(${cols}, minmax(${cardWidth}px, 1fr))`,
          }"
        >
          <template v-for="i in row" :key="i">
            <slot :index="i" />
          </template>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.vgrid-viewport {
  height: 100%;
  overflow-y: auto;
  overflow-x: hidden;
}
.vgrid-spacer {
  position: relative;
}
.vgrid-window {
  position: absolute;
  top: 0;
  left: 0;
  /* 必须显式撑满：绝对定位默认 shrink-to-fit，1fr 轨道会按错误宽度解析 → 卡片宽度异常 */
  width: 100%;
  will-change: transform;
}
.vgrid-row {
  display: grid;
  /* 水平间距由行内 columnGap 提供（gap: inherit 会解析为 normal=0，是旧版卡片粘连/宽度异常的根因） */
  /* 行内所有单元格等高、等宽：卡片内容（有无封面）不得改变格子尺寸 */
  align-items: stretch;
}
.vgrid-row > * {
  min-width: 0;
  height: 100%;
}
</style>
