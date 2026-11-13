<script setup lang="ts">
import { ref, watch } from "vue";

const props = withDefaults(
  defineProps<{ modelValue: string; palette?: string[] }>(),
  {
    // 与皮肤编辑器同一套色板
    palette: () => [
      "#2b2118",
      "#4a3220",
      "#7a5230",
      "#c08a55",
      "#e3b48a",
      "#f0c9a2",
      "#9c6a52",
      "#a8543c",
      "#5e7b49",
      "#88a867",
      "#47623a",
      "#3f4a58",
      "#243a5e",
      "#c9a26a",
      "#f4ebdd",
      "#ffffff",
    ],
  }
);
const emit = defineEmits<{ "update:modelValue": [value: string] }>();

const svArea = ref<HTMLElement | null>(null);
const hueBar = ref<HTMLElement | null>(null);
const hue = ref(100);
const sat = ref(55);
const val = ref(48);
const hexDraft = ref(props.modelValue);
let svDragging = false;
let hueDragging = false;

function clampNumber(value: number, min: number, max: number) {
  return Math.min(max, Math.max(min, value));
}

function hexToRgb(hex: string) {
  const clean = hex.replace("#", "").trim();
  const full =
    clean.length === 3
      ? clean
          .split("")
          .map((ch) => ch + ch)
          .join("")
      : clean;
  if (full.length !== 6 || /[^0-9a-f]/i.test(full)) return null;
  const value = Number.parseInt(full, 16);
  return { r: (value >> 16) & 255, g: (value >> 8) & 255, b: value & 255 };
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
  return { h, s: max === 0 ? 0 : (delta / max) * 100, v: max * 100 };
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

function currentHex() {
  return hsvToHex(hue.value, sat.value, val.value);
}

function applyHsv() {
  emit("update:modelValue", currentHex());
}

/// 外部值变化时回填面板；自己产生的一致性变化不覆盖色相
watch(
  () => props.modelValue,
  (next) => {
    hexDraft.value = next;
    if (currentHex().toLowerCase() === next.toLowerCase()) return;
    const rgb = hexToRgb(next);
    if (!rgb) return;
    const hsv = rgbToHsv(rgb.r, rgb.g, rgb.b);
    hue.value = hsv.h;
    sat.value = hsv.s;
    val.value = hsv.v;
  },
  { immediate: true }
);

function pick(value: string) {
  emit("update:modelValue", value);
}

function commitHex() {
  const value = hexDraft.value.trim();
  const hex = value.startsWith("#") ? value : `#${value}`;
  if (/^#[0-9a-f]{6}$/i.test(hex)) {
    emit("update:modelValue", hex.toLowerCase());
  } else {
    hexDraft.value = props.modelValue;
  }
}

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
</script>

<template>
  <div class="picker">
    <div class="picker__head">
      <span class="picker__chip" :style="{ background: props.modelValue }" />
      <input
        v-model="hexDraft"
        class="picker__hex mono"
        maxlength="7"
        spellcheck="false"
        @change="commitHex"
        @keyup.enter="commitHex"
      />
    </div>

    <div class="picker__swatches">
      <button
        v-for="item in props.palette"
        :key="item"
        class="picker__swatch"
        :class="{ 'picker__swatch--on': (props.modelValue || '').toLowerCase() === item }"
        :style="{ background: item }"
        :title="item"
        @click="pick(item)"
      />
    </div>

    <div
      ref="svArea"
      class="picker__sv"
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
        class="picker__knob"
        :style="{ left: `${sat}%`, top: `${100 - val}%`, background: props.modelValue }"
      />
    </div>

    <div
      ref="hueBar"
      class="picker__hue"
      @pointerdown="onHueDown"
      @pointermove="onHueMove"
      @pointerup="onHueUp"
      @pointercancel="onHueUp"
    >
      <span
        class="picker__hue-knob"
        :style="{ left: `${(hue / 360) * 100}%`, background: `hsl(${hue}, 100%, 50%)` }"
      />
    </div>
  </div>
</template>

<style scoped>
.picker {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.picker__head {
  display: flex;
  align-items: center;
  gap: 8px;
}

.picker__chip {
  width: 30px;
  height: 30px;
  flex-shrink: 0;
  border-radius: 2px;
  border: 1px solid var(--line-strong);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.4), 0 1px 3px rgba(80, 58, 34, 0.2);
}

.picker__hex {
  flex: 1;
  min-width: 0;
  height: 30px;
  border-radius: 2px;
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

.picker__swatches {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
}

.picker__swatch {
  width: 22px;
  height: 22px;
  border-radius: 2px;
  border: 1px solid rgba(0, 0, 0, 0.12);
  transition: transform var(--dur-1) ease;
}

.picker__swatch:hover {
  transform: scale(1.12);
}

.picker__swatch--on {
  box-shadow: 0 0 0 2px var(--moss);
}

.picker__sv {
  position: relative;
  height: 104px;
  border-radius: 2px;
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

.picker__knob {
  position: absolute;
  width: 14px;
  height: 14px;
  margin: -7px 0 0 -7px;
  border-radius: 2px;
  border: 2px solid #fffdf6;
  box-shadow: 0 1px 4px rgba(0, 0, 0, 0.35);
  pointer-events: none;
}

.picker__hue {
  position: relative;
  height: 14px;
  border-radius: 2px;
  border: 1px solid var(--line-strong);
  background: linear-gradient(
    90deg,
    #ff0000,
    #ffff00 17%,
    #00ff00 33%,
    #00ffff 50%,
    #0000ff 67%,
    #ff00ff 83%,
    #ff0000
  );
  cursor: pointer;
  touch-action: none;
}

.picker__hue-knob {
  position: absolute;
  top: -3px;
  width: 8px;
  height: 18px;
  margin-left: -4px;
  border-radius: 2px;
  border: 2px solid #fffdf6;
  box-shadow: 0 1px 4px rgba(0, 0, 0, 0.35);
  pointer-events: none;
}
</style>
