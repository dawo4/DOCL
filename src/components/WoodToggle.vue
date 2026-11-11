<script setup lang="ts">
defineProps<{ modelValue: boolean; disabled?: boolean }>();
defineEmits<{ (e: "update:modelValue", value: boolean): void }>();
</script>

<template>
  <button
    class="toggle"
    :class="{ 'toggle--on': modelValue }"
    role="switch"
    :aria-checked="modelValue"
    :disabled="disabled"
    @click="$emit('update:modelValue', !modelValue)"
  >
    <span class="toggle__track">
      <span class="toggle__knob" />
    </span>
  </button>
</template>

<style scoped>
.toggle {
  display: inline-flex;
  padding: 0;
  border-radius: 2px;
}

/* 槽做成内凹的方槽，滑块是一个带底部厚度的小按键，和按钮同一套语言 */
.toggle__track {
  width: 46px;
  height: 24px;
  border-radius: 2px;
  background: var(--paper-2);
  border: 1px solid var(--line-strong);
  padding: 2px;
  display: flex;
  align-items: center;
  box-shadow: inset 0 1px 2px rgba(40, 28, 16, 0.14);
  transition: background var(--dur-2) var(--ease-in-out),
    border-color var(--dur-2) ease, box-shadow var(--dur-2) ease;
}

.toggle__knob {
  width: 18px;
  height: 18px;
  border-radius: 1px;
  background: var(--panel);
  border-style: solid;
  border-width: 1px 1px 3px;
  border-color: color-mix(in srgb, var(--line-strong) 84%, var(--ink) 16%);
  border-bottom-color: color-mix(in srgb, var(--panel) 74%, #000);
  transition: transform var(--dur-2) var(--ease-out-expo);
}

.toggle--on .toggle__track {
  background: color-mix(in srgb, var(--moss) 22%, transparent);
  border-color: var(--moss);
  box-shadow: inset 0 1px 2px rgba(40, 28, 16, 0.1);
}

.toggle--on .toggle__knob {
  transform: translateX(20px);
  background: color-mix(in srgb, var(--moss) 14%, var(--panel));
}

.toggle:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}
</style>
