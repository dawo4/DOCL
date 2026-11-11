<script setup lang="ts">
import { computed } from "vue";

const props = withDefaults(
  defineProps<{ value: number; indeterminate?: boolean; height?: number }>(),
  { indeterminate: false, height: 7 }
);
const width = computed(() => (props.indeterminate ? 38 : Math.min(100, Math.max(0, props.value))));
</script>

<template>
  <div class="bar" :style="{ height: `${height}px` }">
    <div
      class="bar__fill"
      :class="{ 'bar__fill--indeterminate': indeterminate }"
      :style="{ width: `${width}%` }"
    />
  </div>
</template>

<style scoped>
/* 与按钮同一套「实体」语言：直角、描边、内凹槽，填充色块自带底部暗边 */
.bar {
  width: 100%;
  border-radius: 2px;
  border: 1px solid var(--line);
  background: var(--paper-2);
  overflow: hidden;
  box-shadow: inset 0 1px 2px rgba(40, 28, 16, 0.14);
}

.bar__fill {
  height: 100%;
  border-radius: 1px;
  background: var(--moss);
  box-shadow: inset 0 -2px 0 rgba(0, 0, 0, 0.18);
  transition: width 260ms var(--ease-out-quart);
}

.bar__fill--indeterminate {
  animation: slide 1.4s var(--ease-in-out) infinite;
}

@keyframes slide {
  0% {
    margin-left: -40%;
  }
  100% {
    margin-left: 100%;
  }
}
</style>
