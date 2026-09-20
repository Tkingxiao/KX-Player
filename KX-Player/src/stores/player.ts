/** 播放引擎 store：mpv 内核镜像 + 队列 + 四种模式 + 倍速 + 睡眠定时 + 进度记忆 + 书签。
 *  引擎在 Rust 侧（libmpv），本 store 只镜像 `player:state` 事件并转发命令。 */
import { defineStore } from 'pinia'
import { ref, shallowRef, computed } from 'vue'
import type { Track, Bookmark, AudioDevice, MpvPlayerState, SubtitleTrack } from '@/contracts/api'
import { api, onEvent } from '@/bridge/ipc'
import { useLibraryStore } from '@/stores/library'
import { useSettingsStore } from '@/stores/settings'
import { loadLyricsForTrack, type LoadedLyrics } from '@/utils/lyricsLoader'
import { nextIndex, prevIndex, onEnded } from '@/utils/queue'
import { completedOnWrite } from '@/utils/playback'
import { trackName } from '@/utils/format'
import type { LyricLine } from '@/utils/parsers/lyrics'
import type { SubStyle } from '@/contracts/api'

export interface SleepState {
  active: boolean
  endAt: number
  action: 'pause' | 'stop' | 'finishTrack'
  /** finishTrack：到点时若在播放，等当前曲目结束再停 */
  triggerOnEnded: boolean
}

