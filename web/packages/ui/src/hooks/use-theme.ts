import { useSyncExternalStore } from "react"

export type Theme = "light" | "dark" | "system"
type Resolved = "light" | "dark"

const STORAGE_KEY = "telemux-theme"

function systemPrefersDark(): boolean {
  return typeof window !== "undefined" && window.matchMedia("(prefers-color-scheme: dark)").matches
}

function resolveTheme(theme: Theme): Resolved {
  if (theme === "system") return systemPrefersDark() ? "dark" : "light"
  return theme
}

// 项目 CSS 用 .dark 类作用域（@custom-variant dark），故只需切 <html> 的 class
function applyToDom(resolved: Resolved) {
  const root = document.documentElement
  root.classList.toggle("dark", resolved === "dark")
  // 同步原生控件（滚动条/表单）配色
  root.style.colorScheme = resolved
}

function readStored(): Theme {
  if (typeof window === "undefined") return "system"
  const v = window.localStorage.getItem(STORAGE_KEY)
  return v === "light" || v === "dark" || v === "system" ? v : "system"
}

// 模块级 store：全局单例，跨组件共享，免 Context
type State = { theme: Theme; resolved: Resolved }
let currentState: State = (() => {
  const t = readStored()
  return { theme: t, resolved: resolveTheme(t) }
})()
const listeners = new Set<() => void>()

function notify() {
  listeners.forEach(l => l())
}

function setTheme(theme: Theme) {
  const resolved = resolveTheme(theme)
  currentState = { theme, resolved }
  applyToDom(resolved)
  try {
    window.localStorage.setItem(STORAGE_KEY, theme)
  } catch {
    // 无痕模式等场景忽略写入失败
  }
  notify()
}

function toggleTheme() {
  setTheme(currentState.resolved === "dark" ? "light" : "dark")
}

// 跟随系统：仅当用户选 system 时，由 prefers-color-scheme 变化驱动
if (typeof window !== "undefined") {
  window.matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => {
    if (currentState.theme !== "system") return
    const resolved = systemPrefersDark() ? "dark" : "light"
    currentState = { ...currentState, resolved }
    applyToDom(resolved)
    notify()
  })
}

function subscribe(l: () => void) {
  listeners.add(l)
  return () => {
    listeners.delete(l)
  }
}
function getSnapshot() {
  return currentState
}

export function useTheme() {
  const state = useSyncExternalStore(subscribe, getSnapshot, getSnapshot)
  return {
    theme: state.theme,
    resolvedTheme: state.resolved,
    isDark: state.resolved === "dark",
    isLight: state.resolved === "light",
    setTheme,
    toggleTheme,
  }
}
