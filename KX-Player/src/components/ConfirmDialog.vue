<script setup lang="ts">
/** 确认对话框（Promise 化）：Enter 确认 / Esc 取消 / 自动聚焦 / 关闭后焦点归位。 */
import { ref, onMounted, onBeforeUnmount, nextTick } from 'vue'

export interface ConfirmOptions {
  title: string
  message: string
  confirmText?: string
  cancelText?: string
  danger?: boolean
}

const visible = ref(false)
const opts = ref<ConfirmOptions>({ title: '', message: '' })
let resolver: ((ok: boolean) => void) | null = null
const confirmBtn = ref<HTMLButtonElement | null>(null)
let lastFocus: HTMLElement | null = null

function open(o: ConfirmOptions): Promise<boolean> {
  lastFocus = document.activeElement as HTMLElement | null
  opts.value = o
  visible.value = true
  return new Promise((resolve) => { resolver = resolve })
}

function close(ok: boolean): void {
  if (!visible.value) return
  visible.value = false
  resolver?.(ok)
  resolver = null
  lastFocus?.focus?.()
}

function onKey(e: KeyboardEvent): void {
  if (!visible.value) return
  if (e.key === 'Escape') { e.stopPropagation(); close(false) }
  else if (e.key === 'Enter') { e.stopPropagation(); close(true) }
}

onMounted(async () => {
  window.addEventListener('keydown', onKey, true)
  await nextTick()
  confirmBtn.value?.focus()
})

onBeforeUnmount(() => window.removeEventListener('keydown', onKey, true))

defineExpose({ open })
</script>

<template>
  <Teleport to="body">
    <Transition name="modal">
      <div v-if="visible" class="confirm-overlay" @mousedown.self="close(false)">
        <div class="modal-card confirm-card" role="alertdialog" :aria-label="opts.title">
          <h3 class="confirm-title">{{ opts.title }}</h3>
          <p class="confirm-message">{{ opts.message }}</p>
          <div class="confirm-actions">
            <button class="btn-ghost" @click="close(false)">{{ opts.cancelText || '取消' }}</button>
            <button
              ref="confirmBtn"
              class="btn-primary"
              :style="opts.danger ? 'background:var(--danger)' : ''"
              @click="close(true)"
            >{{ opts.confirmText || '确定' }}</button>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.confirm-overlay {
  position: fixed;
  inset: 0;
  z-index: 950;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--modal-overlay);
}
.confirm-card {
  width: 360px;
  padding: 22px;
}
.confirm-title {
  font-size: 15px;
  font-weight: 600;
  margin-bottom: 8px;
}
.confirm-message {
  font-size: 12.5px;
  color: var(--text-sub);
  margin-bottom: 20px;
  word-break: break-all;
}
.confirm-actions {
  display: flex;
  justify-content: flex-end;
  gap: 10px;
}
</style>
