// === Color Utilities ===

export function hex2rgb(h) {
  const v = parseInt(h.slice(1), 16)
  return [(v >> 16) & 255, (v >> 8) & 255, v & 255]
}

export function rgb2hsl(r, g, b) {
  r /= 255; g /= 255; b /= 255
  const M = Math.max(r, g, b), m = Math.min(r, g, b), d = M - m, l = (M + m) / 2
  let h = 0, s = d === 0 ? 0 : l > 0.5 ? d / (2 - M - m) : d / (M + m)
  if (d !== 0) {
    if (M === r) h = ((g - b) / d + (g < b ? 6 : 0)) * 60
    else if (M === g) h = ((b - r) / d + 2) * 60
    else h = ((r - g) / d + 4) * 60
  }
  return { h: Math.round(h), s: Math.round(s * 100), l: Math.round(l * 100) }
}

export function h2hsl(hex) {
  const [r, g, b] = hex2rgb(hex)
  return rgb2hsl(r, g, b)
}

export function hsvToRgb(h, s, v) {
  s /= 100; v /= 100
  const c = v * s
  const x = c * (1 - Math.abs((h / 60) % 2 - 1))
  const m = v - c
  let r, g, b
  if (h < 60)       [r, g, b] = [c, x, 0]
  else if (h < 120) [r, g, b] = [x, c, 0]
  else if (h < 180) [r, g, b] = [0, c, x]
  else if (h < 240) [r, g, b] = [0, x, c]
  else if (h < 300) [r, g, b] = [x, 0, c]
  else              [r, g, b] = [c, 0, x]
  return [
    Math.round((r + m) * 255),
    Math.round((g + m) * 255),
    Math.round((b + m) * 255),
  ]
}
