<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref, shallowRef, watch } from "vue";
import { api, openExternal, pickCapePng, saveCapeDialog, saveSkinDialog } from "../api";
import diamondImg from "../assets/icons/diamond.png";
import AppIcon from "../components/AppIcon.vue";
import IconButton from "../components/IconButton.vue";
import SkinModel from "../components/SkinModel.vue";
import SkinPaintViewport from "../components/SkinPaintViewport.vue";
import WoodButton from "../components/WoodButton.vue";
import WoodInput from "../components/WoodInput.vue";
import WoodModal from "../components/WoodModal.vue";
import type { AccountView, DefaultSkin, LocalCape, SkinLibraryItem } from "../api";
import {
  canvasFromSource,
  canvasToDataUrl,
  createDefaultSkin,
  seedFromString,
  skinCanvasFromSource,
  SKIN_SIZE,
} from "../skin";
import {
  CAPE_FACES,
  CAPE_HEIGHT,
  CAPE_WIDTH,
  FACE_ORDER,
  buildBoxes,
  faceRect,
  locateTexel,
  type PartFilter,
  type Rect,
} from "../skinEditor/atlas";
import {
  copyPartToPair,
  floodFill,
  hexToRgba,
  paintCapeTemplate,
  paintSkinTemplate,
  type TemplateKind,
} from "../skinEditor/paint";
import { loadSelectedSkin, notify, refreshAccounts, store } from "../store";

const props = defineProps<{ account: AccountView }>();
const emit = defineEmits<{ (e: "close"): void }>();

const drawCanvas = ref<HTMLCanvasElement | null>(null);
const guideCanvas = ref<HTMLCanvasElement | null>(null);

const tool = ref<"brush" | "eraser" | "fill" | "picker">("brush");
const color = ref("#5e7b49");
const brushSize = ref(1);
const model = ref<"classic" | "slim">("classic");
const part = ref<PartFilter>("all");
const mode = ref<"3d" | "2d">("3d");
const layer = ref<"base" | "overlay">("base");
/// 两个图层的显隐：隐藏后 2D/3D 都不显示，并且不可绘制
const layerVisible = reactive({ base: true, overlay: true });
/// 油漆桶“整块填充”：直接铺满命中的整个面，不做连通判断
const fillWhole = ref(false);
const symmetry = ref(false);
const docVersion = ref(0);
const showGuides = ref(true);

/// 同一编辑器里编辑皮肤（64×64）或披风（64×32）
type DocMode = "skin" | "cape";
interface DocState {
  canvas: HTMLCanvasElement | null;
  history: ImageData[];
  cursor: number;
}
const docMode = ref<DocMode>("skin");
const docSource = shallowRef<HTMLCanvasElement | null>(null);
const docs: Record<DocMode, DocState> = {
  skin: { canvas: null, history: [], cursor: -1 },
  cape: { canvas: null, history: [], cursor: -1 },
};
const capeLoading = ref(false);
/// 从披风切回皮肤时恢复原来的 3D/2D 视图
let previousMode: "3d" | "2d" = "3d";
/// 2D 缩放：fit 表示按窗口自适应，其余为每贴图像素放大的倍率
const zoom = ref<"fit" | 1 | 2 | 4>("fit");
/// 自适应时算出的每像素尺寸（CSS 像素）
const fitScale = ref(8);
const canvasScroll = ref<HTMLElement | null>(null);
let canvasObserver: ResizeObserver | null = null;
/// 透视：其它部位/另一图层半透明；关闭时只显示正在编辑的部分
const ghost = ref(true);
/// 全显：所有部位与图层都不透明，用于完整预览
const showAll = ref(false);
/// 行走动画开关
const walk = ref(false);
const saving = ref(false);
const uploadOpen = ref(false);
const uploading = ref(false);
const canUndo = ref(false);
const canRedo = ref(false);
const presets = ref<DefaultSkin[]>([]);
const library = ref<SkinLibraryItem[]>([]);
const libraryOpen = ref(false);
const libraryName = ref("");
const librarySaving = ref(false);
/// 披风库：本地保存的披风，可载入/导出
const capes = ref<LocalCape[]>([]);
const capeName = ref("");
const capeSaving = ref(false);
const capeBusy = ref("");

const palette = [
  "#2b2118", "#4a3220", "#7a5230", "#c08a55",
  "#e3b48a", "#f0c9a2", "#9c6a52", "#a8543c",
  "#5e7b49", "#88a867", "#47623a", "#3f4a58",
  "#243a5e", "#c9a26a", "#f4ebdd", "#ffffff",
];

/* ---- 取色器：色相 + 饱和度/明度面板 ---- */

const svArea = ref<HTMLElement | null>(null);
const hueBar = ref<HTMLElement | null>(null);
const hue = ref(100);
/// 0-100，便于直接当百分比用
const sat = ref(55);
const val = ref(48);
let svDragging = false;
let hueDragging = false;

function clampNumber(value: number, min: number, max: number) {
  return Math.min(max, Math.max(min, value));
}

function hexToRgb(hex: string) {
  const clean = hex.replace("#", "").trim();
  const full = clean.length === 3 ? clean.split("").map((c) => c + c).join("") : clean;
  if (full.length !== 6 || /[^0-9a-f]/i.test(full)) return null;
  const value = Number.parseInt(full, 16);
  return {
    r: (value >> 16) & 255,
    g: (value >> 8) & 255,
    b: value & 255,
  };
}

function rgbToHex(r: number, g: number, b: number) {
  return (
    "#" +
    [r, g, b]
      .map((item) => Math.round(clampNumber(item, 0, 255)).toString(16).padStart(2, "0"))
      .join("")
  );
}

function rgbToHsv(r: number, g: number, b: number) {
  const rn = r / 255;
  const gn = g / 255;
  const bn = b / 255;
  const max = Math.max(rn, gn, bn);
  const min = Math.min(rn, gn, bn);
  const delta = max - min;
  let h = 0;
  if (delta > 0) {
    if (max === rn) h = ((gn - bn) / delta) % 6;
    else if (max === gn) h = (bn - rn) / delta + 2;
    else h = (rn - gn) / delta + 4;
    h *= 60;
    if (h < 0) h += 360;
  }
  return {
    h,
    s: max === 0 ? 0 : (delta / max) * 100,
    v: max * 100,
  };
}

function hsvToHex(h: number, s: number, v: number) {
  const sn = s / 100;
  const vn = v / 100;
  const c = vn * sn;
  const x = c * (1 - Math.abs(((h / 60) % 2) - 1));
  const m = vn - c;
  let r = 0;
  let g = 0;
  let b = 0;
  if (h < 60) [r, g, b] = [c, x, 0];
  else if (h < 120) [r, g, b] = [x, c, 0];
  else if (h < 180) [r, g, b] = [0, c, x];
  else if (h < 240) [r, g, b] = [0, x, c];
  else if (h < 300) [r, g, b] = [x, 0, c];
  else [r, g, b] = [c, 0, x];
  return rgbToHex((r + m) * 255, (g + m) * 255, (b + m) * 255);
}

function applyHsv() {
  color.value = hsvToHex(hue.value, sat.value, val.value);
}

/// 从吸管、色板、十六进制输入回填面板；自己产生的一致性变化不覆盖色相
watch(
  color,
  (next) => {
    if (hsvToHex(hue.value, sat.value, val.value).toLowerCase() === next.toLowerCase()) return;
    const rgb = hexToRgb(next);
    if (!rgb) return;
    const hsv = rgbToHsv(rgb.r, rgb.g, rgb.b);
    hue.value = hsv.h;
    sat.value = hsv.s;
    val.value = hsv.v;
  },
  { immediate: true }
);

function updateSv(event: PointerEvent) {
  const area = svArea.value;
  if (!area) return;
  const rect = area.getBoundingClientRect();
  sat.value = clampNumber(((event.clientX - rect.left) / rect.width) * 100, 0, 100);
  val.value = clampNumber((1 - (event.clientY - rect.top) / rect.height) * 100, 0, 100);
  applyHsv();
}

function onSvDown(event: PointerEvent) {
  svDragging = true;
  (event.currentTarget as HTMLElement).setPointerCapture?.(event.pointerId);
  updateSv(event);
}

function onSvMove(event: PointerEvent) {
  if (svDragging) updateSv(event);
}

function onSvUp() {
  svDragging = false;
}

function updateHue(event: PointerEvent) {
  const bar = hueBar.value;
  if (!bar) return;
  const rect = bar.getBoundingClientRect();
  hue.value = clampNumber(((event.clientX - rect.left) / rect.width) * 360, 0, 359.9);
  applyHsv();
}

function onHueDown(event: PointerEvent) {
  hueDragging = true;
  (event.currentTarget as HTMLElement).setPointerCapture?.(event.pointerId);
  updateHue(event);
}

function onHueMove(event: PointerEvent) {
  if (hueDragging) updateHue(event);
}

function onHueUp() {
  hueDragging = false;
}

