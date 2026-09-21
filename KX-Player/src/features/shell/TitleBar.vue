<script setup lang="ts">
/** 自绘标题栏：Logo + 居中即时搜索 + 主题/导入/窗口控制。 */
import { ref, computed, onMounted, onBeforeUnmount } from 'vue'
import { useDebounceFn } from '@vueuse/core'
import { api } from '@/bridge/ipc'
import { useUiStore } from '@/stores/ui'
import { useLibraryStore } from '@/stores/library'

const ui = useUiStore()
const library = useLibraryStore()
const maximized = ref(false)

const commitSearch = useDebounceFn((v: string) => {
  if (v.trim()) {
    if (!ui.searchActive) ui.searchActive = true
    ui.view = 'search'
  } else if (ui.searchActive) {
    ui.searchActive = false
    ui.view = 'smart'
  }
}, 200)

const searchValue = computed({
  get: () => ui.searchQuery,
  set: (v: string) => {
    ui.searchQuery = v
    commitSearch(v)
  },
})

function clearSearch(): void {
  commitSearch.cancel()
  ui.searchQuery = ''
  ui.searchActive = false
  if (ui.view === 'search') ui.view = 'smart'
}

async function toggleMax(): Promise<void> {
  await api.maximize()
  maximized.value = await api.isMaximized()
}

let unmaxListener: (() => void) | null = null

onMounted(async () => {
  maximized.value = await api.isMaximized()
  unmaxListener = api.onMaximizeChange?.((m) => { maximized.value = m }) ?? null
})

onBeforeUnmount(() => {
  unmaxListener?.()
  unmaxListener = null
})
</script>

<template>
  <header id="titlebar">
    <div class="tb-left">
      <img class="tb-logo" src="/icon.svg" alt="" />
      <span class="tb-name">KX Player</span>
    </div>

    <div class="tb-center">
      <div class="tb-search" :class="{ active: ui.searchActive }">
        <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2"><circle cx="11" cy="11" r="8" /><line x1="21" y1="21" x2="16.65" y2="16.65" /></svg>
        <input
          v-model="searchValue"
          type="text"
          placeholder="搜索标题 / 艺术家 / 文件夹…（Ctrl+F）"
          autocomplete="off"
          spellcheck="false"
        />
        <button v-if="searchValue" class="tb-clear" title="清除" @click="clearSearch">✕</button>
      </div>
    </div>

    <div class="tb-right">
      <button class="tb-btn" title="主题与外观" @click="ui.settingsOpen = true">
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="5" /><path d="M12 1v2M12 21v2M4.22 4.22l1.42 1.42M18.36 18.36l1.42 1.42M1 12h2M21 12h2M4.22 19.78l1.42-1.42M18.36 5.64l1.42-1.42" /></svg>
      </button>
      <button class="tb-btn" title="导入文件夹" @click="library.importFolder()">
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" /><polyline points="7,10 12,15 17,10" /><line x1="12" y1="15" x2="12" y2="3" /></svg>
      </button>
      <button class="tb-btn" :class="{ spinning: library.scanning.active }" title="重新扫描曲库" @click="library.rescan(false)">
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2"><polyline points="23,4 23,10 17,10" /><path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10" /></svg>
      </button>
      <div class="tb-sep" />
      <button class="tb-btn" title="最小化" @click="api.minimize()">
        <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2"><line x1="5" y1="12" x2="19" y2="12" /></svg>
      </button>
      <button class="tb-btn" :title="maximized ? '还原' : '最大化'" @click="toggleMax()">
        <svg v-if="!maximized" viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2"><rect x="4" y="4" width="16" height="16" rx="2" /></svg>
        <svg v-else viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2"><rect x="8" y="8" width="12" height="12" rx="1" /><path d="M4 16V5a1 1 0 0 1 1-1h11" /></svg>
      </button>
      <button class="tb-btn tb-close" title="关闭（隐藏到托盘）" @click="api.close()">
        <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2"><line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" /></svg>
      </button>
    </div>
  </header>
</template>

<style scoped>
#titlebar {
  display: grid;
  grid-template-columns: 1fr minmax(320px, 520px) 1fr;
  align-items: center;
  height: var(--titlebar-h);
  padding: 0 6px;
  background: color-mix(in srgb, var(--bg-titlebar), transparent calc((1 - var(--tb-alpha, 1)) * 100%));
  -webkit-app-region: drag;
  user-select: none;
  position: relative;
  z-index: 20;
}
#titlebar button,
#titlebar input {
  -webkit-app-region: no-drag;
}
.tb-left {
  display: flex;
  align-items: center;
  gap: 8px;
  padding-left: 8px;
}
.tb-logo {
  width: 20px;
  height: 20px;
  border-radius: 5px;
}
.tb-name {
  font-size: 12px;
  font-weight: 600;
  letter-spacing: 0.02em;
}
.tb-center {
  display: flex;
  justify-content: center;
}
.tb-search {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  max-width: 480px;
  height: 26px;
  padding: 0 10px;
  border-radius: 999px;
  background: var(--bg-input);
  border: 1px solid var(--border);
  color: var(--text-muted);
  transition: border-color var(--dur-fast) var(--ease), background var(--dur-fast) var(--ease);
}
.tb-search:focus-within,
.tb-search.active {
  border-color: rgb(var(--accent-rgb));
  color: var(--text-sub);
}
.tb-search input {
  flex: 1;
  border: none;
  background: none;
  font-size: 12px;
  color: var(--text);
}
.tb-search input::placeholder {
  color: var(--text-muted);
}
.tb-clear {
  font-size: 10px;
  color: var(--text-muted);
  padding: 2px;
}
.tb-clear:hover { color: var(--text); }
.tb-right {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 2px;
}
.tb-sep {
  width: 1px;
  height: 14px;
  margin: 0 5px;
  background: var(--border);
}
.tb-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: var(--titlebar-h);
  color: var(--text-sub);
  transition: background var(--dur-fast) var(--ease), color var(--dur-fast) var(--ease);
}
.tb-btn:hover {
  background: var(--bg-hover);
  color: var(--text);
}
.tb-btn.spinning svg {
  animation: tb-spin 1s linear infinite;
}
@keyframes tb-spin {
  to { transform: rotate(360deg); }
}
.tb-close:hover {
  background: var(--win-close);
  color: var(--on-media);
}
</style>
