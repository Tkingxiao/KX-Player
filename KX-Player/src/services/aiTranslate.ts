/**
 * AI 翻译引擎：扫描字幕/歌词文件 → 提取可翻译段 → 分块送 LLM → 回写 .zh 文件。
 * LRC 保留时间戳结构；SRT/VTT 解析 cue 后按标准格式重组。
 */
import { api } from '@/bridge/ipc'
import { parseLrc, parseVttOrSrt } from '@/utils/parsers/lyrics'
import type { DirEntry } from '@/contracts/api'

export const SUBTITLE_EXTS = new Set(['.lrc', '.srt', '.vtt'])

export interface SubtitleFile {
  path: string
  name: string
  ext: string
}

/** 递归收集目录下的 .lrc/.srt/.vtt（深度上限 4，数量上限 500） */
export async function scanSubtitleFiles(dir: string): Promise<SubtitleFile[]> {
  const out: SubtitleFile[] = []
  async function walk(d: string, depth: number): Promise<void> {
    if (depth > 4 || out.length >= 500) return
    let entries: DirEntry[] = []
    try { entries = await api.listDir(d) } catch { return }
    for (const entry of entries) {
      const name = typeof entry === 'string' ? entry : entry.name
      const isDir = typeof entry === 'string' ? undefined : entry.isDirectory
      if (!name) continue
      const lower = name.toLowerCase()
      if (isDir === true) {
        await walk(d + '/' + name, depth + 1)
      } else if (isDir === false) {
        if (SUBTITLE_EXTS.has(lower.slice(lower.lastIndexOf('.')))) {
          out.push({ path: d + '/' + name, name, ext: lower.slice(lower.lastIndexOf('.') + 1) })
        }
      } else {
        // DirEntry 信息缺失时按扩展名判断（无法区分目录，跳过）
        if (SUBTITLE_EXTS.has(lower.slice(lower.lastIndexOf('.')))) {
          out.push({ path: d + '/' + name, name, ext: lower.slice(lower.lastIndexOf('.') + 1) })
        }
      }
      if (out.length >= 500) return
    }
  }
  await walk(dir, 0)
  return out
}

/** 文本块：一段待翻译文字及其在文件中的位置 */
export interface Segment {
  index: number   // 在 segments 数组中的下标
  text: string
}

export interface ParsedDoc {
  kind: 'lrc' | 'srt' | 'vtt'
  segments: Segment[]
  /** 用译后的段文本重建完整文件内容 */
  render: (translated: string[]) => string
}

export function parseDocument(content: string, ext: string): ParsedDoc {
  if (ext === 'lrc') return parseLrcDoc(content)
  return parseCueDoc(content, ext === 'vtt' ? 'vtt' : 'srt')
}

function parseLrcDoc(content: string): ParsedDoc {
  const lines = content.split(/\r?\n/)
  const tsRegex = /\[(\d{1,2}:\d{2}(?:\.\d{1,3})?)\]/g
  const segments: Segment[] = []
  const segIdxByLine = new Map<number, number>()
  lines.forEach((line, i) => {
    if (!line.trim()) return
    tsRegex.lastIndex = 0
    if (!tsRegex.test(line)) return // 无时间戳的元数据行不翻译
    const text = line.replace(/\[\d{1,2}:\d{2}(?:\.\d{1,3})?\]/g, '').trim()
    if (!text) return
    segIdxByLine.set(i, segments.length)
    segments.push({ index: segments.length, text })
  })
  return {
    kind: 'lrc',
    segments,
    render: (translated) => {
      return lines
        .map((line, i) => {
          const segIdx = segIdxByLine.get(i)
          if (segIdx === undefined) return line
          const t = translated[segIdx]
          if (t === undefined) return line
          return line.replace(/(\]\s*)([^[]+)$/, (_m, p1) => p1 + t)
        })
        .join('\n')
    },
  }
}

function fmtTimestamp(sec: number, kind: 'srt' | 'vtt'): string {
  const h = Math.floor(sec / 3600)
  const m = Math.floor((sec % 3600) / 60)
  const s = Math.floor(sec % 60)
  const ms = Math.round((sec - Math.floor(sec)) * 1000)
  const pad = (n: number, w = 2) => String(n).padStart(w, '0')
  const sep = kind === 'srt' ? ',' : '.'
  return `${pad(h)}:${pad(m)}:${pad(s)}${sep}${pad(ms, 3)}`
}

function parseCueDoc(content: string, kind: 'srt' | 'vtt'): ParsedDoc {
  // 复用歌词解析拿 cue，再重组为标准 SRT/VTT
  const cues = parseVttOrSrt(content, kind)
  // 同时需要 end 时间：重新轻量解析时间轴
  const lineRegex = /(\d{1,2}):(\d{2}):(\d{2})[.,](\d{1,3})\s*-->\s*(\d{1,2}):(\d{2}):(\d{2})[.,](\d{1,3})/
  const shortRegex = /(\d{1,2}):(\d{2})[.,](\d{1,3})\s*-->\s*(\d{1,2}):(\d{2})[.,](\d{1,3})/
  const toSec = (h: string, m: string, s: string, ms: string) =>
    parseInt(h, 10) * 3600 + parseInt(m, 10) * 60 + parseInt(s, 10) + parseInt(ms.padEnd(3, '0'), 10) / 1000
  const times: { start: number; end: number }[] = []
  for (const line of content.split(/\r?\n/)) {
    const m = line.match(lineRegex)
    if (m) {
      times.push({ start: toSec(m[1], m[2], m[3], m[4]), end: toSec(m[5], m[6], m[7], m[8]) })
      continue
    }
    const sm = line.match(shortRegex)
    if (sm) {
      times.push({ start: toSec('0', sm[1], sm[2], sm[3]), end: toSec('0', sm[4], sm[5], sm[6]) })
    }
  }
  const segments: Segment[] = cues.map((c, i) => ({ index: i, text: c.text }))
  return {
    kind,
    segments,
    render: (translated) => {
      const parts: string[] = []
      if (kind === 'vtt') parts.push('WEBVTT')
      cues.forEach((cue, i) => {
        const t = times[i] ?? { start: cue.time, end: cue.time + 2 }
        parts.push(
          `${i + 1}\n${fmtTimestamp(t.start, kind)} --> ${fmtTimestamp(t.end, kind)}\n${translated[i] ?? cue.text}`,
        )
      })
      return parts.join('\n\n') + '\n'
    },
  }
}

