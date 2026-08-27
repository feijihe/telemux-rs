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
              <TableCell>
                {h.slot ? (
                  h.slot.type === "control" ? (
                    h.slot.control
                  ) : (
                    <span className="text-primary">{h.slot.sensor}</span>
                  )
                ) : (
                  <span className="text-muted-foreground">空</span>
                )}
              </TableCell>
              <TableCell className="font-mono tabular-nums">{h.raw}</TableCell>
              <TableCell>
                {h.slot ? (
                  h.slot.type === "control" ? (
                    h.slot.writable ? (
                      "可写 u16"
                    ) : (
                      "只读 u16"
                    )
                  ) : h.slot.storage === "u16" ? (
                    "只读 u16 原始值"
                  ) : (
                    "只读 f32"
                  )
                ) : (
                  "—"
                )}
              </TableCell>
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
                {inp.storage === "u16" ? (
                  <span className="font-semibold">{inp.raw_hi}</span>
                ) : (
                  <>
                    {inp.raw_hi} {inp.raw_lo}
                  </>
                )}
              </TableCell>
              <TableCell className="font-mono tabular-nums">
                {inp.storage === "u16" ? (
                  <span className="text-muted-foreground">u16 原始值</span>
                ) : (
                  inp.value_f32?.toFixed(3) ?? "—"
                )}
              </TableCell>
            </TableRow>
          ))}
          {state.coils.map((c) => (
            <TableRow key={`c-${c.addr}`}>
              <TableCell>
                <Badge className="font-normal" variant={c.value ? "default" : "outline"}>
                  线圈
                </Badge>
              </TableCell>
              <TableCell className="font-mono text-muted-foreground">{hex(c.addr)}</TableCell>
              <TableCell>{c.sensor ?? <span className="text-muted-foreground">空</span>}</TableCell>
              <TableCell className="font-mono tabular-nums">{c.value ? "ON" : "OFF"}</TableCell>
              <TableCell>
                {c.value ? (
                  <span className="text-emerald-600">true</span>
                ) : (
                  <span className="text-muted-foreground">false</span>
                )}
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </div>
  );
}
