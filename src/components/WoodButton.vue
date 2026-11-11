<script setup lang="ts">
withDefaults(
  defineProps<{
    variant?: "primary" | "ghost" | "soft" | "danger";
    size?: "sm" | "md" | "lg";
    disabled?: boolean;
    block?: boolean;
    sheen?: boolean;
  }>(),
  { variant: "soft", size: "md", disabled: false, block: false, sheen: false }
);
const emit = defineEmits<{ (e: "click", ev: MouseEvent): void }>();

function onClick(event: MouseEvent) {
  emit("click", event);
}
</script>

<template>
  <button
    class="btn"
    :class="[
      `btn--${variant}`,
      `btn--${size}`,
      { 'btn--block': block, 'btn--sheen': sheen },
    ]"
    :disabled="disabled"
    @click="onClick"
  >
    <span class="btn__label"><slot /></span>
  </button>
</template>

<style scoped>
/* 参照官方启动器的「实体按键」：左上亮边 + 右下暗边构成浮雕，
   底部再留一条更深的色带当厚度；悬停整体上浮、按下边线翻转压进面板。 */
.btn {
  --btn-depth: 3px;
  /* 按钮正面色，亮边/暗边都由它派生，换主题时自动跟着变 */
  --face: var(--panel-2);
  --bevel-hi: color-mix(in srgb, var(--face) 52%, #fff);
  /* 浅色正面只压暗一点，底部色带才不会黑得突兀 */
  --bevel-lo: color-mix(in srgb, var(--face) 72%, #000);
  position: relative;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  box-sizing: border-box;
  border-radius: 2px;
  border-style: solid;
  border-width: 1px 1px var(--btn-depth);
  border-top-color: var(--bevel-hi);
  border-left-color: var(--bevel-hi);
  border-right-color: var(--bevel-lo);
  border-bottom-color: var(--bevel-lo);
  background: var(--face);
  color: var(--walnut);
  font-weight: 700;
  line-height: 1;
  letter-spacing: 0.02em;
  transition: background 150ms ease, border-color 150ms ease,
    transform 100ms ease, box-shadow 100ms ease, color 150ms ease;
}

.btn__label {
  position: relative;
  display: inline-flex;
  align-items: center;
  gap: 8px;
  white-space: nowrap;
}

/* 悬停像从面板上浮起来 */
.btn:hover:not(:disabled) {
  transform: translateY(-1px);
}

/* 按下：亮暗边翻转 + 底边收平 + 内阴影，读作「按进去了」 */
.btn:active:not(:disabled) {
  transform: translateY(calc(var(--btn-depth) - 1px));
  border-top-color: var(--bevel-lo);
  border-left-color: var(--bevel-lo);
  border-right-color: var(--bevel-hi);
  border-bottom-color: var(--bevel-hi);
  border-bottom-width: 1px;
  box-shadow: inset 0 2px 4px rgba(0, 0, 0, 0.26);
}

.btn:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}

.btn--sm {
  height: 30px;
  padding: 0 12px;
  font-size: 12.5px;
}

.btn--md {
  --btn-depth: 4px;
  height: 38px;
  padding: 0 18px;
  font-size: 13.5px;
}

.btn--lg {
  --btn-depth: 5px;
  height: 52px;
  padding: 0 30px;
  font-size: 16px;
}

.btn--block {
  width: 100%;
}

.btn--primary {
  --face: var(--btn-b);
  /* 深色主键保持明显的深色底边，和浅色键的柔和底边分开 */
  --bevel-lo: color-mix(in srgb, var(--btn-b) 42%, #000);
  color: var(--btn-fg);
  text-shadow: 0 1px 0 rgba(0, 0, 0, 0.34);
}

.btn--primary:hover:not(:disabled) {
  --face: var(--btn-a);
}

/* 幽灵键平时没有底色，只靠浮雕边线勾出轮廓 */
.btn--ghost {
  --face: var(--panel-2);
  background: transparent;
  color: var(--ink-soft);
  font-weight: 600;
}

.btn--ghost:hover:not(:disabled) {
  background: rgba(var(--accent-rgb), 0.07);
  color: var(--walnut-deep);
}

.btn--soft:hover:not(:disabled) {
  --face: var(--hover);
  color: var(--walnut-deep);
}

.btn--danger {
  --face: var(--danger-soft);
  color: var(--danger);
}

.btn--danger:hover:not(:disabled) {
  --face: color-mix(in srgb, var(--danger-soft) 78%, var(--danger));
}
</style>