/// 参考框包含两层：上半部分为贴图层，下半部分为第二层（帽子/外套/袖子/裤子）
const guideRects = computed<Record<PartFilter, [number, number, number, number][]>>(() => ({
  all: [
    [0, 0, 32, 16], // 头部基础层 + 帽子层
    [32, 0, 32, 16],
    [16, 16, 24, 16], // 身体基础层 + 外套层
    [16, 32, 32, 16],
    [40, 16, 16, 16], // 右臂 + 右袖
    [40, 32, 16, 16],
    [32, 48, 16, 16], // 左臂 + 左袖
    [48, 48, 16, 16],
    [0, 16, 16, 16], // 右腿 + 右裤
    [0, 32, 16, 16],
    [16, 48, 16, 16], // 左腿 + 左裤
    [0, 48, 16, 16],
  ],
  head: [
    [0, 0, 32, 16],
    [32, 0, 32, 16],
  ],
  body: [
    [16, 16, 24, 16],
    [16, 32, 32, 16],
  ],
  armRight: [
    [40, 16, 16, 16],
    [40, 32, 16, 16],
  ],
  armLeft: [
    [32, 48, 16, 16],
    [48, 48, 16, 16],
  ],
  legRight: [
    [0, 16, 16, 16],
    [0, 32, 16, 16],
  ],
  legLeft: [
    [16, 48, 16, 16],
    [0, 48, 16, 16],
  ],
}));

const limbParts: PartFilter[] = ["armLeft", "armRight", "legLeft", "legRight"];
const canMirror = computed(() => limbParts.includes(part.value));

const docWidth = computed(() => (docMode.value === "cape" ? CAPE_WIDTH : SKIN_SIZE));
const docHeight = computed(() => (docMode.value === "cape" ? CAPE_HEIGHT : SKIN_SIZE));
/// 每个贴图像素占几个屏幕像素
const canvasScale = computed(() => (zoom.value === "fit" ? fitScale.value : zoom.value * 8));
/// 2D 画布显示尺寸：披风是 64×32，高度按比例算
const canvasWidth = computed(() => docWidth.value * canvasScale.value);
const canvasHeight = computed(() => docHeight.value * canvasScale.value);
const isMicrosoft = computed(() => props.account.kind === "microsoft");
/// 3D 披风预览：人物用账号当前皮肤，披风跟随画布内容
const capePreviewUrl = computed(() => {
  void docVersion.value;
  const canvas = docs.cape.canvas;
  return canvas ? canvasToDataUrl(canvas) : "";
});
const previewSkinUrl = computed(() => {
  void docVersion.value;
  return docs.skin.canvas ? canvasToDataUrl(docs.skin.canvas) : "";
});

function fitCanvas() {
  const box = canvasScroll.value;
  if (!box) return;
  // 减去容器的内边距，再按贴图尺寸整除，得到每个贴图像素占几个屏幕像素
  const width = box.clientWidth - 28;
  const height = box.clientHeight - 28;
  if (width <= 0 || height <= 0) return;
  fitScale.value = Math.max(
    1,
    Math.min(
      16,
      Math.floor(Math.min(width / docWidth.value, height / docHeight.value))
    )
  );
}

let ctx: CanvasRenderingContext2D | null = null;
let drawing = false;

function activeState(): DocState {
  return docs[docMode.value];
}

function syncHistoryFlags() {
  const state = activeState();
  canUndo.value = state.cursor > 0;
  canRedo.value = state.cursor < state.history.length - 1;
}

function pushHistory() {
  if (!ctx) return;
  const state = activeState();
  state.history = state.history.slice(0, state.cursor + 1);
  state.history.push(ctx.getImageData(0, 0, ctx.canvas.width, ctx.canvas.height));
  if (state.history.length > 40) state.history.shift();
  state.cursor = state.history.length - 1;
  syncHistoryFlags();
}

function undo() {
  const state = activeState();
  if (!ctx || state.cursor <= 0) return;
  state.cursor -= 1;
  ctx.putImageData(state.history[state.cursor], 0, 0);
  syncHistoryFlags();
  refreshDisplay();
  bumpDoc();
}

function redo() {
  const state = activeState();
  if (!ctx || state.cursor >= state.history.length - 1) return;
  state.cursor += 1;
  ctx.putImageData(state.history[state.cursor], 0, 0);
  syncHistoryFlags();
  refreshDisplay();
  bumpDoc();
}

/// 2D 画布内容被外部改动（撤销、套用预设等）时，通知 3D 视口重新取图
function bumpDoc() {
  docVersion.value += 1;
}

function onViewportCommit() {
  pushHistory();
  refreshDisplay();
}

function onViewportPick(value: string) {
  color.value = value;
}

function mirrorToPair() {
  const canvas = docs.skin.canvas;
  if (!canvas || !canMirror.value) return;
  if (!copyPartToPair(canvas, part.value as Exclude<PartFilter, "all" | "head" | "body">, model.value === "slim")) {
    return;
  }
  pushHistory();
  refreshDisplay();
  bumpDoc();
  notify("已把这一侧镜像到对侧");
}

const templates: { key: TemplateKind; name: string; hint: string }[] = [
  { key: "block", name: "色块模板", hint: "各部位不同颜色" },
  { key: "blank", name: "空白", hint: "清空所有像素" },
  { key: "solid", name: "纯色", hint: "用当前颜色铺满" },
  { key: "stripes", name: "条纹", hint: "双色竖条纹" },
  { key: "checker", name: "棋盘格", hint: "2×2 双色格子" },
  { key: "gradient", name: "渐变", hint: "当前色到浅色" },
];

const visibleTemplates = computed(() =>
  docMode.value === "cape" ? templates.filter((item) => item.key !== "block") : templates
);

/// 模板按钮上的小预览，直接用 CSS 画出来，不必为每张模板建画布
function templateFaceStyle(kind: TemplateKind) {
  const current = color.value;
  const light = "rgba(255, 255, 255, 0.55)";
  if (kind === "solid") return { background: current };
  if (kind === "stripes") {
    return { background: `repeating-linear-gradient(90deg, ${current} 0 4px, ${light} 4px 8px)` };
  }
  if (kind === "checker") {
    return {
      background: `conic-gradient(${current} 0 25%, ${light} 0 50%, ${current} 0 75%, ${light} 0) 0 0 / 12px 12px`,
    };
  }
  if (kind === "gradient") return { background: `linear-gradient(180deg, ${current}, ${light})` };
  if (kind === "blank") {
    return {
      background:
        "repeating-conic-gradient(rgba(0, 0, 0, 0.07) 0 25%, transparent 0 50%) 0 0 / 12px 12px",
    };
  }
  return {};
}

function applyTemplate(kind: TemplateKind) {
  const doc = activeState().canvas;
  if (!doc) return;
  if (docMode.value === "cape") {
    paintCapeTemplate(doc, kind, color.value);
  } else {
    paintSkinTemplate(doc, model.value === "slim", kind, color.value);
  }
  pushHistory();
  refreshDisplay();
  bumpDoc();
  notify(`已套用${templates.find((item) => item.key === kind)?.name ?? "模板"}`);
}

function drawGuides() {
  const overlay = guideCanvas.value;
  if (!overlay) return;
  const width = docWidth.value;
  const height = docHeight.value;
  if (overlay.width !== width) overlay.width = width;
  if (overlay.height !== height) overlay.height = height;
  const g = overlay.getContext("2d");
  if (!g) return;
  g.clearRect(0, 0, width, height);
  if (!showGuides.value) return;
  const rects = docMode.value === "cape" ? CAPE_FACES : guideRects.value[part.value];
  g.save();
  g.setLineDash([2, 2]);
  g.lineWidth = 0.6;
  g.strokeStyle = "rgba(255, 200, 90, 0.9)";
  for (const [x, y, w, h] of rects) {
    g.strokeRect(x + 0.3, y + 0.3, w - 0.6, h - 0.6);
  }
  g.restore();
}

/**
 * 把文档画到 2D 显示画布上。
 * 文档本身保留完整内容，隐藏图层只在显示时抹掉，重新点亮眼睛就能恢复。
 */
function refreshDisplay() {
  const display = drawCanvas.value;
  const doc = activeState().canvas;
  if (!display || !doc) return;
  if (display.width !== doc.width) display.width = doc.width;
  if (display.height !== doc.height) display.height = doc.height;
  const target = display.getContext("2d");
  if (!target) return;
  target.imageSmoothingEnabled = false;
  target.clearRect(0, 0, display.width, display.height);
  target.drawImage(doc, 0, 0);
  if (docMode.value === "skin") {
    const hidden: ("base" | "overlay")[] = [];
    if (!layerVisible.base) hidden.push("base");
    if (!layerVisible.overlay) hidden.push("overlay");
    for (const hiddenLayer of hidden) {
      for (const box of buildBoxes(model.value === "slim")) {
        if (box.layer !== hiddenLayer) continue;
        for (const face of FACE_ORDER) {
          const [rx, ry, rw, rh] = faceRect(box, face);
          target.clearRect(rx, ry, rw, rh);
        }
      }
    }
  }
  drawGuides();
}

