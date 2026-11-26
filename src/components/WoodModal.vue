<script setup lang="ts">
import { onBeforeUnmount, onMounted } from "vue";
import {
  DialogContent,
  DialogOverlay,
  DialogPortal,
  DialogRoot,
  DialogTitle,
} from "reka-ui";
import AppIcon from "./AppIcon.vue";

withDefaults(defineProps<{ title: string; width?: number; zIndex?: number }>(), {
  width: 460,
  zIndex: 80,
});
const emit = defineEmits<{ (e: "close"): void }>();

/// 弹窗叠加时按引用计数锁定页面滚动，最后一个关闭后恢复。
/// 真正滚动的是 .main，所以用 html 上的标记交给 CSS 处理。
let scrollLocks = 0;
function lockScroll() {
  scrollLocks += 1;
  document.documentElement.classList.add("modal-open");
}
function unlockScroll() {
  scrollLocks = Math.max(0, scrollLocks - 1);
  if (scrollLocks === 0) document.documentElement.classList.remove("modal-open");
}

onMounted(lockScroll);
onBeforeUnmount(unlockScroll);

function onOpenChange(open: boolean) {
  if (!open) emit("close");
}
</script>

<template>
  <DialogRoot :open="true" @update:open="onOpenChange">
    <DialogPortal>
      <DialogOverlay class="overlay" :style="{ zIndex }" />
      <DialogContent
        class="modal"
        :style="{ width: `${width}px`, zIndex: zIndex + 1 }"
        :aria-describedby="undefined"
      >
        <span class="modal__accent" />
        <header class="modal__head">
          <div>
            <DialogTitle class="modal__title">{{ title }}</DialogTitle>
          </div>
          <button class="modal__close" aria-label="关闭" @click="emit('close')">
            <AppIcon name="close" :size="14" />
          </button>
        </header>
        <div class="modal__body"><slot /></div>
        <footer v-if="$slots.footer" class="modal__foot"><slot name="footer" /></footer>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>

<style scoped>
.overlay {
  position: fixed;
  inset: 0;
  background: rgba(52, 38, 24, 0.32);
  backdrop-filter: blur(3px);
  display: grid;
  place-items: center;
  z-index: 80;
  animation: fade-in var(--dur-1) ease both;
}

.modal {
  position: fixed;
  left: 50%;
  top: 50%;
  transform: translate(-50%, -50%);
  background: var(--surface-pop);
  border: var(--edge-pop);
  /* 和卡片一致：左上亮、右下暗的浮雕边框 */
  border-top-color: var(--edge-pop-t);
  border-left-color: var(--edge-pop-l);
  border-right-color: var(--edge-pop-r);
  border-bottom-color: var(--edge-pop-b);
  border-radius: 2px;
  box-shadow: var(--shadow-pop);
  max-width: 88vw;
  max-height: 84vh;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  animation: pop-in var(--dur-3) var(--ease-out-expo) both;
}

.modal__accent {
  position: absolute;
  inset: 0 0 auto;
  height: 2px;
  background: var(--moss);
}

.modal__head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  padding: 20px 20px 12px;
}

.modal__title {
  font-size: 18px;
  font-weight: 800;
  color: var(--walnut-deep);
  margin-top: 2px;
}

.modal__close {
  width: 30px;
  height: 30px;
  border-radius: 2px;
  display: grid;
  place-items: center;
  color: var(--ink-soft);
  border: 1px solid transparent;
  transition: background var(--dur-1) ease, border-color var(--dur-1) ease,
    transform var(--dur-1) ease;
}

.modal__close:hover {
  color: var(--ink);
  background: var(--panel-2);
  border-style: solid;
  border-width: 1px 1px 2px;
  border-top-color: color-mix(in srgb, var(--frame) 60%, #fff);
  border-left-color: color-mix(in srgb, var(--frame) 60%, #fff);
  border-right-color: color-mix(in srgb, var(--frame) 70%, #000);
  border-bottom-color: color-mix(in srgb, var(--frame) 70%, #000);
  transform: translateY(-1px);
}

.modal__close:active {
  transform: translateY(1px);
  border-top-color: color-mix(in srgb, var(--frame) 70%, #000);
  border-left-color: color-mix(in srgb, var(--frame) 70%, #000);
  border-right-color: color-mix(in srgb, var(--frame) 60%, #fff);
  border-bottom-width: 1px;
  border-bottom-color: color-mix(in srgb, var(--frame) 60%, #fff);
  box-shadow: var(--shadow-press);
}

.modal__body {
  padding: 8px 20px 20px;
  overflow-y: auto;
}

.modal__foot {
  padding: 13px 20px 17px;
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: flex-end;
  gap: 8px 10px;
  border-top: 1px solid var(--line);
  background: var(--panel-2);
}

@keyframes fade-in {
  from {
    opacity: 0;
  }
}

@keyframes pop-in {
  from {
    opacity: 0;
    transform: translate(-50%, calc(-50% + 16px)) scale(0.97);
  }
}
</style>
