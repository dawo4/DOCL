<script setup lang="ts">
import { computed } from "vue";

const props = withDefaults(
  defineProps<{
    modelValue: number;
    min?: number;
    max?: number;
    step?: number;
  }>(),
  { min: 0, max: 100, step: 1 }
);
const emit = defineEmits<{ (e: "update:modelValue", value: number): void }>();

const percent = computed(() => {
  const span = props.max - props.min || 1;
  return Math.min(100, Math.max(0, ((props.modelValue - props.min) / span) * 100));
});

function onInput(event: Event) {
  emit("update:modelValue", Number((event.target as HTMLInputElement).value));
}
</script>

<template>
  <input
    class="slider"
    type="range"
    :min="min"
    :max="max"
    :step="step"
    :value="modelValue"
    :style="{
      background: `linear-gradient(90deg, var(--moss) 0 ${percent}%, var(--paper-2) ${percent}% 100%)`,
    }"
    @input="onInput"
  />
</template>

<style scoped>
.slider {
  -webkit-appearance: none;
  appearance: none;
  width: 100%;
  height: 8px;
  border-radius: 2px;
  border: 1px solid var(--line);
  box-shadow: inset 0 1px 2px rgba(40, 28, 16, 0.14);
  outline: none;
  cursor: pointer;
}

/* 滑块做成带底部厚度的小按键，和按钮、开关保持一致 */
.slider::-webkit-slider-thumb {
  -webkit-appearance: none;
  appearance: none;
  width: 14px;
  height: 22px;
  border-radius: 2px;
  background: var(--panel);
  border-style: solid;
  border-width: 1px 1px 5px;
  border-color: color-mix(in srgb, var(--line-strong) 84%, var(--ink) 16%);
  border-bottom-color: color-mix(in srgb, var(--panel) 70%, #000);
  box-shadow: none;
  transition: transform var(--dur-1) var(--ease-out-expo);
}

.slider::-webkit-slider-thumb:hover {
  transform: translateY(-1px);
}

.slider::-moz-range-thumb {
  width: 14px;
  height: 22px;
  border-radius: 2px;
  background: var(--panel);
  border-style: solid;
  border-width: 1px 1px 5px;
  border-color: color-mix(in srgb, var(--line-strong) 84%, var(--ink) 16%);
  border-bottom-color: color-mix(in srgb, var(--panel) 70%, #000);
}
</style>
