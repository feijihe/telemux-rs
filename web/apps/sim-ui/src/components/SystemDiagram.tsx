import type { SimState, SimSensor } from "@telemux/ui"
import { useEffect, useRef } from "react"

// ===== Canvas 常量 =====
const CV_W = 1040
const CV_H = 640
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
}

interface PipeSize {
  x: number
  y: number
  w: number
  h?: number
}

const PRI_IN_PIPE_SIZE: PipeSize = {
  x: 40,
  y: 112,
  w: 290,
  h: 10,
}

const PRI_OUT_PIPE_SIZE: PipeSize = {
  x: 40,
  y: 390,
  w: 290,
  h: 10,
}

const SEC_IN_PIPE_SIZE: PipeSize = {
  x: 470,
  y: 390,
  w: 530,
}

const SEC_OUT_PIPE_SIZE: PipeSize = {
  x: 470,
  y: 112,
  w: 530,
}

// 泵转速传感器（从 sensors 取值显示在泵体上）
const PUMP_SPEED_SENSORS = ["cdu.sec.in.pump1.speed", "cdu.sec.in.pump2.speed"]

function roundRect(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  w: number,
  h: number,
  r: number,
) {
  ctx.beginPath()
  ctx.moveTo(x + r, y)
  ctx.arcTo(x + w, y, x + w, y + h, r)
  ctx.arcTo(x + w, y + h, x, y + h, r)
  ctx.arcTo(x, y + h, x, y, r)
  ctx.arcTo(x, y, x + w, y, r)
  ctx.closePath()
}

function drawArrow(
  ctx: CanvasRenderingContext2D,
  x1: number,
  y1: number,
  x2: number,
  y2: number,
  color: string,
) {
  ctx.strokeStyle = color
  ctx.lineWidth = 2
  ctx.beginPath()
  ctx.moveTo(x1, y1)
  ctx.lineTo(x2, y2)
  ctx.stroke()
  const ang = Math.atan2(y2 - y1, x2 - x1)
  ctx.beginPath()
  ctx.moveTo(x2, y2)
  ctx.lineTo(x2 - 8 * Math.cos(ang - 0.4), y2 - 8 * Math.sin(ang - 0.4))
  ctx.lineTo(x2 - 8 * Math.cos(ang + 0.4), y2 - 8 * Math.sin(ang + 0.4))
  ctx.closePath()
  ctx.fillStyle = color
  ctx.fill()
}

function drawPump(
  ctx: CanvasRenderingContext2D,
  cx: number,
  cy: number,
  label: string,
  duty: number,
) {
  const w = 70
  const h = 75
  const r = 6
  ctx.fillStyle = "#e7f5ff"
  ctx.strokeStyle = "#4dabf7"
  ctx.lineWidth = 2
  roundRect(ctx, cx - w / 2, cy - h / 2, w, h, r)
  ctx.fill()
  ctx.stroke()
  ctx.strokeStyle = "#4dabf7"
  ctx.lineWidth = 3
  ctx.beginPath()
  ctx.moveTo(cx - w / 2, cy)
  ctx.lineTo(cx - w / 2 - 8, cy)
  ctx.stroke()
  ctx.beginPath()
  ctx.moveTo(cx + w / 2, cy)
  ctx.lineTo(cx + w / 2 + 8, cy)
  ctx.stroke()
  ctx.fillStyle = "#4dabf7"
  ctx.font = "bold 12px Consolas, monospace"
  ctx.textAlign = "center"
  ctx.fillText(label, cx, cy - 6)
  ctx.fillStyle = "#0a7"
  ctx.font = "bold 13px Consolas, monospace"
  ctx.fillText(`${duty.toFixed(0)} RPM`, cx, cy + 14)
  ctx.textAlign = "left"
}

