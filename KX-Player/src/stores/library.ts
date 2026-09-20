/** 曲库 store：快照加载 → 界面先出 → 后台增量同步 → 文件监听。 */
import { defineStore } from 'pinia'
import { ref, shallowRef, computed } from 'vue'
import type { FolderNode, Track, ScanResult, PlayProgress } from '@/contracts/api'
import { api, onEvent } from '@/bridge/ipc'
import { buildFolderMeta, findNodeByPath, collectAllTracks, type FolderMeta } from '@/utils/folderTree'
import { invalidateFolderCoverMisses } from '@/utils/covers'
import { normDir } from '@/utils/format'
import { useUiStore } from '@/stores/ui'

export interface ScanState {
  active: boolean
  completed: number
  total: number
  stage: string
}

export const useLibraryStore = defineStore('library', () => {
  // shallowRef：数组整体替换，避免深响应式 Proxy 进入 IPC（无法 structured-clone）
  const folderPaths = shallowRef<string[]>([])
  const folderTree = shallowRef<FolderNode[]>([])
  const allTracks = shallowRef<Track[]>([])
  const trackIndex = shallowRef<Map<string, Track>>(new Map())
  const folderMeta = shallowRef<Map<string, FolderMeta>>(new Map())
  const progress = ref<Map<string, PlayProgress>>(new Map())
  const loaded = ref(false)
  const revision = ref(0) // 每次曲库数据变化自增，驱动封面等派生资源重新加载
  const scanning = ref<ScanState>({ active: false, completed: 0, total: 0, stage: '' })
  const scanningFolder = ref('')

  // 防竞态：扫描代数
  let scanGeneration = 0
  /** 用户主动发起扫描时置为 true；后台/启动静默同步不置位，避免无谓的转圈与「构建音乐库」提示 */
  const scanningVisible = ref(false)

  function applyScanResult(result: ScanResult | null): void {
    if (!result) return
    // payload 缺 folderPaths 时保留现值，避免重扫链路把曲库路径归零
    const nextPaths = result.folderPaths && result.folderPaths.length ? result.folderPaths : folderPaths.value
    folderPaths.value = [...nextPaths].map(normDir)
    folderTree.value = result.folderTree || []
    allTracks.value = result.allTracks || []
    const idx = new Map<string, Track>()
    for (const t of allTracks.value) idx.set(t.id, t)
    trackIndex.value = idx
    folderMeta.value = buildFolderMeta(folderTree.value)
    loaded.value = true
    revision.value++
    // 曲库变化后清空「无封面」负缓存，让新增/新生成的封面能被重新发现
    invalidateFolderCoverMisses()
  }

  function setProgressInfo(list: PlayProgress[]): void {
    const m = new Map<string, PlayProgress>()
    for (const p of list) m.set(p.trackId, p)
    progress.value = m
  }

  async function loadProgress(): Promise<void> {
    try { setProgressInfo(await api.getProgressAll()) } catch { /* ignore */ }
  }

  async function init(): Promise<void> {
    void loadProgress()
    const fast = await api.loadLibraryFast()
    if (fast && fast.folderPaths && fast.allTracks) {
      // SQLite 快照先出界面（不含封面字节），随后后台增量同步离线期间的变动
      applyScanResult(fast)
      // 只有配置过扫描路径才执行后台增量同步
      if (folderPaths.value.length > 0) {
        void runStartupIncrementalSync()
      }
    } else if (folderPaths.value.length > 0) {
      // 有路径但无快照时执行一次全量扫描
      await rescan(false)
    }
    // 没有扫描路径时保持空库状态，由用户手动导入
    if (folderPaths.value.length > 0) {
      void api.startWatching(folderPaths.value)
    }
    loaded.value = true
  }

  async function runStartupIncrementalSync(): Promise<void> {
    const gen = ++scanGeneration
    try {
      // 静默同步：无变更时后端不发 stage/progress，UI 也不会闪「构建音乐库」
      const result = await api.startupSync(folderPaths.value)
      if (gen === scanGeneration && result) applyScanResult(result)
    } catch { /* ignore */ }
  }

  async function rescan(incremental: boolean, folder?: string): Promise<void> {
    const gen = ++scanGeneration
    scanningVisible.value = true
    scanning.value = { active: true, completed: 0, total: 0, stage: incremental ? '增量扫描...' : '准备扫描...' }
    scanningFolder.value = folder || ''
    try {
      const paths = folder ? [folder, ...folderPaths.value.filter((p) => p !== normDir(folder))] : folderPaths.value
      const result = incremental
        ? await api.scanFoldersIncremental(paths)
        : await api.scanFoldersWithProgress(paths)
      if (gen === scanGeneration) {
        applyScanResult(result)
        void loadProgress()
      }
    } finally {
      if (gen === scanGeneration) {
        scanningVisible.value = false
        scanning.value = { active: false, completed: 0, total: 0, stage: '' }
        scanningFolder.value = ''
      }
    }
  }

  async function importFolder(): Promise<void> {
    const picked = await api.openFolder()
    if (!picked || !picked.length) return
    const gen = ++scanGeneration
    scanningVisible.value = true
    scanning.value = { active: true, completed: 0, total: 0, stage: '扫描新文件夹...' }
    try {
      const merged = [...new Set([...folderPaths.value, ...picked.map(normDir)])]
      const result = await api.scanFoldersIncremental(merged)
      if (gen === scanGeneration) {
        applyScanResult(result)
        void loadProgress()
        void api.startWatching(merged)
      }
    } finally {
      if (gen === scanGeneration) {
        scanningVisible.value = false
        scanning.value = { active: false, completed: 0, total: 0, stage: '' }
        scanningFolder.value = ''
      }
    }
  }

  async function removeFolder(folderPath: string): Promise<void> {
    const gen = ++scanGeneration
    const np = normDir(folderPath)
    const remaining = folderPaths.value.filter((p) => p !== np)
    const result = await api.removeFolder(np, remaining)
    if (gen === scanGeneration && result) {
      applyScanResult(result)
      void loadProgress()
      void api.startWatching(remaining)
      // 正在浏览被移除的目录（或其子目录）时回到默认视图
      const ui = useUiStore()
      if (ui.view === 'folder' && (normDir(ui.folderPath) === np || normDir(ui.folderPath).startsWith(np + '/'))) {
        ui.view = 'smart'
        ui.folderPath = ''
      }
    }
    return
  }

  // 文件监听变化 → 防抖增量扫描（后台静默：不显示转圈与分区提示）
  let fsTimer: ReturnType<typeof setTimeout> | null = null
  function onFsChanged(): void {
    if (fsTimer) clearTimeout(fsTimer)
    fsTimer = setTimeout(() => { fsTimer = null; void rescanInBackground() }, 1200)
  }

  /** 后台增量同步：不置扫描态（除用户主动点刷新外，UI 不应出现转圈） */
  async function rescanInBackground(): Promise<void> {
    const gen = ++scanGeneration
    try {
      const result = await api.startupSync(folderPaths.value)
      if (gen === scanGeneration && result) applyScanResult(result)
    } catch { /* ignore */ }
  }

  // 进度回写（player store 调用）
  function updateProgressLocal(trackId: string, positionMs: number, completed?: boolean): void {
    const prev = progress.value.get(trackId)
    const next: PlayProgress = {
      trackId,
      positionMs,
      completed: completed ?? prev?.completed ?? false,
      playCount: (prev?.playCount ?? 0) + (prev ? 0 : 1),
      playedAt: Date.now(),
      lastSpeed: prev?.lastSpeed ?? null,
    }
    const m = new Map(progress.value)
    m.set(trackId, next)
    progress.value = m
  }

  // ── 后台响度分析（ffmpeg ebur128，任务 + 事件）──
  const loudnessScan = ref({ active: false, completed: 0, total: 0, done: 0, failed: 0, current: '' })
  let loudnessListenerBound = false

  function bindLoudnessEvents(): void {
    if (loudnessListenerBound) return
    loudnessListenerBound = true
    onEvent<{
      completed: number; total: number; current: string; done: number; failed: number
      finished: boolean; cancelled: boolean
    }>('loudness:progress', (p) => {
      loudnessScan.value = {
        active: !p.finished,
        completed: p.completed,
        total: p.total,
        done: p.done,
        failed: p.failed,
        current: p.current,
      }
      // 任务结束后刷新一次快照，把写回 DB 的 loudnessLufs 同步进内存
      if (p.finished) void refreshLoudnessValues()
    })
  }

  /** 对缺失 loudnessLufs 的条目启动后台分析；返回是否成功启动。 */
  function startLoudnessAnalysis(onlyMissing = true): boolean {
    bindLoudnessEvents()
    const targets: [string, string][] = []
    for (const t of allTracks.value) {
      if (onlyMissing && t.loudnessLufs != null) continue
      targets.push([t.id, t.path])
    }
    if (!targets.length) return false
    loudnessScan.value = { active: true, completed: 0, total: targets.length, done: 0, failed: 0, current: '' }
    void api.analyzeLoudness(targets)
    return true
  }

  function cancelLoudnessAnalysis(): void {
    void api.cancelLoudness()
  }

  /** 轻量刷新：仅拉取最新快照中的 loudnessLufs 合并进现有 track 对象，避免整库替换引发滚动跳动。 */
  async function refreshLoudnessValues(): Promise<void> {
    const fast = await api.loadLibraryFast()
    if (!fast?.allTracks) return
    const lufsById = new Map<string, number | null>()
    for (const t of fast.allTracks) lufsById.set(t.id, t.loudnessLufs ?? null)
    let changed = false
    const next = allTracks.value.map((t) => {
      const v = lufsById.get(t.id)
      if (v !== undefined && v !== (t.loudnessLufs ?? null)) {
        changed = true
        return { ...t, loudnessLufs: v }
      }
      return t
    })
    if (changed) {
      allTracks.value = next
      const idx = new Map<string, Track>()
      for (const t of next) idx.set(t.id, t)
      trackIndex.value = idx
      revision.value++
    }
  }

  const rootFolders = computed<FolderNode[]>(() =>
    folderTree.value.filter((n) => folderMeta.value.get(normDir(n.path))?.hasMusic),
  )

  function nodeByPath(path: string): FolderNode | null {
    return findNodeByPath(folderTree.value, path)
  }

  function tracksOfFolder(path: string): Track[] {
    const node = nodeByPath(path)
    return node ? collectAllTracks(node) : []
  }

  const libraryStats = computed(() => {
    const tracks = allTracks.value
    let seconds = 0
    let videos = 0
    let bytes = 0
    for (const t of tracks) {
      seconds += t.duration || 0
      if (t.isVideo) videos++
      bytes += t.fileSize || 0
    }
    return { count: tracks.length, videos, totalDuration: seconds, totalSize: bytes }
  })

  return {
    folderPaths, folderTree, allTracks, trackIndex, folderMeta, progress,
    loaded, revision, scanning, scanningFolder, scanningVisible,
    init, rescan, importFolder, removeFolder, onFsChanged, loadProgress, updateProgressLocal,
    rootFolders, nodeByPath, tracksOfFolder, libraryStats,
    loudnessScan, startLoudnessAnalysis, cancelLoudnessAnalysis,
  }
})
