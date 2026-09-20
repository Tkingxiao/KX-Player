<script setup lang="ts">
/** 应用骨架：背景层 + 三栏 + 播放栏 + 浮层。
 *  首屏只同步加载「立刻可见」的骨架组件；弹窗与低频视图一律异步，
 *  避免启动时为不可见代码付出解析/求值成本（这是白屏的主要来源之一）。 */
import { onMounted, ref, computed, onBeforeUnmount, defineAsyncComponent } from 'vue'
import TitleBar from '@/features/shell/TitleBar.vue'
import Sidebar from '@/features/shell/Sidebar.vue'
import ContentArea from '@/features/library/ContentArea.vue'
import InspectorPanel from '@/features/inspector/InspectorPanel.vue'
import PlayerBar from '@/features/shell/PlayerBar.vue'
import StageView from '@/features/stage/StageView.vue'
import ToastHost from '@/components/ToastHost.vue'
import ContextMenuHost from '@/components/ContextMenuHost.vue'
import ConfirmDialog from '@/components/ConfirmDialog.vue'
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

// 低频弹窗：仅在打开时才加载对应代码块
const SettingsModal = defineAsyncComponent(() => import('@/features/settings/SettingsModal.vue'))
const BgEditorModal = defineAsyncComponent(() => import('@/features/settings/BgEditorModal.vue'))
const AutoTagReview = defineAsyncComponent(() => import('@/features/taxonomy/AutoTagReview.vue'))

const settings = useSettingsStore()
const library = useLibraryStore()
const player = usePlayerStore()
const ui = useUiStore()
const taxonomy = useTaxonomyStore()

useThemeEffect()
useKeyboardShortcuts()

const confirmRef = ref<InstanceType<typeof ConfirmDialog> | null>(null)
bindConfirmHost((o) => confirmRef.value?.open(o) ?? Promise.resolve(false))

/** 背景图 URL 直接派生自 settings：新增/移除/更换后立即生效，无需重启或切页 */
const bgUrl = computed<string | null>(() => {
  if (!settings.bgPath) return null
  const url = assetUrl(settings.bgPath)
  return url ? url + '?v=' + settings.bgMtime : null
})

/** 背景图样式：适配方式 + 不透明度 + 缩放/平移 + 模糊。
 *  注意 ovl 语义是「不透明度」：不使用黑色遮罩层，而是直接调低图片自身 alpha，
 *  这样下层应用背景色透出来，比盖一层黑更自然（也让浅色主题可用）。 */
const bgStyle = computed<Record<string, string>>(() => {
  const fit = settings.bgSize
  const style: Record<string, string> = {
    opacity: String(Math.max(0, Math.min(1, 1 - settings.ovl))),
    filter: settings.bgBlur > 0 ? `blur(${settings.bgBlur}px)` : '',
  }
  if (fit === 'stretch') {
    style.objectFit = 'fill'
  } else if (fit === 'contain') {
    style.objectFit = 'contain'
  } else if (fit === 'cover') {
    style.objectFit = 'cover'
  } else if (fit === 'center') {
    // 居中：按原始像素比居中显示，不缩放
    style.objectFit = 'none'
    style.objectPosition = 'center'
  } else if (fit === 'tile') {
    // 平铺：以原图尺寸重复
    style.objectFit = 'none'
    style.objectRepeat = 'repeat'
    style.width = 'auto'
    style.height = 'auto'
    style.minWidth = '100%'
    style.minHeight = '100%'
  }
  // 平铺/居中模式不施加缩放平移（避免与 repeat 冲突）
  const e = settings.imgEditState
  if (e && fit !== 'tile' && fit !== 'center') {
    style.transform = `translate(${(e.posX - 50) * 0.8}%, ${(e.posY - 50) * 0.8}%) scale(${e.zoomPct / 100})`
  }
  return style
})

/** 响应式断点：窄窗口自动折叠 inspector，必要时折叠 sidebar */
let ro: ResizeObserver | null = null
function applyResponsive(): void {
  const w = window.innerWidth
  // <1024 自动关闭右检查器，避免内容区被压没
  if (w < 1024 && ui.inspectorOpen) ui.inspectorOpen = false
  // <1200 且侧边栏展开过宽时折叠
  if (w < 1080 && !settings.sidebarCollapsed) {
    settings.sidebarCollapsed = true
    settings.scheduleSave()
  }
}

