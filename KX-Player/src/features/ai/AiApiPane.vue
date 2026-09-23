<!--
  接入区（AI-1/2/4/5/20）：接口地址 + 模型（ModelPicker）+ API Key + 模型类型与合并条数 + 测试连接。
  它是**左列的一次性配置**，不随作业区滚动；三个工种共用同一份接入参数（`useAiAccess`）。
  档位的具体数字不在这里复述 —— 事实源在 Rust `ai_strategy.rs`，抄一遍就会漂（DESIGN.md「AI 工作台」§2）。
-->
<script setup lang="ts">
import { ref } from 'vue'
import { useSettingsStore } from '@/stores/settings'
import { api } from '@/bridge/ipc'
import { apiKey } from './useAiAccess'
import ModelPicker from './ModelPicker.vue'

const settings = useSettingsStore()

const testing = ref(false)
const testResult = ref('')

async function testConnection(): Promise<void> {
  testing.value = true
  testResult.value = ''
  try {
    const r = await api.aiPing({ baseURL: settings.aiBaseURL, apiKey: apiKey.value, model: settings.aiModel })
    // 成功时把 AI-5 回显的档位信息（模型名/延迟/token）拼在后面，失败时只有原文
    const facts = [
      r.model && `模型 ${r.model}`,
      r.latencyMs != null && `延迟 ${r.latencyMs} ms`,
      r.totalTokens != null && `tokens ${r.totalTokens}`,
    ].filter(Boolean)
    testResult.value = (r.ok ? '✓ ' : '✗ ') + r.message + (facts.length ? `（${facts.join(' · ')}）` : '')
  } finally {
    testing.value = false
  }
}

/** 切档位要存下来：它决定 Rust 侧用哪一档参数 */
function setModelKind(k: 'remote' | 'local'): void {
  settings.aiModelKind = k
  settings.scheduleSave()
}

/** 输入框能填任意数，这里先按选项范围收住（Rust 侧还会再钳一次，两边都得有） */
function clampMerge(): void {
  const n = Math.round(Number(settings.aiMergeLines))
  settings.aiMergeLines = isFinite(n) ? Math.min(10, Math.max(2, n)) : 4
  settings.scheduleSave()
}
</script>

<template>
  <section class="wb-side-block">
    <h3 class="pane-title section-title">模型服务（OpenAI 兼容接口）</h3>
    <div class="ai-grid">
      <label class="ai-field">
        <span>接口地址</span>
        <input v-model="settings.aiBaseURL" @change="settings.scheduleSave()" />
      </label>
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
      <!-- 只描述策略、不复述数字：具体参数的事实源在 Rust `ai_strategy.rs`，这里抄一遍就会漂 -->
      <span class="ai-strategy-hint">
        {{ settings.aiModelKind === 'local' ? '本地档：单路请求、长超时、失败自动拆半重试' : '外接档：多路并发、每批多送几条、超时较短' }}
      </span>
    </div>
    <div class="ai-test-row">
      <button class="btn-ghost" :disabled="testing || !settings.aiModel" :title="!settings.aiModel ? '先填模型名，或点「获取模型」选择' : ''" @click="testConnection">
        {{ testing ? '测试中…' : '测试连接' }}
      </button>
      <span class="ai-test-result" :class="{ ok: testResult.startsWith('✓') }">{{ testResult }}</span>
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
</style>
