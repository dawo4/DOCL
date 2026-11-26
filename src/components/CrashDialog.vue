﻿﻿﻿﻿﻿﻿﻿<script setup lang="ts">
import { computed, onMounted, onUnmounted, reactive, ref, watch } from "vue";
import { confirm } from "@tauri-apps/plugin-dialog";
import {
  api,
  openExternal,
  openFolder,
  revealFile,
  type CrashMatch,
  type CrashReport,
  type CrashRuleStatus,
  type RepairPreview,
} from "../api";
import { notify, openInstance, repairInstance, store } from "../store";
import AppIcon from "./AppIcon.vue";
import WoodButton from "./WoodButton.vue";
import WoodModal from "./WoodModal.vue";

const report = ref<CrashReport | null>(null);
const loading = ref(false);
const exporting = ref(false);
const sharing = ref(false);
const shareConfirmOpen = ref(false);
const repairing = ref(false);
const error = ref("");

/// 用户反馈「不是这个原因」的规则，按 id 记在本地，下次不再展示
const IGNORED_RULES_KEY = "docl.crash.ignoredRules";
const ignoredRules = ref<string[]>([]);
try {
  const stored = JSON.parse(localStorage.getItem(IGNORED_RULES_KEY) ?? "[]");
  if (Array.isArray(stored)) {
    ignoredRules.value = stored.filter((item): item is string => typeof item === "string");
  }
} catch {
  ignoredRules.value = [];
}

const visibleMatches = computed(() =>
  (report.value?.matches ?? []).filter((item) => !ignoredRules.value.includes(item.id))
);
const hiddenMatches = computed(() =>
  (report.value?.matches ?? []).filter((item) => ignoredRules.value.includes(item.id))
);

function ignoreMatch(item: CrashMatch) {
  if (ignoredRules.value.includes(item.id)) return;
  ignoredRules.value = [...ignoredRules.value, item.id];
  localStorage.setItem(IGNORED_RULES_KEY, JSON.stringify(ignoredRules.value));
  notify(`已忽略「${item.title}」，下次不再显示`);
}

/// 规则库状态：远端规则误报太多时可以回滚到上一版
const ruleStatus = ref<CrashRuleStatus | null>(null);
const rollingBack = ref(false);

async function loadRuleStatus() {
  try {
    ruleStatus.value = await api.crashRuleStatus();
  } catch {
    ruleStatus.value = null;
  }
}

async function rollbackRules() {
  if (rollingBack.value) return;
  rollingBack.value = true;
  try {
    const count = await api.crashRuleRollback();
    notify(`已回滚到上一版规则（${count} 条），重新分析后生效`);
    await loadRuleStatus();
    await load();
  } catch (err) {
    notify(`回滚规则失败：${String(err)}`, "err");
  } finally {
    rollingBack.value = false;
  }
}

function restoreIgnored() {
  ignoredRules.value = [];
  localStorage.removeItem(IGNORED_RULES_KEY);
}

/// 导出被标记为误报的规则，方便反馈给规则维护者
const exportingFeedback = ref(false);
async function exportFeedback() {
  const versionId = store.crashVersionId;
  if (!versionId || exportingFeedback.value || !hiddenMatches.value.length) return;
  exportingFeedback.value = true;
  try {
    const path = await api.crashExportFeedback(
      versionId,
      hiddenMatches.value.map((item) => item.id)
    );
    const name = path.split(/[\\/]/).pop() ?? path;
    notify(`已导出误报反馈：${name}`);
    await revealFile(path);
  } catch (err) {
    notify(`导出误报反馈失败：${String(err)}`, "err");
  } finally {
    exportingFeedback.value = false;
  }
}

