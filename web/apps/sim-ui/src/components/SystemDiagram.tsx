import { useEffect, useRef } from "react";
import type { SimState } from "@telemux/ui";

// ===== Canvas 常量 =====
const CV_W = 1040;
const CV_H = 640;
const COL = {
  bg: "#fbfcfe",
  pipe_cold: "#5c9be6",
  pipe_hot: "#e07878",
  pipe: "#94a3b8",
  temperature: "#e0850f",
  pressure: "#3377cc",
  flow: "#2f9e44",
  level: "#7048e8",
  ph: "#c2255c",
  leak: "#e03131",
  humidity: "#1098ad",
};

// 传感器在系统图上的坐标（x, y）+ 标签
const SENSOR_POS: Record<string, { x: number; y: number; label: string }> = {
  // 一次侧（冷水，左侧）：进口管 y=112，回水管 y=390
  "cdu.pri.in.p1": { x: 110, y: 90, label: "P1" },
  "cdu.pri.in.t1": { x: 195, y: 90, label: "T1" },
  "cdu.pri.in.p2": { x: 110, y: 135, label: "P2" },
  "cdu.pri.in.t2": { x: 195, y: 135, label: "T2" },
  "cdu.pri.in.f1": { x: 280, y: 102, label: "F1" },
  "cdu.pri.out.t3": { x: 195, y: 350, label: "T3" },
  "cdu.pri.out.p3": { x: 110, y: 350, label: "P3" },
  "cdu.pri.out.t4": { x: 195, y: 395, label: "T4" },
  "cdu.pri.out.p4": { x: 110, y: 395, label: "P4" },
  // 二次侧（热水，右侧）：上管 y=112（PHE 出→服务器），下管 y=390（服务器→PHE 进）
  "cdu.sec.out.f2": { x: 760, y: 102, label: "F2" },
  "cdu.sec.in.t5": { x: 850, y: 90, label: "T5" },
  "cdu.sec.in.p5": { x: 930, y: 90, label: "P5" },
  "cdu.sec.in.t6": { x: 850, y: 135, label: "T6" },
  "cdu.sec.in.p6": { x: 930, y: 135, label: "P6" },
  "cdu.sec.out.t7": { x: 850, y: 350, label: "T7" },
  "cdu.sec.out.p7": { x: 930, y: 350, label: "P7" },
  "cdu.sec.out.t8": { x: 850, y: 395, label: "T8" },
  "cdu.sec.out.p8": { x: 930, y: 395, label: "P8" },
  // 补水/回水管路（原膨胀槽区，不画槽体）
  "cdu.tank.level": { x: 600, y: 60, label: "LL1" },
  "cdu.tank.ph": { x: 600, y: 130, label: "PH1" },
  // 环境 / 泄漏（右下）
  "cdu.env.temp": { x: 890, y: 460, label: "环境 T" },
  "cdu.env.rh": { x: 890, y: 495, label: "环境 RH" },
  "cdu.leak": { x: 890, y: 525, label: "LEAK" },
};

// 泵转速传感器（从 sensors 取值显示在泵体上）
const PUMP_SPEED_SENSORS = ["cdu.sec.in.pump1.speed", "cdu.sec.in.pump2.speed"];

function roundRect(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number) {
  ctx.beginPath();
  ctx.moveTo(x + r, y);
  ctx.arcTo(x + w, y, x + w, y + h, r);
  ctx.arcTo(x + w, y + h, x, y + h, r);
  ctx.arcTo(x, y + h, x, y, r);
  ctx.arcTo(x, y, x + w, y, r);
  ctx.closePath();
}

function drawArrow(ctx: CanvasRenderingContext2D, x1: number, y1: number, x2: number, y2: number, color: string) {
  ctx.strokeStyle = color;
  ctx.lineWidth = 2;
  ctx.beginPath();
  ctx.moveTo(x1, y1);
  ctx.lineTo(x2, y2);
  ctx.stroke();
  const ang = Math.atan2(y2 - y1, x2 - x1);
  ctx.beginPath();
  ctx.moveTo(x2, y2);
  ctx.lineTo(x2 - 8 * Math.cos(ang - 0.4), y2 - 8 * Math.sin(ang - 0.4));
  ctx.lineTo(x2 - 8 * Math.cos(ang + 0.4), y2 - 8 * Math.sin(ang + 0.4));
  ctx.closePath();
  ctx.fillStyle = color;
  ctx.fill();
}

