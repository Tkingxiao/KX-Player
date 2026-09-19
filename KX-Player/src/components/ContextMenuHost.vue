<script setup lang="ts">
/** 右键菜单宿主：点击遮罩/Esc/滚动即关闭；越界回弹；二级菜单用 fixed 定位锚定到父项，
 *  不受父菜单 overflow 裁切，也不产生页面滚动条。 */
import { ref, computed, onMounted, onBeforeUnmount, nextTick, watch } from 'vue'
import { useUiStore, type ContextMenuItem } from '@/stores/ui'

const ui = useUiStore()
const root = ref<HTMLElement | null>(null)
const pos = ref({ x: 0, y: 0 })
const subOpenIdx = ref(-1)
const subPos = ref({ x: 0, y: 0 })
const subFlipLeft = ref(false)

const menu = computed(() => ui.ctxMenu)
const subItems = computed<ContextMenuItem[] | null>(() => {
  if (subOpenIdx.value < 0 || !menu.value) return null
  return menu.value.items[subOpenIdx.value]?.children ?? null
})

watch(menu, async (m) => {
  if (!m) return
  subOpenIdx.value = -1
  await nextTick()
  const el = root.value
  if (!el) return
  const rect = el.getBoundingClientRect()
  let x = m.x
  let y = m.y
  if (x + rect.width > window.innerWidth - 8) x = Math.max(8, window.innerWidth - rect.width - 8)
  if (y + rect.height > window.innerHeight - 8) y = Math.max(8, window.innerHeight - rect.height - 8)
  pos.value = { x, y }
})

async function openSub(i: number, ev: MouseEvent): Promise<void> {
  const item = menu.value?.items[i]
  if (!item?.children?.length) { subOpenIdx.value = -1; return }
  // 以触发行（或鼠标位置）为锚点计算子菜单位置
  const btn = (ev.currentTarget as HTMLElement).getBoundingClientRect()
  const subW = 200 // 估算值；展开后如有越界再次校正
  let x = btn.right - 4
  if (x + subW > window.innerWidth - 8) x = Math.max(8, btn.left - subW + 4)
  const y = Math.min(Math.max(8, btn.top - 5), window.innerHeight - 8)
  subOpenIdx.value = i
  await nextTick()
  // 校正：按真实子菜单尺寸二次调整
  const subEl = document.querySelector('.ctx-sub-fixed') as HTMLElement | null
  if (subEl) {
    const r = subEl.getBoundingClientRect()
    if (x + r.width > window.innerWidth - 8) x = Math.max(8, btn.left - r.width + 4)
    const maxY = window.innerHeight - r.height - 8
    subPos.value = { x, y: Math.min(y, Math.max(8, maxY)) }
    return
  }
  subPos.value = { x, y }
}

function runItem(item: ContextMenuItem): void {
  if (item.disabled) return
  if (item.children) return
  ui.ctxMenu = null
  item.action?.()
}

function close(): void {
  ui.ctxMenu = null
}

function onKey(e: KeyboardEvent): void {
  if (e.key === 'Escape' && ui.ctxMenu) {
    e.stopPropagation()
    close()
  }
}

onMounted(() => {
  window.addEventListener('keydown', onKey, true)
  window.addEventListener('blur', close)
  window.addEventListener('resize', close)
})
onBeforeUnmount(() => {
  window.removeEventListener('keydown', onKey, true)
  window.removeEventListener('blur', close)
  window.removeEventListener('resize', close)
})
</script>

<template>
  <Teleport to="body">
    <div v-if="menu" class="ctx-overlay" @mousedown.self="close" @contextmenu.prevent="close" @wheel.passive="close">
      <div
        ref="root"
        class="ctx-menu"
        :style="{ left: pos.x + 'px', top: pos.y + 'px' }"
      >
        <template v-for="(item, i) in menu.items" :key="i">
          <div v-if="item.separatorBefore || item.label === '-'" class="ctx-sep" />
          <button
            v-else
            class="ctx-item"
            :class="{ danger: item.danger, disabled: item.disabled, open: subOpenIdx === i }"
            :disabled="item.disabled"
            @mouseenter="item.children ? openSub(i, $event) : (subOpenIdx = -1)"
            @click="runItem(item)"
          >
            <span class="ctx-label">{{ item.label }}</span>
            <span v-if="item.children" class="ctx-arrow">▸</span>
          </button>
        </template>
      </div>

      <!-- 二级菜单：fixed 定位，脱离父菜单的滚动容器 -->
      <div
        v-if="subItems"
        class="ctx-menu ctx-sub-fixed"
        :style="{ left: subPos.x + 'px', top: subPos.y + 'px' }"
        @mouseleave="subOpenIdx = -1"
      >
        <button
          v-for="(child, ci) in subItems"
          :key="ci"
          class="ctx-item"
          :class="{ danger: child.danger, disabled: child.disabled }"
          :disabled="child.disabled"
          @click="runItem(child)"
        >
          <span class="ctx-label">{{ child.label }}</span>
        </button>
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
.ctx-overlay {
  position: fixed;
  inset: 0;
  z-index: 940;
}
.ctx-menu {
  position: fixed;
  min-width: 168px;
  max-height: 60vh;
  overflow-y: auto;
  padding: 5px;
  border-radius: var(--radius);
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  box-shadow: var(--shadow-pop);
}
.ctx-sub-fixed {
  max-height: 50vh;
}
.ctx-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  width: 100%;
  padding: 7px 12px;
  border-radius: var(--radius-sm);
  font-size: 12.5px;
  color: var(--text);
  text-align: left;
  white-space: nowrap;
}
.ctx-item:hover {
  background: var(--bg-selected);
}
.ctx-item.open {
  background: var(--bg-selected);
}
.ctx-item.danger {
  color: #e6685f;
}
.ctx-item.danger:hover {
  background: rgba(230, 58, 46, 0.12);
}
.ctx-item.disabled {
  opacity: 0.4;
  cursor: default;
}
.ctx-item.disabled:hover {
  background: none;
}
.ctx-label {
  overflow: hidden;
  text-overflow: ellipsis;
}
.ctx-arrow {
  font-size: 10px;
  color: var(--text-muted);
}
.ctx-sep {
  height: 1px;
  margin: 5px 8px;
  background: var(--border);
}
</style>