export interface TranslateOptions {
  baseURL: string
  apiKey: string
  model: string
  chunkSize?: number
  concurrency?: number
  onProgress?: (done: number, total: number) => void
}

const SYSTEM_PROMPT =
  '你是专业的音声字幕/歌词翻译器。用户会给出若干「编号. 文本」行。' +
  '把每一行翻译成自然、口语化的简体中文：保持编号不变、逐行输出；' +
  '保留原文中的★♪【】※等装饰符号与表情；拟声词可音译或保留；只输出译文，不要解释、不要合并行。'

function parseNumberedReply(reply: string, expected: number): string[] | null {
  const out = new Map<number, string>()
  for (const line of reply.split(/\r?\n/)) {
    const m = line.match(/^\s*(\d+)\s*[.、)）：:]\s*(.+)$/)
    if (m) {
      const n = parseInt(m[1], 10)
      if (n >= 1 && n <= expected && !out.has(n)) out.set(n, m[2].trim())
    }
  }
  if (out.size < Math.ceil(expected * 0.8)) return null
  const result: string[] = []
  for (let i = 1; i <= expected; i++) result.push(out.get(i) ?? '')
  return result
}

async function translateChunk(
  texts: string[],
  opts: TranslateOptions,
  retries = 2,
): Promise<string[]> {
  const numbered = texts.map((t, i) => `${i + 1}. ${t}`).join('\n')
  for (let attempt = 0; attempt <= retries; attempt++) {
    const res = await api.aiChat({
      baseURL: opts.baseURL,
      apiKey: opts.apiKey,
      model: opts.model,
      messages: [
        { role: 'system', content: SYSTEM_PROMPT },
        { role: 'user', content: numbered },
      ],
      temperature: 0.3,
    })
    if (res.ok) {
      const parsed = parseNumberedReply(res.content, texts.length)
      if (parsed) {
        // 空译文回退原文
        return parsed.map((t, i) => t || texts[i])
      }
    }
  }
  return texts // 全部重试失败 → 保留原文
}

/** 翻译一个文档的段文本（并发受 opts.concurrency 控制，默认 2） */
export async function translateSegments(
  segments: string[],
  opts: TranslateOptions,
): Promise<string[]> {
  const chunkSize = opts.chunkSize ?? 15
  const chunks: { start: number; texts: string[] }[] = []
  for (let i = 0; i < segments.length; i += chunkSize) {
    chunks.push({ start: i, texts: segments.slice(i, i + chunkSize) })
  }
  const results = new Array<string[]>(chunks.length)
  let done = 0
  const concurrency = Math.max(1, opts.concurrency ?? 2)
  let next = 0
  async function worker(): Promise<void> {
    while (next < chunks.length) {
      const idx = next++
      const chunk = chunks[idx]
      results[idx] = await translateChunk(chunk.texts, opts)
      done++
      opts.onProgress?.(done, chunks.length)
    }
  }
  await Promise.all(Array.from({ length: Math.min(concurrency, chunks.length) }, worker))
  return results.flat()
}

/** UTF-8 安全的 base64（渲染端无 Node Buffer） */
function utf8ToBase64(str: string): string {
  const bytes = new TextEncoder().encode(str)
  let bin = ''
  for (const b of bytes) bin += String.fromCharCode(b)
  return btoa(bin)
}

/** 翻译单个文件并写回 .zh.<ext>（返回输出路径） */
export async function translateFile(
  file: SubtitleFile,
  opts: TranslateOptions,
): Promise<{ outPath: string; count: number }> {
  const content = await api.readTextFile(file.path)
  if (!content) throw new Error('读取失败')
  const doc = parseDocument(content, file.ext)
  if (!doc.segments.length) throw new Error('没有可翻译的文本')
  const onProgress = opts.onProgress
  const translated = await translateSegments(
    doc.segments.map((s) => s.text),
    {
      ...opts,
      onProgress: onProgress
        ? (done, total) => onProgress(done, total)
        : undefined,
    },
  )
  const output = doc.render(translated)
  const dir = file.path.slice(0, file.path.lastIndexOf('/'))
  const base = file.name.replace(/\.[^.]+$/, '')
  const outPath = `${dir}/${base}.zh.${file.ext}`
  const b64 = utf8ToBase64(output)
  const ok = await api.toolsSaveFile(outPath, b64)
  if (!ok) throw new Error('保存失败')
  return { outPath, count: doc.segments.length }
}
