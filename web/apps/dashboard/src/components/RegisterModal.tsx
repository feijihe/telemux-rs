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
} from "@telemux/ui";
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

        <div className="grid grid-cols-2 gap-x-4 gap-y-3">
          <div>
            <Label>设备</Label>
            <Select className="mt-1" value={device} onChange={(e) => setDevice(e.target.value)}>
              {devices.map((d) => (
                <option key={d} value={d}>
                  {d}
                </option>
              ))}
            </Select>
          </div>
          <div>
            <Label>寄存器名称</Label>
            <Input className="mt-1" placeholder="如 volt_raw" value={name} onChange={(e) => setName(e.target.value)} />
          </div>
          <div>
            <Label>sensor_id（全局唯一）</Label>
            <Input className="mt-1" placeholder="如 pcba-01.volt" value={sensor} onChange={(e) => setSensor(e.target.value)} />
          </div>
          <div>
            <Label>功能码</Label>
            <Select className="mt-1" value={fn} onChange={(e) => setFn(e.target.value)}>
              <option value="holding">holding</option>
              <option value="input">input</option>
            </Select>
          </div>
          <div>
            <Label>起始地址</Label>
            <Input
              className="mt-1"
              type="number"
              min={0}
              max={65535}
              value={address}
              onChange={(e) => setAddress(e.target.value)}
            />
          </div>
          <div>
            <Label>数值类型</Label>
            <Select className="mt-1" value={valueType} onChange={(e) => setValueType(e.target.value)}>
              {["u16", "i16", "u32", "i32", "f32"].map((t) => (
                <option key={t} value={t}>
                  {t}
                </option>
              ))}
            </Select>
          </div>
          <div>
            <Label>字序（多寄存器）</Label>
            <Select className="mt-1" value={wordOrder} onChange={(e) => setWordOrder(e.target.value)}>
              <option value="big">big</option>
              <option value="little">little</option>
            </Select>
          </div>
          <div>
            <Label>单位（可选）</Label>
            <Input className="mt-1" placeholder="如 mV" value={unit} onChange={(e) => setUnit(e.target.value)} />
          </div>
          <div className="col-span-2">
            <label className="flex items-center gap-2 text-sm">
              <input type="checkbox" checked={pipelineOn} onChange={(e) => setPipelineOn(e.target.checked)} />
              配置处理管道（原始值 → 计算值）
            </label>
          </div>
        </div>

        {pipelineOn && (
          <div className="mt-2">
            {stages.map((st) => (
              <div key={st.id} className="mb-2 rounded-lg border p-3">
                <div className="mb-2 flex items-center gap-2">
                  <Select
                    className="w-48"
                    value={st.type}
                    onChange={(e) => patchStage(st.id, { type: e.target.value })}
                  >
                    {STAGE_TYPES.map((t) => (
                      <option key={t} value={t}>
                        {t}
                      </option>
                    ))}
                  </Select>
                  <Button variant="outline" size="sm" onClick={() => setStages((s) => s.filter((x) => x.id !== st.id))}>
                    删除
                  </Button>
                </div>
                <div className="grid grid-cols-3 gap-2">
                  {stageParams(st.type).map((p) => (
                    <div key={p.name}>
                      <Label className="text-xs">{p.label}</Label>
                      {p.type === "select" ? (
                        <Select
                          className="mt-1 h-8"
                          value={st.values[p.name] ?? p.def}
                          onChange={(e) => patchStage(st.id, { values: { ...st.values, [p.name]: e.target.value } })}
                        >
                          {p.options?.map((o) => (
                            <option key={o} value={o}>
                              {o}
                            </option>
                          ))}
                        </Select>
                      ) : (
                        <Input
                          className="mt-1 h-8"
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
            <Button variant="outline" size="sm" onClick={addStage}>
              ＋ 添加 stage
            </Button>
          </div>
        )}

        {err && <div className="whitespace-pre-wrap rounded-md border border-red-200 bg-red-50 p-2 text-xs text-red-600">{err}</div>}
        {ok && <div className="rounded-md border border-emerald-200 bg-emerald-50 p-2 text-xs text-emerald-600">{ok}</div>}

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            取消
          </Button>
          <Button onClick={() => void submit()} disabled={busy}>
            提交
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