function pointerPos(event: PointerEvent) {
  const canvas = drawCanvas.value;
  if (!canvas) return null;
  const rect = canvas.getBoundingClientRect();
  const x = Math.floor(((event.clientX - rect.left) / rect.width) * docWidth.value);
  const y = Math.floor(((event.clientY - rect.top) / rect.height) * docHeight.value);
  if (x < 0 || y < 0 || x >= docWidth.value || y >= docHeight.value) return null;
  return { x, y };
}

/** 点到的贴图属于哪个盒子的哪个面（皮肤）；披风只认正面/背面 */
function boundsAt(x: number, y: number): Rect | null {
  if (docMode.value === "cape") {
    for (const rect of CAPE_FACES) {
      const [rx, ry, rw, rh] = rect;
      if (x >= rx && x < rx + rw && y >= ry && y < ry + rh) return rect;
    }
    return null;
  }
  const hit = locateTexel(buildBoxes(model.value === "slim"), x, y);
  return hit ? faceRect(hit.box, hit.face) : null;
}

/** 2D 画布上这一格属于被隐藏的图层时不可绘制 */
function hiddenLayerAt(x: number, y: number): boolean {
  if (docMode.value !== "skin") return false;
  const hit = locateTexel(buildBoxes(model.value === "slim"), x, y);
  return !!hit && !layerVisible[hit.box.layer];
}

/** 披风双面同步：同一像素在另一面上的镜像坐标 */
function capeMirrorPoint(x: number, y: number): [number, number] | null {
  if (docMode.value !== "cape" || !symmetry.value) return null;
  for (let index = 0; index < CAPE_FACES.length; index += 1) {
    const [rx, ry, rw, rh] = CAPE_FACES[index];
    if (x < rx || x >= rx + rw || y < ry || y >= ry + rh) continue;
    const other = CAPE_FACES[1 - index];
    return [other[0] + (rw - 1 - (x - rx)), other[1] + (y - ry)];
  }
  return null;
}

function paintAt(x: number, y: number) {
  if (!ctx) return;
  if (docMode.value === "skin" && !layerVisible[layer.value]) return;
  const bounds = boundsAt(x, y);
  if (!bounds) return;
  if (hiddenLayerAt(x, y)) return;
  if (tool.value === "picker") {
    const pixel = ctx.getImageData(x, y, 1, 1).data;
    const hex = `#${[pixel[0], pixel[1], pixel[2]]
      .map((v) => v.toString(16).padStart(2, "0"))
      .join("")}`;
    color.value = hex;
    tool.value = "brush";
    return;
  }
  const mirror = capeMirrorPoint(x, y);
  const mirrorBounds = mirror ? boundsAt(mirror[0], mirror[1]) : null;
  if (tool.value === "fill") {
    if (fillWhole.value) {
      ctx.fillStyle = color.value;
      ctx.fillRect(bounds[0], bounds[1], bounds[2], bounds[3]);
      if (mirrorBounds) {
        ctx.fillRect(
          mirrorBounds[0],
          mirrorBounds[1],
          mirrorBounds[2],
          mirrorBounds[3]
        );
      }
    } else {
      // 填充只在本面范围内扩散，不会沿着图集空隙漏到别的部位
      floodFill(ctx, x, y, hexToRgba(color.value), bounds);
      if (mirror && mirrorBounds) {
        floodFill(ctx, mirror[0], mirror[1], hexToRgba(color.value), mirrorBounds);
      }
    }
    refreshDisplay();
    return;
  }
  // 笔刷同样裁到命中面的范围内，避免大笔刷溢出到相邻面
  const [rx, ry, rw, rh] = bounds;
  const size = brushSize.value;
  const offset = Math.floor(size / 2);
  const x0 = Math.max(rx, x - offset);
  const y0 = Math.max(ry, y - offset);
  const x1 = Math.min(rx + rw, x - offset + size);
  const y1 = Math.min(ry + rh, y - offset + size);
  if (x1 <= x0 || y1 <= y0) return;
  if (tool.value === "eraser") {
    ctx.clearRect(x0, y0, x1 - x0, y1 - y0);
  } else {
    ctx.fillStyle = color.value;
    ctx.fillRect(x0, y0, x1 - x0, y1 - y0);
  }
  if (mirror && mirrorBounds) {
    const [mx, my] = mirror;
    const [mrx, mry, mrw, mrh] = mirrorBounds;
    const mx0 = Math.max(mrx, mx - offset);
    const my0 = Math.max(mry, my - offset);
    const mx1 = Math.min(mrx + mrw, mx - offset + size);
    const my1 = Math.min(mry + mrh, my - offset + size);
    if (mx1 > mx0 && my1 > my0) {
      if (tool.value === "eraser") {
        ctx.clearRect(mx0, my0, mx1 - mx0, my1 - my0);
      } else {
        ctx.fillRect(mx0, my0, mx1 - mx0, my1 - my0);
      }
    }
  }
  refreshDisplay();
}

function onPointerDown(event: PointerEvent) {
  const pos = pointerPos(event);
  if (!pos) return;
  if (docMode.value === "skin" && !layerVisible[layer.value]) {
    notify("当前图层已隐藏，点亮眼睛后才能绘制", "err");
    return;
  }
  if (tool.value === "fill" || tool.value === "picker") {
    const bounds = boundsAt(pos.x, pos.y);
    if (!bounds) {
      notify("这里是图集空隙，换一块皮肤区域再操作", "err");
      return;
    }
    paintAt(pos.x, pos.y);
    pushHistory();
    bumpDoc();
    return;
  }
  drawing = true;
  (event.currentTarget as HTMLElement).setPointerCapture?.(event.pointerId);
  paintAt(pos.x, pos.y);
}

function onPointerMove(event: PointerEvent) {
  if (!drawing) return;
  const pos = pointerPos(event);
  if (!pos) return;
  paintAt(pos.x, pos.y);
}

function onPointerUp() {
  if (!drawing) return;
  drawing = false;
  pushHistory();
  bumpDoc();
}

/// 把当前文档接到可见画布与 3D 视口上
function bindActive() {
  const state = activeState();
  docSource.value = state.canvas;
  ctx = state.canvas?.getContext("2d") ?? null;
  if (ctx) ctx.imageSmoothingEnabled = false;
  syncHistoryFlags();
  refreshDisplay();
  requestAnimationFrame(fitCanvas);
}

async function loadSource() {
  const source = store.accountSkin?.data_url;
  let base: HTMLCanvasElement;
  if (source) {
    try {
      // 旧版 64x32 皮肤会补全左臂/左腿，保证预览、编辑与保存都是完整的 64x64 布局
      base = await skinCanvasFromSource(source);
    } catch {
      base = createDefaultSkin(seedFromString(props.account.uuid));
    }
  } else {
    base = createDefaultSkin(seedFromString(props.account.uuid));
  }
  docs.skin.canvas = base;
  docs.skin.history = [];
  docs.skin.cursor = -1;
  const accountModel = store.accountSkin?.model ?? props.account.skin_model;
  model.value = accountModel === "slim" ? "slim" : "classic";
  if (docMode.value === "skin") {
    bindActive();
    pushHistory();
  }
  docVersion.value += 1;
}

/** 读取当前账号的披风；跨域读不到就留一张 64×32 空白披风 */
async function loadCape() {
  const canvas = document.createElement("canvas");
  canvas.width = CAPE_WIDTH;
  canvas.height = CAPE_HEIGHT;
  const target = canvas.getContext("2d");
  if (target) target.imageSmoothingEnabled = false;
  if (target) {
    try {
      const list = await api.accountCapes(props.account.id);
      const cape = list.find((item) => item.active) ?? list[0];
      if (cape?.url) {
        const image = await new Promise<HTMLImageElement>((resolve, reject) => {
          const element = new Image();
          element.crossOrigin = "anonymous";
          element.onload = () => resolve(element);
          element.onerror = () => reject(new Error("披风图加载失败"));
          element.src = cape.url;
        });
        const height = Math.min(
          CAPE_HEIGHT,
          Math.round((image.height / image.width) * CAPE_WIDTH)
        );
        target.drawImage(image, 0, 0, CAPE_WIDTH, height);
        // 跨域且没有 CORS 头时，画完以后读取像素会抛错，此时回退到空白
        target.getImageData(0, 0, 1, 1);
      }
    } catch {
      target.clearRect(0, 0, CAPE_WIDTH, CAPE_HEIGHT);
    }
  }
  docs.cape.canvas = canvas;
  docs.cape.history = [];
  docs.cape.cursor = -1;
}

/* ---------- 披风：本地库 / 导入导出 / 上传 ---------- */

function capePngBase64() {
  const canvas = docs.cape.canvas;
  if (!canvas) return "";
  return canvasToDataUrl(canvas).split(",")[1] ?? "";
}

async function applyCapeSource(url: string) {
  const source = await canvasFromSource(url);
  const target = document.createElement("canvas");
  target.width = CAPE_WIDTH;
  target.height = CAPE_HEIGHT;
  const targetCtx = target.getContext("2d");
  if (!targetCtx) throw new Error("无法创建披风画布");
  targetCtx.imageSmoothingEnabled = false;
  targetCtx.drawImage(source, 0, 0, CAPE_WIDTH, CAPE_HEIGHT);
  docs.cape.canvas = target;
  docs.cape.history = [];
  docs.cape.cursor = -1;
  if (docMode.value === "cape") {
    bindActive();
    pushHistory();
  }
  docVersion.value += 1;
}

