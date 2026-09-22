<script setup lang="ts">
/** Toast 宿主：右下角反馈。进度 Toast 由调用方管理生命周期。 */
import { useUiStore } from '@/stores/ui'

const ui = useUiStore()
</script>

<template>
  <div class="toast-host">
    <TransitionGroup name="pop">
      <div v-for="t in ui.toasts" :key="t.id" class="toast-item" :class="'toast-' + t.kind">
        <span class="toast-text">{{ t.text }}</span>
        <button v-if="t.progress === undefined" class="toast-close" title="关闭" @click="ui.removeToast(t.id)">✕</button>
      </div>
    </TransitionGroup>
  </div>
</template>

<style scoped>
.toast-host {
  position: fixed;
  right: 16px;
  bottom: calc(var(--playerbar-h) + 16px);
  z-index: 900;
  display: flex;
  flex-direction: column;
  gap: 8px;
  align-items: flex-end;
}
.toast-item {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  min-width: 200px;
  max-width: 400px;
  padding: 10px 14px;
  border-radius: var(--radius);
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  box-shadow: var(--shadow-pop);
}
.toast-error { border-color: color-mix(in srgb, var(--danger) 50%, transparent); }
.toast-success { border-color: color-mix(in srgb, var(--success) 50%, transparent); }
.toast-text {
  flex: 1;
  font-size: 12px;
  color: var(--text);
  word-break: break-all;
}
.toast-close {
  font-size: 11px;
  color: var(--text-muted);
  padding: 1px 4px;
  border-radius: 4px;
}
.toast-close:hover {
  color: var(--text);
  background: var(--bg-hover);
}
</style>
