<script setup lang="ts">
import * as THREE from "three";
import { onBeforeUnmount, onMounted, ref, watch } from "vue";
import {
  FACE_BY_GROUP,
  buildBoxes,
  faceFlip,
  faceRect,
  mirrorTexel,
  uvToTexel,
  type BoxSpec,
  type FaceName,
  type PartFilter,
} from "../skinEditor/atlas";
import { bresenham, floodFill, hexToRgba, rgbaToHex } from "../skinEditor/paint";

const props = withDefaults(
  defineProps<{
    source: HTMLCanvasElement | null;
    version: number;
    model: "classic" | "slim";
    tool: "brush" | "eraser" | "fill" | "picker";
    color: string;
    brushSize: number;
    symmetry: boolean;
    part: PartFilter;
    layer: "base" | "overlay";
    /// 透视：其它部位/另一图层半透明显示
    ghost?: boolean;
    /// 全显：所有部位与图层都不透明，用于完整预览
    showAll?: boolean;
    /// 行走动画
    walk?: boolean;
    /// 传 null 时铺满父容器
    height?: number | null;
  }>(),
  { height: 520, ghost: true, showAll: false, walk: false }
);

const emit = defineEmits<{
  (e: "change"): void;
  (e: "commit"): void;
  (e: "pick", color: string): void;
}>();

interface FaceEntry {
  ctx: CanvasRenderingContext2D;
  texture: THREE.CanvasTexture;
  rect: [number, number, number, number];
  flipX: boolean;
  flipY: boolean;
}

interface BoxEntry {
  box: BoxSpec;
  mesh: THREE.Mesh;
  faces: FaceEntry[];
  materials: THREE.MeshStandardMaterial[];
}

const host = ref<HTMLDivElement | null>(null);
let renderer: THREE.WebGLRenderer | null = null;
let scene: THREE.Scene | null = null;
let camera: THREE.PerspectiveCamera | null = null;
let frame = 0;
let resizeObserver: ResizeObserver | null = null;
let boxes: BoxSpec[] = [];
let entries: BoxEntry[] = [];
let worldGroup: THREE.Group | null = null;
let headGroup: THREE.Group | null = null;
let armLeftGroup: THREE.Group | null = null;
let armRightGroup: THREE.Group | null = null;
let legLeftGroup: THREE.Group | null = null;
let legRightGroup: THREE.Group | null = null;
let walkFrame = 0;
let walkClock: THREE.Clock | null = null;
let meshMap = new Map<THREE.Object3D, BoxEntry>();
let docCanvas: HTMLCanvasElement | null = null;
let docCtx: CanvasRenderingContext2D | null = null;
const dirty = new Set<string>();
const raycaster = new THREE.Raycaster();
const orbit = { theta: 0.62, phi: 0.12, radius: 66 };
let orbiting = false;
let painting = false;
let activePointer = -1;
let lastScreen = { x: 0, y: 0 };
let lastPoint: { boxId: string; face: FaceName; x: number; y: number } | null = null;

function docContext(): CanvasRenderingContext2D | null {
  if (!props.source) return null;
  if (docCanvas !== props.source) {
    docCanvas = props.source;
    docCtx = docCanvas.getContext("2d");
    if (docCtx) docCtx.imageSmoothingEnabled = false;
  }
  return docCtx;
}

/* ---------- 材质与贴图 ---------- */

function buildBoxEntry(box: BoxSpec, grow: number): THREE.Mesh {
  const faces: FaceEntry[] = [];
  const materials = FACE_BY_GROUP.map((face) => {
    const [rx, ry, rw, rh] = faceRect(box, face);
    const canvas = document.createElement("canvas");
    canvas.width = rw;
    canvas.height = rh;
    const ctx = canvas.getContext("2d")!;
    ctx.imageSmoothingEnabled = false;
    const texture = new THREE.CanvasTexture(canvas);
    texture.magFilter = THREE.NearestFilter;
    texture.minFilter = THREE.NearestFilter;
    texture.colorSpace = THREE.SRGBColorSpace;
    const overlay = box.layer === "overlay";
    const material = new THREE.MeshStandardMaterial({
      map: texture,
      transparent: true,
      alphaTest: 0.02,
      roughness: 0.95,
      metalness: 0,
      side: THREE.FrontSide,
      // 外层与基础层表面共面，靠深度偏移避免出现一条贯穿的暗线
      polygonOffset: overlay,
      polygonOffsetFactor: overlay ? -1 : 0,
      polygonOffsetUnits: overlay ? -1 : 0,
    });
    const { flipX, flipY } = faceFlip(face);
    faces.push({ ctx, texture, rect: [rx, ry, rw, rh], flipX, flipY });
    return material;
  });
  const geometry = new THREE.BoxGeometry(box.w + grow, box.h + grow, box.d + grow);
  geometry.translate(0, -(box.h + grow) / 2, 0);
  const mesh = new THREE.Mesh(geometry, materials);
  mesh.userData = { boxId: box.id, part: box.part, layer: box.layer };
  if (box.layer === "overlay") mesh.renderOrder = 2;
  entries.push({ box, mesh, faces, materials });
  meshMap.set(mesh, entries[entries.length - 1]);
  redrawBox(box.id);
  return mesh;
}

