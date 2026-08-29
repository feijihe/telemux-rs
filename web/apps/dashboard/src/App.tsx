import { Badge, Button, Separator } from "@telemux/ui"
import { Activity, LayoutDashboard, Plus } from "lucide-react"
import { useState } from "react"
import { DeviceTable } from "./components/DeviceTable"
import { RegisterModal } from "./components/RegisterModal"
import { useDashboard, type ConnState } from "./hooks/useDashboard"

const CONN_TEXT: Record<
  ConnState,
  { label: string; variant: "default" | "secondary" | "destructive" | "outline" }
> = {
  loading: { label: "加载中…", variant: "secondary" },
  connecting: { label: "连接中…", variant: "secondary" },
  connected: { label: "● 已连接", variant: "default" },
  disconnected: { label: "○ 已断开，重连中…", variant: "destructive" },
}

const fmtTime = (ms: number) =>
  ms ? new Date(ms).toLocaleTimeString("zh-CN", { hour12: false }) : ""

export default function App() {
  const [snap, updatedAt, conn, reload] = useDashboard()
  const [modalOpen, setModalOpen] = useState(false)

  const connInfo = CONN_TEXT[conn] ?? CONN_TEXT.connecting
  const devices = snap?.devices.map(d => d.name) ?? []

  return (
    <div className="bg-background min-h-screen">
      <header className="bg-background/95 sticky top-0 z-10 border-b backdrop-blur">
        <div className="mx-auto flex max-w-7xl items-center gap-3 px-4 py-3">
          <LayoutDashboard className="text-primary size-5" />
          <h1 className="text-base font-semibold tracking-tight">
            Telemux <span className="text-muted-foreground font-normal">Dev Dashboard</span>
          </h1>
          <Badge variant={connInfo.variant}>{connInfo.label}</Badge>
          <span className="text-muted-foreground ml-auto text-xs">
            {updatedAt ? `更新于 ${fmtTime(updatedAt)}` : ""}
          </span>
          <Separator orientation="vertical" className="h-4" />
          <Button size="sm" onClick={() => setModalOpen(true)}>
            <Plus data-icon="inline-start" />
            新增寄存器
          </Button>
        </div>
      </header>

      <main className="mx-auto flex max-w-7xl flex-col gap-4 px-4 py-6">
        {!snap ? (
          <div className="text-muted-foreground flex flex-col items-center gap-2 py-24">
            <Activity className="size-8 animate-pulse" />
            <p className="text-sm">正在加载…</p>
          </div>
        ) : snap.devices.length === 0 ? (
          <div className="text-muted-foreground flex flex-col items-center gap-2 py-24">
            <LayoutDashboard className="size-8" />
            <p className="text-sm">未配置设备</p>
          </div>
        ) : (
          snap.devices.map(dev => <DeviceTable key={dev.name} dev={dev} />)
        )}
      </main>

      <RegisterModal
        open={modalOpen}
        devices={devices}
        onOpenChange={setModalOpen}
        onAdded={() => reload()}
      />
    </div>
  )
}
