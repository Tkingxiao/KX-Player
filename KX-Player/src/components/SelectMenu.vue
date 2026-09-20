<script setup lang="ts">
/**
 * 选择栏（下拉选择器）：替代「点击切换」的图标按钮。
 * 支持图标按钮模式（紧凑，用于视图切换）与文字模式（带当前值，用于排序）。
 */
import { ref, computed, onMounted, onBeforeUnmount } from 'vue'

export interface SelectOption {
  value: string
  label: string
  title?: string
}

const props = withDefaults(defineProps<{
  modelValue: string
  options: SelectOption[]
  /** 文字模式：显示当前值；否则只显示图标/触发内容 */
  label?: string
  /** icon = 紧凑图标按钮；text = 带当前值文字 */
  variant?: 'icon' | 'text'
  title?: string
  align?: 'left' | 'right'
  width?: string
}>(), { variant: 'icon', align: 'right', label: '', title: '' })

const emit = defineEmits<{ (e: 'update:modelValue', v: string): void }>()

const open = ref(false)
const rootEl = ref<HTMLElement | null>(null)

const current = computed(() => props.options.find((o) => o.value === props.modelValue))

function pick(v: string): void {
  emit('update:modelValue', v)
  open.value = false
}

function onDocClick(e: MouseEvent): void {
  if (!open.value) return
  const el = rootEl.value
  if (el && !el.contains(e.target as Node)) open.value = false
}

function onKey(e: KeyboardEvent): void {
  if (e.key === 'Escape' && open.value) {
    e.stopPropagation()
    open.value = false
  }
}

onMounted(() => {
  document.addEventListener('mousedown', onDocClick, true)
  window.addEventListener('keydown', onKey, true)
})
onBeforeUnmount(() => {
  document.removeEventListener('mousedown', onDocClick, true)
  window.removeEventListener('keydown', onKey, true)
})
</script>

<template>
  <div ref="rootEl" class="sel-wrap" :class="{ 'sel-open': open }">
    <button
      class="sel-trigger"
      :class="[`sel-${variant}`, { active: open }]"
      :title="title || current?.title || current?.label || ''"
      :aria-label="title || current?.label || label"
      :aria-expanded="open"
      aria-haspopup="listbox"
      @click="open = !open"
    >
      <slot name="icon" :current="current" />
      <span v-if="variant === 'text'" class="sel-value">{{ label || current?.label || '—' }}</span>
      <svg class="sel-caret" viewBox="0 0 24 24" width="10" height="10" fill="none" stroke="currentColor" stroke-width="2.4">
        <polyline points="6,9 12,15 18,9" />
      </svg>
    </button>

    <Transition name="pop-scale">
      <div
        v-if="open"
        class="sel-menu"
        :class="align === 'left' ? 'sel-menu-left' : 'sel-menu-right'"
        :style="width ? { minWidth: width } : undefined"
        role="listbox"
      >
        <button
          v-for="o in options"
          :key="o.value"
          class="sel-item"
          :class="{ active: o.value === modelValue }"
          role="option"
          :aria-selected="o.value === modelValue"
          :title="o.title || o.label"
          @click="pick(o.value)"
        >
          <span class="sel-item-label">{{ o.label }}</span>
          <svg v-if="o.value === modelValue" class="sel-check" viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2.6">
            <polyline points="20,6 9,17 4,12" />
          </svg>
        </button>
      </div>
    </Transition>
  </div>
</template>

<style scoped>
.sel-wrap { position: relative; display: inline-flex; }
.sel-trigger {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  border-radius: var(--radius-sm);
  color: var(--text-sub);
  border: 1px solid transparent;
}
.sel-trigger:hover { background: var(--bg-hover); color: var(--text); }
.sel-trigger.active {
  background: var(--bg-hover);
  color: var(--text);
  border-color: var(--border-strong);
}
.sel-icon {
  width: 30px;
  height: 30px;
  justify-content: center;
}
.sel-text {
  height: 28px;
  padding: 0 8px 0 10px;
  border-color: var(--border);
  font-size: 12px;
}
.sel-value { max-width: 130px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.sel-caret { opacity: 0.6; flex-shrink: 0; }

.sel-menu {
  position: absolute;
  top: calc(100% + 6px);
  z-index: 60;
  min-width: 132px;
  max-height: 320px;
  overflow-y: auto;
  padding: 5px;
  border-radius: var(--radius);
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  box-shadow: var(--shadow-pop);
}
.sel-menu-right { right: 0; }
.sel-menu-left { left: 0; }
.sel-item {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  padding: 6px 8px 6px 10px;
  border-radius: var(--radius-sm);
  font-size: 12px;
  color: var(--text-sub);
  text-align: left;
}
.sel-item:hover { background: var(--bg-hover); color: var(--text); }
.sel-item.active { color: rgb(var(--accent-rgb)); background: var(--bg-selected); font-weight: 500; }
.sel-item-label { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.sel-check { flex-shrink: 0; }
</style>
