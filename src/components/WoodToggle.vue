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
  border-radius: 99px;
}

.toggle__track {
  width: 46px;
  height: 26px;
  border-radius: 99px;
  /* 槽内部做成浅色内凹，和滚动条槽的观感一致 */
  background: rgba(122, 112, 100, 0.1);
  border: 1px solid rgba(122, 112, 100, 0.16);
  padding: 2px;
  display: flex;
  align-items: center;
  box-shadow: inset 0 1px 3px rgba(80, 58, 34, 0.1);
  transition: background var(--dur-2) var(--ease-in-out),
    border-color var(--dur-2) ease, box-shadow var(--dur-2) ease;
}

.toggle__knob {
  width: 20px;
  height: 20px;
  border-radius: 50%;
  background: linear-gradient(180deg, #fffdf6, #f6ebd7);
  box-shadow: 0 2px 5px rgba(80, 58, 34, 0.28), 0 1px 0 rgba(255, 255, 255, 0.8) inset;
  transition: transform var(--dur-2) var(--ease-out-expo);
}

.toggle--on .toggle__track {
  /* 开启态保持可辨识，但槽仍是浅色底而不是整块实心 */
  background: color-mix(in srgb, var(--moss) 26%, transparent);
  border-color: color-mix(in srgb, var(--moss) 55%, transparent);
  box-shadow: inset 0 1px 3px rgba(80, 58, 34, 0.12);
}

.toggle--on .toggle__knob {
  transform: translateX(20px);
}

.toggle:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}
</style>
