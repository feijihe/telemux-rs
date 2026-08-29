import type { DashboardSnapshot, RegisterSnapshot, SampleUpdate } from "@telemux/ui"
import { useCallback, useEffect, useRef, useState } from "react"

export type ConnState = "connecting" | "connected" | "disconnected" | "loading"

/**
 * 网关 dev dashboard 数据：
 * - 首次 HTTP /api/snapshot 全量；
 * - WS /api/ws 增量更新（只改 raw/metric，静态配置不动）。
 * 返回 [snapshot, updatedAtMs, conn, reload]。
 */
export function useDashboard(): [DashboardSnapshot | null, number, ConnState, () => void] {
  const [snap, setSnap] = useState<DashboardSnapshot | null>(null)
  const [updatedAt, setUpdatedAt] = useState(0)
  const [conn, setConn] = useState<ConnState>("loading")
  const wsRef = useRef<WebSocket | null>(null)

  const loadFull = useCallback(async () => {
    try {
      const res = await fetch("/api/snapshot")
      const data = (await res.json()) as DashboardSnapshot
      setSnap(data)
      if (data.generated_at_ms) setUpdatedAt(data.generated_at_ms)
    } catch (e) {
      console.error("load snapshot failed", e)
    }
  }, [])

  useEffect(() => {
    let alive = true
    let connectTimer: ReturnType<typeof setTimeout> | undefined

    void loadFull()

    const connect = () => {
      if (!alive) return
      const proto = location.protocol === "https:" ? "wss://" : "ws://"
      const ws = new WebSocket(`${proto}${location.host}/api/ws`)
      wsRef.current = ws
      ws.onopen = () => alive && setConn("connected")
      ws.onmessage = ev => {
        if (!alive) return
        try {
          const msg = JSON.parse(ev.data as string) as {
            type?: string
            generated_at_ms?: number
            samples?: SampleUpdate[]
          }
          if (msg.type !== "update" || !Array.isArray(msg.samples)) return
          setSnap(prev => (prev ? applyUpdate(prev, msg.samples ?? []) : prev))
          if (msg.generated_at_ms) setUpdatedAt(msg.generated_at_ms)
        } catch {
          /* 忽略坏消息 */
        }
      }
      ws.onclose = () => {
        if (!alive) return
        setConn("disconnected")
        connectTimer = setTimeout(connect, 500) // 指数退避由外层定时器控制，这里固定 500ms 起步
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
  }, [loadFull])

  const reload = useCallback(() => {
    void loadFull()
  }, [loadFull])

  return [snap, updatedAt, conn, reload]
}

/** 把增量样本合并进快照（只更新匹配 sensor_id 的 raw/metric） */
function applyUpdate(snap: DashboardSnapshot, samples: SampleUpdate[]): DashboardSnapshot {
  const byId = new Map(samples.map(s => [s.sensor_id, s]))
  return {
    ...snap,
    devices: snap.devices.map(dev => ({
      ...dev,
      registers: dev.registers.map(reg => {
        const u = byId.get(reg.sensor_id)
        if (!u) return reg
        const next: RegisterSnapshot = { ...reg }
        if (u.raw) {
          next.raw = u.raw
        } else {
          next.raw = null
        }
        if (u.metric) {
          next.metric = u.metric
        } else {
          next.metric = null
        }
        return next
      }),
    })),
  }
}
