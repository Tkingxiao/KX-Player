<script setup lang="ts">
/** 设置中心：主题 / 主题色（HSV 取色器+预设）/ 三处透明度 / 背景图（选择、缩放、拖拽、模糊、明度自适应）/ 快捷键开关。 */
import { ref, computed, watch, onMounted, nextTick } from 'vue'
import { useSettingsStore } from '@/stores/settings'
import { usePlayerStore } from '@/stores/player'
import { useLibraryStore } from '@/stores/library'
import { useUiStore } from '@/stores/ui'
import { api, assetUrl } from '@/bridge/ipc'
import { hexToHsv, hsvToHex } from '@/utils/color'

const settings = useSettingsStore()
const player = usePlayerStore()
const library = useLibraryStore()
const ui = useUiStore()

// ── 响度分析（后台 ffmpeg ebur128）──
const missingLoudnessCount = computed(() => library.allTracks.filter((t) => t.loudnessLufs == null).length)
const loudnessPct = computed(() =>
  library.loudnessScan.total > 0 ? Math.round((library.loudnessScan.completed / library.loudnessScan.total) * 100) : 0,
)
function startLoudness(): void {
  if (!library.startLoudnessAnalysis(true)) ui.toast('没有待分析的条目', 'info')
}

const PRESETS = ['#7c6cf6', '#e63a2e', '#e67e22', '#f1c40f', '#2ecc71', '#1abc9c', '#3498db', '#e84393']

// ── 取色器 ──
const hsv = ref({ h: 250, s: 0.55, v: 0.96 })
const hueCanvas = ref<HTMLCanvasElement | null>(null)
const svCanvas = ref<HTMLCanvasElement | null>(null)

watch(() => settings.clr, (hex) => {
  hsv.value = hexToHsv(hex)
}, { immediate: true })

function applyHsv(): void {
  settings.clr = hsvToHex(hsv.value.h, hsv.value.s, hsv.value.v)
  settings.scheduleSave()
}

function drawHue(): void {
  const c = hueCanvas.value
  if (!c) return
  const ctx = c.getContext('2d')
  if (!ctx) return
  const r = c.width / 2
  ctx.clearRect(0, 0, c.width, c.height)
  for (let deg = 0; deg < 360; deg++) {
    const a0 = ((deg - 0.6) * Math.PI) / 180
    const a1 = ((deg + 0.6) * Math.PI) / 180
    ctx.beginPath()
    ctx.moveTo(r, r)
    ctx.arc(r, r, r - 1, a0, a1)
    ctx.closePath()
    ctx.fillStyle = `hsl(${deg}, 90%, 55%)`
    ctx.fill()
  }
}

function drawSv(): void {
  const c = svCanvas.value
  if (!c) return
  const ctx = c.getContext('2d')
  if (!ctx) return
  const w = c.width
  const h = c.height
  const img = ctx.createImageData(w, h)
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const s = x / w
      const v = 1 - y / h
      const [r, g, b] = hsvToRgb(hsv.value.h, s, v)
      const i = (y * w + x) * 4
      img.data[i] = r
      img.data[i + 1] = g
      img.data[i + 2] = b
      img.data[i + 3] = 255
    }
  }
  ctx.putImageData(img, 0, 0)
}

function hsvToRgb(h: number, s: number, v: number): [number, number, number] {
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
  return [Math.round((r + m) * 255), Math.round((g + m) * 255), Math.round((b + m) * 255)]
}

function pickHue(e: MouseEvent): void {
  const c = hueCanvas.value
  if (!c) return
  const rect = c.getBoundingClientRect()
  const dx = e.clientX - rect.left - rect.width / 2
  const dy = e.clientY - rect.top - rect.height / 2
  hsv.value = { ...hsv.value, h: (Math.atan2(dy, dx) * 180) / Math.PI + 90 + 360 }
  hsv.value = { ...hsv.value, h: ((hsv.value.h % 360) + 360) % 360 }
  applyHsv()
}

function pickSv(e: MouseEvent): void {
  const c = svCanvas.value
  if (!c) return
  const rect = c.getBoundingClientRect()
  hsv.value = {
    ...hsv.value,
    s: Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width)),
    v: Math.max(0, Math.min(1, 1 - (e.clientY - rect.top) / rect.height)),
  }
  applyHsv()
}

watch(() => hsv.value.h, () => drawSv())

