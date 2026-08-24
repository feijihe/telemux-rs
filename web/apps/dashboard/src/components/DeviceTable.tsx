import { useState } from "react";
import {
  Badge,
  Card,
  CardContent,
  CardHeader,
  CardTitle,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@telemux/ui";
import { ChevronDown, ChevronRight, Server } from "lucide-react";
import type { DeviceSnapshot, RegisterSnapshot } from "@telemux/ui";

const COLUMNS = ["寄存器", "sensor_id", "功能", "地址", "类型", "字序", "单位", "原始值", "计算值", "状态", "更新时间"];

const fmt = (v: number | null | undefined): string => {
  if (v === null || v === undefined) return "—";
  const n = parseFloat(String(v));
  if (Number.isNaN(n)) return String(v);
  return Number.isInteger(n) ? String(n) : String(parseFloat(n.toPrecision(6)));
};

const fmtTime = (ms: number | null | undefined): string => {
  if (!ms) return "—";
  return new Date(ms).toLocaleTimeString("zh-CN", { hour12: false });
};

const STATUS_VARIANT: Record<string, "default" | "secondary" | "destructive" | "outline"> = {
  normal: "default",
  warning: "secondary",
  critical: "destructive",
  unknown: "outline",
};

function RegisterRow({ reg }: { reg: RegisterSnapshot }) {
  const [open, setOpen] = useState(false);

  const valueType = reg.value_type + (reg.count > 1 ? `×${reg.count}` : "");
  const rawStale = reg.raw && Date.now() - reg.raw.timestamp_ms > 5000;
  const status = reg.metric?.status ?? "unknown";

  return (
    <>
      <TableRow
        className="cursor-pointer"
        onClick={() => setOpen((o) => !o)}
        data-state={open ? "open" : undefined}
      >
        <TableCell className="font-medium">
          <span className="inline-flex items-center gap-1.5">
            {open ? <ChevronDown className="size-3.5 text-muted-foreground" /> : <ChevronRight className="size-3.5 text-muted-foreground" />}
            {reg.name}
          </span>
        </TableCell>
        <TableCell className="font-mono text-xs">{reg.sensor_id}</TableCell>
        <TableCell>{reg.function}</TableCell>
        <TableCell>{reg.address}</TableCell>
        <TableCell>{valueType}</TableCell>
        <TableCell>{reg.word_order}</TableCell>
        <TableCell>{reg.unit || "—"}</TableCell>
        <TableCell className={`font-mono font-semibold ${rawStale ? "text-muted-foreground" : ""}`}>
          {reg.raw ? fmt(reg.raw.value) : "—"}
        </TableCell>
        <TableCell className="font-mono font-semibold">
          {reg.metric ? `${fmt(reg.metric.value)}${reg.metric.unit ? " " + reg.metric.unit : ""}` : <span className="text-muted-foreground">—</span>}
        </TableCell>
        <TableCell>
          <Badge variant={STATUS_VARIANT[status] ?? "outline"} className="font-normal">
            {reg.metric ? reg.metric.status : "—"}
          </Badge>
        </TableCell>
        <TableCell className="text-xs text-muted-foreground">{reg.raw ? fmtTime(reg.raw.timestamp_ms) : "—"}</TableCell>
      </TableRow>
      {open && (
        <TableRow className="bg-muted/30">
          <TableCell colSpan={COLUMNS.length} className="p-3">
            <div className="text-sm">
              {reg.stages.length > 0 ? (
                <div className="flex flex-col gap-1">
                  <div className="text-muted-foreground">
                    计算链路：<span className="font-semibold text-foreground">{reg.formula || reg.stages.join(" → ")}</span>
                  </div>
                  <ul className="flex flex-col gap-0.5">
                    {reg.stages.map((s, i) => (
                      <li key={i} className="flex items-baseline gap-2 font-mono text-xs">
                        <span className="text-muted-foreground">#{i + 1}</span>
                        <span>{s}</span>
                      </li>
                    ))}
                  </ul>
                </div>
              ) : (
                <div className="text-muted-foreground">该寄存器未配置处理管道（无计算值）。</div>
              )}
            </div>
          </TableCell>
        </TableRow>
      )}
    </>
  );
}

export function DeviceTable({ dev }: { dev: DeviceSnapshot }) {
  return (
    <Card>
      <CardHeader className="flex flex-row items-center gap-2 space-y-0 py-3">
        <Server className="size-4 text-primary" />
        <CardTitle className="text-base">{dev.name}</CardTitle>
        <Badge variant={dev.connected ? "default" : "destructive"} className="font-normal">
          {dev.connected ? "● online" : "○ offline"}
        </Badge>
        <span className="ml-auto text-xs text-muted-foreground">
          {dev.transport} {dev.host}:{dev.port} · {dev.registers.length} 个寄存器
        </span>
      </CardHeader>
      <CardContent className="p-0">
        <Table>
          <TableHeader>
            <TableRow>
              {COLUMNS.map((c) => (
                <TableHead key={c}>{c}</TableHead>
              ))}
            </TableRow>
          </TableHeader>
          <TableBody>
            {dev.registers.map((reg) => (
              <RegisterRow key={reg.sensor_id} reg={reg} />
            ))}
          </TableBody>
        </Table>
      </CardContent>
    </Card>
  );
}
