/** 确认框服务：单例桥（App 挂载 ConfirmDialog 并绑定）。 */
import type { ConfirmOptions } from '@/components/ConfirmDialog.vue'

type OpenFn = (o: ConfirmOptions) => Promise<boolean>

const state: { open: OpenFn | null } = { open: null }

export function bindConfirmHost(fn: OpenFn): void {
  state.open = fn
}

const confirmHost = {
  open(o: ConfirmOptions): Promise<boolean> {
    if (!state.open) return Promise.resolve(false)
    return state.open(o)
  },
}

export default confirmHost