// 弹窗打开时 canvas 才挂载，需要在打开后绘制色轮与 SV 方块
watch(() => ui.settingsOpen, async (open) => {
  if (!open) return
  await nextTick()
  drawHue()
  drawSv()
})

onMounted(() => {
  drawHue()
  drawSv()
})

// ── 背景图 ──
/** 适配方式：与 CSS object-fit 语义对齐 */
const BG_FIT_MODES = [
  { value: 'cover', label: '填充', hint: '等比缩放铺满，裁掉溢出部分（默认）' },
  { value: 'contain', label: '适应', hint: '等比缩放到完整可见，可能留白' },
  { value: 'stretch', label: '拉伸', hint: '拉伸到窗口尺寸，不保持比例' },
  { value: 'center', label: '居中', hint: '原始尺寸居中显示，不缩放' },
  { value: 'tile', label: '平铺', hint: '以原始尺寸重复平铺' },
] as const
// 必须走 asset 协议（convertFileSrc）：CSP 的 img-src 不允许 file://，否则 <img> 显示破图图标。
const bgUrl = computed(() => {
  if (!settings.bgPath) return null
  const base = assetUrl(settings.bgPath)
  if (!base) return null
  return base + '?v=' + settings.bgMtime
})

/** 预览样式：与主界面背景层保持一致（适配方式 + 透明度 + 模糊 + 缩放平移） */
const bgPreviewStyle = computed<Record<string, string>>(() => {
  const fit = settings.bgSize
  const style: Record<string, string> = {
    opacity: String(Math.max(0, Math.min(1, 1 - settings.ovl))),
    filter: settings.bgBlur > 0 ? `blur(${settings.bgBlur}px)` : '',
  }
  if (fit === 'stretch') style.objectFit = 'fill'
  else if (fit === 'contain') style.objectFit = 'contain'
  else if (fit === 'cover') style.objectFit = 'cover'
  else if (fit === 'center') { style.objectFit = 'none'; style.objectPosition = 'center' }
  else if (fit === 'tile') {
    // 平铺：<img> 不支持 CSS repeat，此处仅隐藏 img；
    // 实际平铺由 App.vue 的 #bg-layer background-image 实现
    style.display = 'none'
  }
  const e = settings.imgEditState
  if (e && fit !== 'tile' && fit !== 'center') {
    style.transform = `translate(${(e.posX - 50) * 0.8}%, ${(e.posY - 50) * 0.8}%) scale(${e.zoomPct / 100})`
  }
  return style
})

async function pickBg(): Promise<void> {
  const picked = await api.selectBgImage()
  if (picked) {
    settings.bgPath = picked.path
    settings.bgMtime = picked.mtime ?? Date.now()
    settings.imgEditState = null
    settings.scheduleSave()
    settings.flushOnExit()
  }
}

async function removeBg(): Promise<void> {
  await api.removeBgImage()
  settings.bgPath = ''
  // 清空 mtime：否则残留的 ?v= 会让派生 URL 看似未变，UI 不刷新
  settings.bgMtime = 0
  settings.imgEditState = null
  ui.bgEditorOpen = false
  settings.scheduleSave()
  // 立刻同步落盘，避免「重启后背景图还在」
  settings.flushOnExit()
}

// ── 明度自适应（背景图改变时重采样）──
const textLight = ref<0 | 1>(1)
async function sampleBrightness(): Promise<void> {
  if (!bgUrl.value) { textLight.value = 1; return }
  const img = new Image()
  img.src = bgUrl.value
  await new Promise<void>((resolve) => {
    img.onload = () => resolve()
    img.onerror = () => resolve()
  })
  // 图片加载失败（文件缺失/路径失效）时保持默认文字色
  if (!img.complete || !img.naturalWidth) { textLight.value = 1; return }
  try {
    const c = document.createElement('canvas')
    c.width = 50
    c.height = 50
    const ctx = c.getContext('2d')
    if (!ctx) return
    ctx.drawImage(img, 0, 0, 50, 50)
    const data = ctx.getImageData(0, 0, 50, 50).data
    let sum = 0
    for (let i = 0; i < data.length; i += 4) {
      sum += 0.299 * data[i] + 0.587 * data[i + 1] + 0.114 * data[i + 2]
    }
    textLight.value = sum / (data.length / 4) > 128 ? 0 : 1
  } catch { textLight.value = 1 }
}
watch(bgUrl, () => void sampleBrightness(), { immediate: true })

