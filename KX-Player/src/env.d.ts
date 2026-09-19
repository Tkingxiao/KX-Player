/// <reference types="vite/client" />

declare module '*.vue' {
  import type { DefineComponent } from 'vue'
  const component: DefineComponent<Record<string, unknown>, Record<string, unknown>, unknown>
  export default component
}

declare module 'chinese-conv' {
  export function sify(s: string): string
}

declare module 'pinyin-pro' {
  export function pinyin(s: string, opts?: Record<string, unknown>): string[] | string
}
