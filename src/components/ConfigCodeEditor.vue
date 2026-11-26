<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from "vue";
import type { EditorView } from "@codemirror/view";

/// 配置文件的语法高亮编辑器。CodeMirror 相关的包全部按需加载，
/// 只有真正打开「配置文件」页并选中文件时才下载，不影响首屏体积。
const props = defineProps<{
  modelValue: string;
  /// 用来判断语言，例如 jei/jei.toml
  path: string;
  disabled?: boolean;
}>();
const emit = defineEmits<{
  (e: "update:modelValue", value: string): void;
  (e: "save"): void;
}>();

const host = ref<HTMLElement | null>(null);
const ready = ref(false);
const failed = ref(false);
let view: EditorView | null = null;
/// 可编辑状态放进独立的 compartment，切换实例或只读时热更新
let editable: import("@codemirror/state").Compartment | null = null;
/// EditorView 构造器，供 compartment 重新配置时使用
let EditorViewCtor: typeof EditorView | null = null;
/// 供外部按钮调用「打开搜索面板」
let openSearchPanelFn: (() => void) | null = null;
/// 组件可能在动态 import 完成前就被卸载，用标记避免创建孤儿编辑器
let disposed = false;
/// 防止 watch 回填时把程序化改动再抛回父组件
let syncing = false;

function extensionOf(path: string) {
  return path.split(".").pop()?.toLowerCase() ?? "";
}

/// 按扩展名挑语言包；认不出的按纯文本展示，不做高亮
async function languageFor(path: string) {
  const ext = extensionOf(path);
  if (ext === "json" || ext === "json5") {
    const { json } = await import("@codemirror/lang-json");
    return [json()];
  }
  if (ext === "toml") {
    const [{ StreamLanguage }, { toml }] = await Promise.all([
      import("@codemirror/language"),
      import("@codemirror/legacy-modes/mode/toml"),
    ]);
    return [StreamLanguage.define(toml)];
  }
  if (["properties", "cfg", "conf", "config", "ini"].includes(ext)) {
    const [{ StreamLanguage }, { properties }] = await Promise.all([
      import("@codemirror/language"),
      import("@codemirror/legacy-modes/mode/properties"),
    ]);
    return [StreamLanguage.define(properties)];
  }
  return [];
}