function redrawBox(id: string) {
  const entry = entries.find((item) => item.box.id === id);
  const source = props.source;
  if (!entry || !source) return;
  for (const face of entry.faces) {
    const [rx, ry, rw, rh] = face.rect;
    face.ctx.setTransform(1, 0, 0, 1, 0, 0);
    face.ctx.clearRect(0, 0, rw, rh);
    face.ctx.imageSmoothingEnabled = false;
    if (face.flipX || face.flipY) {
      face.ctx.translate(face.flipX ? rw : 0, face.flipY ? rh : 0);
      face.ctx.scale(face.flipX ? -1 : 1, face.flipY ? -1 : 1);
    }
    face.ctx.drawImage(source, rx, ry, rw, rh, 0, 0, rw, rh);
    face.texture.needsUpdate = true;
  }
}

function redrawAll() {
  for (const entry of entries) redrawBox(entry.box.id);
  requestRender();
}

function disposeScene() {
  if (!scene) return;
  // 整个模型挂在 worldGroup 下，切换模型时必须把旧组整体移出场景，
  // 否则旧网格会继续留在场景里造成重影
  if (worldGroup) {
    scene.remove(worldGroup);
    worldGroup = null;
  }
  entries.forEach((entry) => {
    entry.mesh.geometry.dispose();
    entry.faces.forEach((face) => face.texture.dispose());
    entry.materials.forEach((material) => material.dispose());
  });
  entries = [];
  meshMap = new Map();
  boxes = [];
  headGroup = null;
  armLeftGroup = null;
  armRightGroup = null;
  legLeftGroup = null;
  legRightGroup = null;
}

function buildScene() {
  if (!scene) return;
  disposeScene();
  boxes = buildBoxes(props.model === "slim");
  const byId = (id: string) => {
    const box = boxes.find((item) => item.id === id);
    if (!box) throw new Error(`缺少皮肤盒子：${id}`);
    return box;
  };
  const aw = props.model === "slim" ? 3 : 4;

  const world = new THREE.Group();
  world.position.y = -22;

  const head = new THREE.Group();
  head.position.y = 32;
  head.add(buildBoxEntry(byId("head"), 0));
  const hat = buildBoxEntry(byId("headOverlay"), 1);
  hat.position.y = 0.5;
  head.add(hat);

  const body = buildBoxEntry(byId("body"), 0);
  body.position.y = 24;
  const jacket = buildBoxEntry(byId("bodyOverlay"), 0.5);
  jacket.position.y = 24.25;

  const buildLimb = (baseId: string, overlayId: string, x: number, y: number) => {
    const group = new THREE.Group();
    group.position.set(x, y, 0);
    group.add(buildBoxEntry(byId(baseId), 0));
    const overlay = buildBoxEntry(byId(overlayId), 0.5);
    overlay.position.y = 0.25;
    group.add(overlay);
    return group;
  };

  const armLeft = buildLimb("armLeft", "armLeftOverlay", 4 + aw / 2, 24);
  const armRight = buildLimb("armRight", "armRightOverlay", -(4 + aw / 2), 24);
  const legLeft = buildLimb("legLeft", "legLeftOverlay", 2, 12);
  const legRight = buildLimb("legRight", "legRightOverlay", -2, 12);

  world.add(head, body, jacket, armLeft, armRight, legLeft, legRight);
  worldGroup = world;
  headGroup = head;
  armLeftGroup = armLeft;
  armRightGroup = armRight;
  legLeftGroup = legLeft;
  legRightGroup = legRight;
  scene.add(world);
  refreshPaintable();
  redrawAll();
}