/// 按原因层分组：环境/Mod 归因是最可能的原因，兼容性和兜底作为补充线索
const matchGroups = computed(() => {
  const list = visibleMatches.value;
  const groups: { title: string; items: CrashMatch[] }[] = [];
  const primary = list.filter((item) => item.phase <= 2);
  const others = list.filter((item) => item.phase > 2);
  if (primary.length) groups.push({ title: "最可能的原因", items: primary });
  if (others.length) {
    groups.push({
      title: primary.length ? "其他线索" : "检测到的线索",
      items: others,
    });
  }
  return groups;
});

function confidenceLabel(item: CrashMatch): string {
  if (item.confidence === "high") return "高置信";
  if (item.confidence === "medium") return "中等";
  return "参考";
}

/// Windows 应用日志里与崩溃时间接近的条目
const windowsEventsText = computed(() =>
  (report.value?.windows_events ?? [])
    .map(
      (event) =>
        `${event.time_created} · ${event.provider} · 事件 ${event.event_id}\n${event.message}`
    )
    .join("\n\n")
);

/// 与上一次成功启动对比出的 Mod 变化
const modChangeGroups = computed(() => {
  const list = report.value?.mod_changes ?? [];
  const labels: Record<string, string> = { added: "新增", removed: "移除", modified: "改动" };
  return (["added", "removed", "modified"] as const)
    .map((kind) => ({
      kind,
      label: labels[kind],
      items: list.filter((item) => item.kind === kind),
    }))
    .filter((group) => group.items.length);
});

/// 上一次成功启动后新增的 Mod，可以整体撤销
const addedMods = computed(() =>
  (report.value?.mod_changes ?? []).filter((item) => item.kind === "added")
);
const undoingMods = ref(false);