/** 首帧之后再跑的后台工作：避免与首屏渲染争抢主线程。
 *  搜索库（chinese-conv + pinyin-pro ≈1.5MB）只在用户真正搜索时才必需，
 *  此前它在挂载后立即 import，会拉长可交互时间。 */
function afterFirstPaint(task: () => void): void {
  const run = () => setTimeout(task, 0)
  if (typeof requestIdleCallback === 'function') {
    requestIdleCallback(() => run(), { timeout: 2000 })
  } else {
    requestAnimationFrame(() => run())
  }
}

onMounted(async () => {
  // 关键路径：先拿到设置（决定主题/尺寸），其余全部延后
  await settings.load()
  // 从主进程解析背景图绝对路径（兼容旧设置中的相对文件名，如「kx-player-bg.png」）
  afterFirstPaint(() => {
    void (async () => {
      try {
        const bg = await api.loadBgImage()
        if (bg && bg.path) {
          settings.bgPath = bg.path
          settings.bgMtime = bg.mtime ?? Date.now()
        } else {
          settings.bgPath = ''
        }
      } catch { /* ignore */ }
    })()
  })

  player.init()
  // 曲库/分类照常立即加载（已是非阻塞的 void 调用），保证内容尽快出现
  void library.init()
  void taxonomy.refresh()
  // 搜索索引库较大且仅搜索时用到 → 首帧后再拉
  afterFirstPaint(() => { void loadSearchLibs() })

  // 扫描事件只反映「用户主动发起」的扫描：启动静默同步与文件监听后台扫描不发事件，
  // 因此这里收到事件时才置 active，且结束后清空。
  api.onScanProgress?.(({ completed, total }) => {
    if (!library.scanningVisible) return
    library.scanning = { ...library.scanning, active: total > 0, completed, total }
  })
  api.onScannerStage?.((stage) => {
    if (!library.scanningVisible) return
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

  applyResponsive()
  ro = new ResizeObserver(() => applyResponsive())
  ro.observe(document.body)
})

onBeforeUnmount(() => {
  ro?.disconnect()
})
</script>

<template>
  <div id="bg-layer">
    <img v-if="bgUrl" :src="bgUrl" alt="" draggable="false" loading="lazy" :style="bgStyle" />
  </div>

  <div id="app-shell">
    <TitleBar />
    <div id="app-middle">
      <Sidebar :class="{ collapsed: settings.sidebarCollapsed }" />
      <div id="content-wrap">
        <ContentArea />
        <!-- 影院模式覆盖内容区，标题栏和播放栏保留；常驻挂载以便离开舞台后托管 PiP 几何 -->
        <StageView class="stage-overlay" :class="{ 'stage-hidden': ui.view !== 'stage' }" />
      </div>
      <Transition name="slide-right">
        <InspectorPanel v-if="ui.inspectorOpen" />
      </Transition>
    </div>
    <PlayerBar />
  </div>

  <SettingsModal />
  <BgEditorModal />
  <AutoTagReview />
  <ContextMenuHost />
  <ToastHost />
  <ConfirmDialog ref="confirmRef" />
</template>

<style>
/* 三栏骨架：标题栏/中栏/播放栏，由 CSS grid 稳态驱动 */
#app-shell {
  display: grid;
  grid-template-rows: var(--titlebar-h) 1fr var(--playerbar-h);
  height: 100%;
  position: relative;
  z-index: 1;
  background: transparent;
}

#app-middle {
  display: grid;
  grid-template-columns: auto 1fr auto;
  min-height: 0;
  position: relative;
}

#content-wrap {
  position: relative;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
}

.stage-overlay {
  position: absolute;
  inset: 0;
  z-index: 15;
}
/* 非舞台视图：占位元素不可见且不拦截交互（mpv 覆盖窗口的几何由 StageView 内部托管） */
.stage-hidden {
  visibility: hidden;
  pointer-events: none;
}

.slide-right-enter-active,
.slide-right-leave-active {
  transition: transform var(--dur) var(--ease), opacity var(--dur) var(--ease);
}
.slide-right-enter-from,
.slide-right-leave-to {
  transform: translateX(100%);
  opacity: 0;
}
</style>
