/** 颜色转换工具（hex ↔ hsv）+ 感知亮度（P0-68 亮度自适应）。 */

export function hexToRgb(hex: string): [number, number, number] {
  const m = hex.replace('#', '')
  const full = m.length === 3 ? m.split('').map((c) => c + c).join('') : m
  const n = parseInt(full.slice(0, 6), 16)
  if (isNaN(n)) return [0, 0, 0]
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255]
}

export function rgbToHex(r: number, g: number, b: number): string {
  return '#' + [r, g, b].map((v) => Math.max(0, Math.min(255, Math.round(v))).toString(16).padStart(2, '0')).join('')
}

export function hexToHsv(hex: string): { h: number; s: number; v: number } {
  const [r0, g0, b0] = hexToRgb(hex)
  const r = r0 / 255
  const g = g0 / 255
  const b = b0 / 255
  const max = Math.max(r, g, b)
  const min = Math.min(r, g, b)
  const d = max - min
  let h = 0
  if (d > 0) {
    if (max === r) h = 60 * (((g - b) / d) % 6)
    else if (max === g) h = 60 * ((b - r) / d + 2)
    else h = 60 * ((r - g) / d + 4)
  }
  if (h < 0) h += 360
  return { h, s: max === 0 ? 0 : d / max, v: max }
}

export function hsvToHex(h: number, s: number, v: number): string {
  const c = v * s
  const hp = h / 60
  const x = c * (1 - Math.abs((hp % 2) - 1))
  let r = 0, g = 0, b = 0
  if (hp < 1) [r, g, b] = [c, x, 0]
  else if (hp < 2) [r, g, b] = [x, c, 0]
  else if (hp < 3) [r, g, b] = [0, c, x]
  else if (hp < 4) [r, g, b] = [0, x, c]
  else if (hp < 5) [r, g, b] = [x, 0, c]
  else [r, g, b] = [c, 0, x]
  const m = v - c
  return rgbToHex((r + m) * 255, (g + m) * 255, (b + m) * 255)
}

// ── 感知亮度（P0-68 亮度自适应）──

/** 判「亮底 / 暗底」的阈值：与 02 §P0-68 的 50×50 平均亮度 >128 一致 */
export const BRIGHT_INK_THRESHOLD = 128

/** rec.601 感知亮度（0–255）。与 Rust `bgimage::luma_of_rgb` 同式 ——
 *  跨语言无法共享常量，改一处必须同步另一处，否则阈值在两侧含义不同。 */
export function perceivedLuma(r: number, g: number, b: number): number {
  return 0.299 * r + 0.587 * g + 0.114 * b
}

/** hex（`#rgb` / `#rrggbb`）→ 感知亮度。非法输入返回 null，调用方据此回落到主题墨色。 */
export function hexLuma(hex: string): number | null {
  const m = hex.trim().replace(/^#/, '')
  if (!/^([0-9a-f]{3}|[0-9a-f]{6})$/i.test(m)) return null
  const [r, g, b] = hexToRgb(m)
  return perceivedLuma(r, g, b)
}

/**
 * UI 真正压在什么颜色上：背景图以 `opacity: 1 - ovl` 画在主题底色之上，
 * 所以合成亮度 = (1 - ovl)·图亮度 + ovl·底色亮度。
 *
 * `imgLuma` 缺失（没有背景图，或 Rust 侧解码失败）时退化为底色亮度 —— 判据仍然成立，
 * 只是「跟主题走」。`baseHex` 解析不出来时返回 null：宁可回落到主题墨色，
 * 也不要猜错方向把黑字压在黑底上。
 */
export function compositeLuma(imgLuma: number | null | undefined, ovl: number, baseHex: string): number | null {
  const base = hexLuma(baseHex)
  if (imgLuma === null || imgLuma === undefined) return base
  if (base === null) return imgLuma
  const a = Math.max(0, Math.min(1, ovl))
  return (1 - a) * imgLuma + a * base
}
