/** 全局键盘快捷键（P0-7）：可在设置中关闭；输入框聚焦时跳过播放类按键。 */
import { onMounted, onBeforeUnmount } from 'vue'
import { usePlayerStore } from '@/stores/player'
import { useSettingsStore } from '@/stores/settings'
import { useUiStore } from '@/stores/ui'
import { stepSpeed } from '@/utils/speedPresets'
import { useLibraryStore } from '@/stores/library'

function isTypingTarget(e: KeyboardEvent): boolean {
  const el = e.target as HTMLElement | null
  if (!el) return false
  const tag = el.tagName
  return tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || el.isContentEditable
}

export function useKeyboardShortcuts(): void {
  const player = usePlayerStore()
  const settings = useSettingsStore()
  const ui = useUiStore()
  const library = useLibraryStore()

  const handler = (e: KeyboardEvent): void => {
    if (!settings.keyboardEnabled) return
    if (e.ctrlKey && e.key.toLowerCase() === 'f') {
      e.preventDefault()
      const input = document.querySelector<HTMLInputElement>('#titlebar input[type="text"]')
      input?.focus()
      input?.select()
      return
    }
    if (isTypingTarget(e)) return
    if (e.ctrlKey || e.metaKey || e.altKey) return

    const shift = e.shiftKey
    switch (e.key) {
      case ' ':
        e.preventDefault()
        player.togglePlay()
        break
      case 'ArrowLeft':
        e.preventDefault()
        player.seekBy(shift ? -30 : -5)
        break
      case 'ArrowRight':
        e.preventDefault()
        player.seekBy(shift ? 30 : 5)
        break
      case 'ArrowUp':
        e.preventDefault()
        player.setVolume(settings.vol + 0.02)
        break
      case 'ArrowDown':
        e.preventDefault()
        player.setVolume(settings.vol - 0.02)
        break
      case ',':
        player.setSpeed(stepSpeed(settings.speed, -1))
        break
      case '.':
        player.setSpeed(stepSpeed(settings.speed, 1))
        break
      case 'Escape':
        // 浏览器全屏时 Esc 交给浏览器退出全屏（P0-3：退出全屏而非退出播放）
        if (document.fullscreenElement) return
        ui.closeAllPanels()
        ui.ctxMenu = null
        ui.settingsOpen = false
        if (ui.view === 'lyrics' || ui.view === 'stage') ui.view = 'smart'
        break
      case 'm':
      case 'M':
        player.toggleMute()
        break
      case 'b':
      case 'B': {
        if (player.currentId) void player.addBookmarkAt('')
        break
      }
    }
  }

  onMounted(() => window.addEventListener('keydown', handler))
  onBeforeUnmount(() => window.removeEventListener('keydown', handler))
  void library
}
