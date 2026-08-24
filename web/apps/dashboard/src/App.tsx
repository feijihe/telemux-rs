import { useState } from "react";
import { Badge, Button } from "@telemux/ui";
import { DeviceTable } from "./components/DeviceTable";
import { RegisterModal } from "./components/RegisterModal";
import { useDashboard, type ConnState } from "./hooks/useDashboard";

const CONN_TEXT: Record<ConnState, { label: string; cls: string }> = {
  loading: { label: "加载中…", cls: "bg-gray-200 text-gray-500" },
  connecting: { label: "连接中…", cls: "bg-gray-200 text-gray-500" },
  connected: { label: "● 已连接", cls: "bg-emerald-100 text-emerald-700" },
  disconnected: { label: "○ 已断开，重连中…", cls: "bg-red-100 text-red-600" },
};

const fmtTime = (ms: number) => (ms ? new Date(ms).toLocaleTimeString("zh-CN", { hour12: false }) : "");

export default function App() {
  const [snap, updatedAt, conn, reload] = useDashboard();
  const [modalOpen, setModalOpen] = useState(false);

  const connInfo = CONN_TEXT[conn] ?? CONN_TEXT.connecting;
  const devices = snap?.devices.map((d) => d.name) ?? [];

  return (
    <div className="min-h-screen bg-background p-4">
      <header className="mb-4 flex flex-wrap items-baseline gap-3">
        <h1 className="text-xl">
          Telemux <span className="text-sm font-normal text-muted-foreground">Dev Dashboard</span>
        </h1>
        <Badge className={connInfo.cls}>{connInfo.label}</Badge>
        <span className="ml-auto text-xs text-muted-foreground">{updatedAt ? `更新于 ${fmtTime(updatedAt)}` : ""}</span>
        <Button size="sm" onClick={() => setModalOpen(true)}>
          ＋ 新增寄存器
        </Button>
      </header>

      <main>
        {!snap ? (
          <div className="py-16 text-center text-muted-foreground">正在加载…</div>
        ) : snap.devices.length === 0 ? (
          <div className="py-16 text-center text-muted-foreground">未配置设备</div>
        ) : (
          snap.devices.map((dev) => <DeviceTable key={dev.name} dev={dev} />)
        )}
      </main>

      <RegisterModal
        open={modalOpen}
        devices={devices}
        onOpenChange={setModalOpen}
        onAdded={() => reload()}
      />
    </div>
  );
}
