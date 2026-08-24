import { useState } from "react";
import { Badge, Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@telemux/ui";
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

const STATUS_CLS: Record<string, string> = {
  normal: "text-emerald-600",
  warning: "text-amber-600",
  critical: "text-red-600",
  unknown: "text-gray-400",
};

function RegisterRow({ reg }: { reg: RegisterSnapshot }) {
  const [open, setOpen] = useState(false);

  const valueType = reg.value_type + (reg.count > 1 ? `×${reg.count}` : "");
  const rawStale =
    reg.raw && Date.now() - reg.raw.timestamp_ms > 5000 ? "text-gray-300" : "text-gray-700";

  return (
    <>
      <TableRow className="cursor-pointer hover:bg-blue-50" onClick={() => setOpen((o) => !o)}>
        <TableCell className="font-medium">{reg.name}</TableCell>
        <TableCell className="font-mono text-xs">{reg.sensor_id}</TableCell>
        <TableCell>{reg.function}</TableCell>
        <TableCell>{reg.address}</TableCell>
        <TableCell>{valueType}</TableCell>
        <TableCell>{reg.word_order}</TableCell>
        <TableCell>{reg.unit || "—"}</TableCell>
        <TableCell className={`font-mono font-semibold ${rawStale}`}>{reg.raw ? fmt(reg.raw.value) : "—"}</TableCell>
        <TableCell className={`font-mono font-semibold ${reg.metric ? STATUS_CLS[reg.metric.status] ?? "" : "text-gray-300"}`}>
          {reg.metric ? `${fmt(reg.metric.value)}${reg.metric.unit ? " " + reg.metric.unit : ""}` : "—"}
        </TableCell>
        <TableCell className={reg.metric ? STATUS_CLS[reg.metric.status] ?? "" : "text-gray-300"}>
          {reg.metric ? reg.metric.status : "—"}
        </TableCell>
        <TableCell className="text-xs text-muted-foreground">{reg.raw ? fmtTime(reg.raw.timestamp_ms) : "—"}</TableCell>
      </TableRow>
      {open && (
        <TableRow className="bg-gray-50">
          <TableCell colSpan={COLUMNS.length} className="p-3">
            <div className="text-sm">
              {reg.stages.length > 0 ? (
                <>
                  <div className="mb-1">
                    计算链路：<b className="text-blue-600">{reg.formula || reg.stages.join(" → ")}</b>
                  </div>
                  <ul className="list-none">
                    {reg.stages.map((s, i) => (
                      <li key={i} className="py-0.5">
                        <span className="mr-1.5 text-muted-foreground">#{i + 1}</span>
                        {s}
                      </li>
                    ))}
                  </ul>
                </>
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
    <section className="mb-4 overflow-hidden rounded-lg border bg-card">
      <h2 className="flex items-center gap-2 border-b px-4 py-2.5 text-base">
        {dev.name}
        <Badge className={dev.connected ? "bg-emerald-100 text-emerald-700" : "bg-red-100 text-red-600"}>
          {dev.connected ? "● online" : "○ offline"}
        </Badge>
      </h2>
      <div className="px-4 py-1.5 text-xs text-muted-foreground">
        {dev.transport} {dev.host}:{dev.port} · {dev.registers.length} 个寄存器
      </div>
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
    </section>
  );
}
