<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from "vue";
import type { Clock, Group, Material, Mesh, PerspectiveCamera, Scene, Texture, WebGLRenderer } from "three";
import { store } from "../store";
import { canvasFromSource, createDefaultSkin, seedFromString } from "../skin";

/// 运行时按需加载 three，避免它进入首屏包
type Three = typeof import("three");
let THREE: Three;

const props = withDefaults(
  defineProps<{
    skin?: string;
    seed?: string;
    height?: number;
  }>(),
  { skin: "", seed: "steve", height: 230 }
);

type Region = [number, number, number, number];

const HEAD: Record<string, Region> = {
  right: [0, 8, 8, 8],
  left: [16, 8, 8, 8],
  top: [8, 0, 8, 8],
  bottom: [16, 0, 8, 8],
  front: [8, 8, 8, 8],
  back: [24, 8, 8, 8],
};

const HAT: Record<string, Region> = {
  right: [32, 8, 8, 8],
  left: [48, 8, 8, 8],
  top: [40, 0, 8, 8],
  bottom: [48, 0, 8, 8],
  front: [40, 8, 8, 8],
  back: [56, 8, 8, 8],
};

const host = ref<HTMLDivElement | null>(null);
let renderer: WebGLRenderer | null = null;
let scene: Scene | null = null;
let camera: PerspectiveCamera | null = null;
let group: Group | null = null;
let clock: Clock | null = null;
let frame = 0;
let resizeObserver: ResizeObserver | null = null;
let buildToken = 0;
let disposed = false;
const textures: Texture[] = [];
const materials: Material[] = [];

function faceTexture(source: HTMLCanvasElement, region: Region) {
  const [x, y, w, h] = region;
  const canvas = document.createElement("canvas");
  canvas.width = w;
  canvas.height = h;
  const ctx = canvas.getContext("2d")!;
  ctx.imageSmoothingEnabled = false;
  ctx.drawImage(source, x, y, w, h, 0, 0, w, h);
  const texture = new THREE.CanvasTexture(canvas);
  texture.magFilter = THREE.NearestFilter;
  texture.minFilter = THREE.NearestFilter;
  texture.colorSpace = THREE.SRGBColorSpace;
  textures.push(texture);
  return texture;
}

function headMesh(
  source: HTMLCanvasElement,
  regions: Record<string, Region>,
  size: number,
  transparent: boolean
) {
  const order = [
    regions.right,
    regions.left,
    regions.top,
    regions.bottom,
    regions.front,
    regions.back,
  ];
  const mats = order.map((region) => {
    const material = new THREE.MeshStandardMaterial({
      map: faceTexture(source, region),
      transparent,
      alphaTest: usesAlpha(regions) ? 0.06 : 0.02,
      roughness: 0.92,
      metalness: 0,
      depthWrite: !transparent,
    });
    materials.push(material);
    return material;
  });
  const geometry = new THREE.BoxGeometry(size, size, size);
  return new THREE.Mesh(geometry, mats);
}

function usesAlpha(regions: Record<string, Region>) {
  return regions === HAT;
}

function clearScene() {
  if (!scene || !group) return;
  scene.remove(group);
  group.traverse((object) => {
    const mesh = object as Mesh;
    if (mesh.isMesh) mesh.geometry?.dispose();
  });
  materials.forEach((material) => material.dispose());
  textures.forEach((texture) => texture.dispose());
  materials.length = 0;
  textures.length = 0;
  group = null;
}

async function build() {
  if (!scene) return;
  const token = ++buildToken;
  let canvas: HTMLCanvasElement;
  if (props.skin) {
    try {
      canvas = await canvasFromSource(props.skin);
    } catch {
      canvas = createDefaultSkin(seedFromString(props.seed));
    }
  } else {
    canvas = createDefaultSkin(seedFromString(props.seed));
  }
  if (token !== buildToken) return;
  // 组件在 await 期间卸载时不再建场景
  if (disposed || !scene) return;

  clearScene();
  const root = new THREE.Group();
  const base = headMesh(canvas, HEAD, 8, false);
  const overlay = headMesh(canvas, HAT, 8.55, true);
  overlay.renderOrder = 2;
  base.renderOrder = 1;
  root.add(base, overlay);
  group = root;
  scene.add(root);
}

function animate() {
  frame = requestAnimationFrame(animate);
  if (!renderer || !scene || !camera || !clock || !group) return;
  const delta = clock.getDelta();
  const time = clock.getElapsedTime();
  const animated = store.config?.animations !== false;
  if (animated) {
    group.rotation.y += delta * 0.32;
    group.position.y = Math.sin(time * 1.35) * 0.85;
    group.rotation.z = Math.sin(time * 0.8) * 0.035;
  } else {
    group.rotation.y = 0.55;
    group.position.y = 0;
    group.rotation.z = 0;
  }
  renderer.render(scene, camera);
}

onMounted(async () => {
  const element = host.value;
  if (!element) return;
  THREE = await import("three");
  if (disposed || !host.value) return;
  scene = new THREE.Scene();
  camera = new THREE.PerspectiveCamera(30, 1, 0.1, 200);
  camera.position.set(0, 0, 27);
  camera.lookAt(0, 0, 0);

  renderer = new THREE.WebGLRenderer({ alpha: true, antialias: true });
  renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
  renderer.outputColorSpace = THREE.SRGBColorSpace;
  renderer.domElement.style.width = "100%";
  renderer.domElement.style.height = "100%";
  element.appendChild(renderer.domElement);

  scene.add(new THREE.HemisphereLight(0xfff5e0, 0x3b2b1c, 1.2));
  const key = new THREE.DirectionalLight(0xfff1d8, 1.55);
  key.position.set(12, 22, 20);
  scene.add(key);
  const fill = new THREE.DirectionalLight(0xa8c0d8, 0.5);
  fill.position.set(-16, 8, -14);
  scene.add(fill);

  clock = new THREE.Clock();
  resizeObserver = new ResizeObserver(() => {
    if (!renderer || !camera || !element) return;
    const width = element.clientWidth;
    const height = element.clientHeight || props.height;
    if (!width || !height) return;
    renderer.setSize(width, height, false);
    camera.aspect = width / height;
    camera.updateProjectionMatrix();
  });
  resizeObserver.observe(element);

  void build();
  animate();
});

watch(
  () => [props.skin, props.seed],
  () => {
    void build();
  }
);

onBeforeUnmount(() => {
  disposed = true;
  cancelAnimationFrame(frame);
  resizeObserver?.disconnect();
  if (renderer) {
    renderer.dispose();
    renderer.domElement.remove();
    renderer = null;
  }
  clearScene();
  scene = null;
  camera = null;
  clock = null;
});
</script>

<template>
  <div ref="host" class="skin-head" :style="{ height: `${height}px` }" />
</template>

<style scoped>
.skin-head {
  width: 100%;
  background: transparent;
  pointer-events: none;
}
</style>
