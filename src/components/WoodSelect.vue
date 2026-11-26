<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";

const props = defineProps<{
  modelValue: string;
  options: { value: string; label: string }[];
  disabled?: boolean;
}>();
const emit = defineEmits<{ (e: "update:modelValue", value: string): void }>();

const root = ref<HTMLElement | null>(null);
const trigger = ref<HTMLButtonElement | null>(null);
const menu = ref<HTMLElement | null>(null);
const open = ref(false);
const menuStyle = ref<Record<string, string>>({});
/// 键盘高亮项，-1 表示没有
const active = ref(-1);

const current = computed(
  () => props.options.find((o) => o.value === props.modelValue)?.label ?? props.modelValue
);

function pick(value: string) {
  emit("update:modelValue", value);
  closeMenu(true);
}

/// 按索引选中：options 可能在打开期间被外部替换，越界时直接忽略
function pickAt(index: number) {
  const option = props.options[index];
  if (!option) return;
  pick(option.value);
}

function positionMenu() {
  const el = trigger.value;
  if (!el) return;
  const rect = el.getBoundingClientRect();
  const menuWidth = menu.value?.offsetWidth || rect.width;
  const menuHeight = menu.value?.offsetHeight || 0;
  let left = rect.right - menuWidth;
  if (left < 8) left = Math.max(8, Math.min(rect.left, window.innerWidth - menuWidth - 8));
  let top = rect.bottom + 7;
  // 靠近窗口底部时向上展开，避免被裁掉
  if (menuHeight && top + menuHeight > window.innerHeight - 10) {
    top = Math.max(10, rect.top - menuHeight - 7);
  }
  menuStyle.value = {
    left: `${Math.round(left)}px`,
    top: `${Math.round(top)}px`,
    minWidth: `${Math.round(rect.width)}px`,
  };
}

/// 关闭时把焦点还给触发器，键盘用户可以继续 Tab
function closeMenu(refocus = false) {
  if (!open.value) return;
  open.value = false;
  active.value = -1;
  if (refocus) void nextTick(() => trigger.value?.focus());
}

/// 只有菜单外部的滚动才关闭，否则菜单自己滚动选项时会误关。
function onWindowScroll(event: Event) {
  const target = event.target;
  if (menu.value && target instanceof Node && menu.value.contains(target)) return;
  closeMenu();
}

function toggle() {
  if (props.disabled) return;
  if (open.value) {
    closeMenu(true);
    return;
  }
  openMenu();
}

function openMenu() {
  if (props.disabled) return;
  const index = props.options.findIndex((o) => o.value === props.modelValue);
  active.value = index >= 0 ? index : 0;
  const rect = trigger.value?.getBoundingClientRect();
  if (rect) {
    menuStyle.value = {
      left: `${Math.round(rect.right - rect.width)}px`,
      top: `${Math.round(rect.bottom + 7)}px`,
      minWidth: `${Math.round(rect.width)}px`,
    };
  }
  open.value = true;
}

function moveActive(step: number) {
  const total = props.options.length;
  if (!total) return;
  const start = active.value < 0 ? (step > 0 ? -1 : 0) : active.value;
  active.value = (start + step + total) % total;
}

function onTriggerKeydown(event: KeyboardEvent) {
  if (props.disabled) return;
  if (event.key === "ArrowDown" || event.key === "ArrowUp") {
    event.preventDefault();
    if (!open.value) {
      openMenu();
      return;
    }
    moveActive(event.key === "ArrowDown" ? 1 : -1);
    return;
  }
  if (event.key === "Enter" || event.key === " ") {
    event.preventDefault();
    if (!open.value) openMenu();
    else pickAt(active.value);
  }
}

function onMenuKeydown(event: KeyboardEvent) {
  switch (event.key) {
    case "ArrowDown":
      event.preventDefault();
      moveActive(1);
      return;
    case "ArrowUp":
      event.preventDefault();
      moveActive(-1);
      return;
    case "Home":
      event.preventDefault();
      active.value = 0;
      return;
    case "End":
      event.preventDefault();
      active.value = props.options.length - 1;
      return;
    case "Enter":
    case " ":
      event.preventDefault();
      pickAt(active.value);
      return;
    case "Escape":
      event.preventDefault();
      closeMenu(true);
      return;
    case "Tab":
      // 让焦点自然离开，菜单跟着收起
      closeMenu();
      return;
    default:
      break;
  }
  // 首字母跳转
  if (event.key.length === 1 && /\S/.test(event.key)) {
    const char = event.key.toLowerCase();
    const total = props.options.length;
    for (let offset = 1; offset <= total; offset += 1) {
      const index = (active.value + offset + total) % total;
      if (props.options[index].label.toLowerCase().startsWith(char)) {
        active.value = index;
        return;
      }
    }
  }
}

// 选项被外部替换（例如切换到另一个版本）后修正高亮项，避免越界
watch(
  () => props.options.length,
  (total) => {
    if (active.value >= total) active.value = total ? total - 1 : -1;
  }
);

function onDocClick(event: MouseEvent) {
  const target = event.target as Node;
  if (root.value?.contains(target) || menu.value?.contains(target)) return;
  closeMenu();
}

function onResize() {
  closeMenu();
}

watch(open, (value) => {
  if (value) {
    void nextTick(() => {
      positionMenu();
      menu.value?.focus();
    });
    window.addEventListener("scroll", onWindowScroll, true);
    window.addEventListener("resize", onResize);
  } else {
    window.removeEventListener("scroll", onWindowScroll, true);
    window.removeEventListener("resize", onResize);
  }
});

