import {
  FACE_ORDER,
  MIRROR_FACE,
  buildBoxes,
  faceRect,
  type PartId,
  type Rect,
} from "./atlas";

export type Rgba = [number, number, number, number];

export function hexToRgba(hex: string): Rgba {
  const clean = hex.replace("#", "").trim();
  const full =
    clean.length === 3
      ? clean
          .split("")
          .map((char) => char + char)
          .join("")
      : clean;
  if (full.length !== 6 || /[^0-9a-f]/i.test(full)) return [0, 0, 0, 255];
  const value = Number.parseInt(full, 16);
  return [(value >> 16) & 255, (value >> 8) & 255, value & 255, 255];
}

export function rgbaToHex(r: number, g: number, b: number): string {
  return (
    "#" +
    [r, g, b]
      .map((item) => Math.round(Math.min(255, Math.max(0, item))).toString(16).padStart(2, "0"))
      .join("")
  );
}

/** 整数布雷森汉姆直线，含两个端点 */
export function bresenham(x0: number, y0: number, x1: number, y1: number): [number, number][] {
  const points: [number, number][] = [];
  let x = Math.round(x0);
  let y = Math.round(y0);
  const tx = Math.round(x1);
  const ty = Math.round(y1);
  const dx = Math.abs(tx - x);
  const dy = Math.abs(ty - y);
  const sx = x < tx ? 1 : -1;
  const sy = y < ty ? 1 : -1;
  let err = dx - dy;
  for (let guard = 0; guard < 512; guard += 1) {
    points.push([x, y]);
    if (x === tx && y === ty) break;
    const e2 = err * 2;
    if (e2 > -dy) {
      err -= dy;
      x += sx;
    }
    if (e2 < dx) {
      err += dx;
      y += sy;
    }
  }
  return points;
}

/** 四邻域填充，限制在 bounds 内，避免沿着图集空隙漏到别的地方 */
export function floodFill(
  ctx: CanvasRenderingContext2D,
  seedX: number,
  seedY: number,
  fill: Rgba,
  bounds?: Rect
) {
  const [bx, by, bw, bh] = bounds ?? [0, 0, ctx.canvas.width, ctx.canvas.height];
  if (seedX < bx || seedY < by || seedX >= bx + bw || seedY >= by + bh) return;
  const image = ctx.getImageData(bx, by, bw, bh);
  const data = image.data;
  const index = (x: number, y: number) => ((y - by) * bw + (x - bx)) * 4;
  const start = index(seedX, seedY);
  const target: Rgba = [data[start], data[start + 1], data[start + 2], data[start + 3]];
  if (target.every((value, i) => value === fill[i])) return;

  const stack: [number, number][] = [[seedX, seedY]];
  const seen = new Uint8Array(bw * bh);
  while (stack.length) {
    const point = stack.pop();
    if (!point) break;
    const [x, y] = point;
    if (x < bx || y < by || x >= bx + bw || y >= by + bh) continue;
    const mark = (y - by) * bw + (x - bx);
    if (seen[mark]) continue;
    seen[mark] = 1;
    const offset = mark * 4;
    if (
      data[offset] !== target[0] ||
      data[offset + 1] !== target[1] ||
      data[offset + 2] !== target[2] ||
      data[offset + 3] !== target[3]
    ) {
      continue;
    }
    data[offset] = fill[0];
    data[offset + 1] = fill[1];
    data[offset + 2] = fill[2];
    data[offset + 3] = fill[3];
    stack.push([x + 1, y], [x - 1, y], [x, y + 1], [x, y - 1]);
  }
  ctx.putImageData(image, bx, by);
}

/**
 * 把一侧肢体整段镜像到对侧（手臂/腿部）。
 * 展开图里同一块面宽度一致，因此逐面水平翻转后直接覆盖。
 */
export function copyPartToPair(
  canvas: HTMLCanvasElement,
  part: PartId,
  slim: boolean
): boolean {
  const boxes = buildBoxes(slim);
  const sources = boxes.filter((box) => box.part === part && box.pair);
  if (!sources.length) return false;
  const ctx = canvas.getContext("2d");
  if (!ctx) return false;
  ctx.imageSmoothingEnabled = false;
  for (const box of sources) {
    const target = boxes.find((item) => item.id === box.pair);
    if (!target) continue;
    for (const face of FACE_ORDER) {
      const [sx, sy, sw, sh] = faceRect(box, face);
      const [dx, dy] = faceRect(target, MIRROR_FACE[face]);
      const temp = document.createElement("canvas");
      temp.width = sw;
      temp.height = sh;
      const tctx = temp.getContext("2d");
      if (!tctx) continue;
      tctx.imageSmoothingEnabled = false;
      // 底面在展开图里旋转了 180°，镜像时要上下翻转；其余面左右翻转
      if (face === "bottom") {
        tctx.translate(0, sh);
        tctx.scale(1, -1);
      } else {
        tctx.translate(sw, 0);
        tctx.scale(-1, 1);
      }
      tctx.drawImage(canvas, sx, sy, sw, sh, 0, 0, sw, sh);
      ctx.clearRect(dx, dy, sw, sh);
      ctx.drawImage(temp, dx, dy);
    }
  }
  return true;
}

/** 色块模板：每个部位整块填成不同颜色，方便从零开始区分肢体 */
const TEMPLATE_COLORS: Record<PartId, string> = {
  head: "#e8c39a",
  body: "#6f8a4f",
  armLeft: "#4c7f9e",
  armRight: "#b06a4a",
  legLeft: "#7a6aa8",
  legRight: "#a34a4a",
};

/** 用色块覆盖整张皮肤：只有基础层有颜色，外层保持透明 */
export function paintBlockTemplate(canvas: HTMLCanvasElement, slim: boolean): void {
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  ctx.clearRect(0, 0, canvas.width, canvas.height);
  ctx.imageSmoothingEnabled = false;
  for (const box of buildBoxes(slim)) {
    if (box.layer !== "base") continue;
    ctx.fillStyle = TEMPLATE_COLORS[box.part];
    for (const face of FACE_ORDER) {
      const [x, y, w, h] = faceRect(box, face);
      ctx.fillRect(x, y, w, h);
    }
  }
}