const themeRows = [
  { key: 'titlebarOpacity', label: '标题栏不透明度' },
  { key: 'sidebarOpacity', label: '侧边栏不透明度' },
  { key: 'playerOpacity', label: '播放栏不透明度' },
] as const
</script>

<template>
  <Teleport to="body">
    <Transition name="modal">
      <div v-if="ui.settingsOpen" class="settings-overlay" @mousedown.self="ui.settingsOpen = false">
        <div class="modal-card settings-card">
          <div class="settings-head">
            <h2>外观与设置</h2>
            <button class="icon-btn" title="关闭" @click="ui.settingsOpen = false">✕</button>
          </div>

          <div class="settings-body">
            <!-- 主题 -->
            <section class="set-section">
              <h3 class="section-title">主题</h3>
              <div class="theme-switch">
                <button class="theme-opt" :class="{ active: settings.theme === 'dark' }" @click="settings.theme = 'dark'; settings.scheduleSave()">
                  <span class="theme-dot dark-dot" /> 深色
                </button>
                <button class="theme-opt" :class="{ active: settings.theme === 'light' }" @click="settings.theme = 'light'; settings.scheduleSave()">
                  <span class="theme-dot light-dot" /> 浅色
                </button>
              </div>
            </section>

            <!-- 主题色 -->
            <section class="set-section">
              <h3 class="section-title">主题色</h3>
              <div class="accent-row">
                <div class="color-wheel-wrap">
                  <canvas ref="hueCanvas" class="hue-canvas" width="120" height="120" @click="pickHue" />
                  <span class="hue-pointer" :style="{ transform: `rotate(${hsv.h - 90}deg)` }" />
                </div>
                <div class="sv-wrap">
                  <canvas ref="svCanvas" class="sv-canvas" width="150" height="110" @click="pickSv" />
                </div>
                <div class="accent-current" :style="{ background: settings.clr }" />
              </div>
              <div class="preset-row">
                <button
                  v-for="p in PRESETS"
                  :key="p"
                  class="preset-dot"
                  :class="{ active: settings.clr.toLowerCase() === p }"
                  :style="{ background: p }"
                  @click="settings.clr = p; settings.scheduleSave()"
                />
              </div>
            </section>

            <!-- 透明度 -->
            <section class="set-section">
              <h3 class="section-title">面板不透明度</h3>
              <div v-for="row in themeRows" :key="row.key" class="slider-row">
                <span class="slider-label">{{ row.label }}</span>
                <input
                  type="range"
                  min="0"
                  max="1"
                  step="0.01"
                  :value="settings[row.key]"
                  @input="settings[row.key] = Number(($event.target as HTMLInputElement).value); settings.scheduleSave()"
                />
                <span class="slider-val tnum">{{ Math.round(settings[row.key] * 100) }}%</span>
              </div>
            </section>

            <section class="set-section">
              <h3 class="section-title">播放</h3>
              <label class="check-row">
                <input
                  v-model="settings.loudnessEnabled"
                  type="checkbox"
                  @change="player.applyLoudnessGain(settings.loudnessTarget); settings.scheduleSave()"
                />
                响度均衡
              </label>
              <div class="slider-row" :class="{ disabled: !settings.loudnessEnabled }">
                <span class="slider-label">目标响度</span>
                <input
                  v-model.number="settings.loudnessTarget"
                  type="range"
                  min="-30"
                  max="-12"
                  step="1"
                  :disabled="!settings.loudnessEnabled"
                  @input="player.applyLoudnessGain(settings.loudnessTarget); settings.scheduleSave()"
                />
                <span class="slider-val tnum">{{ settings.loudnessTarget }} LUFS</span>
              </div>
              <p class="set-hint">仅对已分析出响度的条目自动调整；未分析条目保持原音量。</p>
              <div class="loudness-row">
                <template v-if="!library.loudnessScan.active">
                  <button class="btn-ghost" @click="startLoudness">
                    分析缺失响度（{{ missingLoudnessCount }} 条）
                  </button>
                  <span v-if="!missingLoudnessCount" class="set-hint">全部条目已分析</span>
                </template>
                <template v-else>
                  <div class="loudness-progress">
                    <div class="loudness-bar">
                      <div class="loudness-bar-fill" :style="{ width: loudnessPct + '%' }" />
                    </div>
                    <span class="tnum">{{ library.loudnessScan.completed }}/{{ library.loudnessScan.total }}</span>
                    <button class="btn-ghost" @click="library.cancelLoudnessAnalysis()">取消</button>
                  </div>
                  <p class="set-hint loudness-current" :title="library.loudnessScan.current">
                    {{ library.loudnessScan.current || '分析中…' }}
                  </p>
                </template>
              </div>
            </section>

            <!-- 背景图 -->
            <section class="set-section">
              <h3 class="section-title">背景图</h3>
              <div class="bg-row">
                <div class="bg-preview">
                  <img v-if="bgUrl" :src="bgUrl" alt="" :style="bgPreviewStyle" />
                  <span v-else class="bg-empty">未设置背景图</span>
                </div>
                <div class="bg-controls">
                  <div class="bg-btns">
                    <button class="btn-ghost" @click="pickBg">{{ bgUrl ? '更换图片' : '选择图片' }}</button>
                    <button v-if="bgUrl" class="btn-ghost" @click="ui.bgEditorOpen = true">编辑位置与缩放…</button>
                    <button v-if="bgUrl" class="btn-ghost" @click="removeBg">移除</button>
                  </div>
                  <template v-if="bgUrl">
                    <!-- 适配方式：拉伸 / 填充 / 居中 / 平铺 / 适应 -->
                    <div class="bg-fit-row">
                      <span class="slider-label">适配</span>
                      <div class="bg-fit-modes">
                        <button
                          v-for="m in BG_FIT_MODES"
                          :key="m.value"
                          class="bg-fit-btn"
                          :class="{ active: settings.bgSize === m.value }"
                          :title="m.hint"
                          @click="settings.bgSize = m.value; settings.scheduleSave()"
                        >{{ m.label }}</button>
                      </div>
                    </div>
                    <div class="slider-row compact">
                      <span class="slider-label">透明度</span>
                      <input type="range" min="0" max="0.9" step="0.01" :value="settings.ovl" @input="settings.ovl = Number(($event.target as HTMLInputElement).value); settings.scheduleSave()" />
                      <span class="slider-val tnum">{{ Math.round(settings.ovl * 100) }}%</span>
                    </div>
                    <div class="slider-row compact">
                      <span class="slider-label">模糊</span>
                      <input type="range" min="0" max="40" step="1" :value="settings.bgBlur" @input="settings.bgBlur = Number(($event.target as HTMLInputElement).value); settings.scheduleSave()" />
                    </div>
                  </template>
                </div>
              </div>
              <p class="set-hint">背景图明度自适应：文字自动切换深浅（当前{{ textLight === 1 ? '深底白字' : '浅底黑字' }}）</p>
            </section>

            <!-- 快捷键 -->
            <section class="set-section">
              <h3 class="section-title">键盘</h3>
              <label class="check-row">
                <input v-model="settings.keyboardEnabled" type="checkbox" @change="settings.scheduleSave()" />
                启用全局快捷键（Space 播放暂停 · ←→ 快退快进 · ↑↓ 音量 · , . 倍速 · Ctrl+F 搜索 · B 打书签 · Esc 关闭面板）
              </label>
            </section>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.settings-overlay {
  position: fixed;
  inset: 0;
  z-index: 920;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--modal-overlay);
}
.settings-card {
  width: 620px;
  max-height: 86vh;
  display: flex;
  flex-direction: column;
  background: var(--modal-bg);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  box-shadow: var(--shadow-pop);
}
.settings-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 18px 22px 12px;
  flex-shrink: 0;
}
.settings-head h2 { font-size: 16px; font-weight: 650; }
.settings-body {
  overflow-y: auto;
  padding: 0 22px 22px;
}
.set-section { margin-bottom: 22px; }
.set-section > .section-title {
  display: block;
  margin-bottom: 10px;
}
.slider-row.disabled { opacity: 0.5; }

