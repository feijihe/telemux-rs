import { useEffect, useState } from "react";
import { Badge, Button, Card, CardContent, CardHeader, CardTitle, Input, Label } from "@telemux/ui";
import type { SimState } from "@telemux/ui";
import { setControl } from "../hooks/useSimState";

interface FieldDef {
  control: string;
  label: string;
  unit: string;
  step: number;
}

const TEMP_FIELDS: FieldDef[] = [
  { control: "primary_cold_temp", label: "一次侧冷水 T1 (°C)", unit: "°C", step: 0.5 },
  { control: "secondary_hot_temp", label: "二次侧热水 T5 (°C)", unit: "°C", step: 0.5 },
];

const DUTY_FIELDS: FieldDef[] = [
  { control: "pump1_duty", label: "Pump1 duty (%)", unit: "%", step: 1 },
  { control: "pump2_duty", label: "Pump2 duty (%)", unit: "%", step: 1 },
  { control: "valve1_duty", label: "Valve1 duty (%)", unit: "%", step: 1 },
  { control: "fan_duty", label: "Fan duty (%)", unit: "%", step: 1 },
];

function FieldRow({ def, value }: { def: FieldDef; value: number }) {
  const [local, setLocal] = useState<string>(String(value));
  // 外部值变化且输入框未聚焦时同步（避免打断用户编辑）
  const [focused, setFocused] = useState(false);
  useEffect(() => {
    if (!focused) setLocal(String(value));
  }, [value, focused]);

  const apply = async () => {
    const n = parseFloat(local);
    if (Number.isNaN(n)) return;
    const ok = await setControl(def.control, n);
    if (ok) setLocal(String(n));
  };

  return (
    <div className="flex items-center gap-2 py-1">
      <Label className="w-44 shrink-0 text-xs">{def.label}</Label>
      <Input
        type="number"
        step={def.step}
        value={local}
        className="h-8 w-24"
        onFocus={() => setFocused(true)}
        onBlur={() => setFocused(false)}
        onChange={(e) => setLocal(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") void apply();
        }}
      />
      <Button type="button" size="sm" onClick={() => void apply()}>
        应用
      </Button>
      <Badge variant="outline" className="font-mono text-emerald-600">
        {value} {def.unit}
      </Badge>
    </div>
  );
}

export function ControlPanel({ state }: { state: SimState }) {
  const controlValue = (name: string) => state.controls.find((c) => c.name === name)?.value ?? 0;

  return (
    <div className="flex flex-wrap gap-4">
      <Card>
        <CardHeader className="p-4 pb-2">
          <CardTitle className="text-sm">温度设定（立即生效）</CardTitle>
        </CardHeader>
        <CardContent className="p-4 pt-1">
          {TEMP_FIELDS.map((f) => (
            <FieldRow key={f.control} def={f} value={controlValue(f.control)} />
          ))}
        </CardContent>
      </Card>
      <Card>
        <CardHeader className="p-4 pb-2">
          <CardTitle className="text-sm">运行控制</CardTitle>
        </CardHeader>
        <CardContent className="p-4 pt-1">
          {DUTY_FIELDS.map((f) => (
            <FieldRow key={f.control} def={f} value={controlValue(f.control)} />
          ))}
        </CardContent>
      </Card>
    </div>
  );
}