export const usePlayerStore = defineStore('player', () => {
  const library = useLibraryStore()
  const settings = useSettingsStore()

  // ── 状态 ──
  const queue = shallowRef<string[]>([])
  const queueName = ref('')
  const currentId = ref<string | null>(null)
  const playing = ref(false)
  const position = ref(0)
  const duration = ref(0)
  const buffered = ref(0)
  const loading = ref(false)
  const lyrics = ref<LyricLine[]>([])
  const lyricsInfo = ref<LoadedLyrics>({ lines: [], source: null, format: null })
  const devices = ref<AudioDevice[]>([])
  const deviceOpen = ref(false)
  const sleep = ref<SleepState>({ active: false, endAt: 0, action: 'pause', triggerOnEnded: false })
  const sleepRemaining = ref(0)
  const bookmarks = shallowRef<Bookmark[]>([])
  const videoMode = ref<'audio' | 'video'>('audio')
  const subtitleTracks = shallowRef<SubtitleTrack[]>([])
  const subtitleVisible = ref(true)
  const subtitleDelay = ref(0)
  const audioOnly = ref(false) // P0-30：仅音频模式（vid=no，位置不丢）
  const error = ref('')

  let loadGeneration = 0
  let progressTimer: ReturnType<typeof setInterval> | null = null
  let sleepTicker: ReturnType<typeof setInterval> | null = null
  let ending = false

  const current = computed<Track | null>(() => (currentId.value ? library.trackIndex.get(currentId.value) ?? null : null))
  const currentTitle = computed(() => (current.value ? trackName(current.value) : '未在播放'))
  const currentArtist = computed(() => (current.value ? (current.value.metaArtist || current.value.artist || '佚名') : ''))

  // ── 输出设备 ──
  /** 拉取输出设备列表。mpv 在首次播放后才存在，因此启动时可能只拿到默认项；
   *  失败时保留已有列表（不要清空成空数组，否则 UI 显示「未枚举到设备」）。 */
  async function refreshDevices(): Promise<void> {
    try {
      const list = await api.playerListDevices()
      if (list && list.length) devices.value = list
    } catch { /* 保留上一次结果 */ }
  }

  // ── mpv 状态镜像 ──
  function applyMpvState(s: MpvPlayerState): void {
    playing.value = s.playing
    position.value = s.position
    if (s.duration > 0) duration.value = s.duration
    if (s.playing) loading.value = false
  }

  function pushRecent(id: string): void {
    const r = settings.recents.filter((x) => x !== id)
    r.unshift(id)
    if (r.length > 200) r.length = 200
    settings.recents = r
    settings.scheduleSave()
  }

  // ── 加载与播放 ──
  async function loadTrack(track: Track, resumeMs: number, autoplay: boolean): Promise<boolean> {
    const gen = ++loadGeneration
    loading.value = true
    position.value = resumeMs / 1000
    // 歌词异步加载不阻塞出声
    void loadLyricsForTrack(track.path).then((info) => {
      if (gen !== loadGeneration) return
      lyricsInfo.value = info
      lyrics.value = info.lines
    })
    const videoActive = !!autoplay && videoMode.value === 'video' && track.isVideo && !audioOnly.value
    try {
      await api.playerPlay({
        trackPath: track.path,
        resumeMs,
        vol: settings.vol,
        muted: settings.muted,
        speed: settings.speed,
        videoActive,
      })
    } catch (e) {
      error.value = String(e)
      loading.value = false
      return false
    }
    if (gen !== loadGeneration) return false // 已被更新的加载取代，交由新流程收尾
    loading.value = false
    return true
  }

  /** 播放指定曲目（带队列上下文）。fromLast=true 时从上次进度续播。 */
  async function playTrack(track: Track, list: string[], name: string, opts?: { fromLast?: boolean; video?: boolean }): Promise<void> {
    if (!track) return
    queue.value = list
    queueName.value = name
    currentId.value = track.id
    ending = false
    videoMode.value = opts?.video && track.isVideo ? 'video' : 'audio'
    const prevProgress = library.progress.get(track.id)
    const resumeMs = opts?.fromLast ? prevProgress?.positionMs ?? 0 : 0
    if (!library.progress.has(track.id)) library.updateProgressLocal(track.id, 0)
    pushRecent(track.id)
    duration.value = track.duration || 0
    buffered.value = 0
    lyrics.value = []
    lyricsInfo.value = { lines: [], source: null, format: null }
    bookmarks.value = []
    error.value = ''
    void refreshBookmarks(track.id)
    if (!await loadTrack(track, resumeMs, true)) {
      // 加载失败：不查询字幕轨、不启动进度写入，避免为无法播放的条目写入假进度
      playing.value = false
      return
    }
    // 字幕轨要等 mpv 载入文件后才有，稍后刷新
    window.setTimeout(() => void refreshSubtitles(), 900)
    await api.playerApplyLoudnessGain(
      settings.loudnessEnabled ? (track.loudnessLufs ?? null) : null,
      settings.loudnessTarget,
    )
    startProgressWriter()
    // mpv 此时已初始化完成：此时才能枚举到全部输出设备，刷新一次设备列表
    void refreshDevices()
    // 视频舞台几何在 StageView 挂载时同步；这里若已在舞台视图先刷新一次
    void api.playerSetVideoEnabled(videoMode.value === 'video' && track.isVideo && !audioOnly.value).catch(() => false)
  }

  /** 在当前队列中定位播放 */
  async function playFromList(list: string[], index: number, name: string, opts?: { fromLast?: boolean; video?: boolean }): Promise<void> {
    if (index < 0 || index >= list.length) return
    const track = library.trackIndex.get(list[index])
    if (!track) return
    await playTrack(track, list, name, opts)
  }

  function togglePlay(): void {
    if (!current.value) return
    void api.playerToggle(playing.value)
    // 乐观更新，事件到达后校正
    playing.value = !playing.value
    if (playing.value === false) writeProgress(false)
  }

  async function next(): Promise<void> {
    if (!queue.value.length) return
    const idx = queue.value.indexOf(currentId.value ?? '')
    const n = nextIndex(settings.mode, idx, queue.value.length)
    if (n === null) return
    await playFromList(queue.value, n, queueName.value, { fromLast: true, video: videoMode.value === 'video' })
  }

  async function prev(): Promise<void> {
    if (!queue.value.length) return
    const idx = queue.value.indexOf(currentId.value ?? '')
    const p = prevIndex(settings.mode, idx, queue.value.length)
    if (p === null) return
    await playFromList(queue.value, p, queueName.value, { fromLast: true, video: videoMode.value === 'video' })
  }

  function seek(sec: number): void {
    const target = Math.max(0, Math.min(sec, duration.value || sec))
    position.value = target
    void api.playerSeek(target)
    writeProgress(false)
  }

  function seekBy(delta: number): void {
    seek(position.value + delta)
  }

  // ── 进度记忆（每 5 秒 + 暂停/切歌/结束时回写）─────────────────
  function writeProgress(finished: boolean): void {
    const id = currentId.value
    if (!id) return
    const completed = completedOnWrite(position.value, duration.value || 0, finished)
    void api.setProgress(id, Math.round(position.value * 1000), completed ?? null, settings.speed)
    library.updateProgressLocal(id, Math.round(position.value * 1000), completed)
  }

  function startProgressWriter(): void {
    stopProgressWriter()
    progressTimer = setInterval(() => {
      if (playing.value && currentId.value) writeProgress(false)
    }, 5000)
  }

  function stopProgressWriter(): void {
    if (progressTimer) {
      clearInterval(progressTimer)
      progressTimer = null
    }
  }

  // ── mpv 事件（唯一事实源）──
  function initEventListeners(): void {
    onEvent<MpvPlayerState>('player:state', applyMpvState)
    onEvent<unknown>('player:loading', () => { loading.value = true })
    onEvent<{ reason: string }>('player:ended', ({ reason }) => {
      if (reason === 'error') {
        error.value = '解码失败，已跳到下一条'
        if (queue.value.length > 1) void next()
        return
      }
      if (reason === 'stop') return
      if (ending) return
      ending = true
      // EOF：先取最终位置再走播放模式逻辑
      writeProgress(true)
      const behavior = onEnded(settings.mode)
      // 睡眠定时「播完当前再停」
      if (sleep.value.active && sleep.value.action === 'finishTrack' && sleepRemaining.value <= 0) {
        stopForSleep()
        return
      }
      if (behavior === 'repeat') {
        ending = false
        void api.playerSeek(0)
        void api.playerToggle(false)
      } else if (behavior === 'stop') {
        playing.value = false
      } else {
        void next()
      }
    })
    onEvent<{ message: string }>('player:error', ({ message }) => {
      error.value = message
      playing.value = false
      loading.value = false
    })
  }

  // ── 字幕（mpv + libass 属性遥控）─────────────────────────────
  async function refreshSubtitles(): Promise<void> {
    try {
      subtitleTracks.value = await api.playerSubtitleTracks()
      if (subtitleTracks.value.length === 0) subtitleVisible.value = true
    } catch {
      subtitleTracks.value = []
    }
  }

  async function selectSubtitle(id: number): Promise<void> {
    subtitleTracks.value = subtitleTracks.value.map((t) => ({ ...t, selected: t.id === id }))
    await api.playerSetSubtitleTrack(id)
    subtitleVisible.value = id > 0
  }

  async function toggleSubtitleVisible(): Promise<void> {
    subtitleVisible.value = !subtitleVisible.value
    await api.playerSetSubtitleVisible(subtitleVisible.value)
  }

  /** 偏移步进（秒），用于字幕提前/延后 */
  async function nudgeSubtitleDelay(delta: number): Promise<void> {
    subtitleDelay.value = Math.round((subtitleDelay.value + delta) * 100) / 100
    await api.playerSetSubtitleDelay(subtitleDelay.value)
  }

  /** 字幕样式（mpv sub-* 属性遥控） */
  async function setSubStyle(style: SubStyle): Promise<void> {
    await api.playerSetSubStyle(style)
  }

  /** 响度均衡：按目标 LUFS 与当前曲目 loudness_lufs 差值设置增益 */
  async function applyLoudnessGain(targetLufs: number): Promise<void> {
    const lufs = current.value?.loudnessLufs
    await api.playerApplyLoudnessGain(lufs ?? null, targetLufs)
  }

  // ── 音量 / 倍速 ──
  function setVolume(v: number): void {
    settings.vol = Math.max(0, Math.min(1, v))
    if (settings.vol > 0) settings.muted = false
    void api.playerSetVolume(settings.vol)
    void api.playerSetMuted(settings.muted)
    settings.scheduleSave()
  }
  function toggleMute(): void {
    settings.muted = !settings.muted
    void api.playerSetMuted(settings.muted)
    settings.scheduleSave()
  }
  function setSpeed(s: number): void {
    settings.speed = Math.max(0.5, Math.min(2, Math.round(s * 100) / 100))
    void api.playerSetSpeed(settings.speed)
    settings.scheduleSave()
  }
  function cycleSpeed(dir: 1 | -1): void {
    setSpeed(settings.speed + dir * 0.05)
  }

  // ── 仅音频模式（P0-30：vid=no，引擎始终是 mpv）──
  function setAudioOnly(on: boolean): void {
    audioOnly.value = on
    void api.playerSetVideoEnabled(!on).catch(() => false)
  }

  // ── 睡眠定时 ──
  function setSleepTimer(minutes: number, action: SleepState['action'] = 'pause'): void {
    sleepFading = false
    if (minutes <= 0) {
      sleep.value = { active: false, endAt: 0, action, triggerOnEnded: false }
      if (sleepTicker) { clearInterval(sleepTicker); sleepTicker = null }
      return
    }
    sleep.value = { active: true, endAt: Date.now() + minutes * 60000, action, triggerOnEnded: false }
    if (sleepTicker) clearInterval(sleepTicker)
    sleepTicker = setInterval(tickSleep, 1000)
  }

  let sleepFading = false // 防止到点后 tick 每秒重复触发 fadeOut

  function tickSleep(): void {
    if (!sleep.value.active || sleepFading) return
    sleepRemaining.value = Math.max(0, sleep.value.endAt - Date.now())
    if (sleepRemaining.value <= 0) {
      if (sleep.value.action === 'finishTrack') {
        if (!playing.value) stopForSleep()
      } else {
        sleepFading = true
        void fadeOutAndStop(sleep.value.action === 'stop').finally(() => { sleepFading = false })
      }
    }
  }

  async function fadeOutAndStop(hardStop: boolean): Promise<void> {
    const startVol = settings.vol
    const t0 = Date.now()
    const DUR = 5000
    await new Promise<void>((resolve) => {
      const step = () => {
        const k = Math.min(1, (Date.now() - t0) / DUR)
        void api.playerSetVolume(startVol * (1 - k))
        if (k >= 1) resolve()
        else setTimeout(step, 100)
      }
      step()
    })
    if (hardStop) {
      stopProgressWriter()
      void api.playerStop()
      currentId.value = null
      playing.value = false
    } else {
      void api.playerToggle(true)
      void api.playerSetVolume(startVol)
      playing.value = false
      writeProgress(false)
    }
    setSleepTimer(0, sleep.value.action)
  }

  function stopForSleep(): void {
    void api.playerToggle(true)
    playing.value = false
    writeProgress(false)
    setSleepTimer(0, sleep.value.action)
  }

  // ── 书签 ──
  async function refreshBookmarks(trackId: string): Promise<void> {
    try { bookmarks.value = await api.listBookmarks(trackId) } catch { bookmarks.value = [] }
  }

  async function addBookmarkAt(label: string): Promise<void> {
    const id = currentId.value
    if (!id) return
    const bm = await api.addBookmark(id, Math.round(position.value * 1000), label)
    if (bm) bookmarks.value = [...bookmarks.value, bm].sort((a, b) => a.atMs - b.atMs)
  }

  async function removeBookmarkById(id: string): Promise<void> {
    if (await api.removeBookmark(id)) {
      bookmarks.value = bookmarks.value.filter((b) => b.id !== id)
    }
  }

  async function renameBookmarkById(id: string, label: string): Promise<void> {
    if (await api.renameBookmark(id, label)) {
      bookmarks.value = bookmarks.value.map((b) => (b.id === id ? { ...b, label } : b))
    }
  }

  // ── 输出设备 ──
  async function setDevice(id: string): Promise<void> {
    settings.devId = id
    settings.scheduleSave()
    await api.playerSetDevice(id)
  }

  function init(): void {
    initEventListeners()
    void refreshDevices()
  }

  return {
    queue, queueName, currentId, current, currentTitle, currentArtist,
    playing, position, duration, buffered, loading, videoMode, audioOnly, error,
    lyrics, lyricsInfo, devices, deviceOpen,
    subtitleTracks, subtitleVisible, subtitleDelay, refreshSubtitles,
    selectSubtitle, toggleSubtitleVisible, nudgeSubtitleDelay, setSubStyle, applyLoudnessGain,
    sleep, sleepRemaining, bookmarks,
    playTrack, playFromList, togglePlay, next, prev, seek, seekBy,
    setVolume, toggleMute, setSpeed, cycleSpeed, setAudioOnly,
    setSleepTimer, refreshDevices, setDevice,
    addBookmarkAt, removeBookmarkById, renameBookmarkById, refreshBookmarks,
    init,
  }
})
