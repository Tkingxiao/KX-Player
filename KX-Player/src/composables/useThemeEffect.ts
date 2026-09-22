/** 主题应用：把 settings 写入 CSS 变量（accent 派生 + 透明度 + 背景层 + P0-68 亮度自适应）。 */
import { watch } from 'vue'
import { useSettingsStore } from '@/stores/settings'
import { BRIGHT_INK_THRESHOLD, compositeLuma, hexToRgb } from '@/utils/color'

/**
 * P0-68 判据的唯一计算处：UI 此刻该用亮底墨色（1 → 纯黑）还是暗底墨色（0 → 纯白），
 * 判不出来（主题底色解析失败）时返回 null —— 不写属性，墨色跟随主题。
 *
 * 背景图的亮度由 Rust 侧采样（`bgimage::sample_luma`）：图片走 `asset:` 协议，与页面跨源，
 * 前端 canvas 会被 taint，`getImageData` 必然抛 SecurityError —— 旧版自适应因此恒判深底。
 * 设置弹窗的「当前深底/浅底」提示读同一个函数，提示与实际不可能说两套话。
 */
export function bgInkBright(): 0 | 1 | null {
  const settings = useSettingsStore()
  // --bg 由 data-theme 决定，而 computed style 不是响应式的：显式读一次主题，
  // 让调用方的 computed 在切主题时失效重算，否则提示会停在旧主题上。
  void settings.theme
  const base = getComputedStyle(document.documentElement).getPropertyValue('--bg')
  const luma = compositeLuma(settings.bgLuma, settings.ovl, base)
  return luma === null ? null : luma > BRIGHT_INK_THRESHOLD ? 1 : 0
}

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

    const bright = bgInkBright()
    if (bright === null) root.removeAttribute('data-bg-bright')
    else root.setAttribute('data-bg-bright', String(bright))
  }

  watch(
    () => [
      settings.theme, settings.clr,
      settings.titlebarOpacity, settings.sidebarOpacity, settings.playerOpacity,
      settings.ovl, settings.bgLuma,
    ],
    apply,
    { immediate: true },
  )
}