async function refreshCapes() {
  try {
    capes.value = await api.capeLibraryList();
  } catch {
    capes.value = [];
  }
}

async function saveCapeToLibrary() {
  if (!docs.cape.canvas || capeSaving.value) return;
  const name = capeName.value.trim() || `披风 ${new Date().toLocaleDateString("zh-CN")}`;
  capeSaving.value = true;
  try {
    const saved = await api.capeLibrarySave(name, capePngBase64());
    capeName.value = "";
    await refreshCapes();
    notify(`已保存披风「${saved.name}」`);
  } catch (err) {
    notify(`保存披风失败：${String(err)}`, "err");
  } finally {
    capeSaving.value = false;
  }
}

async function applyCapeFromLibrary(cape: LocalCape) {
  if (capeBusy.value) return;
  capeBusy.value = cape.name;
  try {
    await applyCapeSource(cape.data_url);
    notify(`已载入披风「${cape.name}」`);
  } catch (err) {
    notify(`载入披风失败：${String(err)}`, "err");
  } finally {
    capeBusy.value = "";
  }
}

async function removeCapeFromLibrary(cape: LocalCape) {
  try {
    await api.capeLibraryRemove(cape.name);
    await refreshCapes();
    notify("披风已从本地库删除");
  } catch (err) {
    notify(`删除披风失败：${String(err)}`, "err");
  }
}

async function importCapeFile() {
  if (capeBusy.value) return;
  const path = await pickCapePng();
  if (!path) return;
  capeBusy.value = "import";
  try {
    await applyCapeSource(await api.capeImport(path));
    notify("披风已导入");
  } catch (err) {
    notify(`导入披风失败：${String(err)}`, "err");
  } finally {
    capeBusy.value = "";
  }
}

async function exportCapeFile() {
  if (!docs.cape.canvas) return;
  const path = await saveCapeDialog(`${props.account.name}-cape.png`);
  if (!path) return;
  try {
    await api.capeExport(path, capePngBase64());
    notify("披风已导出为 PNG");
  } catch (err) {
    notify(`导出披风失败：${String(err)}`, "err");
  }
}

/// 第三方账号的披风只能在皮肤站网页上传，这里按 API 根推出站点地址并打开角色页
function openCapeUploadPage() {
  if (props.account.kind !== "authlib") {
    notify("微软账号不支持自定义披风，只能在账号页选择已拥有的披风", "err");
    return;
  }
  const trimmed = (props.account.authlib_url ?? "").trim().replace(/\/+$/, "");
  if (!trimmed) {
    notify("这个账号没有记录皮肤站地址，请重新登录该账号", "err");
    return;
  }
  const site = trimmed.endsWith("/api/yggdrasil")
    ? trimmed.slice(0, -"/api/yggdrasil".length)
    : trimmed;
  void openExternal(`${site}/user/player`).catch((err) =>
    notify(`打开皮肤站失败：${String(err)}`, "err")
  );
}

async function switchDoc(next: DocMode) {
  if (docMode.value === next || capeLoading.value) return;
  if (next === "cape") {
    if (!docs.cape.canvas) {
      capeLoading.value = true;
      await loadCape();
      capeLoading.value = false;
    }
    void refreshCapes();
    previousMode = mode.value;
    // 披风没有 3D 绘制视图，统一用 2D 画布 + 穿着预览
    mode.value = "2d";
  }
  part.value = "all";
  docMode.value = next;
  await nextTick();
  bindActive();
  if (next === "skin") {
    mode.value = previousMode;
    await nextTick();
  }
  fitCanvas();
}

function toggleLayer(key: "base" | "overlay") {
  layerVisible[key] = !layerVisible[key];
  if (!layerVisible[key] && layer.value === key) {
    const other = key === "base" ? "overlay" : "base";
    if (layerVisible[other]) layer.value = other;
  }
  refreshDisplay();
}

function currentPng() {
  return docs.skin.canvas?.toDataURL("image/png") ?? "";
}

async function saveLocal() {
  if (saving.value) return;
  saving.value = true;
  try {
    await api.saveAccountSkin(props.account.id, currentPng(), model.value);
    await refreshAccounts();
    await loadSelectedSkin();
    notify("皮肤已保存到本地");
  } catch (err) {
    notify(`保存皮肤失败：${String(err)}`, "err");
  } finally {
    saving.value = false;
  }
}

async function exportPng() {
  const path = await saveSkinDialog(`${props.account.name}-skin.png`);
  if (!path) return;
  try {
    await api.saveAccountSkin(props.account.id, currentPng(), model.value);
    await api.exportAccountSkin(props.account.id, path);
    await refreshAccounts();
    await loadSelectedSkin();
    notify("皮肤已导出");
  } catch (err) {
    notify(`导出失败：${String(err)}`, "err");
  }
}

async function confirmUpload() {
  if (uploading.value) return;
  uploading.value = true;
  try {
    await api.saveAccountSkin(props.account.id, currentPng(), model.value);
    await api.uploadAccountSkin(props.account.id, currentPng(), model.value);
    await refreshAccounts();
    await loadSelectedSkin();
    uploadOpen.value = false;
    notify("皮肤已上传到 Mojang");
  } catch (err) {
    notify(`上传失败：${String(err)}`, "err");
  } finally {
    uploading.value = false;
  }
}

function setTool(value: typeof tool.value) {
  tool.value = value;
}

function selectPart(value: typeof part.value) {
  part.value = value;
  nextTick(drawGuides);
}

function resetSkin() {
  if (docMode.value === "cape") {
    docs.cape.canvas = null;
    void loadCape().then(() => {
      if (docMode.value === "cape") bindActive();
    });
    return;
  }
  void loadSource();
}

async function applyPreset(preset: DefaultSkin) {
  const doc = docs.skin.canvas;
  if (!doc) return;
  try {
    const base = await canvasFromSource(preset.data_url);
    const target = doc.getContext("2d");
    if (!target) return;
    target.clearRect(0, 0, SKIN_SIZE, SKIN_SIZE);
    target.drawImage(base, 0, 0);
    pushHistory();
    refreshDisplay();
    bumpDoc();
    model.value = preset.model === "slim" ? "slim" : "classic";
    notify(`已套用默认皮肤：${preset.name}`);
  } catch (err) {
    notify(`载入默认皮肤失败：${String(err)}`, "err");
  }
}

async function loadLibrary() {
  try {
    library.value = await api.skinLibraryList();
  } catch {
    // 收藏库读取失败不影响编辑
  }
}

async function applyLibrary(item: SkinLibraryItem) {
  const doc = docs.skin.canvas;
  if (!doc) return;
  try {
    const base = await canvasFromSource(item.data_url);
    const target = doc.getContext("2d");
    if (!target) return;
    target.clearRect(0, 0, SKIN_SIZE, SKIN_SIZE);
    target.drawImage(base, 0, 0);
    pushHistory();
    refreshDisplay();
    bumpDoc();
    model.value = item.model === "slim" ? "slim" : "classic";
    notify(`已载入收藏皮肤：${item.name}`);
  } catch (err) {
    notify(`载入收藏皮肤失败：${String(err)}`, "err");
  }
}

function openSaveLibrary() {
  libraryName.value = `${props.account.name} 皮肤`;
  libraryOpen.value = true;
}

async function confirmSaveLibrary() {
  const name = libraryName.value.trim();
  if (!name || librarySaving.value) return;
  librarySaving.value = true;
  try {
    await api.skinLibrarySave(name, model.value, currentPng());
    await loadLibrary();
    libraryOpen.value = false;
    notify(`已存入收藏库：${name}`);
  } catch (err) {
    notify(`存入收藏库失败：${String(err)}`, "err");
  } finally {
    librarySaving.value = false;
  }
}

onMounted(async () => {
  void api
    .defaultSkins()
    .then((list) => {
      presets.value = list;
    })
    .catch(() => {});
  void loadLibrary();
  await nextTick();
  canvasObserver = new ResizeObserver(fitCanvas);
  if (canvasScroll.value) canvasObserver.observe(canvasScroll.value);
  fitCanvas();
  await loadSource();
});

onBeforeUnmount(() => {
  canvasObserver?.disconnect();
  canvasObserver = null;
});

/// 切到 2D 时容器刚恢复显示，重新量一次并重绘图层显隐
watch(mode, async () => {
  if (mode.value === "2d") {
    await nextTick();
    fitCanvas();
    refreshDisplay();
  }
});

/// 披风画布尺寸不同、图层眼睛或模型变化时，重绘显示画布
watch([docMode, () => layerVisible.base, () => layerVisible.overlay, model], async () => {
  await nextTick();
  fitCanvas();
  refreshDisplay();
});

const tools = [
  { value: "brush", label: "画笔" },
  { value: "eraser", label: "橡皮" },
  { value: "fill", label: "填充" },
  { value: "picker", label: "取色" },
] as const;

const partOptions = [
  { value: "all", label: "全身" },
  { value: "head", label: "头部" },
  { value: "body", label: "身体" },
  { value: "armLeft", label: "左臂" },
  { value: "armRight", label: "右臂" },
  { value: "legLeft", label: "左腿" },
  { value: "legRight", label: "右腿" },
] as const;
</script>

