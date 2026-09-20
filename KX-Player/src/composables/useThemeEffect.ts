/** 主题应用：把 settings 写入 CSS 变量（accent 派生 + 透明度 + 背景层）。 */
import { watch } from 'vue'
import { useSettingsStore } from '@/stores/settings'
import { hexToRgb } from '@/utils/color'

export function useThemeEffect(): void {
  const settings = useSettingsStore()

  const apply = (): void => {
    const root = document.documentElement
    root.setAttribute('data-theme', settings.theme)

    const [r, g, b] = hexToRgb(settings.clr || '#E63A2E')
    root.style.setProperty('--accent-rgb', `${r} ${g} ${b}`)
    root.style.setProperty('--tb-alpha', String(settings.titlebarOpacity))
    root.style.setProperty('--sb-alpha', String(settings.sidebarOpacity))
    root.style.setProperty('--pb-alpha', String(settings.playerOpacity))
    root.style.setProperty('--bg-overlay', String(settings.ovl))
  }

  watch(
    () => [
      settings.theme, settings.clr,
      settings.titlebarOpacity, settings.sidebarOpacity, settings.playerOpacity,
      settings.ovl,
    ],
    apply,
    { immediate: true },
  )
}