async function mount() {
  const el = host.value;
  if (!el) return;
  try {
    const [
      { EditorState, Compartment },
      { EditorView, keymap, lineNumbers, highlightActiveLine, drawSelection, highlightSpecialChars },
      { defaultKeymap, history, historyKeymap },
      {
        syntaxHighlighting,
        HighlightStyle,
        foldGutter,
        foldKeymap,
        bracketMatching,
        indentOnInput,
      },
      { tags },
      { search, openSearchPanel, highlightSelectionMatches },
      languageExtensions,
    ] = await Promise.all([
      import("@codemirror/state"),
      import("@codemirror/view"),
      import("@codemirror/commands"),
      import("@codemirror/language"),
      import("@lezer/highlight"),
      import("@codemirror/search"),
      languageFor(props.path),
    ]);

    // 可编辑状态单独放进 compartment，切文件时不用重建整个编辑器
    editable = new Compartment();
    EditorViewCtor = EditorView;
    openSearchPanelFn = () => {
      if (view) openSearchPanel(view);
    };
    // 动态加载期间组件可能已被卸载（快速切换文件），此时不再挂载
    if (disposed || !host.value) return;

    // 颜色全部走主题变量，不引入第二套配色
    const highlight = HighlightStyle.define([
      { tag: tags.propertyName, color: "var(--walnut)" },
      { tag: tags.definition(tags.propertyName), color: "var(--walnut)" },
      { tag: tags.string, color: "var(--moss)" },
      { tag: tags.number, color: "color-mix(in srgb, var(--amber) 62%, var(--walnut-deep))" },
      { tag: [tags.bool, tags.keyword], color: "var(--moss)", fontWeight: "600" },
      { tag: tags.comment, color: "var(--muted)", fontStyle: "italic" },
      { tag: tags.punctuation, color: "var(--ink-soft)" },
    ]);

    const theme = EditorView.theme({
      "&": {
        height: "100%",
        backgroundColor: "transparent",
        color: "var(--ink-soft)",
        fontSize: "11.5px",
      },
      ".cm-scroller": {
        fontFamily: "var(--font-mono)",
        lineHeight: "1.6",
      },
      ".cm-content": { padding: "10px 0" },
      ".cm-line": { padding: "0 12px" },
      ".cm-gutters": {
        backgroundColor: "transparent",
        border: "none",
        color: "var(--muted)",
        paddingLeft: "6px",
      },
      "&.cm-focused": { outline: "none" },
      ".cm-activeLine": {
        backgroundColor: "color-mix(in srgb, var(--moss) 7%, transparent)",
      },
      ".cm-activeLineGutter": { backgroundColor: "transparent", color: "var(--ink-soft)" },
      ".cm-foldGutter .cm-gutterElement": {
        color: "var(--muted)",
        cursor: "pointer",
        padding: "0 2px",
      },
      ".cm-foldPlaceholder": {
        backgroundColor: "var(--panel-2)",
        border: "1px solid var(--line)",
        color: "var(--muted)",
      },
      ".cm-selectionBackground, &.cm-focused .cm-selectionBackground, ::selection": {
        backgroundColor: "color-mix(in srgb, var(--moss) 22%, transparent)",
      },
      ".cm-cursor, .cm-dropCursor": { borderLeftColor: "var(--ink)" },
      /* 搜索面板：跟随主题，避免出现默认的白底工具栏 */
      ".cm-panel.cm-search": {
        padding: "7px 10px",
        backgroundColor: "var(--panel-2)",
        borderTop: "none",
        borderBottom: "1px solid var(--line)",
        fontFamily: "var(--font-ui)",
        fontSize: "11.5px",
        color: "var(--ink-soft)",
        display: "flex",
        flexWrap: "wrap",
        alignItems: "center",
        gap: "6px",
      },
      ".cm-panel.cm-search label": {
        fontSize: "11px",
        color: "var(--muted)",
        display: "inline-flex",
        alignItems: "center",
        gap: "3px",
      },
      ".cm-textfield": {
        height: "24px",
        padding: "0 7px",
        border: "1px solid var(--line-strong)",
        borderRadius: "2px",
        background: "var(--panel)",
        color: "var(--ink)",
        fontFamily: "var(--font-mono)",
        fontSize: "11.5px",
        outline: "none",
      },
      ".cm-button": {
        height: "24px",
        padding: "0 9px",
        border: "1px solid var(--line-strong)",
        borderRadius: "2px",
        background: "var(--panel)",
        backgroundImage: "none",
        color: "var(--ink-soft)",
        fontSize: "11.5px",
        cursor: "pointer",
      },
      ".cm-button:hover": {
        background: "var(--hover)",
        color: "var(--walnut-deep)",
      },
      ".cm-button:active": {
        boxShadow: "var(--shadow-press)",
      },
      ".cm-searchMatch": {
        backgroundColor: "color-mix(in srgb, var(--amber) 32%, transparent)",
        outline: "1px solid color-mix(in srgb, var(--amber) 58%, transparent)",
      },
      ".cm-searchMatch.cm-searchMatch-selected": {
        backgroundColor: "color-mix(in srgb, var(--moss) 34%, transparent)",
        outline: "1px solid color-mix(in srgb, var(--moss) 62%, transparent)",
      },
      ".cm-panel.cm-search [name=close]": {
        color: "var(--ink-soft)",
        fontSize: "15px",
        padding: "0 4px",
      },
    });

    view = new EditorView({
      parent: el,
      state: EditorState.create({
        doc: props.modelValue,
        extensions: [
          lineNumbers(),
          highlightSpecialChars(),
          drawSelection(),
          history(),
          highlightActiveLine(),
          bracketMatching(),
          indentOnInput(),
          foldGutter(),
          syntaxHighlighting(highlight),
          theme,
          search({ top: true }),
          highlightSelectionMatches(),
          // 搜索面板文案本地化
          EditorState.phrases.of({
            Find: "查找",
            Replace: "替换",
            next: "下一个",
            previous: "上一个",
            all: "全部",
            "match case": "区分大小写",
            "by word": "全词匹配",
            regexp: "正则",
            replace: "替换",
            "replace all": "全部替换",
            close: "关闭",
            "current match": "当前匹配",
            "on line": "行",
            "replaced match on line $": "已替换第 $ 行的匹配",
            "replaced $ matches": "已替换 $ 处匹配",
            "Go to line": "跳转到行",
            go: "跳转",
          }),
          EditorView.lineWrapping,
          editable.of(EditorView.editable.of(!props.disabled)),
          keymap.of([
            {
              key: "Mod-s",
              preventDefault: true,
              run: () => {
                emit("save");
                return true;
              },
            },
            ...defaultKeymap,
            ...historyKeymap,
            ...foldKeymap,
          ]),
          EditorView.updateListener.of((update) => {
            if (!update.docChanged || syncing) return;
            emit("update:modelValue", update.state.doc.toString());
          }),
          ...languageExtensions,
        ],
      }),
    });
    ready.value = true;
  } catch {
    // 加载失败时由父组件继续用 textarea，不阻断编辑
    failed.value = true;
  }
}

