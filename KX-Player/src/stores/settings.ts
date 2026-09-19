/** 设置 store：单一数据源 + 防抖落盘（退出时同步 flush）。 */
import { defineStore } from 'pinia'
import { ref, reactive } from 'vue'
import { api } from '@/bridge/ipc'

export interface BgEditState {
  zoomPct: number
  posX: number
  posY: number
  vw: number
  vh: number
}

export const useSettingsStore = defineStore('settings', () => {
  // ── 外观 ──
  const theme = ref<'dark' | 'light'>('dark')
  const clr = ref('#7c6cf6')
  const titlebarOpacity = ref(0.72)
  const sidebarOpacity = ref(0.8)
  const playerOpacity = ref(0.9)
  const ovl = ref(0.4)
  const bgBlur = ref(0)
  const bgPath = ref('')
  const bgMtime = ref(0)
  const bgSize = ref<'cover' | 'contain'>('cover')
  const imgEditState = ref<BgEditState | null>(null)

  // ── 播放偏好 ──
  const vol = ref(0.85)
  const muted = ref(false)
  const mode = ref<0 | 1 | 2 | 3>(0)
  const speed = ref(1)
  const devId = ref('')
  const keyboardEnabled = ref(true)

  // ── 数据 ──
  const recents = ref<string[]>([])
  const favs = ref<{ id: string; name: string; trackIds: string[]; isDefault?: boolean }[]>([])
  const pls = ref<{ id: string; name: string; trackIds: string[] }[]>([])

  // ── 视图记忆 ──
  const folderView = ref<'grid' | 'list'>('grid')
  const folderSort = ref<'name' | 'mtime' | 'count'>('name')
  const folderSortDir = ref<'asc' | 'desc'>('asc')
  const folderStack = ref<string[]>([])
  const gridSize = ref<132 | 176 | 220>(176)

  // ── AI 翻译（API Key 仅存内存，不落盘）──
  const aiBaseURL = ref('')
  const aiModel = ref('')

  // ── 布局尺寸（P0-1 拖拽分隔条）──
  const sidebarWidth = ref(232)
  const inspectorWidth = ref(320)

  let saveTimer: ReturnType<typeof setTimeout> | null = null

  function snapshot(): Record<string, unknown> {
    return {
      // 兼容旧设置键名，便于老用户升级
      theme: theme.value, clr: clr.value, ovl: ovl.value, bgPath: bgPath.value, bgMtime: bgMtime.value,
      bgSize: bgSize.value, bgBlur: bgBlur.value, _imgEditState: imgEditState.value,
      titlebarOpacity: titlebarOpacity.value, sidebarOpacity: sidebarOpacity.value, playerOpacity: playerOpacity.value,
      vol: vol.value, muted: muted.value, mode: mode.value, speed: speed.value, devId: devId.value,
      keyboardEnabled: keyboardEnabled.value,
      recents: recents.value, favs: favs.value, pls: pls.value,
      folderView: folderView.value, folderSort: folderSort.value, folderSortDir: folderSortDir.value,
      folderStack: folderStack.value, gridSize: gridSize.value,
      aiBaseURL: aiBaseURL.value, aiModel: aiModel.value,
      sidebarWidth: sidebarWidth.value, inspectorWidth: inspectorWidth.value,
      _v: 2,
    }
  }

  function persistNow(): void {
    try {
      // 先深拷贝为普通对象：Vue 响应式 Proxy 无法被 IPC structured-clone
      const s = JSON.parse(JSON.stringify(snapshot())) as Record<string, unknown>
      api.syncSaveSettings(s)
      void api.saveSettings(s)
    } catch (e) { console.error('[settings] persist failed:', e) }
  }

  function scheduleSave(): void {
    if (saveTimer) clearTimeout(saveTimer)
    saveTimer = setTimeout(() => { saveTimer = null; persistNow() }, 300)
  }

  async function load(): Promise<void> {
    const s = await api.loadSettings()
    if (!s) return
    const g = <T,>(k: string, d: T): T => (s[k] === undefined || s[k] === null ? d : (s[k] as T))
    // 旧版设置以百分比（0–100）存储 0–1 区间的值；检测后归一化
    const ratio = (k: string, d: number, lo: number, hi: number): number => {
      const raw = Number(s[k])
      if (!isFinite(raw)) return d
      const v = raw > 1 ? raw / 100 : raw
      return Math.max(lo, Math.min(hi, v))
    }
    const rawTheme = String(s.theme ?? 'dark')
    theme.value = rawTheme === 'light' ? 'light' : 'dark'
    clr.value = g('clr', '#7c6cf6')
    ovl.value = ratio('ovl', 0.4, 0, 0.9)
    bgPath.value = g('bgPath', '') || ''
    bgMtime.value = g('bgMtime', 0)
    bgSize.value = g<'cover' | 'contain'>('bgSize', 'cover')
    bgBlur.value = ratio('bgBlur', 0, 0, 40)
    imgEditState.value = g<BgEditState | null>('_imgEditState', null)
    titlebarOpacity.value = ratio('titlebarOpacity', 0.72, 0.2, 1)
    sidebarOpacity.value = ratio('sidebarOpacity', 0.8, 0.2, 1)
    playerOpacity.value = ratio('playerOpacity', 0.9, 0.2, 1)
    vol.value = ratio('vol', 0.85, 0, 1)
    muted.value = g('muted', false)
    mode.value = (g('mode', 0) as 0 | 1 | 2 | 3) ?? 0
    speed.value = g('speed', 1) || 1
    devId.value = g('devId', '')
    keyboardEnabled.value = g('keyboardEnabled', true)
    recents.value = g('recents', [])
    favs.value = g('favs', [])
    if (!favs.value.some((f) => f.isDefault)) {
      favs.value = [{ id: 'fav-default', name: '我的收藏', trackIds: [], isDefault: true }, ...favs.value]
    }
    pls.value = g('pls', [])
    folderView.value = String(s.folderView ?? 'grid') === 'list' ? 'list' : 'grid'
    const rawSort = String(s.folderSort ?? 'name')
    folderSort.value = (['name', 'mtime', 'count'] as const).includes(rawSort as never) ? (rawSort as 'name' | 'mtime' | 'count') : 'name'
    folderSortDir.value = String(s.folderSortDir ?? 'asc') === 'desc' ? 'desc' : 'asc'
    folderStack.value = g('folderStack', [])
    const rawGrid = Number(s.gridSize ?? 176)
    gridSize.value = (rawGrid === 132 || rawGrid === 220 ? rawGrid : 176) as 132 | 176 | 220
    aiBaseURL.value = String(s.aiBaseURL ?? '')
    aiModel.value = String(s.aiModel ?? '')
    const sw = Number(s.sidebarWidth ?? 232)
    sidebarWidth.value = isFinite(sw) ? Math.max(180, Math.min(360, sw)) : 232
    const iw = Number(s.inspectorWidth ?? 320)
    inspectorWidth.value = isFinite(iw) ? Math.max(260, Math.min(480, iw)) : 320
  }

  /** 退出前 flush（onBeforeClose 调用；同步写保证落盘） */
  function flushOnExit(): void {
    if (saveTimer) { clearTimeout(saveTimer); saveTimer = null }
    persistNow()
  }

  const reactiveSnapshot = reactive({})
  void reactiveSnapshot

  return {
    theme, clr, titlebarOpacity, sidebarOpacity, playerOpacity,
    ovl, bgBlur, bgPath, bgMtime, bgSize, imgEditState,
    vol, muted, mode, speed, devId, keyboardEnabled,
    recents, favs, pls,
    folderView, folderSort, folderSortDir, folderStack, gridSize,
    aiBaseURL, aiModel,
    sidebarWidth, inspectorWidth,
    load, scheduleSave, flushOnExit,
  }
})