.theme-switch { display: flex; gap: 8px; }
.theme-opt {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 16px;
  border-radius: var(--radius);
  border: 1px solid var(--border);
  color: var(--text-sub);
  font-size: 12.5px;
}
.theme-opt.active {
  border-color: rgb(var(--accent-rgb));
  color: rgb(var(--accent-rgb));
  background: var(--bg-selected);
}
.theme-dot { width: 12px; height: 12px; border-radius: 50%; }
.dark-dot { background: var(--swatch-theme-dark); border: 1px solid var(--border-strong); }
.light-dot { background: var(--swatch-theme-light); border: 1px solid var(--border-strong); }

.accent-row {
  display: flex;
  align-items: center;
  gap: 14px;
}
.color-wheel-wrap { position: relative; width: 110px; height: 110px; }
.hue-canvas { border-radius: 50%; cursor: crosshair; }
.hue-pointer {
  position: absolute;
  left: 50%;
  top: 50%;
  width: 14px;
  height: 14px;
  margin: -7px;
  pointer-events: none;
}
.hue-pointer::before {
  content: '';
  position: absolute;
  top: -49px;
  left: 50%;
  width: 4px;
  height: 8px;
  margin-left: -2px;
  border-radius: 2px;
  background: var(--on-media);
  box-shadow: 0 0 0 1px rgba(0, 0, 0, 0.4), 0 0 0 4px rgba(255,255,255,0.9);
}
.sv-wrap { flex: 1; max-width: 200px; }
.sv-canvas {
  width: 100%;
  border-radius: var(--radius-sm);
  cursor: crosshair;
  display: block;
}
.accent-current {
  width: 34px;
  height: 34px;
  border-radius: 50%;
  border: 2px solid var(--border-strong);
}
.preset-row { display: flex; gap: 8px; margin-top: 12px; }
.preset-dot {
  width: 20px;
  height: 20px;
  border-radius: 50%;
  border: 2px solid transparent;
  transition: transform var(--dur-fast) var(--ease);
}
.preset-dot:hover { transform: scale(1.15); }
.preset-dot.active { border-color: var(--text); }

