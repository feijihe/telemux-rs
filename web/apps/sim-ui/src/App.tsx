import { Badge } from "@telemux/ui";
import { ControlPanel } from "./components/ControlPanel";
import { RegisterTable } from "./components/RegisterTable";
import { SystemDiagram } from "./components/SystemDiagram";
import { useSimState } from "./hooks/useSimState";

const CONN_TEXT: Record<string, { label: string; cls: string }> = {
  connecting: { label: "连接中…", cls: "bg-gray-200 text-gray-500" },
  connected: { label: "已连接 · WebSocket", cls: "bg-emerald-100 text-emerald-700" },
  disconnected: { label: "断开，重连中…", cls: "bg-red-100 text-red-600" },
};

export default function App() {
  const [state, conn] = useSimState();
  const connInfo = CONN_TEXT[conn] ?? CONN_TEXT.connecting;

  return (
    <div className="min-h-screen bg-background p-4">
      <header className="mb-4 flex items-baseline gap-3">
        <h1 className="text-xl">Telemux-Sim · CDU 仿真器</h1>
        <Badge className={connInfo.cls}>{connInfo.label}</Badge>
        {state && (
          <span className="ml-auto text-xs text-muted-foreground">
            {state.controls.length} 控制变量 · {state.sensors.length} 传感器
          </span>
        )}
      </header>

      {state ? (
        <>
          <ControlPanel state={state} />
          <h2 className="mb-2 mt-6 border-b pb-1 text-base">CDU 系统图（实时）</h2>
          <SystemDiagram state={state} />
          <h2 className="mb-2 mt-6 border-b pb-1 text-base">寄存器地图原始值</h2>
          <RegisterTable state={state} />
        </>
      ) : (
        <div className="py-16 text-center text-muted-foreground">正在加载…</div>
      )}
    </div>
  );
}
