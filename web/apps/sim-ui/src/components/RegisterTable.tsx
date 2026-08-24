import { Badge, Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@telemux/ui";
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
              <TableCell>
                <Badge variant="outline" className="font-normal">
                  保持
                </Badge>
              </TableCell>
              <TableCell className="font-mono text-muted-foreground">{hex(h.addr)}</TableCell>
              <TableCell>{h.slot ? h.slot.control : <span className="text-muted-foreground">空</span>}</TableCell>
              <TableCell className="font-mono tabular-nums">{h.raw}</TableCell>
              <TableCell>{h.slot ? (h.slot.writable ? "可写 u16" : "只读 u16") : "—"}</TableCell>
            </TableRow>
          ))}
          {state.inputs.map((inp) => (
            <TableRow key={`i-${inp.addr}`}>
              <TableCell>
                <Badge variant="secondary" className="font-normal">
                  输入
                </Badge>
              </TableCell>
              <TableCell className="font-mono text-muted-foreground">{hex(inp.addr)}</TableCell>
              <TableCell>{inp.sensor ?? <span className="text-muted-foreground">空</span>}</TableCell>
              <TableCell className="font-mono tabular-nums">
                {inp.raw_hi} {inp.raw_lo}
              </TableCell>
              <TableCell className="font-mono tabular-nums">{inp.value_f32.toFixed(3)}</TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </div>
  );
}