<template>
  <div class="page editor">
    <header class="editor-head">
      <div class="row">
        <IconButton icon="chevron-left" label="返回账号页" :size="34" @click="emit('close')" />
        <div>
          <h1 class="page-title">{{ account.name }} 的皮肤</h1>
        </div>
      </div>
      <div class="row">
        <span class="tag" :class="{ 'tag--grass': isMicrosoft }">
          {{ docMode === "cape" ? "披风 · 本地编辑" : isMicrosoft ? "微软账号" : "离线账号" }}
        </span>
        <WoodButton class="head-btn" size="sm" @click="resetSkin">
          {{ docMode === "cape" ? "重载披风" : "重置" }}
        </WoodButton>
        <WoodButton v-if="docMode === 'skin'" class="head-btn" size="sm" @click="exportPng">
          导出 PNG
        </WoodButton>
        <WoodButton
          v-if="isMicrosoft && docMode === 'skin'"
          variant="danger"
          size="sm"
          @click="uploadOpen = true"
        >
          上传到 Mojang
        </WoodButton>
        <WoodButton
          v-if="docMode === 'skin'"
          variant="primary"
          size="sm"
          sheen
          :disabled="saving"
          @click="saveLocal"
        >
          <AppIcon name="check" :size="14" />
          {{ saving ? "保存中…" : "保存到本地" }}
        </WoodButton>
      </div>
    </header>

    <div class="editor-layout">
      <aside class="card editor-col editor-left">
        <section class="side-block">
          <div class="side-title">编辑内容</div>
          <div class="seg-row">
            <button
              class="tool tool--wide"
              :class="{ 'tool--active': docMode === 'skin' }"
              @click="switchDoc('skin')"
            >
              皮肤
            </button>
            <button
              class="tool tool--wide"
              :class="{ 'tool--active': docMode === 'cape' }"
              :disabled="capeLoading"
              @click="switchDoc('cape')"
            >
              {{ capeLoading ? "载入中…" : "披风" }}
            </button>
          </div>
          <p v-if="docMode === 'cape'" class="muted small cape-note">
            可保存到本地披风库；第三方账号的披风需到皮肤站网页上传。
          </p>
        </section>

        <section v-if="docMode === 'skin'" class="side-block">
          <div class="side-title">模式与模型</div>
          <div class="seg-row">
            <button
              class="tool tool--wide"
              :class="{ 'tool--active': mode === '3d' }"
              @click="mode = '3d'"
            >
              3D 绘制
            </button>
            <button
              class="tool tool--wide"
              :class="{ 'tool--active': mode === '2d' }"
              @click="mode = '2d'"
            >
              2D 精修
            </button>
          </div>
          <div class="seg-row">
            <button
              class="tool tool--wide"
              :class="{ 'tool--active': model === 'classic' }"
              @click="model = 'classic'"
            >
              经典
            </button>
            <button
              class="tool tool--wide"
              :class="{ 'tool--active': model === 'slim' }"
              @click="model = 'slim'"
            >
              纤细
            </button>
          </div>
        </section>

        <section class="side-block">
          <div class="side-head">
            <div class="side-title">工具</div>
            <div class="row-inline">
              <IconButton icon="undo" label="撤销" :size="26" :disabled="!canUndo" @click="undo" />
              <IconButton icon="redo" label="重做" :size="26" :disabled="!canRedo" @click="redo" />
            </div>
          </div>
          <div class="tool-row">
            <button
              v-for="item in tools"
              :key="item.value"
              class="tool tool--wide"
              :class="{ 'tool--active': tool === item.value }"
              @click="setTool(item.value)"
            >
              {{ item.label }}
            </button>
          </div>
          <div class="row-inline">
            <span class="tool-label">笔刷</span>
            <button
              v-for="size in [1, 2, 4]"
              :key="size"
              class="tool tool--size"
              :class="{ 'tool--active': brushSize === size }"
              @click="brushSize = size"
            >
              {{ size }}
            </button>
          </div>
          <button
            class="switch"
            :class="{ 'switch--on': fillWhole }"
            title="填充时直接铺满命中的整个面，不做连通区域判断"
            @click="fillWhole = !fillWhole"
          >
            <span class="switch__dot" />整块填充
          </button>
        </section>

        <section v-if="docMode === 'skin'" class="side-block">
          <div class="side-title">部位</div>
          <div class="part-grid">
            <button
              v-for="item in partOptions"
              :key="item.value"
              class="tool tool--wide"
              :class="{ 'tool--active': part === item.value }"
              @click="selectPart(item.value)"
            >
              {{ item.label }}
            </button>
          </div>
        </section>

        <section v-if="docMode === 'skin'" class="side-block">
          <div class="side-title">图层与选项</div>
          <div class="layer-grid">
            <button
              class="tool tool--wide"
              :class="{ 'tool--active': layer === 'base' }"
              :disabled="!layerVisible.base"
              :title="layerVisible.base ? '编辑基础层' : '基础层已隐藏，先点亮眼睛'"
              @click="layer = 'base'"
            >
              基础
            </button>
            <button
              class="eye-btn"
              :class="{ 'eye-btn--off': !layerVisible.base }"
              :aria-label="layerVisible.base ? '隐藏基础层' : '显示基础层'"
              :title="layerVisible.base ? '隐藏基础层（隐藏后不可绘制）' : '显示基础层'"
              @click="toggleLayer('base')"
            >
              <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
                <path d="M2.5 12S6 6 12 6s9.5 6 9.5 6-3.5 6-9.5 6-9.5-6-9.5-6z" />
                <circle cx="12" cy="12" r="2.6" />
                <path v-if="!layerVisible.base" d="M4 20 20 4" />
              </svg>
            </button>
            <button
              class="tool tool--wide"
              :class="{ 'tool--active': layer === 'overlay' }"
              :disabled="!layerVisible.overlay"
              :title="layerVisible.overlay ? '编辑外层' : '外层已隐藏，先点亮眼睛'"
              @click="layer = 'overlay'"
            >
              外层
            </button>
            <button
              class="eye-btn"
              :class="{ 'eye-btn--off': !layerVisible.overlay }"
              :aria-label="layerVisible.overlay ? '隐藏外层' : '显示外层'"
              :title="layerVisible.overlay ? '隐藏外层（隐藏后不可绘制）' : '显示外层'"
              @click="toggleLayer('overlay')"
            >
              <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
                <path d="M2.5 12S6 6 12 6s9.5 6 9.5 6-3.5 6-9.5 6-9.5-6-9.5-6z" />
                <circle cx="12" cy="12" r="2.6" />
                <path v-if="!layerVisible.overlay" d="M4 20 20 4" />
              </svg>
            </button>
          </div>
          <p v-if="!layerVisible[layer]" class="layer-warn">
            当前图层已隐藏，点亮眼睛后才能绘制
          </p>
          <div v-if="mode === '3d'" class="opt-grid">
            <button
              class="switch"
              :class="{ 'switch--on': symmetry }"
              title="绘制时自动左右镜像"
              @click="symmetry = !symmetry"
            >
              <span class="switch__dot" />对称
            </button>
            <button
              class="switch"
              :disabled="!canMirror"
              :title="canMirror ? '把当前这一侧的绘制内容复制到对侧' : '先选中一条手臂或腿'"
              @click="mirrorToPair"
            >
              <span class="switch__dot" />镜像
            </button>
            <button
              class="switch"
              :class="{ 'switch--on': ghost }"
              title="其它部位和另一图层半透明显示；关闭时只显示正在编辑的部位与图层"
              @click="ghost = !ghost"
            >
              <span class="switch__dot" />透视
            </button>
            <button
              class="switch"
              :class="{ 'switch--on': walk }"
              title="让模型走起来，方便看整体效果"
              @click="walk = !walk"
            >
              <span class="switch__dot" />行走
            </button>
          </div>
        </section>

        <section v-if="mode === '2d'" class="side-block">
          <div class="side-head">
            <div class="side-title">画布</div>
            <button
              class="tool"
              :class="{ 'tool--active': showGuides }"
              @click="showGuides = !showGuides; nextTick(drawGuides)"
            >
              网格
            </button>
          </div>
          <div class="tool-row">
            <button
              class="tool"
              :class="{ 'tool--active': zoom === 'fit' }"
              title="按窗口大小自适应"
              @click="zoom = 'fit'"
            >
              适应
            </button>
            <button
              v-for="value in [1, 2, 4]"
              :key="value"
              class="tool"
              :class="{ 'tool--active': zoom === value }"
              @click="zoom = value"
            >
              {{ value }}×
            </button>
          </div>
        </section>
      </aside>

      <section class="card editor-main">
        <div v-if="docMode === 'skin' && mode === '3d'" class="stage-wrap">
          <SkinPaintViewport
            :source="docSource"
            :version="docVersion"
            :model="model"
            :tool="tool"
            :color="color"
            :brush-size="brushSize"
            :symmetry="symmetry"
            :part="part"
            :layer="layer"
            :ghost="ghost"
            :show-all="showAll"
            :walk="walk"
            :fill-whole="fillWhole"
            :base-visible="layerVisible.base"
            :overlay-visible="layerVisible.overlay"
            :height="null"
            @commit="onViewportCommit"
            @pick="onViewportPick"
          />
          <button
            class="tool preview-toggle"
            :class="{ 'tool--active': showAll }"
            title="所有部位和内外层都不透明显示，用来完整预览皮肤"
            @click="showAll = !showAll"
          >
            <img class="preview-toggle__icon" :src="diamondImg" alt="" />
            预览
          </button>
          <p v-if="!layerVisible[layer]" class="stage-blocked">
            当前图层已隐藏，点亮眼睛后才能绘制
          </p>
        </div>

        <div v-show="docMode === 'cape' || mode === '2d'" class="canvas-wrap">
          <div ref="canvasScroll" class="canvas-scroll">
            <div
              class="canvas-stack"
              :style="{ width: `${canvasWidth}px`, height: `${canvasHeight}px` }"
            >
              <canvas
                ref="drawCanvas"
                width="64"
                height="64"
                class="skin-canvas"
                :class="{ 'skin-canvas--disabled': docMode === 'skin' && !layerVisible[layer] }"
                @pointerdown="onPointerDown"
                @pointermove="onPointerMove"
                @pointerup="onPointerUp"
                @pointerleave="onPointerUp"
              />
              <canvas
                ref="guideCanvas"
                width="64"
                height="64"
                class="guide-canvas"
              />
            </div>
          </div>
          <div v-if="docMode === 'cape'" class="cape-preview">
            <div class="cape-preview__head">
              <span class="side-title">穿着预览</span>
              <button
                class="switch"
                :class="{ 'switch--on': walk }"
                @click="walk = !walk"
              >
                <span class="switch__thumb" />
                行走
              </button>
            </div>
            <SkinModel
              v-if="capePreviewUrl"
              :skin="previewSkinUrl"
              :cape="capePreviewUrl"
              :model="model"
              :walking="walk"
              :height="300"
              view="back"
              class="cape-preview__model"
            />
          </div>
        </div>
      </section>

      <aside class="card editor-col editor-right">
        <section class="side-block">
          <div class="side-title">颜色</div>
          <div class="color-head">
            <div class="color-current" :style="{ background: color }" />
            <input
              v-model="color"
              class="hex-input mono"
              maxlength="7"
              spellcheck="false"
            />
          </div>
          <div class="palette">
            <button
              v-for="item in palette"
              :key="item"
              class="swatch"
              :class="{ 'swatch--active': color.toLowerCase() === item }"
              :style="{ background: item }"
              @click="color = item"
            />
          </div>

          <div
            ref="svArea"
            class="sv-area"
            :style="{
              background:
                'linear-gradient(to top, #000, transparent), linear-gradient(to right, #fff, hsl(' +
                hue +
                ', 100%, 50%))',
            }"
            @pointerdown="onSvDown"
            @pointermove="onSvMove"
            @pointerup="onSvUp"
            @pointercancel="onSvUp"
          >
            <span
              class="sv-knob"
              :style="{ left: `${sat}%`, top: `${100 - val}%`, background: color }"
            />
          </div>

          <div
            ref="hueBar"
            class="hue-bar"
            @pointerdown="onHueDown"
            @pointermove="onHueMove"
            @pointerup="onHueUp"
            @pointercancel="onHueUp"
          >
            <span
              class="hue-knob"
              :style="{ left: `${(hue / 360) * 100}%`, background: `hsl(${hue}, 100%, 50%)` }"
            />
          </div>
        </section>

        <section v-if="docMode === 'cape'" class="side-block">
          <div class="side-title">披风库</div>
          <div class="cape-actions cape-actions--name">
            <WoodInput v-model="capeName" placeholder="披风名称" />
            <WoodButton
              variant="primary"
              size="sm"
              :disabled="capeSaving || !docs.cape.canvas"
              @click="saveCapeToLibrary"
            >
              {{ capeSaving ? "保存中…" : "保存" }}
            </WoodButton>
          </div>
          <div class="cape-actions">
            <WoodButton
              variant="ghost"
              size="sm"
              :disabled="!!capeBusy"
              @click="importCapeFile"
            >
              导入 PNG
            </WoodButton>
            <WoodButton
              variant="ghost"
              size="sm"
              :disabled="!docs.cape.canvas || !!capeBusy"
              @click="exportCapeFile"
            >
              导出 PNG
            </WoodButton>
          </div>
          <div class="cape-actions">
            <WoodButton
              variant="ghost"
              size="sm"
              :title="
                account.kind === 'authlib'
                  ? '在浏览器打开皮肤站角色页上传披风'
                  : '微软账号不支持上传自定义披风'
              "
              @click="openCapeUploadPage"
            >
              {{ account.kind === "authlib" ? "去皮肤站上传" : "上传到皮肤站" }}
            </WoodButton>
          </div>

          <div v-if="capes.length" class="cape-grid">
            <div v-for="cape in capes" :key="cape.name" class="cape-item">
              <button
                class="cape-item__face"
                :title="`载入 ${cape.name}`"
                :style="{ backgroundImage: `url(${cape.data_url})` }"
                @click="applyCapeFromLibrary(cape)"
              />
              <span class="cape-item__name" :title="cape.name">{{ cape.name }}</span>
              <button
                class="cape-item__remove"
                title="从本地库删除"
                @click="removeCapeFromLibrary(cape)"
              >
                ×
              </button>
            </div>
          </div>
          <p v-else class="muted small preset-hint">画好后点「保存」即可反复取用。</p>
        </section>

        <section class="side-block">
          <div class="side-title">模板</div>
          <div class="template-grid">
            <button
              v-for="item in visibleTemplates"
              :key="item.key"
              class="template template--compact"
              :title="item.hint"
              :disabled="docMode === 'skin' && item.key !== 'blank' && !layerVisible.base"
              @click="applyTemplate(item.key)"
            >
              <span
                class="template-face"
                :class="`template-face--${item.key}`"
                :style="templateFaceStyle(item.key)"
              />
              <span class="template-text">
                <span class="template-name">{{ item.name }}</span>
                <span class="template-hint">{{ item.hint }}</span>
              </span>
            </button>
          </div>
          <p v-if="docMode === 'cape'" class="muted small preset-hint">
            披风模板只覆盖正面与背面，四周拼接边保持透明。
          </p>
          <p v-else class="muted small preset-hint">
            皮肤模板只写入基础层，外层保持透明。
          </p>
        </section>

        <section v-if="docMode === 'skin' && presets.length" class="side-block">
          <div class="side-title">默认皮肤</div>
          <div class="preset-grid">
            <button
              v-for="preset in presets"
              :key="preset.id"
              class="preset"
              :title="`套用 ${preset.name}`"
              @click="applyPreset(preset)"
            >
              <span
                class="preset-face"
                :style="{ backgroundImage: `url(${preset.data_url})` }"
              />
              <span class="preset-name">{{ preset.name }}</span>
            </button>
          </div>
        </section>

        <section v-if="docMode === 'skin'" class="side-block">
          <div class="side-title">收藏库</div>
          <div class="preset-grid">
            <button
              v-for="item in library"
              :key="item.id"
              class="preset"
              :title="`载入 ${item.name}`"
              @click="applyLibrary(item)"
            >
              <span
                class="preset-face"
                :style="{ backgroundImage: `url(${item.data_url})` }"
              />
              <span class="preset-name">{{ item.name }}</span>
            </button>
            <button class="preset preset--add" title="把当前皮肤存入收藏库" @click="openSaveLibrary">
              <span class="preset-face preset-face--add">
                <AppIcon name="plus" :size="14" />
              </span>
              <span class="preset-name">存入</span>
            </button>
          </div>
          <p v-if="!library.length" class="muted small preset-hint">
            收藏库还是空的，可在皮肤库导入或抓取
          </p>
        </section>
      </aside>
    </div>

    <WoodModal
      v-if="libraryOpen"
      title="存入皮肤收藏库"
      :width="400"
      @close="libraryOpen = false"
    >
      <label class="library-field">
        <span class="library-field__label">皮肤名称</span>
        <WoodInput v-model="libraryName" placeholder="例如：夏日生存皮肤" />
      </label>
      <p class="muted small">同名皮肤会被覆盖，保存后可在皮肤库页面查看与套用。</p>
      <template #footer>
        <WoodButton variant="ghost" size="sm" @click="libraryOpen = false">取消</WoodButton>
        <WoodButton
          variant="primary"
          size="sm"
          :disabled="librarySaving || !libraryName.trim()"
          @click="confirmSaveLibrary"
        >
          {{ librarySaving ? "保存中…" : "存入收藏库" }}
        </WoodButton>
      </template>
    </WoodModal>

    <WoodModal
      v-if="uploadOpen"
      title="上传皮肤到 Mojang"
      :width="440"
      @close="uploadOpen = false"
    >
      <p class="warning">
        这会把该账号当前皮肤替换为本次编辑的皮肤，并影响所有使用该账号的客户端。
        Mojang 没有官方撤销功能，频繁上传可能触发频率限制。
      </p>
      <p class="muted small">确定要继续吗？</p>
      <template #footer>
        <WoodButton variant="ghost" size="sm" @click="uploadOpen = false">取消</WoodButton>
        <WoodButton
          variant="danger"
          size="sm"
          :disabled="uploading"
          @click="confirmUpload"
        >
          {{ uploading ? "上传中…" : "确认上传" }}
        </WoodButton>
      </template>
    </WoodModal>
  </div>
