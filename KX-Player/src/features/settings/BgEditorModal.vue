<script setup lang="ts">
/**
 * 背景图编辑弹窗：预览区与主页背景使用同一变换公式。
 * - 鼠标滚轮缩放（每次 ±3%）
 * - 按住拖拽调整位置
 * - 滑杆精调（缩放/横向/纵向）
 */
import { ref, computed, onMounted, onBeforeUnmount } from 'vue'
import { useSettingsStore } from '@/stores/settings'
import { useUiStore } from '@/stores/ui'
import { api } from '@/bridge/ipc'

const settings = useSettingsStore()
const ui = useUiStore()

const MIN_ZOOM = 50
const MAX_ZOOM = 300
const WHEEL_STEP = 3 // 每次 3%

const bgUrl = computed(() => {
  if (!settings.bgPath) return null
  return 'file:///' + settings.bgPath.replace(/\\/g, '/').replace(/^\/+/, '') + '?v=' + settings.bgMtime
})

const zoomPct = computed(() => settings.imgEditState?.zoomPct ?? 100)
const posX = computed(() => settings.imgEditState?.posX ?? 50)
const posY = computed(() => settings.imgEditState?.posY ?? 50)

function commitEdit(patch: Partial<{ zoomPct: number; posX: number; posY: number }>): void {
  const base = settings.imgEditState ?? { zoomPct: 100, posX: 50, posY: 50, vw: 100, vh: 100 }
  const next = { ...base, ...patch }
  next.zoomPct = Math.max(MIN_ZOOM, Math.min(MAX_ZOOM, Math.round(next.zoomPct)))
  next.posX = Math.max(0, Math.min(100, next.posX))
  next.posY = Math.max(0, Math.min(100, next.posY))
  settings.imgEditState = next
  settings.scheduleSave()
}

// ── 滚轮缩放（每次 ±3%）──
function onWheel(e: WheelEvent): void {
  e.preventDefault()
  const dir = e.deltaY > 0 ? -1 : 1
  commitEdit({ zoomPct: zoomPct.value + dir * WHEEL_STEP })
}

// ── 拖拽移动 ──
const preview = ref<HTMLElement | null>(null)
const dragging = ref(false)
let dragStart = { x: 0, y: 0, posX: 50, posY: 50 }

function onDragStart(e: MouseEvent): void {
  if (!bgUrl.value) return
  dragging.value = true
  dragStart = { x: e.clientX, y: e.clientY, posX: posX.value, posY: posY.value }
  window.addEventListener('mousemove', onDragMove)
  window.addEventListener('mouseup', onDragEnd)
}

function onDragMove(e: MouseEvent): void {
  if (!dragging.value) return
  const rect = preview.value?.getBoundingClientRect()
  const w = rect?.width || 1
  const h = rect?.height || 1
  // translate 百分比基于图片盒尺寸：ΔposX = dx / w * 100 / 0.8 → 预览内图片 1:1 跟手
  commitEdit({
    posX: dragStart.posX + ((e.clientX - dragStart.x) / w) * 125,
    posY: dragStart.posY + ((e.clientY - dragStart.y) / h) * 125,
  })
}

function onDragEnd(): void {
  dragging.value = false
  window.removeEventListener('mousemove', onDragMove)
  window.removeEventListener('mouseup', onDragEnd)
}

function resetEdit(): void {
  settings.imgEditState = null
  settings.scheduleSave()
}

function close(): void {
  ui.bgEditorOpen = false
}

function onKey(e: KeyboardEvent): void {
  if (e.key === 'Escape' && ui.bgEditorOpen) {
    e.stopPropagation()
    close()
  }
}

const previewStyle = computed(() => ({
  transform: `translate(${(posX.value - 50) * 0.8}%, ${(posY.value - 50) * 0.8}%) scale(${zoomPct.value / 100})`,
  filter: `blur(${settings.bgBlur}px)`,
}))

onMounted(() => window.addEventListener('keydown', onKey, true))
onBeforeUnmount(() => {
  window.removeEventListener('keydown', onKey, true)
  onDragEnd()
})
</script>

<template>
  <Teleport to="body">
    <Transition name="modal">
      <div v-if="ui.bgEditorOpen" class="bge-overlay" @mousedown.self="close">
        <div class="modal-card bge-card">
          <div class="bge-head">
            <h2>编辑背景图</h2>
            <button class="icon-btn" title="关闭（Esc）" @click="close">✕</button>
          </div>

          <div
            ref="preview"
            class="bge-preview"
            :class="{ dragging }"
            @wheel="onWheel"
            @mousedown="onDragStart"
          >
            <img v-if="bgUrl" :src="bgUrl" alt="" draggable="false" :style="previewStyle" />
            <div v-else class="bge-empty">尚未设置背景图<br />请先在设置中选择图片</div>
            <div class="bge-overlay" :style="{ background: `rgba(0,0,0,${settings.ovl})` }" />
            <span class="bge-hint" v-if="bgUrl">滚轮缩放（每次 {{ WHEEL_STEP }}%） · 拖拽移动位置</span>
          </div>

          <div class="bge-controls">
            <span class="bge-zoom-readout tnum">缩放 {{ zoomPct }}% · 位置 {{ Math.round(posX) }}, {{ Math.round(posY) }}</span>
          </div>

          <div class="bge-actions">
            <button class="btn-ghost" @click="resetEdit">重置</button>
            <button class="btn-primary" @click="close">完成</button>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.bge-overlay {
  position: fixed;
  inset: 0;
  z-index: 925;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--modal-overlay);
}
.bge-card {
  width: 680px;
  max-width: calc(100vw - 80px);
  background: var(--modal-bg);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  box-shadow: var(--shadow-pop);
}
.bge-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 18px 22px 12px;
}
.bge-head h2 { font-size: 16px; font-weight: 650; }

.bge-preview {
  position: relative;
  margin: 0 22px;
  /* 与主窗口内容区大致同比例，预览位置即主页实际效果 */
  aspect-ratio: 1216 / 668;
  border-radius: var(--radius);
  overflow: hidden;
  background: var(--bg-input);
  cursor: grab;
  user-select: none;
  touch-action: none;
}
.bge-preview.dragging { cursor: grabbing; }
.bge-preview img {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  object-fit: cover;
  pointer-events: none;
}
.bge-empty {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  text-align: center;
  font-size: 12.5px;
  color: var(--text-muted);
  line-height: 2;
}
.bge-overlay {
  position: absolute;
  inset: 0;
  pointer-events: none;
}
.bge-hint {
  position: absolute;
  left: 50%;
  bottom: 10px;
  transform: translateX(-50%);
  padding: 3px 12px;
  border-radius: 999px;
  background: rgba(0, 0, 0, 0.55);
  color: rgba(255, 255, 255, 0.85);
  font-size: 11px;
  pointer-events: none;
  white-space: nowrap;
}

.bge-controls {
  padding: 12px 22px 4px;
}
.bge-zoom-readout {
  font-size: 11.5px;
  color: var(--text-sub);
}
.bge-actions {
  display: flex;
  justify-content: flex-end;
  gap: 10px;
  padding: 14px 22px 20px;
}
</style>
