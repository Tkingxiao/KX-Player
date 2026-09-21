#!/usr/bin/env node
/**
 * 构建前校验 ffmpeg 的编译期能力（02 §1.2 / 00 §外部二进制）。
 *
 * 能力清单写死在这里，是因为「装了 ffmpeg」和「装了能跑转换页的 ffmpeg」是两件事：
 * 精简版常缺 libx264 / libmp3lame / wavpack，运行时才报「未知编码器」，用户已经在队列里了。
 * 缺失即 exit 1，让 `npm run tauri build` 之前就能挡住。
 *
 * 查找顺序与 Rust 侧 `paths::locate_ffmpeg()` 一致：KX_FFMPEG → sidecar 目录 → 常见安装路径 → PATH。
 */
import { execFileSync } from 'node:child_process'
import { existsSync } from 'node:fs'
import path from 'node:path'
import process from 'node:process'

const ROOT = path.resolve(import.meta.dirname, '..')
const TRIPLE = 'x86_64-pc-windows-msvc'

/** 键是 ffmpeg 里的**实现名**，不是容器/编解码器族名：libx264 是 h264 的编码器，mp3 的是 libmp3lame。 */
const REQUIRED = {
  encoders: ['libmp3lame', 'flac', 'pcm_s16le', 'libvorbis', 'aac', 'libx264'],
  decoders: ['ape', 'wavpack', 'wmav2', 'opus', 'vorbis', 'flac', 'mp3', 'aac', 'alac'],
  subtitleDecoders: ['subrip', 'ass', 'webvtt', 'mov_text', 'dvd_subtitle'],
  muxers: ['matroska', 'avi', 'flv', 'asf', 'webm', 'mp4', 'mpegts'],
  demuxers: ['matroska', 'avi', 'flv', 'asf', 'webm', 'mpegts'],
}

function candidates() {
  const env = process.env.KX_FFMPEG
  const sidecar = path.join(ROOT, 'src-tauri', 'binaries', `ffmpeg-${TRIPLE}.exe`)
  const built = ['release', 'debug'].map((p) => path.join(ROOT, 'src-tauri', 'target', p, 'ffmpeg.exe'))
  return [env, sidecar, ...built, 'C:\\ffmpeg\\bin\\ffmpeg.exe', 'C:\\Program Files\\ffmpeg\\bin\\ffmpeg.exe']
    .filter(Boolean)
    .map((p) => path.resolve(p))
    .filter((p) => existsSync(p))
}

function fromPath() {
  const probe = process.platform === 'win32' ? 'where.exe' : 'which'
  try {
    const out = execFileSync(probe, ['ffmpeg'], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] })
    const first = out.split(/\r?\n/).map((l) => l.trim()).find(Boolean)
    return first && existsSync(first) ? first : null
  } catch {
    return null
  }
}

/** `-codecs` 每行 `<flags> <name> <desc> (encoders: …) (decoders: …)`；flags 位数随版本变化，按空格切。 */
function parseCodecs(text) {
  const map = new Map()
  for (const line of text.split(/\r?\n/)) {
    const m = /^\s*([.A-Za-z]{2,})\s+(\S+)\s+(.*)$/.exec(line)
    if (!m) continue
    const list = (kind) => {
      const hit = new RegExp(`\\(${kind}: ([^)]*)\\)`).exec(m[3])
      return hit ? new Set(hit[1].split(/\s+/).filter(Boolean)) : new Set()
    }
    map.set(m[2], { flags: m[1], name: m[2], encoders: list('encoders'), decoders: list('decoders') })
  }
  return map
}

/**
 * `-formats` 把共用一套代码的解封装器并成一个名字（`matroska,webm`、`mov,mp4,m4a,…`），
 * 而同名条目又可能单独作为封装器出现 —— 所以按逗号拆别名，并把各行的标志位**合并**。
 */
function parseFormats(text) {
  const map = new Map()
  for (const line of text.split(/\r?\n/)) {
    const m = /^\s*([.A-Za-z]{1,3})\s+(\S+)\s+(.*)$/.exec(line)
    if (!m) continue
    for (const alias of m[2].split(',')) map.set(alias, (map.get(alias) ?? '') + m[1])
  }
  return map
}

function hasEncoder(codecs, impl) {
  for (const c of codecs.values()) {
    if (c.encoders.has(impl)) return true
    if (c.name === impl && c.flags.includes('E')) return true
  }
  return false
}

function hasDecoder(codecs, impl) {
  for (const c of codecs.values()) {
    if (c.decoders.has(impl)) return true
    if (c.name === impl && c.flags.includes('D')) return true
  }
  return false
}

function hasSubtitleDecoder(codecs, name) {
  const c = codecs.get(name) ?? [...codecs.values()].find((x) => x.decoders.has(name))
  return !!c && c.flags.includes('S') && hasDecoder(codecs, name)
}

function run(bin, args) {
  return execFileSync(bin, args, { encoding: 'utf8', maxBuffer: 8 << 20 })
}

const found = candidates()
if (!found.length) {
  const hint = fromPath()
  if (!hint) {
    console.error(
      'check:ffmpeg 失败 —— 未找到 ffmpeg.exe。\n' +
        `  sidecar 位置：src-tauri/binaries/ffmpeg-${TRIPLE}.exe\n` +
        '  或设置环境变量 KX_FFMPEG 指向已有的 ffmpeg.exe',
    )
    process.exit(1)
  }
  found.push(hint)
}

const bin = found[0]
const version = run(bin, ['-hide_banner', '-version'])
const banner = version.split(/\r?\n/)[0]
const gpl = version.includes('--enable-gpl')
const codecs = parseCodecs(run(bin, ['-hide_banner', '-codecs']))
const formats = parseFormats(run(bin, ['-hide_banner', '-formats']))

const checks = [
  ['编码器', REQUIRED.encoders, (n) => hasEncoder(codecs, n)],
  ['解码器', REQUIRED.decoders, (n) => hasDecoder(codecs, n)],
  ['字幕解码器', REQUIRED.subtitleDecoders, (n) => hasSubtitleDecoder(codecs, n)],
  ['封装器', REQUIRED.muxers, (n) => (formats.get(n) ?? '').includes('E')],
  ['解封装器', REQUIRED.demuxers, (n) => (formats.get(n) ?? '').includes('D')],
]

const pcmCount = [...codecs.keys()].filter((n) => n.startsWith('pcm_')).length
console.log(`ffmpeg  ${bin}`)
console.log(`${banner}`)
console.log(`许可证  ${gpl ? 'GPL v2+（含 libx264，分发需附源码获取方式）' : '未启用 --enable-gpl：无 libx264，视频压缩不可用'}`)
console.log(`pcm_*   ${pcmCount} 个 PCM 编解码器\n`)

const missing = []
for (const [label, names, test] of checks) {
  const lost = names.filter((n) => !test(n))
  console.log(`${lost.length ? '✗' : '✓'} ${label.padEnd(6)} ${names.length - lost.length}/${names.length}${lost.length ? `  缺：${lost.join(' ')}` : ''}`)
  for (const n of lost) missing.push(`${label} ${n}`)
}

if (missing.length) {
  console.error(`\ncheck:ffmpeg 失败 —— 该构建不足以支撑转换页与响度/抽帧：\n  ${missing.join('\n  ')}`)
  console.error('  换用完整构建（gyan.build 的 full / BtbN 的 full），或把上面的 sidecar 路径放一个后再跑。')
  process.exit(1)
}
console.log('\ncheck:ffmpeg 通过')
