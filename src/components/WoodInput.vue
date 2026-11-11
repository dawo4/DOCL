<script setup lang="ts">
withDefaults(
  defineProps<{
    modelValue: string;
    placeholder?: string;
    type?: string;
    disabled?: boolean;
    mono?: boolean;
  }>(),
  { placeholder: "", type: "text", disabled: false, mono: false }
);
defineEmits<{ (e: "update:modelValue", value: string): void }>();
</script>

<template>
  <input
    class="input"
    :class="{ mono }"
    :type="type"
    :value="modelValue"
    :placeholder="placeholder"
    :disabled="disabled"
    @input="$emit('update:modelValue', ($event.target as HTMLInputElement).value)"
  />
</template>

<style scoped>
/* 输入框是凹进去的：上/左暗边 + 下/右亮边，配合内阴影 */
.input {
  width: 100%;
  height: 38px;
  border-radius: 2px;
  border: 1px solid var(--line-strong);
  border-top-color: color-mix(in srgb, var(--line-strong) 45%, var(--ink));
  border-left-color: color-mix(in srgb, var(--line-strong) 45%, var(--ink));
  border-bottom-color: color-mix(in srgb, var(--line-strong) 58%, #fff);
  border-right-color: color-mix(in srgb, var(--line-strong) 58%, #fff);
  background: var(--panel);
  padding: 0 13px;
  transition: border-color var(--dur-2) ease, box-shadow var(--dur-2) ease,
    background var(--dur-2) ease;
  color: var(--ink);
  box-shadow: inset 0 1px 3px rgba(40, 28, 16, 0.16);
}

.input::placeholder {
  color: #b7a68f;
}

.input:hover:not(:disabled) {
  border-top-color: color-mix(in srgb, var(--line-strong) 30%, var(--ink));
  border-left-color: color-mix(in srgb, var(--line-strong) 30%, var(--ink));
}

.input:focus {
  outline: none;
  border-top-color: color-mix(in srgb, var(--moss) 62%, #000);
  border-left-color: color-mix(in srgb, var(--moss) 62%, #000);
  border-right-color: color-mix(in srgb, var(--moss) 40%, #fff);
  border-bottom-color: color-mix(in srgb, var(--moss) 40%, #fff);
  background: var(--panel);
  box-shadow: inset 0 1px 3px rgba(40, 28, 16, 0.16),
    0 0 0 2px rgba(94, 123, 73, 0.18);
}

.input:disabled {
  opacity: 0.55;
}

.input.mono {
  font-family: var(--font-mono);
  font-size: 12.5px;
}
</style>
