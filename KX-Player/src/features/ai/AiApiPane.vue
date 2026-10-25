<!--
  接入区（AI-1/2/4/5/20）：接口地址 + 模型（ModelPicker）+ API Key + 模型类型与合并条数 + 测试连接。
  它是**左列的一次性配置**，不随作业区滚动；三个工种共用同一份接入参数（`useAiAccess`）。
  档位的具体数字不在这里复述 —— 事实源在 Rust `ai_strategy.rs`，抄一遍就会漂（DESIGN.md「AI 工作台」§2）。
-->
<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { useSettingsStore } from '@/stores/settings'
import { useUiStore } from '@/stores/ui'
import { api } from '@/bridge/ipc'
import { errText } from '@/contracts/result'
import type { AiApiStyle } from '@/contracts/dto'
import { apiKey } from './useAiAccess'
import ModelPicker from './ModelPicker.vue'
import type { AiEndpointPolicy } from '@/contracts/api'

const settings = useSettingsStore()
const ui = useUiStore()

const testing = ref(false)
const testResult = ref('')

async function testConnection(): Promise<void> {
  testing.value = true
  testResult.value = ''
  try {
    // 成功时把 AI-5 回显的档位信息（模型名/延迟/token）拼在后面，失败时只有原文
    const r = await api.aiPing({ baseURL: settings.aiBaseURL, apiKey: apiKey.value, model: settings.aiModel, modelKind: settings.aiModelKind, apiStyle: settings.aiApiStyle })
    const facts = [
      r.model && `模型 ${r.model}`,
      r.latencyMs != null && `延迟 ${r.latencyMs} ms`,
      r.totalTokens != null && `tokens ${r.totalTokens}`,
    ].filter(Boolean)
    testResult.value = (r.ok ? '✓ ' : '✗ ') + r.message + (facts.length ? `（${facts.join(' · ')}）` : '')
    // 被拦的地址失败后立刻体检，把「放行」按钮递到用户眼前，不用等他再动一下输入框
    if (!r.ok) void refreshPolicy()
  } catch (e) {
    testResult.value = `✗ ${errText(e)}`
    // reject 也要收口体检：端点策略提示不能因为连接命令失败而消失
    await refreshPolicy()
  } finally {
    testing.value = false
  }
}

// ── 显式放行（P0-3 的唯一例外）──
// 公网明文 / 非 443 端口默认拒绝（防 Key 明文跨公网）；自建反代是正当场景，
// 但放行必须逐条确认：体检回执给出「放行即可用」的端点，用户点了才落库。
const policy = ref<AiEndpointPolicy | null>(null)
const allowBusy = ref(false)

/** 地址一变就体检：被拦时立刻亮出原因与放行按钮，而不是等测试失败 */
async function refreshPolicy(): Promise<void> {
  const url = settings.aiBaseURL.trim()
  if (!url) {
    policy.value = null
    return
  }
  try {
    policy.value = await api.aiEndpointCheck(url)
  } catch {
    policy.value = null
  }
}

function onUrlChange(): void {
  settings.scheduleSave()
  void refreshPolicy()
}

async function allowEndpoint(): Promise<void> {
  const ep = policy.value?.endpoint
  if (!ep || allowBusy.value) return
  allowBusy.value = true
  try {
    await api.aiPlaintextSet(ep, true)
    await refreshPolicy()
    testResult.value = ''
    ui.toast(`已放行 ${ep}，重新测试连接即可`)
    if (settings.aiModel) await testConnection()
  } catch (e) {
    ui.toast(errText(e), 'error')
  } finally {
    allowBusy.value = false
  }
}

async function revokeEndpoint(ep: string): Promise<void> {
  try {
    await api.aiPlaintextSet(ep, false)
    await refreshPolicy()
  } catch (e) {
    ui.toast(errText(e), 'error')
  }
}

onMounted(() => void refreshPolicy())

/** 切档位要存下来：它决定 Rust 侧用哪一档参数 */
function setModelKind(k: 'remote' | 'local'): void {
  settings.aiModelKind = k
  settings.scheduleSave()
}

// ── 接口协议 ──
// 同一份接入参数按三种线形状之一发：URL 末段、鉴权头、请求/响应 JSON 的差别都由 Rust
// 侧分派（ai.rs 的 endpoint_url / auth_headers / *_body / parse_outcome），前端只传名字。
const API_STYLES: { value: AiApiStyle; label: string; title: string }[] = [
  {
    value: 'openai-chat',
    label: 'OpenAI Chat Completions',
    title: '经典 OpenAI 兼容接口：POST {base}/v1/chat/completions，Bearer 鉴权。DeepSeek、Ollama、vLLM 与各类中转站都在这一档（历史缺省）',
  },
  {
    value: 'openai-responses',
    label: 'OpenAI Responses',
    title: 'OpenAI 新版 Responses 接口：POST {base}/v1/responses，同为 Bearer 鉴权（OpenAI 官方与兼容新版 的网关）',
  },
  {
    value: 'anthropic',
    label: 'Anthropic Messages',
    title: 'Anthropic Messages 接口：POST {base}/v1/messages，x-api-key + anthropic-version 鉴权（Claude 官方与兼容网关）',
  },
]

