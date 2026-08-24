import { useState } from "react";
import {
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Switch,
} from "@telemux/ui";
import { Plus, Trash2 } from "lucide-react";
import type { CreateRegisterRequest } from "@telemux/ui";

const STAGE_TYPES = ["scale", "sliding_average", "median", "math", "threshold", "aggregate"];

interface StageParamDef {
  name: string;
  type: "number" | "text" | "select";
  label: string;
  def: string;
  options?: string[];
  min?: string;
  step?: string;
}

function stageParams(type: string): StageParamDef[] {
  switch (type) {
    case "scale":
      return [
        { name: "scale", type: "number", label: "scale", def: "1", step: "any" },
        { name: "offset", type: "number", label: "offset", def: "0", step: "any" },
        { name: "unit", type: "text", label: "单位(可选)", def: "" },
      ];
    case "sliding_average":
    case "median":
      return [{ name: "window", type: "number", label: "window", def: "5", min: "1" }];
    case "math":
      return [{ name: "expression", type: "text", label: "表达式(变量 v)", def: "v * 1" }];
    case "threshold":
      return [
        { name: "low_critical", type: "number", label: "低临界", def: "", step: "any" },
        { name: "low_warning", type: "number", label: "低警告", def: "", step: "any" },
        { name: "high_warning", type: "number", label: "高警告", def: "", step: "any" },
        { name: "high_critical", type: "number", label: "高临界", def: "", step: "any" },
      ];
    case "aggregate":
      return [
        { name: "window", type: "number", label: "window", def: "4", min: "1" },
        { name: "mode", type: "select", label: "mode", def: "avg", options: ["min", "max", "avg"] },
      ];
    default:
      return [];
  }
}

interface StageState {
  id: number;
  type: string;
  values: Record<string, string>;
}

let stageSeq = 1;