onMounted(mount);

onBeforeUnmount(() => {
  disposed = true;
  view?.destroy();
  view = null;
});

// 外部「重新读取」或切换文件时把内容推回编辑器
watch(
  () => props.modelValue,
  (value) => {
    if (!view) return;
    const current = view.state.doc.toString();
    if (current === value) return;
    syncing = true;
    view.dispatch({
      changes: { from: 0, to: current.length, insert: value },
    });
    syncing = false;
  }
);

watch(
  () => props.disabled,
  (disabled) => {
    if (!view || !editable || !EditorViewCtor) return;
    view.dispatch({
      effects: editable.reconfigure(EditorViewCtor.editable.of(!disabled)),
    });
  }
);

/// 高亮编辑器加载失败时的兜底输入
function onFallbackInput(event: Event) {
  emit("update:modelValue", (event.target as HTMLTextAreaElement).value);
}

defineExpose({
  focus: () => view?.focus(),
  openSearch: () => openSearchPanelFn?.(),
});
</script>

<template>
  <div class="code-editor">
    <div
      v-show="!failed"
      ref="host"
      class="code-editor__host"
      :class="{ 'code-editor__host--loading': !ready }"
    />
    <div v-if="!ready && !failed" class="loading-block code-editor__hint">
      <span class="loading-block__bar" />
      正在加载编辑器…
    </div>
    <template v-if="failed">
      <p class="code-editor__fallback muted small">
        高亮编辑器加载失败，已切换为纯文本模式，仍可编辑与保存。
      </p>
      <textarea
        class="code-editor__plain"
        spellcheck="false"
        :value="modelValue"
        :disabled="disabled"
        @input="onFallbackInput"
        @keydown.ctrl.s.prevent="emit('save')"
      />
    </template>
  </div>
</template>

<style scoped>
.code-editor {
  position: relative;
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-height: 0;
}

.code-editor__host {
  height: 480px;
  border: 1px solid var(--line-strong);
  background: var(--paper-2);
  overflow: auto;
}

.code-editor__host :deep(.cm-editor) {
  height: 100%;
}

.code-editor__host :deep(.cm-editor.cm-focused) {
  outline: none;
}

.code-editor__host--loading {
  visibility: hidden;
}

.code-editor__hint {
  position: absolute;
  inset: 0;
  justify-content: center;
}

.code-editor__plain {
  width: 100%;
  height: 480px;
  resize: vertical;
  padding: 10px 12px;
  border: 1px solid var(--line-strong);
  border-radius: 0;
  background: var(--paper-2);
  color: var(--ink-soft);
  font-family: var(--font-mono);
  font-size: 11.5px;
  line-height: 1.6;
  white-space: pre;
  overflow: auto;
  user-select: text;
}

.code-editor__fallback {
  margin: 0;
}
</style>
