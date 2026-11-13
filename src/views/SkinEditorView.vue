<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { api, saveSkinDialog } from "../api";
import diamondImg from "../assets/icons/diamond.png";
import AppIcon from "../components/AppIcon.vue";
import IconButton from "../components/IconButton.vue";
import SkinPaintViewport from "../components/SkinPaintViewport.vue";
import WoodButton from "../components/WoodButton.vue";
import WoodInput from "../components/WoodInput.vue";
import WoodModal from "../components/WoodModal.vue";
import type { AccountView, DefaultSkin, SkinLibraryItem } from "../api";
import { canvasFromSource, createDefaultSkin, seedFromString, skinCanvasFromSource, SKIN_SIZE } from "../skin";
import { copyPartToPair, paintBlockTemplate } from "../skinEditor/paint";
import type { PartFilter } from "../skinEditor/atlas";
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
const symmetry = ref(false);
const docVersion = ref(0);
const showGuides = ref(true);
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

/// 自适应时按可用空间取整到整像素，保证放大后依然清晰
const canvasWidth = computed(
  () => SKIN_SIZE * (zoom.value === "fit" ? fitScale.value : zoom.value * 8)
);
const isMicrosoft = computed(() => props.account.kind === "microsoft");

function fitCanvas() {
  const box = canvasScroll.value;
  if (!box) return;
  // 减去容器的内边距，再按 64 整除，得到每个贴图像素占几个屏幕像素
  const width = box.clientWidth - 28;
  const height = box.clientHeight - 28;
  if (width <= 0 || height <= 0) return;
  fitScale.value = Math.max(1, Math.min(16, Math.floor(Math.min(width, height) / SKIN_SIZE)));
}

let ctx: CanvasRenderingContext2D | null = null;
let history: ImageData[] = [];
let cursor = -1;
let drawing = false;

function pushHistory() {
  if (!ctx) return;
  history = history.slice(0, cursor + 1);
  history.push(ctx.getImageData(0, 0, SKIN_SIZE, SKIN_SIZE));
  if (history.length > 40) history.shift();
  cursor = history.length - 1;
  canUndo.value = cursor > 0;
  canRedo.value = false;
}

function undo() {
  if (!ctx || cursor <= 0) return;
  cursor -= 1;
  ctx.putImageData(history[cursor], 0, 0);
  canUndo.value = cursor > 0;
  canRedo.value = true;
  bumpDoc();
}

function redo() {
  if (!ctx || cursor >= history.length - 1) return;
  cursor += 1;
  ctx.putImageData(history[cursor], 0, 0);
  canUndo.value = true;
  canRedo.value = cursor < history.length - 1;
  bumpDoc();
}

/// 2D 画布内容被外部改动（撤销、套用预设等）时，通知 3D 视口重新取图
function bumpDoc() {
  docVersion.value += 1;
}

function onViewportCommit() {
  pushHistory();
}

function onViewportPick(value: string) {
  color.value = value;
}

function mirrorToPair() {
  const canvas = drawCanvas.value;
  if (!canvas || !canMirror.value) return;
  if (!copyPartToPair(canvas, part.value as Exclude<PartFilter, "all" | "head" | "body">, model.value === "slim")) {
    return;
  }
  pushHistory();
  bumpDoc();
  notify("已把这一侧镜像到对侧");
}

/// 色块模板：清空皮肤，每个部位填不同颜色，方便从零开始画
function applyBlockTemplate() {
  const canvas = drawCanvas.value;
  if (!canvas) return;
  paintBlockTemplate(canvas, model.value === "slim");
  pushHistory();
  bumpDoc();
  notify("已套用色块模板");
}

function drawGuides() {
  const overlay = guideCanvas.value;
  if (!overlay) return;
  const g = overlay.getContext("2d");
  if (!g) return;
  g.clearRect(0, 0, SKIN_SIZE, SKIN_SIZE);
  if (!showGuides.value) return;
  g.save();
  g.setLineDash([2, 2]);
  g.lineWidth = 0.6;
  g.strokeStyle = "rgba(255, 200, 90, 0.9)";
  for (const [x, y, w, h] of guideRects.value[part.value]) {
    g.strokeRect(x + 0.3, y + 0.3, w - 0.6, h - 0.6);
  }
  g.restore();
}

function pointerPos(event: PointerEvent) {
  const canvas = drawCanvas.value;
  if (!canvas) return null;
  const rect = canvas.getBoundingClientRect();
  const x = Math.floor(((event.clientX - rect.left) / rect.width) * SKIN_SIZE);
  const y = Math.floor(((event.clientY - rect.top) / rect.height) * SKIN_SIZE);
  if (x < 0 || y < 0 || x >= SKIN_SIZE || y >= SKIN_SIZE) return null;
  return { x, y };
}

