/** UI store：视图导航 / 搜索 / 面板 / 多选 / 右键菜单。 */
import { defineStore } from 'pinia'
import { ref, shallowRef, computed, watch } from 'vue'
import { api } from '@/bridge/ipc'
import { PIP_GUTTER } from '@/contracts/pip'

export type ViewKind =
  | 'folder' | 'smart' | 'recent' | 'fav' | 'playlist' | 'search' | 'tools' | 'lyrics' | 'stage' | 'ai'

export type SmartKey =
  | 'all' | 'unfinished' | 'recent' | 'recentAdded' | 'completed'
  | 'favorites' | 'video' | 'audio' | 'withSubs'

export interface ContextMenuItem {
  label: string
  action?: () => void
  danger?: boolean
  disabled?: boolean
  separatorBefore?: boolean
  children?: ContextMenuItem[]
}

export interface ContextMenuState {
  x: number
  y: number
  items: ContextMenuItem[]
}

export interface Toast {
  id: number
  text: string
  kind: 'info' | 'success' | 'error'
  progress?: number
}

export const useUiStore = defineStore('ui', () => {
  const view = ref<ViewKind>('smart')
  const smartKey = ref<SmartKey>('all')
  const durationKey = ref<string | null>(null)
  // 分类/标签组合筛选（P0-16，「且」关系叠加）
  const categoryId = ref<number | null>(null)
  const tagIds = shallowRef<Set<number>>(new Set())
  const tagMode = ref<'any' | 'all'>('any')
  const taxonomyExpanded = ref<Set<number>>(new Set())
  const autoTagReview = ref<{ trackIds: string[] } | null>(null)
  const folderPath = ref('')          // 当前文件夹（folder 视图内导航）
  const listPath = ref<string | null>(null) // 当前打开的收藏/播放列表 id
  const aiFolder = ref<string>('')        // AI 翻译：目标文件夹
  const searchQuery = ref('')
  const searchActive = ref(false)

  const settingsOpen = ref(false)
  const inspectorOpen = ref(false)
  const inspectorTab = ref<'queue' | 'subtitle' | 'info' | 'bookmark'>('queue')
  const queueOpen = ref(false)
  const bgEditorOpen = ref(false)

  // 视图返回上一页
  const previousView = ref<ViewKind>('smart')
  const previousFolderPath = ref('')

  // 画中画状态：悬浮窗是独立系统窗口（pip），主窗口只镜像开关状态
  const pipEnabled = ref(false)
  /** 钉住：切视图不自动关闭悬浮窗 */
  const pipPinned = ref(false)

  // ── 视频遮挡压制 ────────────────────────────────────────────
  // mpv 覆盖窗口是原生顶层窗口，恒在 WebView 之上；任何需要显示在视频
  // 区域上方的浮层（播放栏弹窗/设置模态/右键菜单/确认框）打开时都必须
  // 先把覆盖窗口藏起来，否则会被视频盖住。
  const pbBlockCount = ref(0)
  const confirmOpen = ref(false)
  const videoBlocked = computed(
    () =>
      pbBlockCount.value > 0 ||
      confirmOpen.value ||
      settingsOpen.value ||
      bgEditorOpen.value ||
      !!autoTagReview.value ||
      !!ctxMenu.value,
  )
  function pushVideoBlock(): void {
    pbBlockCount.value++
  }
  function popVideoBlock(): void {
    if (pbBlockCount.value > 0) pbBlockCount.value--
  }

  // 视图切换时记录上一页（用于返回；stage 是覆盖层，不记录为返回目标）
  watch(view, (newV, oldV) => {
    if (oldV && oldV !== 'stage') {
      previousView.value = oldV
      previousFolderPath.value = folderPath.value
    }
  })

  function goBack(): void {
    view.value = previousView.value
    if (previousFolderPath.value) folderPath.value = previousFolderPath.value
  }

  /** 悬浮窗几何（物理像素，本地持久化于 pip 窗口自身） */
  interface PipRect { x: number; y: number; w: number; h: number }
  function defaultPipRect(): PipRect {
    const dpr = window.devicePixelRatio || 1
    // 首开尺寸 320x180；实际比例由 PipRoot 打开后按 mpv 画面参数再对齐一次
    const w = Math.round((320 + PIP_GUTTER * 2) * dpr)
    const h = Math.round((180 + PIP_GUTTER * 2) * dpr)
    return {
      x: Math.max(0, Math.round((screen.availWidth - w) / 2)),
      y: Math.max(0, Math.round((screen.availHeight - h) / 2) - Math.round(24 * dpr)),
      w,
      h,
    }
  }
  function loadPipRect(): PipRect | null {
    try {
      const raw = localStorage.getItem('kx.pipRect')
      if (!raw) return null
      const r = JSON.parse(raw) as PipRect
      if (typeof r.x !== 'number' || typeof r.y !== 'number' || typeof r.w !== 'number' || typeof r.h !== 'number') return null
      return r
    } catch {
      return null
    }
  }

  async function togglePip(): Promise<void> {
    if (pipEnabled.value) await closePip()
    else await openPip()
  }

  async function openPip(): Promise<void> {
    const rect = loadPipRect() ?? defaultPipRect()
    const ok = await api.pipOpen(rect.x, rect.y, rect.w, rect.h)
    if (!ok) return
    pipEnabled.value = true
    // 规范 P0-4：悬浮期间库浏览不中断 —— 从舞台切出浮窗后主窗口退回进入前的视图，
    // 而不是留在一块被浮窗接管画面的黑舞台（否则既看不了库也点不到返回）。
    if (view.value === 'stage' && previousView.value !== 'stage') {
      goBack()
    }
  }

  /** 关闭悬浮窗。先通知 Rust（Rust 会 detach_overlay + hide + emit pip:closed），
   *  再由 pip:closed 事件回调清 pipEnabled，确保时序正确。
   *  此函数 await api.pipClose() 后主动清 pipEnabled（兜底，防事件延迟）。 */
  async function closePip(): Promise<void> {
    await api.pipClose()
    // pip:closed 事件会设 pipEnabled=false；这里也主动设一次（兜底）
    pipEnabled.value = false
    pipPinned.value = false
  }

  /** 从浮窗还原（双击画面 / Esc）：回到主窗口舞台视图继续播放 */
  function restoreToStage(): void {
    pipEnabled.value = false
    pipPinned.value = false
    view.value = 'stage'
  }

  // ── 独立悬浮窗事件同步 ──────────────────────────────────────
  // pip 窗口是独立系统窗口，其打开/关闭/钉住状态以 Rust 广播为准，
  // 主窗口这里只镜像（PipRoot 在 pip 窗口内管理自身的交互状态）。
  api.onPipClosed(() => {
    pipEnabled.value = false
    pipPinned.value = false
  })
  api.onPipRestored(() => {
    restoreToStage()
  })
  api.onPipPinned((pinned) => {
    pipPinned.value = pinned
  })

  // 多选（批量操作）
  const selectionMode = ref(false)
  const selection = shallowRef<Set<string>>(new Set())

  const ctxMenu = ref<ContextMenuState | null>(null)
  const toasts = ref<Toast[]>([])
  let toastSeq = 0

  // 逐级滚动位置记忆（folder 视图）
  const folderScrolls = new Map<string, number>()

  function openFolder(path: string): void {
    if (folderPath.value) folderScrolls.set(folderPath.value, window.scrollY)
    folderPath.value = path
  }

  function savedScroll(path: string): number {
    return folderScrolls.get(path) ?? 0
  }

  function recordScroll(path: string, top: number): void {
    folderScrolls.set(path, top)
  }

  function toast(text: string, kind: Toast['kind'] = 'info', duration = 3000): number {
    const id = ++toastSeq
    toasts.value = [...toasts.value, { id, text, kind }]
    if (duration > 0) {
      setTimeout(() => removeToast(id), duration)
    }
    return id
  }

  function updateToast(id: number, text: string, progress?: number): void {
    toasts.value = toasts.value.map((t) => (t.id === id ? { ...t, text, progress } : t))
  }

  function removeToast(id: number): void {
    toasts.value = toasts.value.filter((t) => t.id !== id)
  }

  function toggleSelect(id: string): void {
    const next = new Set(selection.value)
    if (next.has(id)) next.delete(id)
    else next.add(id)
    selection.value = next
  }

  function clearSelection(): void {
    selection.value = new Set()
  }

  function toggleTag(tagId: number): void {
    const next = new Set(tagIds.value)
    if (next.has(tagId)) next.delete(tagId)
    else next.add(tagId)
    tagIds.value = next
  }

  function toggleExpanded(id: number): void {
    const next = new Set(taxonomyExpanded.value)
    if (next.has(id)) next.delete(id)
    else next.add(id)
    taxonomyExpanded.value = next
  }

  function clearTaxonomy(): void {
    categoryId.value = null
    tagIds.value = new Set()
  }

  function closeAllPanels(): void {
    ctxMenu.value = null
    queueOpen.value = false
    if (searchActive.value) {
      searchActive.value = false
      searchQuery.value = ''
      view.value = 'smart'
    }
  }

  return {
    view, smartKey, durationKey, categoryId, tagIds, tagMode, taxonomyExpanded, autoTagReview,
    folderPath, listPath, aiFolder, searchQuery, searchActive,
    settingsOpen, inspectorOpen, inspectorTab, queueOpen, bgEditorOpen,
    previousView, previousFolderPath, pipEnabled, pipPinned,
    videoBlocked, pushVideoBlock, popVideoBlock, confirmOpen,
    selectionMode, selection, ctxMenu, toasts,
    openFolder, savedScroll, recordScroll, toast, updateToast, removeToast,
    toggleSelect, clearSelection, toggleTag, toggleExpanded, clearTaxonomy, closeAllPanels,
    goBack, togglePip, closePip, restoreToStage,
  }
})
