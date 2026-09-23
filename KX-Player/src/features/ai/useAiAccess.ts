/**
 * AI 接入参数的唯一持有者（三个工种共用一份「这批怎么发」）。
 *
 * **API Key 只在内存**：`ref` 活在这个模块里，刷新页面 / 重开应用即失，
 * 永不写进 settings JSON 或 SQLite（AI-4 的裁决，见 `04 §3`）。
 * 不放 props 是因为工种面板各自要发请求 —— 传一层下去会变成「谁改了 Key」有两处。
 */
import { ref } from 'vue'
import { useSettingsStore } from '@/stores/settings'

export const apiKey = ref('')

/**
 * 两个任务共用的接入参数。
 * 批量大小 / 并发 / 超时**不在这里决定** —— 前端只说「本地还是外接、本地合几条」，
 * 档位参数在 Rust 的 `ai_strategy.rs` 里，两处各定一半必然漂移。
 */
export function jobOpts() {
  const settings = useSettingsStore()
  return {
    baseURL: settings.aiBaseURL,
    apiKey: apiKey.value,
    model: settings.aiModel,
    modelKind: settings.aiModelKind,
    mergeLines: settings.aiMergeLines,
  }
}
