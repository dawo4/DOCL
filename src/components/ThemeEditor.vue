<script setup lang="ts">
import { computed, reactive, ref } from "vue";
import type { CustomTheme } from "../api";
import ColorPicker from "./ColorPicker.vue";
import WoodButton from "./WoodButton.vue";
import WoodInput from "./WoodInput.vue";
import WoodModal from "./WoodModal.vue";

const props = defineProps<{ theme: CustomTheme; editing: boolean }>();
const emit = defineEmits<{
  save: [theme: CustomTheme];
  close: [];
}>();

type ColorKey =
  | "paper"
  | "panel"
  | "primary"
  | "accent"
  | "title"
  | "text"
  | "muted"
  | "line";

const slots: { key: ColorKey; label: string; hint: string }[] = [
  { key: "paper", label: "页面底色", hint: "窗口与页面的大面积底色" },
  { key: "panel", label: "卡片底色", hint: "卡片、弹窗、输入框的底色" },
  { key: "primary", label: "主色", hint: "实体按钮、链接与主描边" },
  { key: "accent", label: "强调色", hint: "选中态、进度条、分区标记" },
  { key: "title", label: "标题文字", hint: "页面标题与卡片标题" },
  { key: "text", label: "正文文字", hint: "正文与常规内容" },
  { key: "muted", label: "次要文字", hint: "说明、提示与弱化文字" },
  { key: "line", label: "边框线", hint: "分隔线与卡片描边" },
];

const draft = reactive<CustomTheme>({ ...props.theme });
const openKey = ref<ColorKey | null>(null);

/// 和 applyCustomTheme 的判定一致：浅色主色上按钮用深色字
const buttonText = computed(() => {
  const clean = draft.primary.replace("#", "");
  if (!/^[0-9a-f]{6}$/i.test(clean)) return "#fff8ea";
  const num = Number.parseInt(clean, 16);
  const r = (num >> 16) & 255;
  const g = (num >> 8) & 255;
  const b = num & 255;
  return 0.2126 * r + 0.7152 * g + 0.0722 * b > 150 ? "#2b2118" : "#fff8ea";
});

function toggle(key: ColorKey) {
  openKey.value = openKey.value === key ? null : key;
}

function save() {
  emit("save", { ...draft });
}
</script>

<template>
  <WoodModal
    :title="editing ? '编辑主题' : '添加主题'"
    :width="560"
    :z-index="95"
    @close="emit('close')"
  >
    <div class="editor">
      <div class="editor__preview" :style="{
        '--pv-paper': draft.paper,
        '--pv-panel': draft.panel,
        '--pv-primary': draft.primary,
        '--pv-accent': draft.accent,
        '--pv-title': draft.title,
        '--pv-text': draft.text,
        '--pv-muted': draft.muted,
        '--pv-line': draft.line,
      }">
        <span class="editor__preview-side">
          <i />
          <i />
          <i />
        </span>
        <span class="editor__preview-main">
          <strong>Aa 主题预览</strong>
          <span class="editor__preview-text">正文文字效果</span>
          <span class="editor__preview-row">
            <span class="editor__preview-btn" :style="{ color: buttonText }">按钮</span>
            <span class="editor__preview-tag">选中</span>
          </span>
        </span>
      </div>

      <label class="editor__name">
        <span class="editor__name-label">主题名称</span>
        <WoodInput v-model="draft.name" placeholder="例如：深海夜航" />
      </label>

      <div class="editor__slots">
        <div
          v-for="slot in slots"
          :key="slot.key"
          class="slot"
          :class="{ 'slot--open': openKey === slot.key }"
        >
          <button class="slot__row" @click="toggle(slot.key)">
            <span class="slot__label">{{ slot.label }}</span>
            <span class="slot__chip" :style="{ background: draft[slot.key] }" />
            <span class="slot__hex mono">{{ draft[slot.key] }}</span>
            <span class="slot__hint">{{ slot.hint }}</span>
          </button>
          <div v-if="openKey === slot.key" class="slot__pop">
            <ColorPicker
              :model-value="draft[slot.key]"
              @update:model-value="draft[slot.key] = $event"
            />
          </div>
        </div>
      </div>

      <p class="editor__note">
        保存后会立即应用并写入设置；其余颜色会按这几个关键色自动派生。
      </p>
    </div>

    <template #footer>
      <WoodButton variant="ghost" size="sm" @click="emit('close')">取消</WoodButton>
      <WoodButton variant="primary" size="sm" @click="save">
        {{ editing ? "保存修改" : "保存主题" }}
      </WoodButton>
    </template>
  </WoodModal>
