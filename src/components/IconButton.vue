<script setup lang="ts">
import { onBeforeUnmount, ref } from "vue";
import AppIcon from "./AppIcon.vue";
import { store } from "../store";

const props = withDefaults(
  defineProps<{
    icon: string;
    label: string;
    variant?: "ghost" | "soft" | "danger" | "primary" | "danger-quiet";
    size?: number;
    disabled?: boolean;
    active?: boolean;
  }>(),
  { variant: "ghost", size: 34, disabled: false, active: false }
);
const emit = defineEmits<{ (e: "click", ev: MouseEvent): void }>();

const button = ref<HTMLButtonElement | null>(null);
const visible = ref(false);
const position = ref({ left: 0, top: 0 });
let timer: number | undefined;

function show() {
  if (props.disabled) return;
  if (timer) window.clearTimeout(timer);
  timer = window.setTimeout(() => {
    const element = button.value;
    if (!element) return;
    const rect = element.getBoundingClientRect();
    const width = Math.min(240, Math.max(60, props.label.length * 13 + 26));
    let left = rect.left + rect.width / 2 - width / 2;
    left = Math.max(8, Math.min(window.innerWidth - width - 8, left));
    const top = rect.top > 60 ? rect.top - 34 : rect.bottom + 8;
    position.value = { left, top };
    visible.value = true;
  }, 320);
}

function hide() {
  if (timer) window.clearTimeout(timer);
  visible.value = false;
}

onBeforeUnmount(() => {
  if (timer) window.clearTimeout(timer);
});
</script>

<template>
  <span class="icon-btn-wrap">
    <button
      ref="button"
      class="icon-btn"
      :class="[`icon-btn--${variant}`, { 'icon-btn--active': active }]"
      :style="{ width: `${size}px`, height: `${size}px` }"
      :disabled="disabled"
      :aria-label="label"
      @mouseenter="show"
      @mouseleave="hide"
      @focus="show"
      @blur="hide"
      @click="$emit('click', $event)"
    >
      <AppIcon :name="icon" :size="Math.round(size * 0.53)" />
    </button>

    <Teleport to="body">
      <Transition name="tip">
        <div
          v-if="visible"
          class="icon-tip"
          :class="{ 'icon-tip--instant': store.config && !store.config.animations }"
          :style="{ left: `${position.left}px`, top: `${position.top}px` }"
          role="tooltip"
        >
          {{ label }}
        </div>
      </Transition>
    </Teleport>
  </span>
</template>

<style scoped>
.icon-btn-wrap {
  display: inline-flex;
}