function refreshPaintable() {
  for (const entry of entries) {
    const matchPart = props.part === "all" || entry.box.part === props.part;
    const active = matchPart && entry.box.layer === props.layer;
    // 全显时所有部位/图层都不透明；否则非活动部分按透视开关处理
    const solid = active || props.showAll;
    entry.mesh.visible = solid || props.ghost;
    for (const material of entry.materials) {
      material.opacity = solid ? 1 : 0.16;
      material.depthWrite = solid;
      material.transparent = true;
    }
  }
  requestRender();
}

/* ---------- 绘制 ---------- */

function markDirty(id: string) {
  dirty.add(id);
}

function flushTextures() {
  if (!dirty.size) return;
  for (const id of dirty) redrawBox(id);
  dirty.clear();
  requestRender();
}

function stamp(ctx: CanvasRenderingContext2D, box: BoxSpec, face: FaceName, x: number, y: number) {
  const [rx, ry, rw, rh] = faceRect(box, face);
  const size = Math.max(1, Math.min(8, Math.round(props.brushSize)));
  const offset = Math.floor(size / 2);
  const x0 = Math.max(rx, x - offset);
  const y0 = Math.max(ry, y - offset);
  const x1 = Math.min(rx + rw, x - offset + size);
  const y1 = Math.min(ry + rh, y - offset + size);
  if (x1 <= x0 || y1 <= y0) return;
  if (props.tool === "eraser") {
    ctx.clearRect(x0, y0, x1 - x0, y1 - y0);
  } else {
    ctx.fillStyle = props.color;
    ctx.fillRect(x0, y0, x1 - x0, y1 - y0);
  }
  markDirty(box.id);
}

interface Hit {
  box: BoxSpec;
  face: FaceName;
  x: number;
  y: number;
}

function hitAt(event: PointerEvent): Hit | null {
  if (!renderer || !camera) return null;
  const rect = renderer.domElement.getBoundingClientRect();
  if (!rect.width || !rect.height) return null;
  raycaster.setFromCamera(
    new THREE.Vector2(
      ((event.clientX - rect.left) / rect.width) * 2 - 1,
      -((event.clientY - rect.top) / rect.height) * 2 + 1
    ),
    camera
  );
  // 用全部盒子参与求交，才能判断“前面有没有别的部位挡住”
  const hits = raycaster.intersectObjects(
    entries.map((entry) => entry.mesh),
    false
  );
  for (const hit of hits) {
    if (!hit.uv || hit.faceIndex == null) continue;
    const entry = meshMap.get(hit.object);
    if (!entry) continue;
    const active =
      (props.part === "all" || entry.box.part === props.part) && entry.box.layer === props.layer;
    // 其它部位以半透明幽灵显示，只作参考，不参与拾取：
    // 否则选中手臂后，身体那层半透明仍会挡住射线，手臂就画不上去
    if (!active) continue;
    const face = FACE_BY_GROUP[Math.floor(hit.faceIndex / 2)];
    if (!face) continue;
    const texel = uvToTexel(entry.box, face, hit.uv.x, hit.uv.y);
    return { box: entry.box, face, x: texel.x, y: texel.y };
  }
  return null;
}

function paintAt(hit: Hit) {
  const ctx = docContext();
  if (!ctx) return;
  const points: [number, number][] = [];
  if (lastPoint && lastPoint.boxId === hit.box.id && lastPoint.face === hit.face) {
    points.push(...bresenham(lastPoint.x, lastPoint.y, hit.x, hit.y).slice(1));
  } else {
    points.push([hit.x, hit.y]);
  }
  for (const [x, y] of points) {
    stamp(ctx, hit.box, hit.face, x, y);
    if (!props.symmetry) continue;
    const mirror = mirrorTexel(boxes, hit.box, hit.face, x, y);
    if (mirror) stamp(ctx, mirror.box, mirror.face, mirror.x, mirror.y);
  }
  lastPoint = { boxId: hit.box.id, face: hit.face, x: hit.x, y: hit.y };
  flushTextures();
  emit("change");
}

function fillAt(hit: Hit) {
  const ctx = docContext();
  if (!ctx) return;
  const color = hexToRgba(props.color);
  const [rx, ry, rw, rh] = faceRect(hit.box, hit.face);
  floodFill(ctx, hit.x, hit.y, color, [rx, ry, rw, rh]);
  markDirty(hit.box.id);
  if (props.symmetry) {
    const mirror = mirrorTexel(boxes, hit.box, hit.face, hit.x, hit.y);
    if (mirror) {
      const [mx, my, mw, mh] = faceRect(mirror.box, mirror.face);
      floodFill(ctx, mirror.x, mirror.y, color, [mx, my, mw, mh]);
      markDirty(mirror.box.id);
    }
  }
  flushTextures();
  emit("change");
  emit("commit");
}

