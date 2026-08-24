import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@telemux/ui";
import type { SimState } from "@telemux/ui";

const hex = (n: number) => `0x${n.toString(16).padStart(4, "0")}`;

export function RegisterTable({ state }: { state: SimState }) {
  return (
    <div className="overflow-x-auto">
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>区域</TableHead>
            <TableHead>地址</TableHead>
            <TableHead>槽位</TableHead>
            <TableHead>原始值</TableHead>
            <TableHead>解码</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {state.holding.map((h) => (
            <TableRow key={`h-${h.addr}`}>
              <TableCell>保持</TableCell>
              <TableCell className="font-mono text-muted-foreground">{hex(h.addr)}</TableCell>
              <TableCell>{h.slot ? h.slot.control : "空"}</TableCell>
              <TableCell className="font-mono">{h.raw}</TableCell>
              <TableCell>{h.slot ? (h.slot.writable ? "可写 u16" : "只读 u16") : "—"}</TableCell>
            </TableRow>
          ))}
          {state.inputs.map((inp) => (
            <TableRow key={`i-${inp.addr}`}>
              <TableCell>输入</TableCell>
              <TableCell className="font-mono text-muted-foreground">{hex(inp.addr)}</TableCell>
              <TableCell>{inp.sensor ?? "空"}</TableCell>
              <TableCell className="font-mono">
                {inp.raw_hi} {inp.raw_lo}
              </TableCell>
              <TableCell className="font-mono">{inp.value_f32.toFixed(3)}</TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </div>
  );
}
