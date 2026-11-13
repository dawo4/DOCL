<script setup lang="ts">
import { ref } from "vue";
import ColorPicker from "./ColorPicker.vue";
import WoodButton from "./WoodButton.vue";

const props = withDefaults(
  defineProps<{
    /// 空表示跟随主题
    modelValue: string | null;
    label: string;
    /// 跟随主题时色块展示的颜色，可传 var(--xxx)
    fallback?: string;
    hint?: string;
  }>(),
  { fallback: "var(--moss)", hint: "" }
);
const emit = defineEmits<{ "update:modelValue": [value: string | null] }>();

const open = ref(false);
const seed = ref("#5e7b49");

function isHex(value: string) {
  return /^#([0-9a-f]{3}|[0-9a-f]{6})$/i.test(value);
}

/// 取主题里当前的变量值作为取色器起始色
function resolveFallback() {
  const value = props.fallback.trim();
  if (value.startsWith("var(")) {
    const name = value.slice(4, -1).trim();
    const raw = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
    return isHex(raw) ? raw : "#5e7b49";
  }
  return isHex(value) ? value : "#5e7b49";
}

function toggle() {
  if (!open.value) seed.value = props.modelValue || resolveFallback();
  open.value = !open.value;
}

function update(value: string) {
  emit("update:modelValue", value);
}

function reset() {
  emit("update:modelValue", null);
  open.value = false;
}
</script>

<template>
  <div class="tone">
    <div class="tone__row">
      <span class="tone__label">{{ label }}</span>
      <button
        class="tone__chip"
        :style="{ background: modelValue || fallback }"
        :title="modelValue || '跟随主题'"
        @click="toggle"
      />
      <span class="tone__hex mono">{{ modelValue || "跟随主题" }}</span>
      <WoodButton variant="soft" size="sm" @click="toggle">
        {{ open ? "收起" : "调色" }}
      </WoodButton>
      <WoodButton variant="ghost" size="sm" :disabled="!modelValue" @click="reset">
        恢复
      </WoodButton>
    </div>
    <p v-if="hint" class="tone__hint">{{ hint }}</p>
    <div v-if="open" class="tone__pop">
      <ColorPicker
        :model-value="modelValue || seed"
        @update:model-value="update"
      />
    </div>
  </div>
</template>

<style scoped>
.tone {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 10px 12px;
  border-radius: 2px;
  border: 1px solid var(--line);
  background: var(--panel-2);
  box-shadow: inset 0 1px 2px rgba(70, 48, 26, 0.06);
}

.tone__row {
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 0;
}

.tone__label {
  flex-shrink: 0;
  font-size: 12.5px;
  font-weight: 700;
  color: var(--ink-soft);
}

/* 色块和按钮同源：2px 直角 + 左上亮边、右下暗边 */
.tone__chip {
  width: 28px;
  height: 28px;
  flex-shrink: 0;
  border-radius: 2px;
  border-style: solid;
  border-width: 1px 1px 3px;
  border-top-color: rgba(255, 255, 255, 0.55);
  border-left-color: rgba(255, 255, 255, 0.55);
  border-right-color: rgba(0, 0, 0, 0.35);
  border-bottom-color: rgba(0, 0, 0, 0.35);
  box-shadow: 0 2px 5px rgba(40, 28, 16, 0.18);
}

.tone__chip:hover {
  transform: translateY(-1px);
}

.tone__hex {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 11.5px;
  color: var(--muted);
}

.tone__hint {
  margin: 0;
  font-size: 11.5px;
  color: var(--muted);
}

/* 展开的调色盘：凹槽面板，和分段控件的底槽一致 */
.tone__pop {
  padding: 10px;
  border-radius: 2px;
  border: 1px solid var(--line);
  background: var(--paper-2);
  box-shadow: inset 0 2px 4px rgba(70, 48, 26, 0.08);
}
</style>