function pickAt(hit: Hit) {
  const ctx = docContext();
  if (!ctx) return;
  const pixel = ctx.getImageData(hit.x, hit.y, 1, 1).data;
  if (pixel[3] === 0) return;
  emit("pick", rgbaToHex(pixel[0], pixel[1], pixel[2]));
}

/* ---------- 相机与交互 ---------- */

function updateCamera() {
  if (!camera) return;
  const cos = Math.cos(orbit.phi);
  camera.position.set(
    Math.sin(orbit.theta) * cos * orbit.radius,
    Math.sin(orbit.phi) * orbit.radius,
    Math.cos(orbit.theta) * cos * orbit.radius
  );
  camera.lookAt(0, 0, 0);
  requestRender();
}

function requestRender() {
  if (frame) return;
  frame = requestAnimationFrame(() => {
    frame = 0;
    if (renderer && scene && camera) renderer.render(scene, camera);
  });
}

/* ---------- 行走动画 ---------- */

function resetPose() {
  for (const group of [armLeftGroup, armRightGroup, legLeftGroup, legRightGroup]) {
    if (group) group.rotation.x = 0;
  }
  if (headGroup) headGroup.rotation.y = 0;
  if (worldGroup) worldGroup.position.y = -22;
}

function startWalk() {
  if (walkFrame) return;
  if (!walkClock) walkClock = new THREE.Clock();
  walkClock.start();
  const tick = () => {
    walkFrame = requestAnimationFrame(tick);
    if (!props.walk || !renderer || !scene || !camera || !walkClock) return;
    const t = walkClock.getElapsedTime();
    const swing = Math.sin(t * 4);
    if (armLeftGroup) armLeftGroup.rotation.x = swing * 0.65;
    if (armRightGroup) armRightGroup.rotation.x = -swing * 0.65;
    if (legLeftGroup) legLeftGroup.rotation.x = -swing * 0.6;
    if (legRightGroup) legRightGroup.rotation.x = swing * 0.6;
    if (headGroup) headGroup.rotation.y = Math.sin(t * 1.3) * 0.08;
    // 摆动会让脚在透视下看起来更长，行走时整体上抬一点，避免踩出画面
    if (worldGroup) worldGroup.position.y = -19.8 + Math.abs(swing) * 0.55;
    renderer.render(scene, camera);
  };
  walkFrame = requestAnimationFrame(tick);
}

function stopWalk() {
  if (walkFrame) cancelAnimationFrame(walkFrame);
  walkFrame = 0;
  resetPose();
  requestRender();
}

function onPointerDown(event: PointerEvent) {
  const element = renderer?.domElement;
  if (!element) return;
  if (event.button === 2 || event.button === 1) {
    orbiting = true;
    lastScreen = { x: event.clientX, y: event.clientY };
    try {
      element.setPointerCapture(event.pointerId);
    } catch {
      /* 合成事件没有活动指针，捕获失败不影响拖拽 */
    }
    return;
  }
  if (event.button !== 0) return;
  const hit = hitAt(event);
  if (!hit) return;
  if (props.tool === "picker") {
    pickAt(hit);
    return;
  }
  if (props.tool === "fill") {
    fillAt(hit);
    return;
  }
  painting = true;
  activePointer = event.pointerId;
  lastPoint = null;
  // 先落笔再捕获指针：捕获失败（例如脚本合成的指针事件）也不影响这一笔
  paintAt(hit);
  try {
    element.setPointerCapture(event.pointerId);
  } catch {
    /* 没有活动指针时忽略 */
  }
}

function onPointerMove(event: PointerEvent) {
  if (orbiting) {
    orbit.theta -= (event.clientX - lastScreen.x) * 0.008;
    orbit.phi = Math.max(-1.05, Math.min(1.05, orbit.phi + (event.clientY - lastScreen.y) * 0.006));
    lastScreen = { x: event.clientX, y: event.clientY };
    updateCamera();
    return;
  }
  if (!painting || event.pointerId !== activePointer) return;
  const hit = hitAt(event);
  if (hit) paintAt(hit);
}

function onPointerUp(event: PointerEvent) {
  if (orbiting && renderer) {
    orbiting = false;
    try {
      renderer.domElement.releasePointerCapture(event.pointerId);
    } catch {
      /* 未捕获时忽略 */
    }
  }
  if (painting && event.pointerId === activePointer) {
    painting = false;
    lastPoint = null;
    emit("commit");
  }
}