function drawPump(ctx: CanvasRenderingContext2D, cx: number, cy: number, label: string, duty: number) {
  const w = 70;
  const h = 75;
  const r = 6;
  ctx.fillStyle = "#e7f5ff";
  ctx.strokeStyle = "#4dabf7";
  ctx.lineWidth = 2;
  roundRect(ctx, cx - w / 2, cy - h / 2, w, h, r);
  ctx.fill();
  ctx.stroke();
  ctx.strokeStyle = "#4dabf7";
  ctx.lineWidth = 3;
  ctx.beginPath();
  ctx.moveTo(cx - w / 2, cy);
  ctx.lineTo(cx - w / 2 - 8, cy);
  ctx.stroke();
  ctx.beginPath();
  ctx.moveTo(cx + w / 2, cy);
  ctx.lineTo(cx + w / 2 + 8, cy);
  ctx.stroke();
  ctx.fillStyle = "#4dabf7";
  ctx.font = "bold 12px Consolas, monospace";
  ctx.textAlign = "center";
  ctx.fillText(label, cx, cy - 6);
  ctx.fillStyle = "#0a7";
  ctx.font = "bold 13px Consolas, monospace";
  ctx.fillText(`${duty.toFixed(0)} RPM`, cx, cy + 14);
  ctx.textAlign = "left";
}