</template>

<style scoped>
.editor {
  display: flex;
  flex-direction: column;
  gap: 14px;
}

/* 预览：直接把草稿色套进一块迷你界面 */
.editor__preview {
  display: flex;
  height: 92px;
  border-radius: 2px;
  overflow: hidden;
  background: var(--pv-paper);
  border: 1px solid var(--pv-line);
}

.editor__preview-side {
  display: flex;
  flex-direction: column;
  gap: 6px;
  width: 26%;
  padding: 10px 8px;
  background: color-mix(in srgb, var(--pv-panel) 86%, var(--pv-paper));
  border-right: 1px solid var(--pv-line);
}

.editor__preview-side i {
  height: 6px;
  border-radius: 1px;
  background: color-mix(in srgb, var(--pv-text) 26%, transparent);
}

.editor__preview-side i:first-child {
  background: var(--pv-accent);
}

.editor__preview-main {
  flex: 1;
  display: flex;
  flex-direction: column;
  justify-content: center;
  gap: 7px;
  padding: 0 14px;
  background: var(--pv-panel);
}

.editor__preview-main strong {
  color: var(--pv-title);
  font-size: 15px;
  line-height: 1.2;
}

.editor__preview-text {
  color: var(--pv-text);
  font-size: 11.5px;
}

.editor__preview-row {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 2px;
}

.editor__preview-btn {
  padding: 3px 10px;
  border-radius: 2px;
  background: var(--pv-primary);
  font-size: 10.5px;
  font-weight: 700;
  border-bottom: 2px solid color-mix(in srgb, var(--pv-primary) 60%, #000);
}

.editor__preview-tag {
  padding: 2px 8px;
  border-radius: 2px;
  border: 1px solid var(--pv-accent);
  color: var(--pv-accent);
  font-size: 10.5px;
  font-weight: 700;
}

.editor__name {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.editor__name-label {
  font-size: 12.5px;
  font-weight: 700;
  color: var(--ink-soft);
}

.editor__slots {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

/* 每个色槽一行：点击整行展开调色盘 */
.slot {
  border-radius: 2px;
  border: 1px solid var(--line);
  background: var(--panel-2);
  overflow: hidden;
}

.slot--open {
  border-color: var(--moss);
}

.slot__row {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  padding: 7px 10px;
  text-align: left;
}

.slot__label {
  width: 64px;
  flex-shrink: 0;
  font-size: 12px;
  font-weight: 700;
  color: var(--ink-soft);
}

.slot__chip {
  width: 24px;
  height: 24px;
  flex-shrink: 0;
  border-radius: 2px;
  border-style: solid;
  border-width: 1px 1px 2px;
  border-top-color: rgba(255, 255, 255, 0.55);
  border-left-color: rgba(255, 255, 255, 0.55);
  border-right-color: rgba(0, 0, 0, 0.35);
  border-bottom-color: rgba(0, 0, 0, 0.35);
}

.slot__hex {
  width: 70px;
  flex-shrink: 0;
  font-size: 11px;
  color: var(--muted);
}

.slot__hint {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 11px;
  color: var(--muted);
}

.slot__pop {
  padding: 10px;
  border-top: 1px dashed var(--line);
  background: var(--paper-2);
}

.editor__note {
  margin: 0;
  font-size: 11.5px;
  color: var(--muted);
}
</style>
