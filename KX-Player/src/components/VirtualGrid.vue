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
let resizeTimer: ReturnType<typeof setTimeout> | null = null

function recalcCols(): void {
  const el = viewport.value
  if (!el) return
  viewportH.value = el.clientHeight
  const c = Math.max(1, Math.floor((el.clientWidth + props.gap) / (props.cardWidth + props.gap)))
  if (c !== cols.value) cols.value = c
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
  resizeObserver = new ResizeObserver(() => {
    // 窗口 resize 期间暂停渲染，结束后重算列数
    if (resizeTimer) clearTimeout(resizeTimer)
    resizeTimer = setTimeout(recalcCols, 200)
  })
  resizeObserver.observe(el)
})

onBeforeUnmount(() => {
  resizeObserver?.disconnect()
  resizeObserver = null
  if (resizeTimer) clearTimeout(resizeTimer)
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
            gridTemplateColumns: `repeat(${cols}, minmax(0, 1fr))`,
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
  will-change: transform;
}
.vgrid-row {
  display: grid;
  gap: inherit;
}
</style>
