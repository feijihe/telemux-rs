import type { SimState } from "@telemux/ui"
import { useEffect, useRef, useState } from "react"

/** 连接状态 */
export type ConnState = "connecting" | "connected" | "disconnected"

/**
 * 订阅模拟器状态：首次 HTTP /api/state 立即显示，随后 WebSocket /api/ws
 * 500ms 推送；断线自动重连。返回 [state, connState]。
 */
export function useSimState(): [SimState | null, ConnState] {
  const [state, setState] = useState<SimState | null>(null)
  const [conn, setConn] = useState<ConnState>("connecting")
  const wsRef = useRef<WebSocket | null>(null)

  useEffect(() => {
    let alive = true
    let connectTimer: ReturnType<typeof setTimeout> | undefined

    // 首次 HTTP 拉取（立即渲染）
    fetch("/api/state")
      .then(r => r.json())
      .then(d => {
        if (alive) setState(d as SimState)
      })
      .catch(() => {})

    const connect = () => {
      if (!alive) return
      const proto = location.protocol === "https:" ? "wss://" : "ws://"
      const ws = new WebSocket(`${proto}${location.host}/api/ws`)
      wsRef.current = ws
      ws.onopen = () => {
        if (alive) setConn("connected")
      }
      ws.onmessage = ev => {
        if (!alive) return
        try {
          const d = JSON.parse(ev.data as string) as SimState
          setState(d)
        } catch {
          /* 忽略坏消息 */
        }
      }
      ws.onclose = () => {
        if (!alive) return
        setConn("disconnected")
        connectTimer = setTimeout(connect, 1500) // 自动重连
      }
      ws.onerror = () => ws.close()
    }

    // 延迟一个宏任务再建立连接：StrictMode 开发模式会双执行 effect
    // （挂载→卸载→再挂载），首次挂载的 cleanup 借此在连接创建前取消，
    // 避免浏览器报 "WebSocket is closed before the connection is established"。
    connectTimer = setTimeout(connect, 0)

    return () => {
      alive = false
      clearTimeout(connectTimer)
      wsRef.current?.close()
      wsRef.current = null
    }
  }, [])

  return [state, conn]
}

/** 设置控制变量（POST /api/control），成功后返回是否 ok */
export async function setControl(name: string, value: number): Promise<boolean> {
  try {
    const res = await fetch("/api/control", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ name, value }),
    })
    const j = (await res.json()) as { ok?: boolean; error?: string }
    if (!j.ok) {
      window.alert(`设置失败: ${j.error ?? "未知错误"}`)
      return false
    }
    return true
  } catch (e) {
    window.alert(`请求失败: ${e}`)
    return false
  }
}