onMounted(() => document.addEventListener("mousedown", onDocClick));
onBeforeUnmount(() => {
  document.removeEventListener("mousedown", onDocClick);
  window.removeEventListener("scroll", onWindowScroll, true);
  window.removeEventListener("resize", onResize);
});
</script>

<template>
  <div ref="root" class="select" :class="{ 'select--open': open }">
    <button
      ref="trigger"
      class="select__value"
      :disabled="disabled"
      aria-haspopup="listbox"
      :aria-expanded="open"
      @click.stop="toggle"
      @keydown="onTriggerKeydown"
    >
      <span class="select__text">{{ current }}</span>
      <span class="select__chev" />
    </button>

    <Teleport to="body">
      <div
        v-if="open"
        ref="menu"
        class="select__menu"
        role="listbox"
        tabindex="-1"
        :style="menuStyle"
        @keydown="onMenuKeydown"
      >
        <button
          v-for="(option, index) in options"
          :key="option.value"
          class="select__option"
          :class="{
            'select__option--active': option.value === modelValue,
            'select__option--hl': index === active,
          }"
          role="option"
          :aria-selected="option.value === modelValue"
          @click.stop="pick(option.value)"
          @mouseenter="active = index"
        >
          <span class="select__dot" />
          {{ option.label }}
        </button>
      </div>
    </Teleport>
  </div>
</template>

<style scoped>
.select {
  position: relative;
  min-width: 160px;
}

.select__value {
  width: 100%;
  height: 38px;
  border-radius: 2px;
  border: 1px solid var(--line-strong);
  border-top-color: color-mix(in srgb, var(--line-strong) 45%, var(--ink));
  border-left-color: color-mix(in srgb, var(--line-strong) 45%, var(--ink));
  border-bottom-color: color-mix(in srgb, var(--line-strong) 58%, #fff);
  border-right-color: color-mix(in srgb, var(--line-strong) 58%, #fff);
  background: var(--panel);
  padding: 0 34px 0 13px;
  box-shadow: inset 0 1px 3px rgba(40, 28, 16, 0.14);
  text-align: left;
  position: relative;
  color: var(--ink);
  white-space: nowrap;
  transition: border-color var(--dur-2) ease, box-shadow var(--dur-2) ease;
}

.select__value:hover:not(:disabled) {
  border-top-color: color-mix(in srgb, var(--line-strong) 30%, var(--ink));
  border-left-color: color-mix(in srgb, var(--line-strong) 30%, var(--ink));
}

.select--open .select__value {
  border-top-color: color-mix(in srgb, var(--moss) 62%, #000);
  border-left-color: color-mix(in srgb, var(--moss) 62%, #000);
  border-right-color: color-mix(in srgb, var(--moss) 40%, #fff);
  border-bottom-color: color-mix(in srgb, var(--moss) 40%, #fff);
  box-shadow: inset 0 1px 3px rgba(40, 28, 16, 0.16),
    0 0 0 2px rgba(94, 123, 73, 0.18);
}

.select__value:disabled {
  opacity: var(--opacity-disabled);
  cursor: not-allowed;
}

.select__text {
  display: block;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.select__chev {
  position: absolute;
  right: 13px;
  top: 50%;
  width: 7px;
  height: 7px;
  border-right: 1.6px solid var(--ink-soft);
  border-bottom: 1.6px solid var(--ink-soft);
  transform: translateY(-70%) rotate(45deg);
  transition: transform var(--dur-2) var(--ease-out-expo);
}

.select--open .select__chev {
  transform: translateY(-30%) rotate(225deg);
}

.select__menu {
  position: fixed;
  z-index: 95;
  width: max-content;
  max-width: min(320px, calc(100vw - 16px));
  background: var(--surface-pop);
  /* 浮层和卡片同一种 2px 浮雕框，弹出时像一块实体板 */
  border: var(--edge-pop);
  border-top-color: var(--edge-pop-t);
  border-left-color: var(--edge-pop-l);
  border-right-color: var(--edge-pop-r);
  border-bottom-color: var(--edge-pop-b);
  border-radius: 2px;
  padding: 5px;
  box-shadow: var(--shadow-pop);
  max-height: 260px;
  overflow-y: auto;
  outline: none;
  animation: select-in var(--dur-1) var(--ease-out-expo);
}

.select__option {
  width: 100%;
  height: 34px;
  padding: 0 10px;
  border-radius: 3px;
  text-align: left;
  color: var(--ink-soft);
  white-space: nowrap;
  display: flex;
  align-items: center;
  gap: var(--space-3);
  transition: background var(--dur-1) ease, color var(--dur-1) ease;
}

.select__option:hover,
.select__option--hl {
  background: var(--panel-2);
  color: var(--walnut-deep);
}

.select__dot {
  width: 4px;
  height: 4px;
  border-radius: 50%;
  background: transparent;
  transition: background var(--dur-1) ease;
}

.select__option--active {
  background: var(--moss-soft);
  color: var(--moss);
  font-weight: 700;
  box-shadow: var(--shadow-inset-soft);
}

.select__option--active .select__dot {
  background: var(--moss);
}

@keyframes select-in {
  from {
    opacity: 0;
    transform: translateY(-5px) scale(0.99);
  }
}
</style>