.slider-row {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-bottom: 8px;
}
.slider-row.compact { margin-bottom: 6px; }
.slider-label { width: 100px; flex-shrink: 0; font-size: 12px; color: var(--text-sub); }
.slider-row input[type="range"] {
  flex: 1;
  accent-color: rgb(var(--accent-rgb));
}
.slider-val { width: 40px; text-align: right; font-size: 11px; color: var(--text-muted); }

.bg-row {
  display: flex;
  gap: 16px;
}
.bg-preview {
  position: relative;
  width: 200px;
  height: 120px;
  border-radius: var(--radius);
  overflow: hidden;
  background: var(--bg-input);
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: center;
}
.bg-btns {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-bottom: 6px;
}
.bg-preview img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  pointer-events: none;
}
.bg-empty { font-size: 11px; color: var(--text-muted); }
.bg-controls {
  flex: 1;
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  align-content: flex-start;
}
.bg-controls .slider-row { width: 100%; }
.bg-controls .slider-row .slider-label { width: 48px; }

/* 适配方式选择栏 */
.bg-fit-row {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
}
.bg-fit-row .slider-label { width: 48px; flex-shrink: 0; }
.bg-fit-modes {
  display: flex;
  gap: 4px;
  flex-wrap: wrap;
}
.bg-fit-btn {
  padding: 4px 10px;
  border-radius: 999px;
  border: 1px solid var(--border);
  font-size: 11.5px;
  color: var(--text-sub);
}
.bg-fit-btn:hover { border-color: var(--border-strong); color: var(--text); }
.bg-fit-btn.active {
  border-color: rgb(var(--accent-rgb));
  color: rgb(var(--accent-rgb));
  background: var(--bg-selected);
}
.set-hint {
  margin-top: 10px;
  font-size: 11px;
  color: var(--text-muted);
}

.loudness-row {
  margin-top: 10px;
}
.loudness-progress {
  display: flex;
  align-items: center;
  gap: 10px;
}
.loudness-bar {
  flex: 1;
  height: 4px;
  border-radius: 2px;
  background: var(--bg-input);
  overflow: hidden;
}
.loudness-bar-fill {
  height: 100%;
  border-radius: 2px;
  background: rgb(var(--accent-rgb));
  transition: width var(--dur-fast) var(--ease);
}
.loudness-progress .tnum {
  font-size: 11px;
  color: var(--text-muted);
  flex-shrink: 0;
}
.loudness-current {
  margin-top: 6px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.check-row {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  font-size: 12px;
  color: var(--text-sub);
  cursor: pointer;
}
.check-row input { margin-top: 2px; accent-color: rgb(var(--accent-rgb)); }
</style>