</template>

<style scoped>
.editor-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 18px;
  flex-wrap: wrap;
}

/* 头部次级按钮统一成偏白底，和苔绿主按钮拉开层级 */
.editor-head .head-btn {
  --face: var(--panel);
}

.editor {
  /* 标题栏 54 + 主区上下内边距 28/44，让编辑器正好铺满可视高度 */
  height: calc(100vh - 126px);
  min-height: 400px;
  gap: var(--space-5);
}

.editor-layout {
  flex: 1;
  min-height: 0;
  display: grid;
  grid-template-columns: 182px minmax(0, 1fr) 224px;
  gap: var(--space-5);
}

/* 两侧工具列：内容多时自己滚动，不挤压中间画布 */
.editor-col {
  padding: var(--space-4);
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  min-height: 0;
}

.editor-col .tool,
.editor-col .switch {
  height: 26px;
}

.side-block {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
}

.side-block + .side-block {
  padding-top: 6px;
  border-top: 1px dashed var(--line);
}

.side-title {
  font-size: var(--text-xs);
  letter-spacing: 0.14em;
  color: var(--muted);
  text-transform: uppercase;
}

.side-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-2);
}

.seg-row {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: var(--space-1);
}

.tool-row {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: var(--space-1);
}

.tool-row .tool {
  padding: 0 2px;
  font-size: var(--text-sm);
}