.icon-btn {
  --depth: 2px;
  --face: var(--panel-2);
  /* 浮雕边色：左上取亮、右下取暗，按下时翻转 */
  --bevel-hi: color-mix(in srgb, var(--face) 52%, #fff);
  /* 浅色正面只压暗一点，底边才不会黑得突兀 */
  --bevel-lo: color-mix(in srgb, var(--face) 72%, #000);
  display: grid;
  place-items: center;
  box-sizing: border-box;
  border-radius: 2px;
  border-style: solid;
  border-width: 1px 1px var(--depth);
  border-color: transparent;
  color: var(--ink-soft);
  background: transparent;
  transition: background 150ms ease, color 150ms ease,
    border-color 150ms ease, transform 100ms ease, box-shadow 100ms ease;
}

.icon-btn:hover:not(:disabled) {
  background: rgba(var(--accent-rgb), 0.07);
  color: var(--walnut-deep);
  transform: translateY(-1px);
  border-top-color: var(--bevel-hi);
  border-left-color: var(--bevel-hi);
  border-right-color: var(--bevel-lo);
  border-bottom-color: var(--bevel-lo);
}

.icon-btn:active:not(:disabled) {
  transform: translateY(calc(var(--depth) - 1px));
  background: rgba(var(--accent-rgb), 0.12);
  border-top-color: var(--bevel-lo);
  border-left-color: var(--bevel-lo);
  border-right-color: var(--bevel-hi);
  border-bottom-color: var(--bevel-hi);
  border-bottom-width: 1px;
  box-shadow: inset 0 2px 3px rgba(0, 0, 0, 0.24);
}

.icon-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

/* 有底色的变体常驻浮雕边 */
.icon-btn--soft {
  background: var(--face);
  border-top-color: var(--bevel-hi);
  border-left-color: var(--bevel-hi);
  border-right-color: var(--bevel-lo);
  border-bottom-color: var(--bevel-lo);
}

.icon-btn--soft:hover:not(:disabled) {
  --face: var(--hover);
  background: var(--face);
}

.icon-btn--danger {
  --face: var(--danger-soft);
  color: var(--danger);
  background: var(--face);
  border-top-color: color-mix(in srgb, var(--danger) 24%, #fff);
  border-left-color: color-mix(in srgb, var(--danger) 24%, #fff);
  border-right-color: color-mix(in srgb, var(--danger) 62%, #000);
  border-bottom-color: color-mix(in srgb, var(--danger) 62%, #000);
}

.icon-btn--danger:hover:not(:disabled) {
  --face: color-mix(in srgb, var(--danger-soft) 78%, var(--danger));
  background: var(--face);
  color: var(--danger);
}

/* 常显灰调、无框无底，悬停才转红——用于列表行内删除 */
.icon-btn--danger-quiet {
  color: var(--muted);
  background: transparent;
  border-color: transparent;
}

.icon-btn--danger-quiet:hover:not(:disabled) {
  color: var(--danger);
  background: rgba(190, 92, 70, 0.12);
  border-top-color: color-mix(in srgb, var(--danger) 24%, #fff);
  border-left-color: color-mix(in srgb, var(--danger) 24%, #fff);
  border-right-color: color-mix(in srgb, var(--danger) 62%, #000);
  border-bottom-color: color-mix(in srgb, var(--danger) 62%, #000);
}

.icon-btn--primary {
  --face: var(--btn-b);
  /* 深色主键保留明显的深色底边 */
  --bevel-lo: color-mix(in srgb, var(--btn-b) 42%, #000);
  color: var(--btn-fg);
  background: var(--face);
  border-top-color: var(--bevel-hi);
  border-left-color: var(--bevel-hi);
  border-right-color: var(--bevel-lo);
  border-bottom-color: var(--bevel-lo);
  text-shadow: 0 1px 0 rgba(0, 0, 0, 0.3);
}

.icon-btn--primary:hover:not(:disabled) {
  --face: var(--btn-a);
  color: var(--btn-fg);
  background: var(--face);
}

.icon-btn--active {
  --face: var(--moss-soft);
  color: var(--moss);
  background: var(--face);
  border-top-color: color-mix(in srgb, var(--moss) 36%, #fff);
  border-left-color: color-mix(in srgb, var(--moss) 36%, #fff);
  border-right-color: color-mix(in srgb, var(--moss) 72%, #000);
  border-bottom-color: color-mix(in srgb, var(--moss) 72%, #000);
}

.icon-tip {
  position: fixed;
  z-index: 120;
  padding: 5px 9px;
  border-radius: 2px;
  background: #3c2b1c;
  color: #fbf4e6;
  font-size: 11.5px;
  line-height: 1.4;
  white-space: nowrap;
  pointer-events: none;
  box-shadow: 0 6px 18px rgba(40, 28, 16, 0.28);
  border: 1px solid rgba(255, 253, 246, 0.08);
}

.icon-tip--instant {
  transition: none !important;
}

.tip-enter-active,
.tip-leave-active {
  transition: opacity 130ms ease, transform 130ms var(--ease-out-expo);
}

.tip-enter-from,
.tip-leave-to {
  opacity: 0;
  transform: translateY(3px);
}
</style>
