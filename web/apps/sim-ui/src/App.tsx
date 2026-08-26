import { Badge, Card, CardContent, CardDescription, CardHeader, CardTitle, Separator } from "@telemux/ui";
import { Activity, Database, Gauge, Server } from "lucide-react";
import { ControlPanel } from "./components/ControlPanel";
import { RegisterTable } from "./components/RegisterTable";
import { SystemDiagram } from "./components/SystemDiagram";
import { useSimState } from "./hooks/useSimState";

const CONN_TEXT: Record<string, { label: string; variant: "default" | "secondary" | "destructive" | "outline" }> = {
  connecting: { label: "连接中…", variant: "secondary" },
  connected: { label: "已连接 · WebSocket", variant: "default" },
  disconnected: { label: "断开，重连中…", variant: "destructive" },
};

export default function App() {
  const [state, conn] = useSimState();
  const connInfo = CONN_TEXT[conn] ?? CONN_TEXT.connecting;

  return (
    <div className="min-h-screen bg-background">
      <header className="sticky top-0 z-10 border-b bg-background/95 backdrop-blur">
        <div className="mx-auto flex max-w-7xl items-center gap-3 px-4 py-3">
          <Server className="size-5 text-primary" />
          <h1 className="text-base font-semibold tracking-tight">Telemux-Sim · CDU 仿真器</h1>
          <Badge variant={connInfo.variant}>{connInfo.label}</Badge>
          {state && (
            <span className="ml-auto flex items-center gap-2 text-xs text-muted-foreground">
              <span className="flex items-center gap-1">
                <Gauge className="size-3.5" />
                {state.controls.length} 控制变量
              </span>
              <Separator orientation="vertical" className="h-3" />
              <span className="flex items-center gap-1">
                <Database className="size-3.5" />
                {state.sensors.length} 传感器
              </span>
            </span>
          )}
        </div>
      </header>

      <main className="mx-auto flex max-w-7xl flex-col gap-6 px-4 py-6">
        {state ? (
          <>
            <ControlPanel state={state} />

            <section className="flex flex-col gap-3">
              <CardHeader className="p-0">
                <CardTitle className="flex items-center gap-2 text-sm">
                  <Activity className="size-4 text-primary" />
                  CDU 系统图（实时）
                </CardTitle>
                <CardDescription>一次侧冷却水 → 二次侧负载回路，稳态物理模型，每 500ms 更新。</CardDescription>
              </CardHeader>
              <SystemDiagram state={state} />
            </section>

            <section className="flex flex-col gap-3">
              <CardHeader className="p-0">
                <CardTitle className="flex items-center gap-2 text-sm">
                  <Database className="size-4 text-primary" />
                  寄存器地图原始值
                </CardTitle>
                <CardDescription>Modbus 保持寄存器与输入寄存器在仿真器中的实时映射。</CardDescription>
              </CardHeader>
              <Card>
                <CardContent className="p-0">
                  <RegisterTable state={state} />
                </CardContent>
              </Card>
            </section>
          </>
        ) : (
          <div className="flex flex-col items-center justify-center gap-2 py-24 text-muted-foreground">
            <Activity className="size-8 animate-pulse" />
            <p className="text-sm">正在加载仿真状态…</p>
          </div>
        )}
      </main>
    </div>
  );
}
