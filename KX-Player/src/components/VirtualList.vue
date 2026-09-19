<script setup lang="ts">
/**
 * 通用虚拟滚动列表：固定行高，行内容用 slot 渲染。
 * 合并旧版 virtualList/virtualFolderList 两套实现；仅渲染可见行 ± buffer。
 */
import { ref, computed, onMounted, onBeforeUnmount, watch } from 'vue'

const props = withDefaults(defineProps<{
  count: number
  rowHeight: number
  buffer?: number
  rowKey?: (i: number) => string | number
}>(), { buffer: 6 })

const emit = defineEmits<{
  (e: 'visible', start: number, end: number): void
  (e: 'scroll', top: number): void
}>()

const viewport = ref<HTMLElement | null>(null)
const scrollTop = ref(0)
const viewportH = ref(600)
let rafId = 0

const totalH = computed(() => props.count * props.rowHeight)
const start = computed(() => Math.max(0, Math.floor(scrollTop.value / props.rowHeight) - props.buffer))
const end = computed(() =>
  Math.min(props.count, Math.ceil((scrollTop.value + viewportH.value) / props.rowHeight) + props.buffer),
)
const offsetY = computed(() => start.value * props.rowHeight)
const indices = computed(() => {
  const out: number[] = []
  for (let i = start.value; i < end.value; i++) out.push(i)
  return out
})

let resizeObserver: ResizeObserver | null = null

function onScroll(): void {
  if (rafId) return
  rafId = requestAnimationFrame(() => {
    rafId = 0
    const el = viewport.value
    if (!el) return
    scrollTop.value = el.scrollTop
    emit('scroll', el.scrollTop)
  })
}

onMounted(() => {
  const el = viewport.value
  if (!el) return
  viewportH.value = el.clientHeight
  resizeObserver = new ResizeObserver(() => {
    viewportH.value = el.clientHeight
  })
  resizeObserver.observe(el)
})

onBeforeUnmount(() => {
  resizeObserver?.disconnect()
  resizeObserver = null
  if (rafId) cancelAnimationFrame(rafId)
})

watch(() => props.count, () => {
  const el = viewport.value
  if (el && el.scrollTop > totalH.value) el.scrollTop = 0
})

watch([start, end], ([s, e]) => emit('visible', s, e))

function scrollToTop(top: number): void {
  const el = viewport.value
  if (el) el.scrollTop = top
}

defineExpose({ scrollToTop, viewport })
</script>

<template>
  <div ref="viewport" class="virtual-viewport" @scroll.passive="onScroll">
    <div class="virtual-spacer" :style="{ height: totalH + 'px' }">
      <div class="virtual-window" :style="{ transform: `translateY(${offsetY}px)` }">
        <div
          v-for="i in indices"
          :key="props.rowKey ? props.rowKey(i) : i"
          class="virtual-row"
          :style="{ height: props.rowHeight + 'px' }"
        >
          <slot :index="i" />
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.virtual-viewport {
  height: 100%;
  overflow-y: auto;
  overflow-x: hidden;
}
.virtual-spacer {
  position: relative;
  width: 100%;
}
.virtual-window {
  position: absolute;
  top: 0;
  left: 0;
  width: 100%;
  will-change: transform;
}
.virtual-row {
  width: 100%;
}
</style>
