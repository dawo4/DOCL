<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import {
  api,
  openExternal,
  openFolder,
  revealFile,
  type CrashMatch,
  type CrashReport,
} from "../api";
import { notify, openInstance, repairInstance, store } from "../store";
import AppIcon from "./AppIcon.vue";
import WoodButton from "./WoodButton.vue";
import WoodModal from "./WoodModal.vue";

const report = ref<CrashReport | null>(null);
const loading = ref(false);
const exporting = ref(false);
const repairing = ref(false);
const error = ref("");

/// 按优先级分组：高优先级是「最可能的原因」，其余作为补充线索
const matchGroups = computed(() => {
  const list = report.value?.matches ?? [];
  const groups: { title: string; items: CrashMatch[] }[] = [];
  const primary = list.filter((item) => item.priority >= 70);
  const others = list.filter((item) => item.priority < 70);
  if (primary.length) groups.push({ title: "最可能的原因", items: primary });
  if (others.length) {
    groups.push({
      title: primary.length ? "其他线索" : "检测到的线索",
      items: others,
    });
  }
  return groups;
});

const sourceLabel = computed(() => {
  switch (report.value?.source_kind) {
    case "crash-report":
      return "崩溃报告";
    case "hs-err":
      return "JVM 崩溃日志";
    case "latest-log":
      return "latest.log";
    case "launcher-log":
      return "启动器启动日志";
    default:
      return "无日志";
  }
});

const sourceTime = computed(() => {
  const value = report.value?.modified ?? 0;
  if (!value) return "";
  return new Date(value * 1000).toLocaleString("zh-CN", { hour12: false });
});

const exitLabel = computed(() => {
  const code = store.crashExitCode;
  return code === null ? "最近一次运行" : `退出代码 ${code}`;
});

function close() {
  store.crashOpen = false;
}

async function load() {
  const versionId = store.crashVersionId;
  if (!versionId) {
    error.value = "没有指定要分析的版本";
    return;
  }
  loading.value = true;
  error.value = "";
  try {
    report.value = await api.crashAnalyze(versionId);
  } catch (err) {
    report.value = null;
    error.value = String(err);
  } finally {
    loading.value = false;
  }
}

function openLogDir() {
  const path = report.value?.source_path;
  if (!path) return;
  const dir = path.replace(/[\\/][^\\/]+$/, "");
  if (dir) void openFolder(dir);
}

/// 导出分析结果 + 日志 ZIP，方便发到社区
async function exportReport() {
  const versionId = store.crashVersionId;
  if (!versionId || exporting.value) return;
  exporting.value = true;
  try {
    const path = await api.crashExport(versionId);
    const name = path.split(/[\\/]/).pop() ?? path;
    notify(`已导出崩溃报告：${name}`);
    await revealFile(path);
  } catch (err) {
    notify(`导出失败：${String(err)}`, "err");
  } finally {
    exporting.value = false;
  }
}

/// 一键修复：修清单、补文件、禁用重复与已知不兼容的模组
async function runRepair() {
  const versionId = store.crashVersionId;
  if (!versionId || repairing.value) return;
  repairing.value = true;
  try {
    const result = await repairInstance(versionId);
    if (!result) return;
    close();
    const parts: string[] = [];
    if (result.version_json) parts.push("修复版本清单");
    if (result.files_restored) parts.push(`补回 ${result.files_restored} 个文件`);
    if (result.natives) parts.push("重解本地库");
    if (result.disabled_mods.length)
      parts.push(`禁用 ${result.disabled_mods.length} 个重复模组`);
    if (result.incompatible_mods.length)
      parts.push(`禁用 ${result.incompatible_mods.length} 个不兼容模组`);
    const pending = result.issues.length + result.files_failed.length;
    if (parts.length) {
      notify(
        `一键修复完成：${parts.join("、")}${
          pending ? `，仍有 ${pending} 项需要手动处理` : ""
        }`
      );
    } else if (pending) {
      notify(`一键修复未能解决全部问题，仍有 ${pending} 项需要处理`, "err");
    } else {
      notify("检查完成，没有发现问题");
    }
  } finally {
    repairing.value = false;
  }
}

/// 跳到版本设置，方便禁用可疑 Mod
function openInstanceSettings() {
  const versionId = store.crashVersionId;
  if (!versionId) return;
  close();
  openInstance(versionId);
}

watch(
  () => [store.crashOpen, store.crashVersionId] as const,
  ([open]) => {
    if (open) void load();
  }
);

onMounted(() => {
  if (store.crashOpen) void load();
});
</script>

