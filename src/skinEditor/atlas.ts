/** 皮肤 UV 图集：12 个盒子（头/身/左右腿/左右臂 × 基础层/外层）在 64×64 贴图上的位置 */

export type FaceName = "top" | "bottom" | "right" | "left" | "front" | "back";
export type LayerId = "base" | "overlay";
export type PartId = "head" | "body" | "armLeft" | "armRight" | "legLeft" | "legRight";
export type PartFilter = "all" | PartId;
export type Rect = [number, number, number, number];

export interface BoxSpec {
  id: string;
  part: PartId;
  layer: LayerId;
  /** 展开图左上角 */
  tx: number;
  ty: number;
  /** 盒子宽、高、厚（像素） */
  w: number;
  h: number;
  d: number;
  /** 左右对称的另一侧盒子 id */
  pair?: string;
}

export const FACE_ORDER: FaceName[] = ["top", "bottom", "right", "left", "front", "back"];

/** BoxGeometry 的材质组顺序：+X / -X / +Y / -Y / +Z / -Z */
export const FACE_BY_GROUP: FaceName[] = ["left", "right", "top", "bottom", "front", "back"];

/** 镜像时左右侧面互换，其余面原地翻转 */
export const MIRROR_FACE: Record<FaceName, FaceName> = {
  top: "top",
  bottom: "bottom",
  front: "front",
  back: "back",
  left: "right",
  right: "left",
};

/** 展开图里一个盒子由六块拼成：顶部一行 [d][w][d][w] 对应 right / front / left / back */
export function faceRect(box: BoxSpec, face: FaceName): Rect {
  const { tx, ty, w, h, d } = box;
  switch (face) {
    case "top":
      return [tx + d, ty, w, d];
    case "bottom":
      return [tx + d + w, ty, w, d];
    case "right":
      return [tx, ty + d, d, h];
    case "front":
      return [tx + d, ty + d, w, h];
    case "left":
      return [tx + d + w, ty + d, d, h];
    case "back":
      return [tx + d + w + d, ty + d, w, h];
  }
}

/** 底面在展开图里整体旋转了 180°，贴图时要同时翻转两轴 */
export function faceFlip(face: FaceName): { flipX: boolean; flipY: boolean } {
  return face === "bottom" ? { flipX: true, flipY: true } : { flipX: false, flipY: false };
}

/** 纤薄模型的手臂只有 3 像素宽，其余部位与经典模型一致 */
export function buildBoxes(slim: boolean): BoxSpec[] {
  const aw = slim ? 3 : 4;
  return [
    { id: "head", part: "head", layer: "base", tx: 0, ty: 0, w: 8, h: 8, d: 8 },
    { id: "headOverlay", part: "head", layer: "overlay", tx: 32, ty: 0, w: 8, h: 8, d: 8 },
    { id: "body", part: "body", layer: "base", tx: 16, ty: 16, w: 8, h: 12, d: 4 },
    { id: "bodyOverlay", part: "body", layer: "overlay", tx: 16, ty: 32, w: 8, h: 12, d: 4 },
    { id: "legLeft", part: "legLeft", layer: "base", tx: 16, ty: 48, w: 4, h: 12, d: 4, pair: "legRight" },
    { id: "legRight", part: "legRight", layer: "base", tx: 0, ty: 16, w: 4, h: 12, d: 4, pair: "legLeft" },
    { id: "legLeftOverlay", part: "legLeft", layer: "overlay", tx: 0, ty: 48, w: 4, h: 12, d: 4, pair: "legRightOverlay" },
    { id: "legRightOverlay", part: "legRight", layer: "overlay", tx: 0, ty: 32, w: 4, h: 12, d: 4, pair: "legLeftOverlay" },
    { id: "armLeft", part: "armLeft", layer: "base", tx: 32, ty: 48, w: aw, h: 12, d: 4, pair: "armRight" },
    { id: "armRight", part: "armRight", layer: "base", tx: 40, ty: 16, w: aw, h: 12, d: 4, pair: "armLeft" },
    { id: "armLeftOverlay", part: "armLeft", layer: "overlay", tx: 48, ty: 48, w: aw, h: 12, d: 4, pair: "armRightOverlay" },
    { id: "armRightOverlay", part: "armRight", layer: "overlay", tx: 40, ty: 32, w: aw, h: 12, d: 4, pair: "armLeftOverlay" },
  ];
}

/**
 * 射线命中的 UV 换算成贴图像素。
 * 贴图上传时 flipY=true，所以 v=1 对应区域第一行；底面在绘制时已翻转，这里要还原。
 */
export function uvToTexel(
  box: BoxSpec,
  face: FaceName,
  u: number,
  v: number
): { x: number; y: number } {
  const [rx, ry, rw, rh] = faceRect(box, face);
  const { flipX, flipY } = faceFlip(face);
  const dx = Math.min(rw - 1, Math.max(0, Math.floor(u * rw)));
  const dy = Math.min(rh - 1, Math.max(0, Math.floor((1 - v) * rh)));
  return {
    x: rx + (flipX ? rw - 1 - dx : dx),
    y: ry + (flipY ? rh - 1 - dy : dy),
  };
}

export interface TexelHit {
  box: BoxSpec;
  face: FaceName;
  x: number;
  y: number;
}

/** 求左右镜像后的贴图坐标：面内水平翻转，左右侧面互换，四肢跨到对侧盒子 */
export function mirrorTexel(
  boxes: BoxSpec[],
  box: BoxSpec,
  face: FaceName,
  x: number,
  y: number
): TexelHit | null {
  const [rx, ry, rw, rh] = faceRect(box, face);
  const u = x - rx;
  const v = y - ry;
  if (u < 0 || v < 0 || u >= rw || v >= rh) return null;
  const target = box.pair ? boxes.find((item) => item.id === box.pair) ?? box : box;
  const targetFace = MIRROR_FACE[face];
  const [tx, ty, tw, th] = faceRect(target, targetFace);
  // 底面在展开图里旋转了 180°，镜像到贴图上就是上下翻转；其余面都是左右翻转
  if (face === "bottom") {
    return { box: target, face: targetFace, x: tx + u, y: ty + (th - 1 - v) };
  }
  return { box: target, face: targetFace, x: tx + (tw - 1 - u), y: ty + v };
}

/** 贴图坐标属于哪个盒子的哪个面，用于 2D 模式判定与镜像按钮 */
export function locateTexel(
  boxes: BoxSpec[],
  x: number,
  y: number
): { box: BoxSpec; face: FaceName } | null {
  for (const box of boxes) {
    for (const face of FACE_ORDER) {
      const [rx, ry, rw, rh] = faceRect(box, face);
      if (x >= rx && x < rx + rw && y >= ry && y < ry + rh) return { box, face };
    }
  }
  return null;
}