function floodFill(x: number, y: number, fillColor: string) {
  if (!ctx) return;
  const image = ctx.getImageData(0, 0, SKIN_SIZE, SKIN_SIZE);
  const data = image.data;
  const index = (px: number, py: number) => (py * SKIN_SIZE + px) * 4;
  const start = index(x, y);
  const target = [data[start], data[start + 1], data[start + 2], data[start + 3]];
  const hex = fillColor.replace("#", "");
  const fill = [
    parseInt(hex.slice(0, 2), 16),
    parseInt(hex.slice(2, 4), 16),
    parseInt(hex.slice(4, 6), 16),
    255,
  ];
  if (target.every((v, i) => v === fill[i])) return;
  const stack: [number, number][] = [[x, y]];
  while (stack.length) {
    const [px, py] = stack.pop()!;
    if (px < 0 || py < 0 || px >= SKIN_SIZE || py >= SKIN_SIZE) continue;
    const i = index(px, py);
    if (
      data[i] !== target[0] ||
      data[i + 1] !== target[1] ||
      data[i + 2] !== target[2] ||
      data[i + 3] !== target[3]
    ) {
      continue;
    }
    data[i] = fill[0];
    data[i + 1] = fill[1];
    data[i + 2] = fill[2];
    data[i + 3] = fill[3];
    stack.push([px + 1, py], [px - 1, py], [px, py + 1], [px, py - 1]);
  }
  ctx.putImageData(image, 0, 0);
}

function paintAt(x: number, y: number) {
  if (!ctx) return;
  if (tool.value === "picker") {
    const pixel = ctx.getImageData(x, y, 1, 1).data;
    const hex = `#${[pixel[0], pixel[1], pixel[2]]
      .map((v) => v.toString(16).padStart(2, "0"))
      .join("")}`;
    color.value = hex;
    tool.value = "brush";
    return;
  }
  if (tool.value === "fill") {
    floodFill(x, y, color.value);
    return;
  }
  const size = brushSize.value;
  const offset = Math.floor(size / 2);
  if (tool.value === "eraser") {
    ctx.clearRect(x - offset, y - offset, size, size);
  } else {
    ctx.fillStyle = color.value;
    ctx.fillRect(x - offset, y - offset, size, size);
  }
}