.opt-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: var(--space-1);
}

.part-grid {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: var(--space-1);
}

.part-grid .tool {
  padding: 0 2px;
  font-size: var(--text-sm);
}

.row-inline {
  display: flex;
  align-items: center;
  gap: var(--space-1);
}

/* 选项开关：勾选时点亮苔藓色，和工具按钮一致 */
.switch {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  height: 28px;
  padding: 0 8px;
  border-radius: 2px;
  border: 1px solid var(--line);
  background: var(--panel-2);
  color: var(--ink-soft);
  font-size: var(--text-sm);
  font-weight: 600;
  text-align: left;
  transition: border-color var(--dur-1) ease, background var(--dur-1) ease,
    color var(--dur-1) ease;
}

.switch:hover {
  border-color: var(--line-strong);
}

.switch:disabled {
  opacity: var(--opacity-disabled);
  cursor: not-allowed;
}

.switch__dot {
  width: 8px;
  height: 8px;
  flex-shrink: 0;
  border-radius: 50%;
  border: 1px solid var(--line-strong);
  background: transparent;
}

.switch--on {
  color: var(--moss);
  background: var(--moss-soft);
  border-color: color-mix(in srgb, var(--moss) 38%, var(--line));
}

.switch--on .switch__dot {
  background: var(--moss);
  border-color: var(--moss);
}

.editor-main {
  padding: var(--space-4);
  display: flex;
  flex-direction: column;
  min-height: 0;
}

.stage-wrap {
  position: relative;
  flex: 1;
  min-height: 0;
  display: flex;
}

.stage-wrap > * {
  flex: 1;
  min-height: 0;
}

/* 画布右上角的预览开关：沿用木质立体按钮材质，叠在 3D 视口之上 */
.preview-toggle {
  position: absolute;
  top: 10px;
  right: 10px;
  z-index: 2;
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  gap: 5px;
  height: 28px;
  background: var(--panel-2);
  border-color: color-mix(in srgb, var(--line-strong) 75%, transparent);
  box-shadow: 0 1px 2px rgba(43, 33, 24, 0.12);
}

.preview-toggle__icon {
  display: block;
  width: 15px;
  height: 15px;
  border-radius: 2px;
  image-rendering: pixelated;
}

/* 激活时沿用按钮的苔绿底色（覆盖上方的纸质底） */
.preview-toggle.tool--active {
  background: var(--moss-soft);
}

.tool-label {
  font-size: var(--text-xs);
  letter-spacing: 0.12em;
  color: var(--muted);
  text-transform: uppercase;
  margin-right: 2px;
  white-space: nowrap;
}

/* 工具按钮沿用同一套浮雕：悬停浮起，选中压进面板 */
.tool {
  --face: var(--panel-2);
  height: 28px;
  min-width: 34px;
  padding: 0 9px;
  border-radius: 2px;
  border-style: solid;
  border-width: 1px 1px 2px;
  border-color: transparent;
  background: transparent;
  color: var(--ink-soft);
  font-size: var(--text-sm);
  font-weight: 600;
  white-space: nowrap;
  transition: background var(--dur-1) ease, color var(--dur-1) ease,
    border-color var(--dur-1) ease, transform var(--dur-0) ease, box-shadow var(--dur-0) ease;
}