export function RegisterModal({
  open,
  devices,
  onOpenChange,
  onAdded,
}: {
  open: boolean;
  devices: string[];
  onOpenChange: (v: boolean) => void;
  onAdded: () => void;
}) {
  const [device, setDevice] = useState(devices[0] ?? "");
  const [name, setName] = useState("");
  const [sensor, setSensor] = useState("");
  const [fn, setFn] = useState("holding");
  const [address, setAddress] = useState("0");
  const [valueType, setValueType] = useState("u16");
  const [wordOrder, setWordOrder] = useState("big");
  const [unit, setUnit] = useState("");
  const [pipelineOn, setPipelineOn] = useState(false);
  const [stages, setStages] = useState<StageState[]>([]);
  const [err, setErr] = useState("");
  const [ok, setOk] = useState("");
  const [busy, setBusy] = useState(false);

  const reset = () => {
    setName("");
    setSensor("");
    setFn("holding");
    setAddress("0");
    setValueType("u16");
    setWordOrder("big");
    setUnit("");
    setPipelineOn(false);
    setStages([]);
    setErr("");
    setOk("");
    setBusy(false);
  };

  const addStage = () => {
    setStages((s) => [...s, { id: stageSeq++, type: "scale", values: {} }]);
  };

  const patchStage = (id: number, patch: Partial<StageState>) => {
    setStages((s) => s.map((st) => (st.id === id ? { ...st, ...patch } : st)));
  };

  const collectStages = () =>
    stages.map((st) => {
      const stage: Record<string, unknown> = { type: st.type };
      for (const p of stageParams(st.type)) {
        const v = (st.values[p.name] ?? "").trim();
        if (v === "") continue;
        stage[p.name] = p.type === "number" ? parseFloat(v) : v;
      }
      return stage;
    });

  const submit = async () => {
    const addressNum = parseInt(address, 10);
    if (!name.trim() || !sensor.trim()) {
      setErr("寄存器名称和 sensor_id 必填");
      return;
    }
    if (!Number.isInteger(addressNum) || addressNum < 0 || addressNum > 65535) {
      setErr("地址必须是 0-65535 的整数");
      return;
    }
    const payload: CreateRegisterRequest = {
      device,
      register: {
        name: name.trim(),
        sensor_id: sensor.trim(),
        function: fn,
        address: addressNum,
        count: null,
        value_type: valueType,
        word_order: wordOrder,
        unit: unit.trim() || null,
        access: "read",
      },
      pipeline: null,
    };
    if (pipelineOn) {
      const stagesData = collectStages();
      if (stagesData.length === 0) {
        setErr("已勾选处理管道，请至少添加一个 stage");
        return;
      }
      payload.pipeline = { sensor_id: sensor.trim(), stages: stagesData };
    }

    setBusy(true);
    setErr("");
    setOk("");
    try {
      const res = await fetch("/api/registers", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(payload),
      });
      const data = (await res.json().catch(() => ({}))) as { error?: string };
      if (res.ok) {
        setOk("✓ 提交成功，正在刷新寄存器表…");
        onAdded(); // 父组件重拉全量
        setTimeout(() => {
          onOpenChange(false);
          reset();
        }, 1200);
      } else {
        setErr(`校验失败：\n${data.error ?? "未知错误"}`);
      }
    } catch (e) {
      setErr(`请求失败：${(e as Error).message}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(v) => (v ? reset() : onOpenChange(false))}>
      <DialogContent className="max-w-2xl">
        <DialogHeader>
          <DialogTitle>新增寄存器</DialogTitle>
          <DialogDescription>配置一个寄存器并（可选）绑定处理管道。</DialogDescription>
        </DialogHeader>

        <div className="grid grid-cols-2 gap-x-4 gap-y-4">
          <div className="flex flex-col gap-1.5">
            <Label>设备</Label>
            <Select value={device} onValueChange={setDevice}>
              <SelectTrigger>
                <SelectValue placeholder="选择设备" />
              </SelectTrigger>
              <SelectContent>
                {devices.map((d) => (
                  <SelectItem key={d} value={d}>
                    {d}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="flex flex-col gap-1.5">
            <Label>寄存器名称</Label>
            <Input placeholder="如 volt_raw" value={name} onChange={(e) => setName(e.target.value)} />
          </div>
          <div className="flex flex-col gap-1.5">
            <Label>sensor_id（全局唯一）</Label>
            <Input placeholder="如 pcba-01.volt" value={sensor} onChange={(e) => setSensor(e.target.value)} />
          </div>
          <div className="flex flex-col gap-1.5">
            <Label>功能码</Label>
            <Select value={fn} onValueChange={setFn}>
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="holding">holding</SelectItem>
                <SelectItem value="input">input</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div className="flex flex-col gap-1.5">
            <Label>起始地址</Label>
            <Input
              type="number"
              min={0}
              max={65535}
              value={address}
              onChange={(e) => setAddress(e.target.value)}
            />
          </div>
          <div className="flex flex-col gap-1.5">
            <Label>数值类型</Label>
            <Select value={valueType} onValueChange={setValueType}>
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {["u16", "i16", "u32", "i32", "f32"].map((t) => (
                  <SelectItem key={t} value={t}>
                    {t}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="flex flex-col gap-1.5">
            <Label>字序（多寄存器）</Label>
            <Select value={wordOrder} onValueChange={setWordOrder}>
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="big">big</SelectItem>
                <SelectItem value="little">little</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div className="flex flex-col gap-1.5">
            <Label>单位（可选）</Label>
            <Input placeholder="如 mV" value={unit} onChange={(e) => setUnit(e.target.value)} />
          </div>
          <div className="col-span-2 flex items-center gap-2">
            <Switch id="pipeline-switch" checked={pipelineOn} onCheckedChange={setPipelineOn} />
            <Label htmlFor="pipeline-switch">配置处理管道（原始值 → 计算值）</Label>
          </div>
        </div>

        {pipelineOn && (
          <div className="flex flex-col gap-3">
            {stages.map((st) => (
              <div key={st.id} className="rounded-lg border p-3">
                <div className="mb-2 flex items-center gap-2">
                  <Select value={st.type} onValueChange={(v) => patchStage(st.id, { type: v })}>
                    <SelectTrigger className="w-48">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {STAGE_TYPES.map((t) => (
                        <SelectItem key={t} value={t}>
                          {t}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                  <Button variant="outline" size="sm" onClick={() => setStages((s) => s.filter((x) => x.id !== st.id))}>
                    <Trash2 data-icon="inline-start" />
                    删除
                  </Button>
                </div>
                <div className="grid grid-cols-3 gap-2">
                  {stageParams(st.type).map((p) => (
                    <div key={p.name} className="flex flex-col gap-1">
                      <Label className="text-xs">{p.label}</Label>
                      {p.type === "select" ? (
                        <Select
                          value={st.values[p.name] ?? p.def}
                          onValueChange={(v) => patchStage(st.id, { values: { ...st.values, [p.name]: v } })}
                        >
                          <SelectTrigger className="h-8">
                            <SelectValue />
                          </SelectTrigger>
                          <SelectContent>
                            {p.options?.map((o) => (
                              <SelectItem key={o} value={o}>
                                {o}
                              </SelectItem>
                            ))}
                          </SelectContent>
                        </Select>
                      ) : (
                        <Input
                          className="h-8"
                          type={p.type === "number" ? "number" : "text"}
                          min={p.min}
                          step={p.step}
                          value={st.values[p.name] ?? p.def}
                          onChange={(e) => patchStage(st.id, { values: { ...st.values, [p.name]: e.target.value } })}
                        />
                      )}
                    </div>
                  ))}
                </div>
              </div>
            ))}
            <Button variant="outline" size="sm" onClick={addStage} className="self-start">
              <Plus data-icon="inline-start" />
              添加 stage
            </Button>
          </div>
        )}

        {err && (
          <div className="whitespace-pre-wrap rounded-md border border-destructive/30 bg-destructive/10 p-2 text-xs text-destructive">
            {err}
          </div>
        )}
        {ok && (
          <div className="rounded-md border border-emerald-200 bg-emerald-50 p-2 text-xs text-emerald-700">{ok}</div>
        )}

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            取消
          </Button>
          <Button onClick={() => void submit()} disabled={busy}>
            {busy ? "提交中…" : "提交"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
