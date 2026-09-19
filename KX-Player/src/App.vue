<script setup lang="ts">
/** 应用骨架：背景层 + 三栏 + 播放栏 + 浮层。 */
import { onMounted, ref } from 'vue'
import TitleBar from '@/features/shell/TitleBar.vue'
import Sidebar from '@/features/shell/Sidebar.vue'
import ContentArea from '@/features/library/ContentArea.vue'
import InspectorPanel from '@/features/inspector/InspectorPanel.vue'
import PlayerBar from '@/features/shell/PlayerBar.vue'
import QueuePanel from '@/features/shell/QueuePanel.vue'
import BgEditorModal from '@/features/settings/BgEditorModal.vue'
import ToastHost from '@/components/ToastHost.vue'
import ConfirmDialog from '@/components/ConfirmDialog.vue'
import ContextMenuHost from '@/components/ContextMenuHost.vue'
import SettingsModal from '@/features/settings/SettingsModal.vue'
import AutoTagReview from '@/features/taxonomy/AutoTagReview.vue'
import { useSettingsStore } from '@/stores/settings'
import { useLibraryStore } from '@/stores/library'
import { usePlayerStore } from '@/stores/player'
import { useUiStore } from '@/stores/ui'
import { useTaxonomyStore } from '@/stores/taxonomy'
import { useThemeEffect } from '@/composables/useThemeEffect'
import { useKeyboardShortcuts } from '@/composables/useKeyboardShortcuts'
import { api, assetUrl } from '@/bridge/ipc'
import { bindConfirmHost } from '@/services/confirmHost'
import { loadSearchLibs } from '@/utils/fuzzy'

const settings = useSettingsStore()
const library = useLibraryStore()
const player = usePlayerStore()
const ui = useUiStore()
const taxonomy = useTaxonomyStore()

useThemeEffect()
useKeyboardShortcuts()

const confirmRef = ref<InstanceType<typeof ConfirmDialog> | null>(null)
bindConfirmHost((o) => confirmRef.value?.open(o) ?? Promise.resolve(false))

const bgUrl = ref<string | null>(null)

function refreshBg(): void {
  if (!settings.bgPath) {
    bgUrl.value = null
    return
  }
  const url = assetUrl(settings.bgPath)
  bgUrl.value = url ? url + '?v=' + settings.bgMtime : null
}

const bgStyle = () => {
  const e = settings.imgEditState
  if (!e) return {}
  return {
    transform: `translate(${(e.posX - 50) * 0.8}%, ${(e.posY - 50) * 0.8}%) scale(${e.zoomPct / 100})`,
    filter: `blur(${settings.bgBlur}px)`,
  }
}

onMounted(async () => {
  await settings.load()
  // 从主进程解析背景图绝对路径（兼容旧设置中的相对文件名，如「kx-player-bg.png」）
  try {
    const bg = await api.loadBgImage()
    if (bg && bg.path) {
      settings.bgPath = bg.path
      settings.bgMtime = bg.mtime ?? Date.now()
    } else {
      settings.bgPath = ''
    }
  } catch { /* ignore */ }
  refreshBg()
  player.init()
  void library.init()
  void taxonomy.refresh()
  void loadSearchLibs()

  api.onScanProgress?.(({ completed, total }) => {
    library.scanning = { ...library.scanning, active: total > 0, completed, total }
  })
  api.onScannerStage?.((stage) => {
    library.scanning = { ...library.scanning, stage }
  })
  api.onFsChanged?.(() => library.onFsChanged())

  // 退出前保存
  let saved = false
  const saveOnce = () => {
    if (saved) return
    saved = true
    settings.folderStack = []
    settings.flushOnExit()
  }
  api.onBeforeClose?.(saveOnce)
  window.addEventListener('beforeunload', saveOnce)
})
</script>

<template>
  <div id="bg-layer">
    <img v-if="bgUrl" :src="bgUrl" alt="" draggable="false" :style="bgStyle()" />
  </div>

  <div id="app-shell">
    <TitleBar />
    <div id="app-middle">
      <Sidebar />
      <ContentArea />
      <InspectorPanel v-if="ui.inspectorOpen" />
    </div>
    <PlayerBar />
  </div>

  <QueuePanel />
  <SettingsModal />
  <BgEditorModal />
  <AutoTagReview />
  <ContextMenuHost />
  <ToastHost />
  <ConfirmDialog ref="confirmRef" />
</template>