function drawSensor(ctx: CanvasRenderingContext2D, cx: number, cy: number, sensor: SimSensor) {
  const color = (COL as Record<string, string>)[sensor.kind] ?? "#888"
  const { value, unit, name } = sensor
  ctx.fillStyle = color
  ctx.strokeStyle = "#fff"
  ctx.lineWidth = 1.5
  ctx.beginPath()
  ctx.arc(cx, cy, 7, 0, Math.PI * 2)
  ctx.fill()
  ctx.stroke()
  ctx.fillStyle = "#555"
  ctx.font = "11px Consolas, monospace"
  ctx.fillText(name, cx + 11, cy - 8)
  ctx.fillStyle = color
  ctx.font = "bold 12px Consolas, monospace"
  ctx.fillText(`${value?.toFixed(2) ?? "--"} ${unit ?? ""}`, cx + 11, cy + 8)
}

function drawPriPipe(ctx: CanvasRenderingContext2D) {
  function _drawPipe(g: CanvasRenderingContext2D, pipeSize: PipeSize) {
    const { x, y, w } = pipeSize
    g.strokeStyle = COL.pipe_cold
    g.beginPath()
    g.moveTo(x, y)
    g.lineTo(x + w, y) // 冷水进管
    g.stroke()
  }

  _drawPipe(ctx, PRI_IN_PIPE_SIZE)
  _drawPipe(ctx, PRI_OUT_PIPE_SIZE)
}

function _drawSensors(ctx: CanvasRenderingContext2D, sensors: SimSensor[], pipeSize: PipeSize) {
  const priInInterval = pipeSize.w / (sensors.length + 1)
  sensors.forEach((sensor, i) => {
    const ex = pipeSize.x + priInInterval * (i + 1)
    const cy = pipeSize.y - 12
    drawSensor(ctx, ex, cy, sensor)
  })
}

