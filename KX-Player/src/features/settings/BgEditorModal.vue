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
import { assetUrl } from '@/bridge/ipc'

const settings = useSettingsStore()
const ui = useUiStore()

const MIN_ZOOM = 50
const MAX_ZOOM = 300
const WHEEL_STEP = 3 // 每次 3%

// 走 asset 协议：CSP 不允许 file://，否则预览显示破图
const bgUrl = computed(() => {
  if (!settings.bgPath) return null
  const base = assetUrl(settings.bgPath)
  return base ? base + '?v=' + settings.bgMtime : null
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

/** 预览样式：与主界面一致（适配方式 + 透明度 + 模糊 + 缩放平移） */
const FIT_MODES = [
  { value: 'cover', label: '填充', hint: '等比缩放铺满，裁掉溢出部分' },
  { value: 'contain', label: '适应', hint: '等比缩放到完整可见' },
  { value: 'stretch', label: '拉伸', hint: '拉伸到窗口尺寸' },
  { value: 'center', label: '居中', hint: '原始尺寸居中' },
  { value: 'tile', label: '平铺', hint: '以原始尺寸重复平铺' },
] as const

const previewStyle = computed<Record<string, string>>(() => {
  const fit = settings.bgSize
  const style: Record<string, string> = {
    opacity: String(Math.max(0, Math.min(1, 1 - settings.ovl))),
    filter: settings.bgBlur > 0 ? `blur(${settings.bgBlur}px)` : '',
  }
  if (fit === 'stretch') style.objectFit = 'fill'
  else if (fit === 'contain') style.objectFit = 'contain'
  else if (fit === 'cover') style.objectFit = 'cover'
  else if (fit === 'center') { style.objectFit = 'none'; style.objectPosition = 'center' }
  else if (fit === 'tile') {
    // 平铺：<img> 不支持 CSS repeat，隐藏 img，由 previewContainerStyle 用 background-image 实现
    style.display = 'none'
  }
  // 居中/平铺不施加缩放平移（与 repeat/none 冲突）
  if (fit !== 'tile' && fit !== 'center') {
    style.transform = `translate(${(posX.value - 50) * 0.8}%, ${(posY.value - 50) * 0.8}%) scale(${zoomPct.value / 100})`
  }
  return style
})

/** 平铺模式：在预览容器上用 background-image 实现 */
const previewContainerStyle = computed<Record<string, string>>(() => {
  if (settings.bgSize !== 'tile' || !bgUrl.value) return {} as Record<string, string>
  return {
    backgroundImage: `url(${bgUrl.value})`,
    backgroundRepeat: 'repeat',
    backgroundSize: 'auto',
    opacity: String(Math.max(0, Math.min(1, 1 - settings.ovl))),
  }
})

function setFit(v: 'stretch' | 'cover' | 'center' | 'tile' | 'contain'): void {
  settings.bgSize = v
  settings.scheduleSave()
}

onMounted(() => window.addEventListener('keydown', onKey, true))
onBeforeUnmount(() => {
  window.removeEventListener('keydown', onKey, true)
  onDragEnd()
})
</script>

<template>
  <Teleport to="body">
    <Transition name="modal">
      <div v-if="ui.bgEditorOpen" class="bge-backdrop" @mousedown.self="close">
        <div class="modal-card bge-card">
          <div class="bge-head">
            <h2>编辑背景图</h2>
            <button class="icon-btn" title="关闭（Esc）" aria-label="关闭" @click="close">✕</button>
          </div>

          <div
            ref="preview"
            class="bge-preview"
            :class="{ dragging }"
            :style="previewContainerStyle"
            @wheel="onWheel"
            @mousedown="onDragStart"
          >
            <img v-if="bgUrl" :src="bgUrl" alt="" draggable="false" :style="previewStyle" />
            <div v-else class="bge-empty">尚未设置背景图<br />请先在设置中选择图片</div>
            <span class="bge-hint" v-if="bgUrl && settings.bgSize !== 'tile' && settings.bgSize !== 'center'">滚轮缩放（每次 {{ WHEEL_STEP }}%） · 拖拽移动位置</span>
          </div>

          <!-- 适配方式：拉伸 / 填充 / 居中 / 平铺 / 适应 -->
          <div class="bge-fit">
            <span class="bge-fit-label">适配</span>
            <div class="bge-fit-modes">
              <button
                v-for="m in FIT_MODES"
                :key="m.value"
                class="bge-fit-btn"
                :class="{ active: settings.bgSize === m.value }"
                :title="m.hint"
                @click="setFit(m.value)"
              >{{ m.label }}</button>
            </div>
          </div>

          <div class="bge-controls">
            <span class="bge-zoom-readout tnum">缩放 {{ zoomPct }}% · 位置 {{ Math.round(posX) }}, {{ Math.round(posY) }} · 透明度 {{ Math.round(settings.ovl * 100) }}%</span>
            <div class="slider-row compact bge-alpha">
              <span class="slider-label">透明度</span>
              <input
                type="range"
                min="0"
                max="0.9"
                step="0.01"
                :value="settings.ovl"
                @input="settings.ovl = Number(($event.target as HTMLInputElement).value); settings.scheduleSave()"
              />
            </div>
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
/* 遮罩层：必须能接收指针事件（原先与内层压暗层共用 .bge-overlay 类名，
   被后者的 pointer-events: none 覆盖，导致整个弹窗鼠标失效、关闭按钮点不到） */
.bge-backdrop {
  position: fixed;
  inset: 0;
  z-index: 925;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--modal-overlay);
}
/* 弹窗需要更高的层级与独立的指针行为：z-index 高于 settings 弹窗（920） */
.bge-card {
  width: 680px;
  max-width: calc(100vw - 80px);
  background: var(--modal-bg);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  box-shadow: var(--shadow-pop);
  pointer-events: auto;
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
.bge-shade {
  position: absolute;
  inset: 0;
  pointer-events: none;
}

/* 适配方式选择栏 */
.bge-fit {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 12px 22px 0;
}
.bge-fit-label { font-size: 11.5px; color: var(--text-sub); flex-shrink: 0; }
.bge-fit-modes { display: flex; gap: 5px; flex-wrap: wrap; }
.bge-fit-btn {
  padding: 4px 11px;
  border-radius: 999px;
  border: 1px solid var(--border);
  font-size: 11.5px;
  color: var(--text-sub);
}
.bge-fit-btn:hover { border-color: var(--border-strong); color: var(--text); }
.bge-fit-btn.active {
  border-color: rgb(var(--accent-rgb));
  color: rgb(var(--accent-rgb));
  background: var(--bg-selected);
}
.bge-alpha { margin-top: 8px; }
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
