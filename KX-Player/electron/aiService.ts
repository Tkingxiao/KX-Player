/**
 * AI 翻译服务：OpenAI 兼容 /chat/completions（本地 Ollama / llama.cpp / 在线 API 通用）。
 * 网络请求只发生在主进程，渲染端不直连（符合 01-constitution §5-25）。
 * API Key 仅存于渲染端内存，不落盘。
 */

export interface ChatMessage {
  role: 'system' | 'user' | 'assistant'
  content: string
}

export interface ChatOptions {
  baseURL: string
  apiKey: string
  model: string
  messages: ChatMessage[]
  temperature?: number
  timeoutMs?: number
}

/** 规范化 baseURL：去尾部斜杠，无 /v1 时自动补（Ollama 原生地址兼容） */
export function normalizeBaseURL(raw: string): string {
  let u = String(raw || '').trim().replace(/\/+$/, '')
  if (!u) u = 'http://127.0.0.1:11434'
  if (!/\/v\d+(\/|$)/.test(u)) u += '/v1'
  return u
}

export async function chatCompletion(opts: ChatOptions): Promise<string> {
  const url = normalizeBaseURL(opts.baseURL) + '/chat/completions'
  const controller = new AbortController()
  const timer = setTimeout(() => controller.abort(), opts.timeoutMs ?? 120_000)
  try {
    const headers: Record<string, string> = { 'Content-Type': 'application/json' }
    if (opts.apiKey) headers.Authorization = `Bearer ${opts.apiKey}`
    const res = await fetch(url, {
      method: 'POST',
      headers,
      signal: controller.signal,
      body: JSON.stringify({
        model: opts.model,
        messages: opts.messages,
        temperature: opts.temperature ?? 0.3,
        stream: false,
      }),
    })
    if (!res.ok) {
      const text = await res.text().catch(() => '')
      throw new Error(`HTTP ${res.status}: ${text.slice(0, 300)}`)
    }
    const data = (await res.json()) as {
      choices?: { message?: { content?: string } }[]
    }
    const content = data.choices?.[0]?.message?.content
    if (typeof content !== 'string') throw new Error('响应缺少 choices[0].message.content')
    return content
  } finally {
    clearTimeout(timer)
  }
}

/** 连通性测试：发送一个最小请求 */
export async function pingProvider(opts: ChatOptions): Promise<{ ok: boolean; message: string }> {
  try {
    const reply = await chatCompletion({
      ...opts,
      messages: [{ role: 'user', content: 'ping' }],
      temperature: 0,
      timeoutMs: 15_000,
    })
    return { ok: true, message: `连接成功：${reply.slice(0, 60) || '(空响应)'}` }
  } catch (e) {
    return { ok: false, message: e instanceof Error ? e.message : String(e) }
  }
}

/** 获取模型列表（GET /v1/models） */
export async function listModels(baseURL: string, apiKey: string): Promise<{ ok: boolean; models: string[]; error?: string }> {
  try {
    const url = normalizeBaseURL(baseURL) + '/models'
    const controller = new AbortController()
    const timer = setTimeout(() => controller.abort(), 15_000)
    try {
      const headers: Record<string, string> = {}
      if (apiKey) headers.Authorization = `Bearer ${apiKey}`
      const res = await fetch(url, { headers, signal: controller.signal })
      if (!res.ok) {
        const text = await res.text().catch(() => '')
        return { ok: false, models: [], error: `HTTP ${res.status}: ${text.slice(0, 200)}` }
      }
      const data = (await res.json()) as { data?: { id?: string }[] }
      const models = (data.data ?? [])
        .map((m) => String(m.id ?? ''))
        .filter(Boolean)
        .sort((a, b) => a.localeCompare(b))
      return { ok: true, models }
    } finally {
      clearTimeout(timer)
    }
  } catch (e) {
    return { ok: false, models: [], error: e instanceof Error ? e.message : String(e) }
  }
}

/** 目录重命名（同父目录内；目标已存在则拒绝） */
export async function renameDir(oldPath: string, newPath: string): Promise<{ ok: boolean; error?: string }> {
  try {
    const fs = await import('node:fs')
    if (!fs.existsSync(oldPath)) return { ok: false, error: '原目录不存在' }
    if (fs.existsSync(newPath)) return { ok: false, error: '目标目录已存在' }
    const oldParent = oldPath.replace(/[\\/]+$/, '').split(/[\\/]/).slice(0, -1).join('/')
    const newParent = newPath.replace(/[\\/]+$/, '').split(/[\\/]/).slice(0, -1).join('/')
    if (oldParent !== newParent) return { ok: false, error: '只允许同父目录内重命名' }
    await fs.promises.rename(oldPath, newPath)
    return { ok: true }
  } catch (e) {
    return { ok: false, error: e instanceof Error ? e.message : String(e) }
  }
}
