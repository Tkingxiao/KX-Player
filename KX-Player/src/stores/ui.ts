/** UI store：视图导航 / 搜索 / 面板 / 多选 / 右键菜单。 */
import { defineStore } from 'pinia'
import { ref, shallowRef } from 'vue'

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
  const inspectorTab = ref<'queue' | 'info' | 'bookmark'>('queue')
  const queueOpen = ref(false)
  const bgEditorOpen = ref(false)

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
    selectionMode, selection, ctxMenu, toasts,
    openFolder, savedScroll, recordScroll, toast, updateToast, removeToast,
    toggleSelect, clearSelection, toggleTag, toggleExpanded, clearTaxonomy, closeAllPanels,
  }
})