function draw(ctx: CanvasRenderingContext2D, state: SimState) {
  ctx.clearRect(0, 0, CV_W, CV_H);
  ctx.fillStyle = COL.bg;
  ctx.fillRect(0, 0, CV_W, CV_H);

  // ---- 管路 ----
  ctx.lineWidth = 4;
  ctx.lineCap = "round";

  // 一次侧（冷水回路）
  ctx.strokeStyle = COL.pipe_cold;
  ctx.beginPath();
  ctx.moveTo(40, 112);
  ctx.lineTo(330, 112); // 冷水进管
  ctx.stroke();
  ctx.beginPath();
  ctx.moveTo(330, 390);
  ctx.lineTo(40, 390); // 一次侧回水管
  ctx.stroke();

  // 二次侧：上管（PHE 出水→服务器）、下管（服务器→PHE 进水）
  ctx.strokeStyle = COL.pipe_cold;
  ctx.beginPath();
  ctx.moveTo(470, 112);
  ctx.lineTo(1000, 112);
  ctx.stroke();
  ctx.strokeStyle = COL.pipe_hot;
  ctx.beginPath();
  ctx.moveTo(1000, 390);
  ctx.lineTo(470, 390);
  ctx.stroke();

  // 泵组并联环路：泵2 在上横管、泵1 在下管主线
  const PLX = 545;
  const PRX = 655;
  const PTY = 268;
  const PBY = 390;
  ctx.strokeStyle = COL.pipe;
  ctx.lineWidth = 3;
  ctx.beginPath();
  ctx.moveTo(PLX, PBY);
  ctx.lineTo(PLX, PTY);
  ctx.stroke();
  ctx.beginPath();
  ctx.moveTo(PRX, PBY);
  ctx.lineTo(PRX, PTY);
  ctx.stroke();
  ctx.beginPath();
  ctx.moveTo(PLX, PTY);
  ctx.lineTo(PRX, PTY);
  ctx.stroke();
  // 补水支路
  ctx.lineWidth = 2.5;
  ctx.beginPath();
  ctx.moveTo(705, 112);
  ctx.lineTo(705, 140);
  ctx.stroke();

  // ---- PHEX ----
  ctx.fillStyle = "#dff0d8";
  ctx.strokeStyle = "#5b8c5a";
  ctx.lineWidth = 2;
  roundRect(ctx, 340, 90, 130, 310, 8);
  ctx.fill();
  ctx.stroke();
  ctx.fillStyle = "#5b8c5a";
  ctx.font = "bold 14px Consolas, monospace";
  ctx.textAlign = "center";
  ctx.fillText("PHEX", 405, 240);
  ctx.font = "10px Consolas, monospace";
  ctx.fillText("板式换热器", 405, 256);

  // ---- 泵（转速）----
  const byId = new Map(state.sensors.map((s) => [s.sensor_id, s]));
  const pump1 = byId.get(PUMP_SPEED_SENSORS[0])?.value ?? 0;
  const pump2 = byId.get(PUMP_SPEED_SENSORS[1])?.value ?? 0;
  drawPump(ctx, 600, 268, "泵2", pump2);
  drawPump(ctx, 600, 390, "泵1", pump1);

  // ---- 服务器 ----
  ctx.fillStyle = "#eee";
  ctx.strokeStyle = "#999";
  roundRect(ctx, 1000, 60, 26, 360, 4);
  ctx.fill();
  ctx.stroke();
  ctx.save();
  ctx.translate(1013, 240);
  ctx.rotate(Math.PI / 2);
  ctx.fillStyle = "#666";
  ctx.font = "10px Consolas, monospace";
  ctx.fillText("SERVER", 0, 0);
  ctx.restore();

  // ---- H1 比例阀（duty 来自控制变量）----
  const valve = state.controls.find((c) => c.name === "valve1_duty");
  const v1 = valve?.value ?? 0;
  ctx.fillStyle = "#f59f00";
  ctx.beginPath();
  ctx.moveTo(318, 104);
  ctx.lineTo(336, 112);
  ctx.lineTo(318, 120);
  ctx.closePath();
  ctx.fill();
  ctx.fillStyle = "#e8590c";
  ctx.font = "bold 11px Consolas, monospace";
  ctx.fillText("H1", 340, 118);
  ctx.fillText(`${v1.toFixed(0)}%`, 340, 132);

  // ---- 传感器标点 ----
  ctx.textAlign = "left";
  for (const [id, pos] of Object.entries(SENSOR_POS)) {
    const sv = byId.get(id);
    const kind = sv?.kind ?? "other";
    const color = (COL as Record<string, string>)[kind] ?? "#888";
    const val = sv && sv.value !== null ? Number(sv.value).toFixed(1) : "—";
    const unit = sv?.unit ?? "";
    ctx.fillStyle = color;
    ctx.strokeStyle = "#fff";
    ctx.lineWidth = 1.5;
    ctx.beginPath();
    ctx.arc(pos.x, pos.y, 7, 0, Math.PI * 2);
    ctx.fill();
    ctx.stroke();
    ctx.fillStyle = "#555";
    ctx.font = "11px Consolas, monospace";
    ctx.fillText(pos.label, pos.x + 11, pos.y - 8);
    ctx.fillStyle = color;
    ctx.font = "bold 12px Consolas, monospace";
    ctx.fillText(`${val} ${unit}`, pos.x + 11, pos.y + 8);
  }

  // ---- 回路标注 + 流向箭头 ----
  ctx.fillStyle = "#777";
  ctx.font = "12px Consolas, monospace";
  ctx.fillText("一次侧（冷水回路）", 40, 30);
  ctx.fillText("二次侧（热水回路）", 700, 30);
  drawArrow(ctx, 300, 112, 330, 112, COL.pipe_cold);
  drawArrow(ctx, 330, 390, 300, 390, COL.pipe_hot);
  drawArrow(ctx, 710, 112, 740, 112, COL.pipe_hot);
  drawArrow(ctx, 740, 390, 710, 390, COL.pipe_cold);
}

export function SystemDiagram({ state }: { state: SimState }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const cv = canvasRef.current;
    if (!cv) return;
    const ctx = cv.getContext("2d");
    if (!ctx) return;

    // 逻辑分辨率固定；显示尺寸按容器宽等比缩放
    const container = cv.parentElement;
    const avail = container?.clientWidth || CV_W;
    const dispW = Math.min(avail, CV_W);
    const dispH = (dispW * CV_H) / CV_W;
    cv.width = CV_W;
    cv.height = CV_H;
    cv.style.width = `${dispW}px`;
    cv.style.height = `${dispH}px`;

    draw(ctx, state);
  }, [state]);

  return (
    <div className="w-full max-w-full overflow-hidden rounded-lg border bg-card shadow-sm">
      <canvas ref={canvasRef} className="block max-w-full" />
    </div>
  );
}