/** 各协议的常用入口；openai-chat 留空 = 历史缺省（本机 Ollama） */
const CANONICAL_BASE: Record<AiApiStyle, string> = {
  'openai-chat': '',
  'openai-responses': 'https://api.openai.com',
  anthropic: 'https://api.anthropic.com',
}

function setApiStyle(v: AiApiStyle): void {
  settings.aiApiStyle = v
  const cur = settings.aiBaseURL.trim().replace(/\/+$/, '')
  // 地址空着、或是上一家协议的常用入口（还没填过自己的地址），就补本协议的入口；
  // 用户自己填过的地址一个字都不动（切协议常用于同一家中转换方言）
  const canonicals = Object.values(CANONICAL_BASE).filter(Boolean)
  if (!cur || canonicals.includes(cur)) {
    settings.aiBaseURL = CANONICAL_BASE[v]
    void refreshPolicy()
  }
  settings.scheduleSave()
}

// ── 多路并发 ──
// 并发的是**分好的块**（一次在飞几个请求），不是每条请求塞几行文本（那是上面的「一次合并」）。
// 勾选 = 显式并发 2–12（默认 4），两档通用且优先于档位缺省；取消 = 不传，外接 2 路 / 本地 1 路。
// Rust 侧所有文件的块共用这一个池子，前一个文件没飞完时后面的文件已经在切块排队。
const concurrencyOn = computed({
  get: () => settings.aiConcurrency != null,
  set: (v: boolean) => {
    settings.aiConcurrency = v ? 4 : null
    settings.scheduleSave()
  },
})

/** 输入框能填任意数，先按 2–12 收住（Rust 侧还会再钳一次，两边都得有） */
function clampConcurrency(): void {
  const n = Math.round(Number(settings.aiConcurrency))
  settings.aiConcurrency = isFinite(n) ? Math.min(12, Math.max(2, n)) : 4
  settings.scheduleSave()
}

const strategyHint = computed(() => {
  const base = settings.aiModelKind === 'local'
    ? '本地档：单路请求、长超时、失败自动拆半重试'
    : '外接档：每批 15 行、默认两路并发、超时较短'
  return settings.aiConcurrency != null ? `${base}；已显式并发 ${settings.aiConcurrency}，两档都按它发` : base
})

/** 输入框能填任意数，这里先按选项范围收住（Rust 侧还会再钳一次，两边都得有） */
function clampMerge(): void {
  const n = Math.round(Number(settings.aiMergeLines))
  settings.aiMergeLines = isFinite(n) ? Math.min(10, Math.max(2, n)) : 4
  settings.scheduleSave()
}
</script>

<template>
  <section class="wb-side-block">
    <h3 class="pane-title section-title">模型服务</h3>
    <div class="ai-grid">
      <div class="ai-field">
        <span>接口协议</span>
        <div class="ai-seg ai-style-seg">
          <button
            v-for="s in API_STYLES"
            :key="s.value"
            class="btn-ghost"
            :class="{ on: settings.aiApiStyle === s.value }"
            :title="s.title"
            @click="setApiStyle(s.value)"
          >{{ s.label }}</button>
        </div>
      </div>
      <label class="ai-field">
        <span>接口地址</span>
        <input v-model="settings.aiBaseURL" @change="onUrlChange" />
        <span class="ai-field-hint">填到域名或版本段为止（无版本段自动补 /v1）；公网地址必须 https 且走 443，本机（127.0.0.1 / localhost）与局域网可 http 并自带端口；自建反代被拦时可在下方显式放行</span>
      </label>
      <!-- 体检回执：被拦但放行即可用 → 给按钮；地址本身不合规 → 只有原因，不给按钮 -->
      <div v-if="policy?.endpoint" class="ai-allow">
        <span class="ai-allow-reason">{{ policy.reason }}</span>
        <button class="btn-ghost ai-allow-btn" :disabled="allowBusy" @click="allowEndpoint">
          放行 {{ policy.endpoint }}（明文直连，我了解风险）
        </button>
      </div>
      <div v-if="policy?.allowlist.length" class="ai-allow-chips">
        <span class="ai-allow-chips-label">已放行（点一下撤销）：</span>
        <button
          v-for="ep in policy.allowlist"
          :key="ep"
          class="ai-allow-chip"
          :title="`撤销放行 ${ep}，恢复默认策略`"
          @click="revokeEndpoint(ep)"
        >{{ ep }} ×</button>
      </div>
      <ModelPicker />
      <label class="ai-field">
        <span>API Key（仅本次运行记忆，不写入磁盘）</span>
        <input v-model="apiKey" type="password" autocomplete="off" />
      </label>
    </div>
    <div class="ai-strategy">
      <span class="ai-strategy-label">模型类型</span>
      <div class="ai-seg">
        <button
          class="btn-ghost"
          :class="{ on: settings.aiModelKind === 'remote' }"
          title="云端 OpenAI 兼容接口：一批多送几条、多路并发、超时较短"
          @click="setModelKind('remote')"
        >外接模型</button>
        <button
          class="btn-ghost"
          :class="{ on: settings.aiModelKind === 'local' }"
          title="本机 Ollama 等：一次只发一路、给足加载权重的时间、编号解析失败自动拆开重试"
          @click="setModelKind('local')"
        >本地模型</button>
      </div>
      <label v-if="settings.aiModelKind === 'local'" class="ai-merge">
        <span>一次合并</span>
        <input
          v-model.number="settings.aiMergeLines"
          type="number"
          min="2"
          max="10"
          step="1"
          @change="clampMerge()"
        />
        <span>条发送</span>
      </label>
      <label class="ai-merge" title="同时发几个分块请求（2–12，默认 4）。并发的是分好的块，不是每条请求塞几行文本（那是「一次合并」）；Rust 侧所有文件共用这个池子，前一个文件没飞完时后面的已在切块排队">
        <input type="checkbox" v-model="concurrencyOn" />
        <span>多路并发</span>
      </label>
      <label v-if="settings.aiConcurrency != null" class="ai-merge">
        <span>同时发</span>
        <input
          v-model.number="settings.aiConcurrency"
          type="number"
          min="2"
          max="12"
          step="1"
          @change="clampConcurrency()"
        />
        <span>个分块</span>
      </label>
      <!-- 只描述策略、不复述数字：具体参数的事实源在 Rust `ai_strategy.rs`，这里抄一遍就会漂 -->
      <span class="ai-strategy-hint">{{ strategyHint }}</span>
    </div>
    <div class="ai-test-row">
      <button class="btn-ghost" :disabled="testing || !settings.aiModel" :title="!settings.aiModel ? '先填模型名，或点「获取模型」选择' : ''" @click="testConnection">
        {{ testing ? '测试中…' : '测试连接' }}
      </button>
      <span class="ai-test-result" :class="{ ok: testResult.startsWith('✓'), fail: testResult.startsWith('✗') }">{{ testResult }}</span>
    </div>
  </section>