function onPointerDown(event: PointerEvent) {
  const pos = pointerPos(event);
  if (!pos) return;
  if (tool.value === "fill" || tool.value === "picker") {
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
  const canvas = drawCanvas.value;
  if (!canvas) return;
  ctx = canvas.getContext("2d");
  if (!ctx) return;
  ctx.imageSmoothingEnabled = false;
  ctx.clearRect(0, 0, SKIN_SIZE, SKIN_SIZE);
  ctx.drawImage(base, 0, 0);
  history = [];
  cursor = -1;
  pushHistory();
  const accountModel = store.accountSkin?.model ?? props.account.skin_model;
  model.value = accountModel === "slim" ? "slim" : "classic";
  docVersion.value += 1;
  drawGuides();
}

function currentPng() {
  return drawCanvas.value?.toDataURL("image/png") ?? "";
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
  void loadSource();
}

async function applyPreset(preset: DefaultSkin) {
  if (!ctx) return;
  try {
    const base = await canvasFromSource(preset.data_url);
    ctx.clearRect(0, 0, SKIN_SIZE, SKIN_SIZE);
    ctx.drawImage(base, 0, 0);
    pushHistory();
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
  if (!ctx) return;
  try {
    const base = await canvasFromSource(item.data_url);
    ctx.clearRect(0, 0, SKIN_SIZE, SKIN_SIZE);
    ctx.drawImage(base, 0, 0);
    pushHistory();
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

/// 切到 2D 时容器刚恢复显示，重新量一次
watch(mode, () => {
  if (mode.value === "2d") nextTick(fitCanvas);
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
          {{ isMicrosoft ? "微软账号" : "离线账号" }}
        </span>
        <WoodButton class="head-btn" size="sm" @click="resetSkin">重置</WoodButton>
        <WoodButton class="head-btn" size="sm" @click="exportPng">导出 PNG</WoodButton>
        <WoodButton
          v-if="isMicrosoft"
          variant="danger"
          size="sm"
          @click="uploadOpen = true"
        >
          上传到 Mojang
        </WoodButton>
        <WoodButton variant="primary" size="sm" sheen :disabled="saving" @click="saveLocal">
          <AppIcon name="check" :size="14" />
          {{ saving ? "保存中…" : "保存到本地" }}
        </WoodButton>
      </div>
    </header>

    <div class="editor-layout">
      <aside class="card editor-col editor-left">
        <section class="side-block">
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
        </section>

        <section class="side-block">
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

        <section v-if="mode === '3d'" class="side-block">
          <div class="side-title">图层与选项</div>
          <div class="seg-row">
            <button
              class="tool tool--wide"
              :class="{ 'tool--active': layer === 'base' }"
              @click="layer = 'base'"
            >
              基础
            </button>
            <button
              class="tool tool--wide"
              :class="{ 'tool--active': layer === 'overlay' }"
              @click="layer = 'overlay'"
            >
              外层
            </button>
          </div>
          <div class="opt-grid">
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
        <div v-if="mode === '3d'" class="stage-wrap">
          <SkinPaintViewport
            :source="drawCanvas"
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
        </div>

        <div ref="canvasScroll" v-show="mode === '2d'" class="canvas-scroll">
          <div class="canvas-stack" :style="{ width: `${canvasWidth}px`, height: `${canvasWidth}px` }">
            <canvas
              ref="drawCanvas"
              width="64"
              height="64"
              class="skin-canvas"
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

        <section class="side-block">
          <div class="side-title">模板</div>
          <button class="template" title="清空皮肤，每个部位填成不同颜色" @click="applyBlockTemplate">
            <span class="template-face" />
            <span class="template-text">
              <span class="template-name">色块模板</span>
              <span class="template-hint">各部位不同颜色，从零开始画</span>
            </span>
          </button>
        </section>

        <section v-if="presets.length" class="side-block">
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

        <section class="side-block">
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
  gap: 12px;
}

.editor-layout {
  flex: 1;
  min-height: 0;
  display: grid;
  grid-template-columns: 182px minmax(0, 1fr) 224px;
  gap: 12px;
}

/* 两侧工具列：内容多时自己滚动，不挤压中间画布 */
.editor-col {
  padding: 10px;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-height: 0;
}

.editor-col .tool,
.editor-col .switch {
  height: 26px;
}

.side-block {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.side-block + .side-block {
  padding-top: 6px;
  border-top: 1px dashed var(--line);
}

.side-title {
  font-size: 10px;
  letter-spacing: 0.14em;
  color: var(--muted);
  text-transform: uppercase;
}

.side-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 6px;
}

.seg-row {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 4px;
}

.tool-row {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: 4px;
}

.tool-row .tool {
  padding: 0 2px;
  font-size: 11px;
}

.opt-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 4px;
}

.part-grid {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: 4px;
}

.part-grid .tool {
  padding: 0 2px;
  font-size: 11px;
}

.row-inline {
  display: flex;
  align-items: center;
  gap: 4px;
}

/* 选项开关：勾选时点亮苔藓色，和工具按钮一致 */
.switch {
  display: flex;
  align-items: center;
  gap: 8px;
  height: 28px;
  padding: 0 8px;
  border-radius: 2px;
  border: 1px solid var(--line);
  background: var(--panel-2);
  color: var(--ink-soft);
  font-size: 11.5px;
  font-weight: 600;
  text-align: left;
  transition: border-color var(--dur-1) ease, background var(--dur-1) ease,
    color var(--dur-1) ease;
}

.switch:hover {
  border-color: var(--line-strong);
}

.switch:disabled {
  opacity: 0.38;
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
  padding: 10px;
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
  font-size: 10px;
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
  font-size: 11.5px;
  font-weight: 600;
  white-space: nowrap;
  transition: background 150ms ease, color 150ms ease,
    border-color 150ms ease, transform 100ms ease, box-shadow 100ms ease;
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
  box-shadow: inset 0 2px 3px rgba(0, 0, 0, 0.18);
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
  box-shadow: inset 0 2px 3px rgba(0, 0, 0, 0.14);
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
  opacity: 0.38;
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
  gap: 8px;
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
  gap: 4px;
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
  gap: 10px;
  padding: 8px;
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
  font-size: 11.5px;
  font-weight: 700;
  color: var(--ink-soft);
}

.template-hint {
  font-size: 9.5px;
  color: var(--muted);
  letter-spacing: 0.02em;
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
  gap: 6px;
  margin-bottom: 10px;
}

.library-field__label {
  font-size: 11px;
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
  gap: 4px;
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
  font-size: 12px;
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
  font-size: 12.5px;
  line-height: 1.6;
  margin-bottom: 12px;
}

/* 窗口变窄时压缩两侧，优先保证中间画布 */
@media (max-width: 1100px) {
  .editor-layout {
    grid-template-columns: 160px minmax(0, 1fr) 206px;
    gap: 10px;
  }
}
</style>