.tool:not(.tool--active):hover {
  color: var(--walnut-deep);
  background: var(--hover);
  transform: translateY(-1px);
  border-top-color: color-mix(in srgb, var(--face) 52%, #fff);
  border-left-color: color-mix(in srgb, var(--face) 52%, #fff);
  border-right-color: color-mix(in srgb, var(--face) 42%, #000);
  border-bottom-color: color-mix(in srgb, var(--face) 42%, #000);
}

.tool:active {
  transform: translateY(1px);
  border-top-color: color-mix(in srgb, var(--face) 42%, #000);
  border-left-color: color-mix(in srgb, var(--face) 42%, #000);
  border-right-color: color-mix(in srgb, var(--face) 52%, #fff);
  border-bottom-color: color-mix(in srgb, var(--face) 52%, #fff);
  border-bottom-width: 1px;
  box-shadow: var(--shadow-inset-soft);
}

.tool--active {
  background: var(--moss-soft);
  color: var(--moss);
  transform: translateY(1px);
  border-top-color: color-mix(in srgb, var(--moss) 42%, #000);
  border-left-color: color-mix(in srgb, var(--moss) 42%, #000);
  border-right-color: color-mix(in srgb, var(--moss) 34%, #fff);
  border-bottom-color: color-mix(in srgb, var(--moss) 34%, #fff);
  border-bottom-width: 1px;
  box-shadow: var(--shadow-inset-soft);
}

.tool--size {
  min-width: 28px;
  padding: 0 6px;
}

.tool--wide {
  width: 100%;
  min-width: 0;
  padding: 0 4px;
}

.tool:disabled {
  opacity: var(--opacity-disabled);
  cursor: not-allowed;
}

.tool:disabled:hover {
  transform: none;
  background: transparent;
  color: var(--ink-soft);
  border-color: transparent;
}

.color-head {
  display: flex;
  align-items: center;
  gap: var(--space-3);
}

.sv-area {
  position: relative;
  height: 112px;
  border-radius: 0;
  border-style: solid;
  border-width: 1px;
  border-top-color: color-mix(in srgb, var(--line-strong) 66%, #000);
  border-left-color: color-mix(in srgb, var(--line-strong) 66%, #000);
  border-right-color: color-mix(in srgb, var(--line-strong) 38%, #fff);
  border-bottom-color: color-mix(in srgb, var(--line-strong) 38%, #fff);
  cursor: crosshair;
  touch-action: none;
  box-shadow: inset 0 1px 3px rgba(80, 58, 34, 0.2);
}

.sv-knob {
  position: absolute;
  width: 14px;
  height: 14px;
  margin: -7px 0 0 -7px;
  border-radius: 0;
  border: 2px solid #fffdf6;
  box-shadow: 0 1px 4px rgba(0, 0, 0, 0.35);
  pointer-events: none;
}

.hue-bar {
  position: relative;
  height: 14px;
  border-radius: 0;
  border-style: solid;
  border-width: 1px;
  border-top-color: color-mix(in srgb, var(--line-strong) 66%, #000);
  border-left-color: color-mix(in srgb, var(--line-strong) 66%, #000);
  border-right-color: color-mix(in srgb, var(--line-strong) 38%, #fff);
  border-bottom-color: color-mix(in srgb, var(--line-strong) 38%, #fff);
  cursor: pointer;
  touch-action: none;
  background: linear-gradient(
    to right,
    #f00 0%,
    #ff0 17%,
    #0f0 33%,
    #0ff 50%,
    #00f 67%,
    #f0f 83%,
    #f00 100%
  );
}

.hue-knob {
  position: absolute;
  top: 50%;
  width: 16px;
  height: 16px;
  margin: -8px 0 0 -8px;
  border-radius: 0;
  border: 2px solid #fffdf6;
  box-shadow: 0 1px 4px rgba(0, 0, 0, 0.3);
  pointer-events: none;
}

.preset-grid {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: var(--space-1);
}

.preset {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 3px;
  padding: 4px 2px;
  border-radius: 0;
  border: 1px solid transparent;
  transition: border-color var(--dur-1) ease, background var(--dur-1) ease,
    transform var(--dur-1) ease;
}

.preset:hover {
  border-color: var(--line-strong);
  background: var(--panel-2);
  transform: translateY(-1px);
}

.preset-face {
  width: 44px;
  height: 44px;
  border-radius: 0;
  border: 1px solid rgba(0, 0, 0, 0.12);
  background-size: 352px 352px;
  background-position: -44px -44px;
  background-repeat: no-repeat;
  image-rendering: pixelated;
}

.preset-name {
  font-size: 9.5px;
  letter-spacing: 0.04em;
  color: var(--muted);
  line-height: 1.1;
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.preset-hint {
  line-height: 1.5;
}

/* 色块模板入口 */
.template {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  padding: var(--space-3);
  border-radius: 2px;
  border: 1px solid var(--line);
  background: var(--panel-2);
  text-align: left;
  transition: border-color var(--dur-1) ease, transform var(--dur-1) ease;
}

.template:hover {
  border-color: var(--moss);
  transform: translateY(-1px);
}

.template-face {
  width: 34px;
  height: 34px;
  flex-shrink: 0;
  border: 1px solid rgba(0, 0, 0, 0.12);
  background: linear-gradient(
    160deg,
    #e8c39a 0 30%,
    #6f8a4f 30% 52%,
    #4c7f9e 52% 70%,
    #b06a4a 70% 84%,
    #7a6aa8 84% 100%
  );
}

.template-text {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.template-name {
  font-size: var(--text-sm);
  font-weight: 700;
  color: var(--ink-soft);
}

.template-hint {
  font-size: 9.5px;
  color: var(--muted);
  letter-spacing: 0.02em;
}

/* 模板卡片：两列紧凑排布，保持与原来一致的纸质卡片风格 */
.template-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: var(--space-1);
}

.template--compact {
  flex-direction: column;
  align-items: stretch;
  gap: 5px;
  padding: var(--space-2);
}

.template--compact .template-face {
  width: 100%;
  height: 26px;
}

.template--compact .template-name {
  font-size: var(--text-sm);
}

.template--compact .template-hint {
  font-size: 9px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.template:disabled {
  opacity: var(--opacity-disabled);
  cursor: not-allowed;
}

.template:disabled:hover {
  border-color: var(--line);
  transform: none;
}

.cape-note {
  line-height: 1.5;
}

/* 图层：左边选择、右边眼睛，隐藏后按钮与绘制一起禁用 */
.layer-grid {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 28px;
  gap: var(--space-1);
  align-items: center;
}

.eye-btn {
  display: grid;
  place-items: center;
  height: 26px;
  border-radius: 2px;
  border: 1px solid var(--line);
  background: var(--panel-2);
  color: var(--moss);
  transition: border-color var(--dur-1) ease, color var(--dur-1) ease,
    background var(--dur-1) ease;
}

.eye-btn:hover {
  border-color: var(--line-strong);
}

.eye-btn--off {
  color: var(--muted);
  background: transparent;
  border-style: dashed;
}

.layer-warn {
  margin: 2px 0 0;
  font-size: var(--text-xs);
  line-height: 1.4;
  color: var(--danger);
}

.stage-blocked {
  position: absolute;
  top: 10px;
  left: 10px;
  z-index: 2;
  margin: 0;
  padding: 5px 9px;
  border: 1px solid #e7cfc5;
  border-radius: 2px;
  background: var(--danger-soft);
  color: var(--danger);
  font-size: var(--text-xs);
  pointer-events: none;
}

.canvas-wrap {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}

.canvas-wrap .canvas-scroll {
  flex: 1;
  min-height: 0;
}

.skin-canvas--disabled {
  cursor: not-allowed;
  opacity: 0.55;
}

.cape-preview {
  flex: 0 0 auto;
  display: flex;
  flex-direction: column;
  align-items: stretch;
  gap: var(--space-3);
  padding: 10px 12px;
  border: 1px solid var(--line);
  border-radius: 2px;
  background: var(--panel-2);
}

.cape-preview__head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-3);
}

.cape-preview__model {
  border: 1px solid var(--line-strong);
  background:
    linear-gradient(45deg, rgba(0, 0, 0, 0.05) 25%, transparent 25% 75%, rgba(0, 0, 0, 0.05) 75%),
    linear-gradient(45deg, rgba(0, 0, 0, 0.05) 25%, transparent 25% 75%, rgba(0, 0, 0, 0.05) 75%);
  background-size: 10px 10px;
  background-position: 0 0, 5px 5px;
}

/* 披风库 */
.cape-actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-2);
}

.cape-actions .input {
  flex: 1 1 0;
  min-width: 0;
}

.cape-actions .btn {
  flex: 1 1 0;
  min-width: 0;
  white-space: nowrap;
}

.cape-actions--name .btn {
  flex: 0 0 auto;
  padding: 0 14px;
}

.cape-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(64px, 1fr));
  gap: var(--space-3);
  margin-top: 4px;
}

.cape-item {
  position: relative;
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
}

.cape-item__face {
  height: 56px;
  border-radius: 2px;
  border: 1px solid var(--line-strong);
  background-color: var(--panel);
  background-size: 100% 100%;
  background-repeat: no-repeat;
  image-rendering: pixelated;
}

.cape-item__face:hover {
  border-color: var(--moss);
}

.cape-item__name {
  font-size: var(--text-xs);
  color: var(--muted);
  text-align: center;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.cape-item__remove {
  position: absolute;
  top: -4px;
  right: -4px;
  width: 18px;
  height: 18px;
  display: grid;
  place-items: center;
  border-radius: 50%;
  border: 1px solid var(--line-strong);
  background: var(--panel);
  color: var(--muted);
  font-size: var(--text-base);
  line-height: 1;
}

.cape-item__remove:hover {
  color: var(--danger);
  border-color: var(--danger);
}

.preset-face--add {
  display: grid;
  place-items: center;
  border-style: dashed;
  color: var(--muted);
  background: transparent;
}

.preset--add:hover .preset-face--add {
  border-color: var(--moss);
  color: var(--moss);
}

.library-field {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  margin-bottom: 10px;
}

.library-field__label {
  font-size: var(--text-sm);
  font-weight: 700;
  color: var(--ink-soft);
  letter-spacing: 0.04em;
}

.color-current {
  width: 30px;
  height: 30px;
  border-radius: 0;
  border: 1px solid var(--line-strong);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.4),
    0 1px 3px rgba(80, 58, 34, 0.2);
  flex-shrink: 0;
}

.palette {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-1);
  min-width: 0;
}

.swatch {
  width: 22px;
  height: 22px;
  border-radius: 0;
  border: 1px solid rgba(0, 0, 0, 0.12);
  transition: transform var(--dur-1) ease;
}

.swatch:hover {
  transform: scale(1.12);
}

/* 选中色块：外面压一圈苔藓实线框，方块感更强 */
.swatch--active {
  box-shadow: 0 0 0 2px var(--moss);
}

.hex-input {
  flex: 1;
  min-width: 0;
  height: 30px;
  border-radius: 0;
  border-style: solid;
  border-width: 1px;
  border-top-color: color-mix(in srgb, var(--line-strong) 66%, #000);
  border-left-color: color-mix(in srgb, var(--line-strong) 66%, #000);
  border-right-color: color-mix(in srgb, var(--line-strong) 38%, #fff);
  border-bottom-color: color-mix(in srgb, var(--line-strong) 38%, #fff);
  background: var(--panel);
  box-shadow: inset 0 1px 2px rgba(60, 40, 20, 0.08);
  padding: 0 8px;
  color: var(--ink);
  font-size: var(--text-base);
}

.canvas-scroll {
  flex: 1;
  min-height: 0;
  display: flex;
  overflow: auto;
  border-radius: 2px;
  border: 1px solid var(--line);
  background: var(--paper-2);
  padding: 14px;
}

.canvas-stack {
  position: relative;
  flex: 0 0 auto;
  /* flex 中 auto 外边距在放得下时居中、放不下时贴左上，滚动行为正常 */
  margin: auto;
}

.skin-canvas,
.guide-canvas {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  image-rendering: pixelated;
}

.skin-canvas {
  cursor: crosshair;
  border-radius: 4px;
  box-shadow: 0 8px 24px rgba(60, 40, 20, 0.18);
  background:
    linear-gradient(45deg, rgba(0, 0, 0, 0.04) 25%, transparent 25% 75%, rgba(0, 0, 0, 0.04) 75%),
    linear-gradient(45deg, rgba(0, 0, 0, 0.04) 25%, transparent 25% 75%, rgba(0, 0, 0, 0.04) 75%);
  background-size: 12px 12px;
  background-position: 0 0, 6px 6px;
}

.guide-canvas {
  pointer-events: none;
}

.warning {
  padding: 11px 13px;
  border-radius: 4px;
  background: var(--danger-soft);
  border: 1px solid #e7cfc5;
  color: var(--danger);
  font-size: var(--text-base);
  line-height: 1.6;
  margin-bottom: 12px;
}

/* 窗口变窄时压缩两侧，优先保证中间画布 */
@media (max-width: 1100px) {
  .editor-layout {
    grid-template-columns: 160px minmax(0, 1fr) 206px;
    gap: var(--space-4);
  }
}
</style>