async function undoAddedMods() {
  const versionId = store.crashVersionId;
  if (!versionId || undoingMods.value || !addedMods.value.length) return;
  const ok = await confirm(
    `会把新增的 ${addedMods.value.length} 个 Mod 移到 mods-disabled（不删除），然后重新分析。继续吗？`,
    { title: "撤销新增的 Mod", kind: "warning" }
  );
  if (!ok) return;
  undoingMods.value = true;
  try {
    const moved = await api.crashUndoAddedMods(versionId);
    notify(
      moved.length ? `已撤销 ${moved.length} 个新增 Mod，重新启动后生效` : "没有找到可以撤销的新增 Mod"
    );
    await load();
  } catch (err) {
    notify(`撤销新增 Mod 失败：${String(err)}`, "err");
  } finally {
    undoingMods.value = false;
  }
}

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
  void loadRuleStatus();
  try {
    report.value = await api.crashAnalyze(versionId, store.crashExitCode);
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

/// 上传脱敏后的日志到 mclo.gs，并复制链接
async function shareLog() {
  const versionId = store.crashVersionId;
  if (!versionId || sharing.value) return;
  shareConfirmOpen.value = false;
  sharing.value = true;
  try {
    const url = await api.crashShare(versionId);
    try {
      await navigator.clipboard.writeText(url);
      notify(`分享链接已复制：${url}`);
    } catch {
      notify(`分享链接：${url}`);
    }
  } catch (err) {
    notify(`分享失败：${String(err)}`, "err");
  } finally {
    sharing.value = false;
  }
}

/// 一键修复：修清单、补文件、禁用重复与已知不兼容的模组
const planOpen = ref(false);
const planOptions = reactive({
  install_deps: false,
  reset_graphics: false,
  repair_assets: false,
  reinstall_loader: false,
});

interface RepairPlanItem {
  title: string;
  detail: string;
  risk: "低" | "中";
}

/// 根据命中的规则生成修复计划，让用户在动手前先看清会改什么
const repairPlan = computed<RepairPlanItem[]>(() => {
  const list = report.value?.matches ?? [];
  const ids = new Set(list.map((item) => item.id));
  const has = (...keys: string[]) => keys.some((key) => ids.has(key));
  const items: RepairPlanItem[] = [
    {
      title: "校验版本清单与游戏文件",
      detail: "修复损坏的 json、补回缺失的 libraries 与资源，不会删除存档。",
      risk: "低",
    },
  ];
  if (has("duplicate-mod-files", "mod-conflict", "confirmed-mod-crash", "mod-init-failed")) {
    items.push({
      title: "禁用重复或命中的 Mod",
      detail: "把冲突的 jar 改名为 .disabled 并备份，不会直接删除。",
      risk: "中",
    });
  }
  if (
    has(
      "java-version",
      "vm-options",
      "java-32bit-exact",
      "forge-java-incompatible",
      "mod-needs-java11",
      "java-agent-failed"
    )
  ) {
    items.push({
      title: "切换到匹配的 Java",
      detail: "按游戏版本选择 8 / 11 / 17 / 21，并排除 32 位 Java。",
      risk: "低",
    });
  }
  if (has("optifine-sodium", "optifine-performant", "shadersmod-optifine", "optifine-world-load")) {
    items.push({
      title: "处理 OptiFine / 渲染冲突",
      detail: "禁用冲突的渲染 Mod，保留 OptiFine 或 Sodium 其中一个。",
      risk: "中",
    });
  }
  if (has("mod-config", "night-config-bug", "mod-config-corrupt")) {
    items.push({
      title: "重置损坏的 Mod 配置",
      detail: "先备份 config，再让游戏重新生成。",
      risk: "中",
    });
  }
  return items;
});

function openRepairPlan() {
  if (!report.value?.found || repairing.value) return;
  planOpen.value = true;
  void loadPlanPreview();
}

/// 命中的规则与证据，修复前用它预览会改动哪些文件。
/// 已标记为「不是这个原因」的规则不再参与修复，否则和忽略操作自相矛盾。
function repairScope() {
  const matches = visibleMatches.value;
  return {
    rules: matches.map((item) => item.id),
    evidence: matches.flatMap((item) => item.evidence),
  };
}

const planPreview = ref<RepairPreview | null>(null);
const planPreviewing = ref(false);
/// 连续切换勾选项时只认最后一次请求
let planPreviewToken = 0;
let planPreviewTimer: ReturnType<typeof setTimeout> | undefined;

onUnmounted(() => {
  if (planPreviewTimer) clearTimeout(planPreviewTimer);
});

async function loadPlanPreview() {
  const versionId = store.crashVersionId;
  if (!versionId) return;
  const token = ++planPreviewToken;
  planPreviewing.value = true;
  try {
    const result = await api.previewRepair(versionId, {
      ...repairScope(),
      ...planOptions,
    });
    if (token === planPreviewToken) planPreview.value = result;
  } catch {
    if (token === planPreviewToken) planPreview.value = null;
  } finally {
    if (token === planPreviewToken) planPreviewing.value = false;
  }
}

watch(
  planOptions,
  () => {
    if (!planOpen.value) return;
    if (planPreviewTimer) clearTimeout(planPreviewTimer);
    planPreviewTimer = setTimeout(() => void loadPlanPreview(), 250);
  },
  { deep: true }
);

async function runRepair() {
  const versionId = store.crashVersionId;
  if (!versionId || repairing.value) return;
  planOpen.value = false;
  repairing.value = true;
  try {
    // 把命中的规则与证据带进去，修复才能针对具体原因和配置文件下手；
    // 用户已忽略的规则不再参与修复
    const matches = visibleMatches.value;
    const rules = matches.map((item) => item.id);
    const evidence = matches.flatMap((item) => item.evidence);
    const result = await repairInstance(versionId, { rules, evidence, ...planOptions });
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
    if (result.java_switched) parts.push("切换 Java");
    if (result.config_reset) parts.push("重置 Mod 配置");
    if (result.loader_reinstalled) parts.push("重装加载器");
    if (result.deps_installed.length)
      parts.push(`补装 ${result.deps_installed.length} 个前置`);
    if (result.quarantined_mods.length)
      parts.push(`隔离 ${result.quarantined_mods.length} 个解压残留`);
    if (result.renamed_mods.length)
      parts.push(`重命名 ${result.renamed_mods.length} 个 Mod`);
    if (result.installed_fixes.length)
      parts.push(`安装 ${result.installed_fixes.length} 个修复 Mod`);
    if (result.removed_paths.length)
      parts.push(`清理 ${result.removed_paths.length} 个冲突文件`);
    if (result.removed_java_agents.length)
      parts.push(`移除 ${result.removed_java_agents.length} 个无效 -javaagent`);
    const pending = result.issues.length + result.files_failed.length;
    const undoHint = result.backup_dir ? "，可在版本设置里还原" : "";
    if (parts.length) {
      notify(
        `一键修复完成：${parts.join("、")}${
          pending ? `，仍有 ${pending} 项需要手动处理` : ""
        }${undoHint}`
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

        <div v-if="report.mod_changes.length" class="crash-mod-changes">
          <div class="crash-group__title">自上次成功启动以来的 Mod 变化</div>
          <div v-for="group in modChangeGroups" :key="group.kind" class="crash-mod-group">
            <span class="tag">{{ group.label }} {{ group.items.length }}</span>
            <ul>
              <li v-for="item in group.items" :key="item.file_name" class="mono">
                {{ item.file_name }}
              </li>
            </ul>
          </div>
          <WoodButton
            v-if="addedMods.length"
            variant="ghost"
            size="sm"
            :disabled="undoingMods"
            @click="undoAddedMods"
          >
            {{ undoingMods ? "撤销中…" : `撤销新增的 ${addedMods.length} 个 Mod` }}
          </WoodButton>
        </div>

        <div class="crash-matches">
          <div v-for="group in matchGroups" :key="group.title" class="crash-group">
            <div class="crash-group__title">{{ group.title }}</div>
            <article v-for="item in group.items" :key="item.id" class="crash-match">
              <header class="crash-match__head">
                <strong>{{ item.title }}</strong>
                <span class="tag">{{ confidenceLabel(item) }}</span>
                <span class="tag">{{ item.id }}</span>
                <button class="crash-ignore" @click="ignoreMatch(item)">不是这个原因</button>
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

        <div v-if="hiddenMatches.length" class="crash-ignored">
          已忽略 {{ hiddenMatches.length }} 条原因
          <button @click="restoreIgnored">恢复显示</button>
          <button :disabled="exportingFeedback" @click="exportFeedback">
            {{ exportingFeedback ? "导出中…" : "导出误报反馈" }}
          </button>
        </div>

        <div v-if="ruleStatus" class="crash-ignored">
          规则库：{{ ruleStatus.source === "cache" ? "远端更新" : "内置" }} ·
          {{ ruleStatus.count }} 条
          <button
            v-if="ruleStatus.has_previous"
            :disabled="rollingBack"
            @click="rollbackRules"
          >
            {{ rollingBack ? "回滚中…" : "回滚上一版规则" }}
          </button>
        </div>
      </template>

      <details v-if="report.windows_events.length" class="crash-excerpt">
        <summary>Windows 应用日志（{{ report.windows_events.length }}）</summary>
        <pre class="mono">{{ windowsEventsText }}</pre>
      </details>

      <details v-if="report.excerpt.length" class="crash-excerpt">
        <summary>日志摘录（{{ report.source }}）</summary>
        <pre class="mono">{{ report.excerpt.join("\n") }}</pre>
      </details>
    </template>

    <template #footer>
      <div class="crash-foot">
        <div v-if="report?.found" class="crash-foot__group">
          <WoodButton
            variant="ghost"
            size="sm"
            :disabled="exporting"
            @click="exportReport"
          >
            <AppIcon name="download" :size="13" />
            {{ exporting ? "导出中…" : "导出报告" }}
          </WoodButton>
          <WoodButton
            variant="ghost"
            size="sm"
            :disabled="sharing"
            @click="shareConfirmOpen = true"
          >
            <AppIcon name="external" :size="13" />
            {{ sharing ? "上传中…" : "分享日志" }}
          </WoodButton>
          <WoodButton variant="ghost" size="sm" @click="openLogDir">
            <AppIcon name="folder" :size="13" />
            日志目录
          </WoodButton>
        </div>
        <div class="crash-foot__group crash-foot__group--main">
          <WoodButton
            v-if="report?.found"
            variant="soft"
            size="sm"
            :disabled="repairing || loading"
            @click="openRepairPlan"
          >
            <AppIcon name="wrench" :size="13" />
            {{ repairing ? "修复中…" : "一键修复" }}
          </WoodButton>
          <WoodButton variant="soft" size="sm" :disabled="loading" @click="load">
            <AppIcon name="refresh" :size="13" />
            重新分析
          </WoodButton>
          <WoodButton variant="primary" size="sm" @click="close">关闭</WoodButton>
        </div>
      </div>
    </template>
  </WoodModal>

  <WoodModal
    v-if="planOpen"
    title="修复计划"
    :width="560"
    :z-index="120"
    @close="planOpen = false"
  >
    <p class="crash-note">
      下面是根据本次崩溃分析生成的操作。带「中」风险的项目会改动 Mod 文件或配置，执行前会自动备份，可在版本设置里还原。
    </p>
    <ul class="repair-plan">
      <li v-for="(item, index) in repairPlan" :key="index">
        <div class="repair-plan__head">
          <strong>{{ item.title }}</strong>
          <span class="tag" :class="{ 'repair-plan__risk': item.risk === '中' }">
            {{ item.risk }}风险
          </span>
        </div>
        <p>{{ item.detail }}</p>
      </li>
    </ul>

    <div class="repair-plan__options">
      <label>
        <input v-model="planOptions.install_deps" type="checkbox" />
        补装缺失前置（联网下载）
      </label>
      <label>
        <input v-model="planOptions.repair_assets" type="checkbox" />
        校验并补全资源文件（可能下载较多）
      </label>
      <label>
        <input v-model="planOptions.reset_graphics" type="checkbox" />
        调低画质相关设置（会改 options.txt）
      </label>
      <label>
        <input v-model="planOptions.reinstall_loader" type="checkbox" />
        必要时重装加载器（联网下载）
      </label>
    </div>

    <div class="repair-plan__files">
      <p class="repair-plan__files-title">将改动的文件</p>
      <p v-if="planPreviewing" class="muted small">正在扫描现有文件…</p>
      <template v-else-if="planPreview">
        <p v-if="!planPreview.actions.length" class="muted small">
          没有需要改动的内容。
        </p>
        <div
          v-for="action in planPreview.actions"
          :key="action.id"
          class="repair-plan__group"
        >
          <div class="repair-plan__group-head">
            <strong>{{ action.title }}</strong>
            <span class="muted small">
              {{ action.count ? `${action.count} 个文件` : "自动选择" }}
            </span>
          </div>
          <ul v-if="action.files.length" class="repair-plan__file-list">
            <li v-for="file in action.files" :key="file">{{ file }}</li>
          </ul>
          <p v-if="action.count > action.files.length" class="muted small">
            仅列出前 {{ action.files.length }} 项，其余
            {{ action.count - action.files.length }} 项会在执行时一并处理。
          </p>
        </div>
        <p
          v-for="issue in planPreview.issues"
          :key="issue"
          class="muted small"
        >
          {{ issue }}
        </p>
      </template>
    </div>

    <template #footer>
      <WoodButton variant="ghost" size="sm" @click="planOpen = false">取消</WoodButton>
      <WoodButton variant="primary" size="sm" :disabled="repairing" @click="runRepair">
        <AppIcon name="wrench" :size="13" />
        {{ repairing ? "修复中…" : "开始修复" }}
      </WoodButton>
    </template>
  </WoodModal>

  <WoodModal
    v-if="shareConfirmOpen"
    title="分享崩溃日志"
    :width="500"
    :z-index="130"
    @close="shareConfirmOpen = false"
  >
    <p class="crash-note">
      将把脱敏后的分析结果和相关日志上传到 mclo.gs（公开站点），并复制链接。上传前会隐藏账号令牌和本机用户目录。
    </p>
    <template #footer>
      <WoodButton variant="ghost" size="sm" @click="shareConfirmOpen = false">取消</WoodButton>
      <WoodButton variant="primary" size="sm" :disabled="sharing" @click="shareLog">
        {{ sharing ? "上传中…" : "上传并复制链接" }}
      </WoodButton>
    </template>
  </WoodModal>
</template>

<style scoped>
/* 底部工具栏：按钮分组并允许换行，窄窗口下也不会被裁掉 */
.crash-foot {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: 8px 12px;
  width: 100%;
}

.crash-foot__group {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-3);
  min-width: 0;
}

.crash-foot__group--main {
  margin-left: auto;
}

.crash-mod-changes {
  margin: 12px 0 4px;
  padding: 10px 12px;
  border: 1px solid var(--line);
  background: var(--panel-2);
}

.crash-mod-changes :deep(button) {
  margin-top: 8px;
}

.crash-mod-group {
  display: flex;
  gap: var(--space-3);
  align-items: flex-start;
  margin-top: 7px;
  font-size: var(--text-sm);
}

.crash-mod-group ul {
  margin: 0;
  padding: 0;
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 2px;
  color: var(--muted);
  min-width: 0;
}

.crash-mod-group li {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.crash-ignore {
  margin-left: auto;
  border: 0;
  background: none;
  padding: 0;
  color: var(--muted);
  font-size: var(--text-xs);
  cursor: pointer;
}

.crash-ignore:hover {
  color: var(--danger);
  text-decoration: underline;
}

.crash-ignored {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  margin-top: 8px;
  color: var(--muted);
  font-size: var(--text-sm);
}

.crash-ignored button {
  border: 0;
  background: none;
  padding: 0;
  color: var(--accent, var(--muted));
  font-size: var(--text-sm);
  cursor: pointer;
  text-decoration: underline;
}

.repair-plan {
  list-style: none;
  margin: 0 0 14px;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}

.repair-plan li {
  padding: 9px 11px;
  border: 1px solid var(--line);
  background: var(--panel-2);
}

.repair-plan__head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-3);
  font-size: var(--text-base);
}

.repair-plan li p {
  margin: 4px 0 0;
  color: var(--muted);
  font-size: var(--text-sm);
  line-height: 1.55;
}

.repair-plan__risk {
  color: var(--danger);
  border-color: var(--danger);
}

.repair-plan__options {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  padding-top: 10px;
  border-top: 1px solid var(--line);
  font-size: var(--text-base);
  color: var(--muted);
}

.repair-plan__options label {
  display: flex;
  align-items: center;
  gap: 7px;
  cursor: pointer;
}

.repair-plan__files {
  margin-top: 12px;
  padding-top: 10px;
  border-top: 1px solid var(--line);
  max-height: 260px;
  overflow-y: auto;
}

.repair-plan__files-title {
  margin: 0 0 6px;
  font-size: var(--text-base);
  color: var(--ink);
}

.repair-plan__group {
  margin-top: 9px;
}

.repair-plan__group:first-of-type {
  margin-top: 0;
}

.repair-plan__group-head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: var(--space-3);
  font-size: var(--text-base);
}

.repair-plan__group p {
  margin: 2px 0 0;
}

.repair-plan__file-list {
  margin: 3px 0 0;
  padding-left: 18px;
  font-size: var(--text-sm);
  line-height: 1.7;
  color: var(--muted);
  word-break: break-all;
}

.crash-loading {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  padding: 18px 0;
  color: var(--muted);
  font-size: var(--text-base);
}

.crash-error {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  padding: 10px 12px;
  border-radius: 0;
  border: 1px solid var(--line-strong);
  background: var(--danger-soft);
  color: var(--danger);
  font-size: var(--text-base);
}

.crash-headline {
  display: flex;
  align-items: center;
  gap: var(--space-5);
  padding: 12px 14px;
  border-radius: 0;
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
  border-radius: 0;
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
  font-size: var(--text-lg);
  font-weight: 700;
  color: var(--walnut-deep);
  overflow-wrap: anywhere;
}

.crash-note {
  margin: 14px 0 0;
  padding: 12px 14px;
  border-radius: 0;
  border: 1px dashed var(--line-strong);
  background: var(--panel-2);
  color: var(--muted);
  font-size: var(--text-base);
  line-height: 1.6;
}

.crash-context {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-2);
  margin-top: 12px;
}

.crash-frame {
  margin: 10px 0 0;
  padding: 8px 10px;
  border-radius: 0;
  border: 1px solid var(--line);
  background: var(--panel-2);
  font-size: var(--text-sm);
  color: var(--ink-soft);
  overflow-wrap: anywhere;
}

.crash-suspects {
  margin-top: 14px;
  padding: 10px 12px;
  border-radius: 0;
  border: 1px solid #d3dfc8;
  background: var(--moss-soft);
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}

.crash-suspects__head {
  display: flex;
  align-items: center;
  gap: var(--space-3);
}

.crash-suspects__head strong {
  font-size: var(--text-base);
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
  gap: var(--space-4);
  padding: 8px 10px;
  border-radius: 0;
  background: rgba(255, 255, 255, 0.6);
}

.crash-suspect__text {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.crash-suspect__text strong {
  font-size: var(--text-base);
  color: var(--walnut-deep);
  overflow-wrap: anywhere;
}

.crash-group {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}

.crash-group__title {
  margin-top: 2px;
  font-size: var(--text-sm);
  font-weight: 700;
  letter-spacing: 0.08em;
  color: var(--muted);
}

.crash-matches {
  display: flex;
  flex-direction: column;
  gap: var(--space-5);
  margin-top: 14px;
}

.crash-match {
  padding: 12px 14px;
  border-radius: 0;
  border: 1px solid var(--line);
  background: var(--panel);
}

.crash-match__head {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-4);
}

.crash-match__head strong {
  font-size: var(--text-lg);
  font-weight: 700;
  color: var(--walnut-deep);
}

.crash-match__summary {
  margin: 6px 0 8px;
  font-size: var(--text-base);
  line-height: 1.6;
  color: var(--ink-soft);
}

.crash-advice {
  margin: 0;
  padding-left: 18px;
  display: flex;
  flex-direction: column;
  gap: 3px;
  font-size: var(--text-base);
  line-height: 1.55;
  color: var(--ink-soft);
}

.crash-advice li::marker {
  color: var(--moss);
}

.crash-evidence {
  margin-top: 9px;
  padding: 8px 10px;
  border-radius: 0;
  border: 1px dashed var(--line-strong);
  background: var(--paper-2);
  font-size: var(--text-xs);
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
  gap: var(--space-3);
  margin-top: 8px;
}

.crash-links button {
  display: inline-flex;
  align-items: center;
  gap: var(--space-1);
  font-size: var(--text-sm);
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
  font-size: var(--text-base);
  font-weight: 600;
  color: var(--ink-soft);
  cursor: pointer;
}

.crash-excerpt pre {
  margin: 8px 0 0;
  padding: 10px 12px;
  max-height: 240px;
  overflow: auto;
  border-radius: 0;
  border: 1px solid var(--line);
  background: var(--paper-2);
  font-size: var(--text-xs);
  line-height: 1.6;
  color: var(--muted);
  white-space: pre-wrap;
  word-break: break-all;
  user-select: text;
}
</style>