function draw(ctx: CanvasRenderingContext2D, state: SimState) {
  ctx.clearRect(0, 0, CV_W, CV_H)
  ctx.fillStyle = COL.bg
  ctx.fillRect(0, 0, CV_W, CV_H)

  const [priInSensors, priOutSensors, secInSensors, senOutSensors, otherSensors] = Array.from(
    { length: 5 },
    () => [] as SimState["sensors"],
  )
  state.sensors.forEach(sensor => {
    if (sensor.sensor_id.startsWith("cdu.pri.in")) priInSensors.push(sensor)
    else if (sensor.sensor_id.startsWith("cdu.pri.out")) priOutSensors.push(sensor)
    else if (sensor.sensor_id.startsWith("cdu.sec.in")) secInSensors.push(sensor)
    else if (sensor.sensor_id.startsWith("cdu.sec.out")) senOutSensors.push(sensor)
    else otherSensors.push(sensor)
  })

  // ---- 管路 ----
  ctx.lineWidth = 4
  ctx.lineCap = "round"

  drawPriPipe(ctx)
  _drawSensors(ctx, priInSensors, PRI_IN_PIPE_SIZE)
  _drawSensors(ctx, priOutSensors, PRI_OUT_PIPE_SIZE)
  _drawSensors(ctx, secInSensors, SEC_IN_PIPE_SIZE)
  _drawSensors(ctx, senOutSensors, SEC_OUT_PIPE_SIZE)

  // 二次侧：上管（PHE 出水→服务器）、下管（服务器→PHE 进水）
  ctx.strokeStyle = COL.pipe_cold
  ctx.beginPath()
  ctx.moveTo(470, 112)
  ctx.lineTo(1000, 112)
  ctx.stroke()
  ctx.strokeStyle = COL.pipe_hot
  ctx.beginPath()
  ctx.moveTo(1000, 390)
  ctx.lineTo(470, 390)
  ctx.stroke()

  // 泵组并联环路：泵2 在上横管、泵1 在下管主线
  const PLX = 545
  const PRX = 655
  const PTY = 268
  const PBY = 390
  ctx.strokeStyle = COL.pipe
  ctx.lineWidth = 3
  ctx.beginPath()
  ctx.moveTo(PLX, PBY)
  ctx.lineTo(PLX, PTY)
  ctx.stroke()
  ctx.beginPath()
  ctx.moveTo(PRX, PBY)
  ctx.lineTo(PRX, PTY)
  ctx.stroke()
  ctx.beginPath()
  ctx.moveTo(PLX, PTY)
  ctx.lineTo(PRX, PTY)
  ctx.stroke()
  // 补水支路
  ctx.lineWidth = 2.5
  ctx.beginPath()
  ctx.moveTo(705, 112)
  ctx.lineTo(705, 140)
  ctx.stroke()

  // ---- PHEX ----
  ctx.fillStyle = "#dff0d8"
  ctx.strokeStyle = "#5b8c5a"
  ctx.lineWidth = 2
  roundRect(ctx, 340, 90, 130, 310, 8)
  ctx.fill()
  ctx.stroke()
  ctx.fillStyle = "#5b8c5a"
  ctx.font = "bold 14px Consolas, monospace"
  ctx.textAlign = "center"
  ctx.fillText("PHEX", 405, 240)
  ctx.font = "10px Consolas, monospace"
  ctx.fillText("板式换热器", 405, 256)

  // ---- 泵（转速）----
  const byId = new Map(state.sensors.map(s => [s.sensor_id, s]))
  const pump1 = byId.get(PUMP_SPEED_SENSORS[0])?.value ?? 0
  const pump2 = byId.get(PUMP_SPEED_SENSORS[1])?.value ?? 0
  drawPump(ctx, 600, 268, "泵2", pump2)
  drawPump(ctx, 600, 390, "泵1", pump1)

  // ---- 服务器 ----
  ctx.fillStyle = "#eee"
  ctx.strokeStyle = "#999"
  roundRect(ctx, 1000, 60, 26, 360, 4)
  ctx.fill()
  ctx.stroke()
  ctx.save()
  ctx.translate(1013, 240)
  ctx.rotate(Math.PI / 2)
  ctx.fillStyle = "#666"
  ctx.font = "10px Consolas, monospace"
  ctx.fillText("SERVER", 0, 0)
  ctx.restore()

  // ---- H1 比例阀（duty 来自控制变量）----
  const valve = state.controls.find(c => c.name === "valve1_duty")
  const v1 = valve?.value ?? 0
  ctx.fillStyle = "#f59f00"
  ctx.beginPath()
  ctx.moveTo(318, 104)
  ctx.lineTo(336, 112)
  ctx.lineTo(318, 120)
  ctx.closePath()
  ctx.fill()
  ctx.fillStyle = "#e8590c"
  ctx.font = "bold 11px Consolas, monospace"
  ctx.fillText("H1", 340, 118)
  ctx.fillText(`${v1.toFixed(0)}%`, 340, 132)

  // ---- 传感器标点 ----
  ctx.textAlign = "left"

  // ---- 回路标注 + 流向箭头 ----
  ctx.fillStyle = "#777"
  ctx.font = "12px Consolas, monospace"
  ctx.fillText("一次侧（冷水回路）", 40, 30)
  ctx.fillText("二次侧（热水回路）", 700, 30)
  drawArrow(ctx, 300, 112, 330, 112, COL.pipe_cold)
  drawArrow(ctx, 330, 390, 300, 390, COL.pipe_hot)
  drawArrow(ctx, 710, 112, 740, 112, COL.pipe_hot)
  drawArrow(ctx, 740, 390, 710, 390, COL.pipe_cold)
}

export function SystemDiagram({ state }: { state: SimState }) {
  const canvasRef = useRef<HTMLCanvasElement>(null)

  useEffect(() => {
    const cv = canvasRef.current
    if (!cv) return
    const ctx = cv.getContext("2d")
    if (!ctx) return

    // 逻辑分辨率固定；显示尺寸按容器宽等比缩放
    const container = cv.parentElement
    const avail = container?.clientWidth || CV_W
    const dispW = Math.min(avail, CV_W)
    const dispH = (dispW * CV_H) / CV_W
    cv.width = CV_W
    cv.height = CV_H
    cv.style.width = `${dispW}px`
    cv.style.height = `${dispH}px`

    draw(ctx, state)
  }, [state])

  return (
    <div className="bg-card w-full max-w-full overflow-hidden rounded-lg border shadow-sm">
      <canvas ref={canvasRef} className="block max-w-full" />
    </div>
  )
}