<template>
  <WoodModal
    v-if="store.crashOpen"
    title="崩溃分析"
    :width="600"
    :z-index="110"
    @close="close"
  >
    <div v-if="loading" class="crash-loading">
      <span class="skeleton" />
      正在读取日志并匹配已知问题…
    </div>

    <div v-else-if="error" class="crash-error">
      <AppIcon name="alert" :size="16" />
      {{ error }}
      <WoodButton variant="ghost" size="sm" @click="load">重试</WoodButton>
    </div>

    <template v-else-if="report">
      <div
        class="crash-headline"
        :class="{ 'crash-headline--ok': report.found && !report.matches.length }"
      >
        <span class="crash-headline__icon">
          <AppIcon :name="report.matches.length ? 'alert' : 'check'" :size="18" />
        </span>
        <div class="crash-headline__text">
          <strong>{{ report.headline }}</strong>
          <span class="muted small">
            {{ report.version_id }} · {{ exitLabel }}
            <template v-if="sourceTime"> · {{ sourceTime }}</template>
          </span>
        </div>
        <span class="tag mono">{{ sourceLabel }}</span>
      </div>

      <p v-if="!report.found" class="crash-note">
        这个版本还没有生成日志。先启动一次游戏，退出后会自动生成
        <span class="mono">logs/latest.log</span> 或崩溃报告。
      </p>

      <template v-else>
        <div class="crash-context">
          <span v-if="report.context.game_version" class="tag">
            {{ report.context.game_version }}
          </span>
          <span v-if="report.context.loader" class="tag">{{ report.context.loader }}</span>
          <span class="tag">
            {{ report.context.java_major ? `Java ${report.context.java_major}` : "Java 未检测到" }}
          </span>
          <span class="tag">{{ report.context.memory_mb }} MB</span>
          <span class="tag">{{ report.context.mod_count }} 个 Mod</span>
        </div>

        <p v-if="report.context.problematic_frame" class="crash-frame mono">
          Problematic frame：{{ report.context.problematic_frame }}
        </p>

        <div v-if="report.suspects.length" class="crash-suspects">
          <div class="crash-suspects__head">
            <strong>可疑 Mod</strong>
            <span class="muted small">按崩溃堆栈与报告标注推断，未必是最终结论</span>
            <WoodButton variant="ghost" size="sm" @click="openInstanceSettings">
              打开版本设置
            </WoodButton>
          </div>
          <div
            v-for="suspect in report.suspects"
            :key="`${suspect.name}-${suspect.file_name}`"
            class="crash-suspect"
          >
            <div class="crash-suspect__text">
              <strong>{{ suspect.name }}</strong>
              <span v-if="suspect.file_name" class="mono small">{{ suspect.file_name }}</span>
              <span v-else class="muted small">未在 mods 目录找到对应文件</span>
              <span v-if="suspect.package" class="mono small muted">{{ suspect.package }}</span>
            </div>
            <span class="tag">
              {{ suspect.reason === "stack" ? "来自堆栈" : "报告标注" }}
            </span>
          </div>
        </div>

        <div class="crash-matches">
          <div v-for="group in matchGroups" :key="group.title" class="crash-group">
            <div class="crash-group__title">{{ group.title }}</div>
            <article v-for="item in group.items" :key="item.id" class="crash-match">
              <header class="crash-match__head">
                <strong>{{ item.title }}</strong>
                <span class="tag">{{ item.id }}</span>
              </header>
              <p class="crash-match__summary">{{ item.summary }}</p>
              <ul class="crash-advice">
                <li v-for="(tip, index) in item.advice" :key="index">{{ tip }}</li>
              </ul>
              <div v-if="item.evidence.length" class="crash-evidence mono">
                <div v-for="(line, index) in item.evidence" :key="index">{{ line }}</div>
              </div>
              <div v-if="item.links.length" class="crash-links">
                <button
                  v-for="link in item.links"
                  :key="link.url"
                  @click="openExternal(link.url)"
                >
                  {{ link.label }}
                  <AppIcon name="external" :size="11" />
                </button>
              </div>
            </article>
          </div>
        </div>
      </template>

      <details v-if="report.excerpt.length" class="crash-excerpt">
        <summary>日志摘录（{{ report.source }}）</summary>
        <pre class="mono">{{ report.excerpt.join("\n") }}</pre>
      </details>
    </template>

    <template #footer>
      <WoodButton
        v-if="report?.found"
        variant="ghost"
        size="sm"
        :disabled="exporting"
        @click="exportReport"
      >
        <AppIcon name="download" :size="13" />
        {{ exporting ? "导出中…" : "导出报告" }}
      </WoodButton>
      <WoodButton
        v-if="report?.found"
        variant="ghost"
        size="sm"
        @click="openLogDir"
      >
        <AppIcon name="folder" :size="13" />
        打开日志目录
      </WoodButton>
      <WoodButton
        v-if="report?.found"
        variant="soft"
        size="sm"
        :disabled="repairing || loading"
        @click="runRepair"
      >
        <AppIcon name="wrench" :size="13" />
        {{ repairing ? "修复中…" : "一键修复" }}
      </WoodButton>
      <WoodButton variant="soft" size="sm" :disabled="loading" @click="load">
        <AppIcon name="refresh" :size="13" />
        重新分析
      </WoodButton>
      <WoodButton variant="primary" size="sm" @click="close">关闭</WoodButton>
    </template>
  </WoodModal>
