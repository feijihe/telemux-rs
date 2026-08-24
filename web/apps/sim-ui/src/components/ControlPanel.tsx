import { useEffect, useState } from "react";
import { Badge, Button, Card, CardContent, CardDescription, CardHeader, CardTitle, Input, Label } from "@telemux/ui";
import { Fan, Thermometer, Waves } from "lucide-react";
import type { SimState } from "@telemux/ui";
import { setControl } from "../hooks/useSimState";

interface FieldDef {
  control: string;
  label: string;
  unit: string;
  step: number;
}

const TEMP_FIELDS: FieldDef[] = [
  { control: "primary_cold_temp", label: "一次侧冷水 T1", unit: "°C", step: 0.5 },
  { control: "secondary_hot_temp", label: "二次侧热水 T5", unit: "°C", step: 0.5 },
];

const DUTY_FIELDS: FieldDef[] = [
  { control: "pump1_duty", label: "Pump1 duty", unit: "%", step: 1 },
  { control: "pump2_duty", label: "Pump2 duty", unit: "%", step: 1 },
  { control: "valve1_duty", label: "Valve1 duty", unit: "%", step: 1 },
  { control: "fan_duty", label: "Fan duty", unit: "%", step: 1 },
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
      <Label className="w-36 shrink-0 text-xs text-muted-foreground">{def.label}</Label>
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
      <Button type="button" size="sm" variant="secondary" onClick={() => void apply()}>
        应用
      </Button>
      <Badge variant="outline" className="w-20 justify-end font-mono tabular-nums">
        {value} {def.unit}
      </Badge>
    </div>
  );
}

export function ControlPanel({ state }: { state: SimState }) {
  const controlValue = (name: string) => state.controls.find((c) => c.name === name)?.value ?? 0;

  return (
    <div className="grid gap-4 lg:grid-cols-2">
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2 text-sm">
            <Thermometer className="size-4 text-primary" />
            温度设定（立即生效）
          </CardTitle>
          <CardDescription className="text-xs">设定一次侧冷水与二次侧热水目标温度。</CardDescription>
        </CardHeader>
        <CardContent>
          {TEMP_FIELDS.map((f) => (
            <FieldRow key={f.control} def={f} value={controlValue(f.control)} />
          ))}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2 text-sm">
            <Waves className="size-4 text-primary" />
            运行控制
          </CardTitle>
          <CardDescription className="text-xs">
            <span className="flex items-center gap-1">
              <Fan className="size-3" />
              泵/阀/风扇开度（0-100%）
            </span>
          </CardDescription>
        </CardHeader>
        <CardContent>
          {DUTY_FIELDS.map((f) => (
            <FieldRow key={f.control} def={f} value={controlValue(f.control)} />
          ))}
        </CardContent>
      </Card>
    </div>
  );
}
