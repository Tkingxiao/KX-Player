---
project: KX-Player
register: product
aesthetic_direction: technical / utilitarian + aurora dark
color_strategy: restrained
design_system: bespoke (Vue 3 + scoped CSS + CSS variables)
design_variance: 5
motion_intensity: 4
visual_density: 7
---

# KX-Player Design Language

## Design Read
"A focused, low-light cockpit for long-form ASMR media — quiet surfaces, one warm accent, density where it matters, and motion that never interrupts sleep."

## Signature
The **aurora progress thumb**: a soft, warm accent glow on the playback progress bar and a subtle top-edge sheen on the active sidebar item. It says "this is playing now" without shouting. Everything else stays muted so the media stays central.

## Inspiration
- PotPlayer / mpv: control density, immediate access, no decorative chrome.
- foobar2000: information-rich lists that still feel light.
- Rejected: generic glassmorphism, purple/blue tech glow, oversized hero imagery.

## Color (locked)

All colors derived from a single accent seed. Neutrals are tinted warm (hue ~30) in dark mode and cool (hue ~260) in light mode to avoid generic gray.

| role | OKLCH | hex | use |
|------|-------|-----|-----|
| Dark bg | oklch(13% 0.01 285) | #0d0d12 | app background |
| Dark surface | oklch(17% 0.012 285 / 0.92) | #16161e | cards, panels |
| Dark surface-elevated | oklch(21% 0.014 285 / 0.98) | #1c1c26 | popovers, modals |
| Dark text | oklch(96% 0.002 285) | #f0f0f5 | primary text |
| Dark text-sub | oklch(75% 0.008 285) | #b8b8c2 | secondary text |
| Dark text-muted | oklch(52% 0.006 285) | #6e6e7a | hints, disabled |
| Dark border | oklch(100% 0.005 285 / 0.10) | rgba(255,255,255,0.10) | dividers |
| Dark border-strong | oklch(100% 0.005 285 / 0.18) | rgba(255,255,255,0.18) | focus, active |
| Accent | oklch(58% 0.22 25) | #E63A2E | active state, progress, play button |
| Accent glow | oklch(58% 0.22 25 / 0.35) | rgba(230,58,46,0.35) | sheen on active items |
| Success | oklch(65% 0.18 145) | #40c878 | completed, success toast |
| Warning | oklch(75% 0.14 85) | #f0b84d | attention |
| Danger | oklch(60% 0.18 25) | #e6685f | destructive actions |
| Light bg | oklch(96% 0.008 260) | #f2f2f7 | light mode background |
| Light surface | oklch(100% 0.006 260 / 0.92) | #ffffff | cards, panels |
| Light text | oklch(18% 0.008 260) | #17171c | primary text |
| Light text-sub | oklch(40% 0.012 260) | #3a3a3c | secondary text |
| Light text-muted | oklch(55% 0.010 260) | #6e6e7a | hints |

Contrast: all text/UI pairings target WCAG AA (4.5:1 for body, 3:1 for large text/UI).

## Type (locked)

| role | family | use | notes |
|------|--------|-----|-------|
| body / UI | Inter, 'Noto Sans SC', 'Yu Gothic UI', Meiryo | all interface text | metric-matched fallback |
| data | Inter | times, durations, counts | `font-variant-numeric: tabular-nums` |

Scale:
- page title: 18px / 600
- section title: 11px uppercase / 500 / letter-spacing 0.06em
- card title: 13px / 500
- card sub: 11px
- list row: 13px
- control label: 12px
- caption: 10.5px

## Scales (locked)

Spacing: 0, 2, 4, 6, 8, 10, 12, 16, 20, 24, 32, 40, 48, 64 px

Layout constants:
- titlebar-h: 36px
- sidebar-w: 232px (range 180–360)
- sidebar-w-collapsed: 56px
- toolbar-h: 44px
- inspector-w: 320px (range 260–480)
- playerbar-h: 76px
- grid-min: 176px
- card-row-h: 232px
- row-h: 46px
- gap: 12px

Radius:
- sm: 6px
- md: 10px
- lg: 16px
- full: 999px

Shadow:
- card: 0 6px 24px rgba(0,0,0,0.4)
- pop: 0 12px 40px rgba(0,0,0,0.55)

Motion:
- fast: 120ms
- base: 200ms
- slow: 360ms
- ease: cubic-bezier(0.4, 0, 0.2, 1)
- Only opacity and transform. No width/height/box-shadow animation.
- Honor `prefers-reduced-motion`.

## Voice
- Plain, direct, tool-like. No buzzwords.
- Action vocabulary consistent: "导入", "扫描", "播放", "转换", "收藏".
- Empty states explain why and give one next action.

## Every screen must read as the same product if placed side by side.