function onWheel(event: WheelEvent) {
  event.preventDefault();
  orbit.radius = Math.max(34, Math.min(120, orbit.radius + event.deltaY * 0.05));
  updateCamera();
}

function onContextMenu(event: MouseEvent) {
  event.preventDefault();
}

onMounted(() => {
  const element = host.value;
  if (!element) return;
  scene = new THREE.Scene();
  camera = new THREE.PerspectiveCamera(32, 1, 0.1, 400);
  updateCamera();

  renderer = new THREE.WebGLRenderer({ alpha: true, antialias: true });
  renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
  renderer.outputColorSpace = THREE.SRGBColorSpace;
  renderer.domElement.style.width = "100%";
  renderer.domElement.style.height = "100%";
  renderer.domElement.style.touchAction = "none";
  renderer.domElement.style.cursor = "crosshair";
  element.appendChild(renderer.domElement);

  // 编辑皮肤要看清真实颜色：以均匀环境光为主，只留一点方向光做立体感
  scene.add(new THREE.HemisphereLight(0xfffaf2, 0x9d9488, 2.1));
  const key = new THREE.DirectionalLight(0xfff6e6, 0.55);
  key.position.set(14, 30, 26);
  scene.add(key);

  renderer.domElement.addEventListener("pointerdown", onPointerDown);
  renderer.domElement.addEventListener("pointermove", onPointerMove);
  renderer.domElement.addEventListener("pointerup", onPointerUp);
  renderer.domElement.addEventListener("pointercancel", onPointerUp);
  renderer.domElement.addEventListener("wheel", onWheel, { passive: false });
  renderer.domElement.addEventListener("contextmenu", onContextMenu);

  resizeObserver = new ResizeObserver(() => {
    if (!renderer || !camera || !element) return;
    const width = element.clientWidth;
    const height = element.clientHeight || props.height;
    if (!width || !height) return;
    renderer.setSize(width, height, false);
    camera.aspect = width / height;
    camera.updateProjectionMatrix();
    requestRender();
  });
  resizeObserver.observe(element);

  buildScene();
  if (props.walk) startWalk();
});

watch(
  () => [props.part, props.layer, props.ghost, props.showAll],
  () => refreshPaintable()
);

watch(
  () => props.walk,
  (on) => {
    if (on) startWalk();
    else stopWalk();
  }
);

watch(
  () => props.model,
  () => buildScene()
);

watch(
  () => props.version,
  () => redrawAll()
);

watch(
  () => props.source,
  () => redrawAll()
);

onBeforeUnmount(() => {
  if (frame) cancelAnimationFrame(frame);
  frame = 0;
  if (walkFrame) cancelAnimationFrame(walkFrame);
  walkFrame = 0;
  walkClock = null;
  resizeObserver?.disconnect();
  resizeObserver = null;
  if (renderer) {
    renderer.domElement.removeEventListener("pointerdown", onPointerDown);
    renderer.domElement.removeEventListener("pointermove", onPointerMove);
    renderer.domElement.removeEventListener("pointerup", onPointerUp);
    renderer.domElement.removeEventListener("pointercancel", onPointerUp);
    renderer.domElement.removeEventListener("wheel", onWheel);
    renderer.domElement.removeEventListener("contextmenu", onContextMenu);
    renderer.dispose();
    renderer.domElement.remove();
    renderer = null;
  }
  disposeScene();
  scene = null;
  camera = null;
});
</script>

<template>
  <div
    class="paint-stage"
    :style="height == null ? { height: '100%' } : { height: `${height}px` }"
  >
    <div ref="host" class="paint-host" />
    <div class="paint-hint">
      <span>左键绘制</span>
      <span>右键拖拽旋转</span>
      <span>滚轮缩放</span>
    </div>
  </div>
</template>

<style scoped>
.paint-stage {
  position: relative;
  width: 100%;
  border-radius: 2px;
  border: 1px solid var(--line-strong);
  background: radial-gradient(circle at 50% 34%, rgba(var(--ambient-a), 0.2), transparent 68%),
    linear-gradient(180deg, var(--panel-2), var(--panel));
  overflow: hidden;
}

.paint-host {
  position: absolute;
  inset: 0;
}

.paint-hint {
  position: absolute;
  left: 10px;
  bottom: 8px;
  display: flex;
  gap: 10px;
  padding: 4px 8px;
  border-radius: 2px;
  border: 1px solid var(--line);
  background: color-mix(in srgb, var(--panel) 82%, transparent);
  color: var(--muted);
  font-size: 10.5px;
  letter-spacing: 0.04em;
  pointer-events: none;
}
</style>