</template>

<style scoped>
.crash-loading {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 18px 0;
  color: var(--muted);
  font-size: 12.5px;
}

.crash-error {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 12px;
  border-radius: 4px;
  border: 1px solid var(--line-strong);
  background: var(--danger-soft);
  color: var(--danger);
  font-size: 12px;
}

.crash-headline {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 12px 14px;
  border-radius: 6px;
  border: 1px solid var(--line-strong);
  background: var(--danger-soft);
}

.crash-headline--ok {
  background: var(--moss-soft);
  border-color: #d3dfc8;
}

.crash-headline__icon {
  display: grid;
  place-items: center;
  width: 36px;
  height: 36px;
  border-radius: 4px;
  background: rgba(255, 255, 255, 0.55);
  color: var(--danger);
  flex-shrink: 0;
}

.crash-headline--ok .crash-headline__icon {
  color: var(--moss);
}

.crash-headline__text {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.crash-headline__text strong {
  font-size: 13px;
  font-weight: 700;
  color: var(--walnut-deep);
  overflow-wrap: anywhere;
}

.crash-note {
  margin: 14px 0 0;
  padding: 12px 14px;
  border-radius: 4px;
  border: 1px dashed var(--line-strong);
  background: var(--panel-2);
  color: var(--muted);
  font-size: 12px;
  line-height: 1.6;
}

.crash-context {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-top: 12px;
}

.crash-frame {
  margin: 10px 0 0;
  padding: 8px 10px;
  border-radius: 4px;
  border: 1px solid var(--line);
  background: var(--panel-2);
  font-size: 11.5px;
  color: var(--ink-soft);
  overflow-wrap: anywhere;
}

.crash-suspects {
  margin-top: 14px;
  padding: 10px 12px;
  border-radius: 6px;
  border: 1px solid #d3dfc8;
  background: var(--moss-soft);
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.crash-suspects__head {
  display: flex;
  align-items: center;
  gap: 8px;
}

.crash-suspects__head strong {
  font-size: 12.5px;
  color: var(--moss);
}

.crash-suspects__head .small {
  flex: 1;
  min-width: 0;
}

.crash-suspect {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  padding: 8px 10px;
  border-radius: 4px;
  background: rgba(255, 255, 255, 0.6);
}

.crash-suspect__text {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.crash-suspect__text strong {
  font-size: 12.5px;
  color: var(--walnut-deep);
  overflow-wrap: anywhere;
}

.crash-group {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.crash-group__title {
  margin-top: 2px;
  font-size: 11px;
  font-weight: 700;
  letter-spacing: 0.08em;
  color: var(--muted);
}

.crash-matches {
  display: flex;
  flex-direction: column;
  gap: 12px;
  margin-top: 14px;
}

.crash-match {
  padding: 12px 14px;
  border-radius: 6px;
  border: 1px solid var(--line);
  background: var(--panel);
}

.crash-match__head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
}

.crash-match__head strong {
  font-size: 13px;
  font-weight: 700;
  color: var(--walnut-deep);
}

.crash-match__summary {
  margin: 6px 0 8px;
  font-size: 12px;
  line-height: 1.6;
  color: var(--ink-soft);
}

.crash-advice {
  margin: 0;
  padding-left: 18px;
  display: flex;
  flex-direction: column;
  gap: 3px;
  font-size: 12px;
  line-height: 1.55;
  color: var(--ink-soft);
}

.crash-advice li::marker {
  color: var(--moss);
}

.crash-evidence {
  margin-top: 9px;
  padding: 8px 10px;
  border-radius: 4px;
  border: 1px dashed var(--line-strong);
  background: var(--paper-2);
  font-size: 10.5px;
  line-height: 1.6;
  color: var(--muted);
  max-height: 120px;
  overflow: auto;
  user-select: text;
}

.crash-evidence div {
  white-space: pre-wrap;
  word-break: break-all;
}

.crash-links {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-top: 8px;
}

.crash-links button {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: 11.5px;
  color: var(--moss);
  text-decoration: underline;
  padding: 0;
}

.crash-excerpt {
  margin-top: 14px;
  border-top: 1px dashed var(--line);
}

.crash-excerpt summary {
  padding-top: 10px;
  font-size: 12px;
  font-weight: 600;
  color: var(--ink-soft);
  cursor: pointer;
}

.crash-excerpt pre {
  margin: 8px 0 0;
  padding: 10px 12px;
  max-height: 240px;
  overflow: auto;
  border-radius: 4px;
  border: 1px solid var(--line);
  background: var(--paper-2);
  font-size: 10.5px;
  line-height: 1.6;
  color: var(--muted);
  white-space: pre-wrap;
  word-break: break-all;
  user-select: text;
}
</style>