</template>

<style scoped>
/* 左列块的标题间距：父级的 `.wb-side-block > .section-title` 选不进子组件内部，这里自己给 */
.pane-title { display: block; margin-bottom: 10px; }

.ai-grid {
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.ai-field {
  display: flex;
  flex-direction: column;
  gap: 4px;
  font-size: 11.5px;
  color: var(--text-muted);
  min-width: 0;
}
.ai-field input {
  padding: 7px 10px;
  border-radius: var(--radius-sm);
  border: 1px solid var(--border);
  background: var(--bg-input);
  font-size: 12px;
}
.ai-field input:focus { border-color: rgb(var(--accent-rgb)); }
/* 地址规则写在输入框下面：Rust 侧会拒掉不合规的地址，先让人知道线画在哪 */
.ai-field-hint { font-size: 10.5px; color: var(--text-sub); line-height: 1.5; }
.ai-test-row {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-top: 12px;
}
/* 档位选择：与测试连接行分开，它是「这批怎么发」的参数，不是连接参数 */
.ai-strategy {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
  margin-top: 12px;
  font-size: 11.5px;
  color: var(--text-muted);
}
.ai-seg { display: flex; gap: 6px; }
.ai-seg .btn-ghost { padding: 5px 12px; font-size: 11.5px; }
/* 协议选择：三个全名按钮一行放不下，允许换行、字号收到与侧栏匹配 */
.ai-style-seg { flex-wrap: wrap; }
.ai-style-seg .btn-ghost { padding: 4px 9px; font-size: 10.5px; }
.ai-seg .btn-ghost.on {
  border-color: rgb(var(--accent-rgb));
  color: rgb(var(--accent-rgb));
  background: var(--bg-selected);
}
.ai-merge { display: flex; align-items: center; gap: 6px; }
.ai-merge input {
  width: 52px;
  padding: 5px 8px;
  border-radius: var(--radius-sm);
  border: 1px solid var(--border);
  background: var(--bg-input);
  font-size: 12px;
  color: var(--text);
}
.ai-strategy-hint { font-size: 11px; color: var(--text-sub); }
.ai-test-result {
  font-size: 11.5px;
  color: var(--text-muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.ai-test-result.ok { color: var(--success); }
.ai-test-result.fail { color: var(--danger); }

/* ── 显式放行（P0-3 的唯一例外）：原因 + 确认按钮 + 已放行名单 ── */
.ai-allow {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.ai-allow-reason {
  font-size: 10.5px;
  color: var(--danger);
  line-height: 1.5;
}
.ai-allow-btn {
  align-self: flex-start;
  padding: 5px 10px;
  font-size: 11px;
}
.ai-allow-chips {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 6px;
}
.ai-allow-chips-label { font-size: 10.5px; color: var(--text-muted); }
.ai-allow-chip {
  padding: 2px 8px;
  border: 1px solid var(--border);
  border-radius: 999px;
  font-size: 10.5px;
  color: var(--text-sub);
}
.ai-allow-chip:hover {
  color: var(--danger);
  border-color: color-mix(in srgb, var(--danger) 45%, transparent);
}
</style>
