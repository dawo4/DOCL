<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from "vue";
import AppIcon from "./AppIcon.vue";
import IconButton from "./IconButton.vue";
import WoodButton from "./WoodButton.vue";
import WoodProgress from "./WoodProgress.vue";
import { cancelInstall, retryFailedDownloads, store } from "../store";
import { formatBytes, formatDuration, formatSpeed } from "../utils";

const hovered = ref(false);
let hoverTimer: number | undefined;
let hideTimer: number | undefined;

const snapshot = computed(() => store.progress ?? store.lastProgress);
const active = computed(() => !!store.installingId);
/// 有文件没下载成功但任务本身跑完了：算「部分完成」，不算安装失败
const partial = computed(() => !active.value && !!store.lastJobPartial);
const failed = computed(() => !!store.lastJobError && !partial.value);
const finished = computed(
  () => !active.value && !failed.value && !partial.value && !!store.lastProgress
);

const percent = computed(() => {
  const p = snapshot.value;
  if (!p) return finished.value ? 100 : 0;
  if (!active.value && !failed.value && !partial.value) return 100;
  if (p.bytes_total > 0) return Math.min(100, (p.bytes_done / p.bytes_total) * 100);
  if (p.files_total > 0) return Math.min(100, (p.files_done / p.files_total) * 100);
  return 0;
});

const speed = computed(() => {
  const p = snapshot.value;
  if (!p) return "—";
  return formatSpeed(p.bytes_done, p.elapsed_ms);
});

const eta = computed(() => {
  const p = snapshot.value;
  // 整合包清单不带文件大小（bytes_total 为 0）时无法估算剩余时间
  if (!p || p.bytes_total <= 0) return "—";
  if (p.bytes_total <= p.bytes_done) return active.value ? "计算中…" : "—";
  const elapsed = Math.max(1, p.elapsed_ms);
  const bytesPerMs = p.bytes_done / elapsed;
  if (bytesPerMs <= 0) return "计算中…";
  return formatDuration((p.bytes_total - p.bytes_done) / bytesPerMs);
});

const stageLabel = computed(() => {
  const p = snapshot.value;
  if (!p) return "";
  if (failed.value) return "安装失败";
  if (partial.value) return "部分完成";
  if (!active.value) return "安装完成";
  if (p.stage === "java") return "安装 Java 运行时";
  if (p.stage === "install") return "安装游戏文件";
  return p.stage;
});

const title = computed(
  () => store.installingId || store.lastJobTitle || "下载任务"
);

/// 后端记录的阶段序列；旧任务没有步骤时用当前消息兜底
const steps = computed(() => {
  const list = snapshot.value?.steps ?? [];
  if (list.length) return list;
  const fallback = snapshot.value?.message || stageLabel.value;
  return fallback ? [{ label: fallback, done: !active.value }] : [];
});

const remainingFiles = computed(() => {
  const p = snapshot.value;
  if (!p || !p.files_total) return "—";
  return String(Math.max(0, p.files_total - p.files_done));
});

/// 进度改成方块内部自下而上的水位高度，颜色按状态由 CSS 变量接管
const fillStyle = computed(() => ({ "--fill": `${percent.value}%` }));

function closePanel() {
  store.downloadPanel = active.value || !!store.lastProgress ? "ball" : "hidden";
}

function openPanel() {
  store.downloadPanel = "open";
  hovered.value = false;
}

function hideBall() {
  if (!active.value) store.downloadPanel = "hidden";
}

function showTip() {
  if (hoverTimer) window.clearTimeout(hoverTimer);
  hoverTimer = window.setTimeout(() => {
    hovered.value = true;
  }, 300);
}

function hideTip() {
  if (hoverTimer) window.clearTimeout(hoverTimer);
  hovered.value = false;
}

watch(
  () => store.downloadPanel,
  (mode) => {
    if (hideTimer) window.clearTimeout(hideTimer);
    if (mode === "ball" && !active.value) {
      hideTimer = window.setTimeout(() => {
        if (!active.value) store.downloadPanel = "hidden";
      }, 6000);
    }
  }
);

watch(
  () => store.installingId,
  (value, old) => {
    if (old && !value) {
      if (hideTimer) window.clearTimeout(hideTimer);
      hideTimer = window.setTimeout(() => {
        if (store.downloadPanel === "ball") store.downloadPanel = "hidden";
      }, 6000);
    }
  }
);

onBeforeUnmount(() => {
  if (hoverTimer) window.clearTimeout(hoverTimer);
  if (hideTimer) window.clearTimeout(hideTimer);
});
</script>

<template>
  <teleport to="body">
    <transition name="dl-fade">
      <div
        v-if="store.downloadPanel === 'open' && snapshot"
        class="dl-overlay"
        @mousedown.self="closePanel"
      >
        <section class="dl-panel">
          <header class="dl-panel__head">
            <div>
              <h3 class="dl-panel__title">{{ title }}</h3>
            </div>
            <div class="row">
              <span
                class="dl-status"
                :class="{
                  'dl-status--ok': finished && !failed,
                  'dl-status--err': failed,
                  'dl-status--busy': active && !failed,
                }"
                :title="stageLabel"
              >
                <AppIcon :name="failed ? 'alert' : active ? 'download' : 'check'" :size="14" />
              </span>
              <IconButton icon="minimize" label="收起到右下角" :size="30" @click="closePanel" />
            </div>
          </header>

          <div class="dl-body">
            <aside class="dl-metrics">
              <div class="dl-metric">
                <span class="dl-metric__label">总进度</span>
                <strong class="dl-metric__value">
                  {{ Math.round(percent) }}<em>%</em>
                </strong>
              </div>
              <div class="dl-metric">
                <span class="dl-metric__label">下载速度</span>
                <strong class="dl-metric__value dl-metric__value--small">{{ speed }}</strong>
              </div>
              <div class="dl-metric">
                <span class="dl-metric__label">剩余文件</span>
                <strong class="dl-metric__value dl-metric__value--small">{{ remainingFiles }}</strong>
              </div>
            </aside>

            <section class="dl-task">
              <header class="dl-task__head">
                <span class="dl-task__label">任务明细</span>
                <span class="dl-task__state" :class="{ 'dl-task__state--err': failed }">
                  {{ failed ? "安装失败" : partial ? "部分完成" : active ? "进行中" : "已完成" }}
                </span>
              </header>

              <ul class="dl-steps">
                <li
                  v-for="(step, index) in steps"
                  :key="`${index}-${step.label}`"
                  class="dl-step"
                  :class="{
                    'dl-step--done': step.done,
                    'dl-step--current': !step.done && active,
                  }"
                >
                  <span class="dl-step__mark">
                    <AppIcon v-if="step.done" name="check" :size="10" />
                    <span v-else class="dl-step__dot" />
                  </span>
                  <span class="dl-step__label">{{ step.label }}</span>
                  <span v-if="!step.done && active" class="dl-step__pct mono">
                    {{ Math.round(percent) }}%
                  </span>
                </li>
              </ul>

              <WoodProgress :value="percent" :height="6" />
              <div class="dl-task__bytes mono">
                <template v-if="(snapshot?.bytes_total ?? 0) > 0">
                  {{ formatBytes(snapshot?.bytes_done ?? 0) }} /
                  {{ formatBytes(snapshot?.bytes_total ?? 0) }}
                </template>
                <template v-else>{{ formatBytes(snapshot?.bytes_done ?? 0) }}</template>
                · {{ formatDuration(snapshot?.elapsed_ms ?? 0) }}
              </div>

              <p v-if="store.lastJobError" class="dl-error">{{ store.lastJobError }}</p>
            </section>
          </div>

          <footer class="dl-panel__foot">
            <WoodButton v-if="active" variant="danger" size="sm" @click="cancelInstall">
              取消安装
            </WoodButton>
            <WoodButton
              v-else-if="partial"
              variant="primary"
              size="sm"
              @click="retryFailedDownloads()"
            >
              重试失败文件
            </WoodButton>
            <WoodButton variant="ghost" size="sm" @click="closePanel">
              收起到悬浮球
            </WoodButton>
          </footer>
        </section>
      </div>
    </transition>

    <div v-if="store.downloadPanel === 'ball' && snapshot" class="dl-ball-wrap">
      <transition name="dl-tip">
        <div v-if="hovered" class="dl-tip">
          <div class="dl-tip__head">
            <strong>{{ title }}</strong>
            <span
              class="dl-status dl-status--small"
              :class="{
                'dl-status--ok': finished && !failed,
                'dl-status--err': failed,
                'dl-status--busy': active && !failed,
              }"
              :title="stageLabel"
            >
              <AppIcon :name="failed ? 'alert' : active ? 'download' : 'check'" :size="11" />
            </span>
          </div>
          <div class="dl-tip__bar">
            <span :style="{ width: `${percent}%` }" />
          </div>
          <div class="dl-tip__rows">
            <span>{{ Math.round(percent) }}%</span>
            <span>{{ speed }}</span>
            <span v-if="(snapshot?.bytes_total ?? 0) > 0">
              {{ formatBytes(snapshot?.bytes_done ?? 0) }} /
              {{ formatBytes(snapshot?.bytes_total ?? 0) }}
            </span>
            <span v-else>{{ formatBytes(snapshot?.bytes_done ?? 0) }}</span>
            <span>剩余 {{ eta }}</span>
          </div>
          <p v-if="store.lastJobError" class="dl-tip__err">{{ store.lastJobError }}</p>
          <button class="dl-tip__open" @click="openPanel">打开下载详情</button>
        </div>
      </transition>

      <button
        class="dl-ball"
        :class="{
          'dl-ball--done': finished && !failed,
          'dl-ball--failed': failed,
          'dl-ball--partial': partial,
          'dl-ball--busy': active && !failed,
        }"
        :style="fillStyle"
        aria-label="下载进度"
        @mouseenter="showTip"
        @mouseleave="hideTip"
        @click="openPanel"
      >
        <span class="dl-ball__glow" />
        <span class="dl-ball__face">
          <span class="dl-ball__water" />
          <span class="dl-ball__sheen" />
          <span class="dl-ball__content">
            <span class="dl-ball__icon">
              <AppIcon
                :name="failed || partial ? 'alert' : active ? 'download' : 'check'"
                :size="13"
              />
            </span>
            <span class="dl-ball__pct">
              {{
                failed ? "失败" : partial ? "部分" : finished ? "完成" : `${Math.round(percent)}%`
              }}
            </span>
          </span>
        </span>
      </button>

      <button
        v-if="finished || failed || partial"
        class="dl-ball__dismiss"
        title="关闭"
        @click="hideBall"
      >
        ×
      </button>
    </div>
  </teleport>
</template>

<style scoped>
.dl-overlay {
  position: fixed;
  inset: 0;
  z-index: 95;
  display: grid;
  place-items: center;
  background: rgba(40, 28, 16, 0.34);
  backdrop-filter: blur(4px);
  animation: dl-fade-in 180ms ease both;
}

.dl-panel {
  width: min(680px, 92vw);
  background: var(--panel);
  border: 2px solid var(--frame);
  border-top-color: color-mix(in srgb, var(--frame) 66%, #fff);
  border-left-color: color-mix(in srgb, var(--frame) 66%, #fff);
  border-right-color: color-mix(in srgb, var(--frame) 74%, #000);
  border-bottom-color: color-mix(in srgb, var(--frame) 74%, #000);
  border-radius: 2px;
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--panel) 46%, #fff),
    0 30px 70px rgba(40, 28, 16, 0.32);
  padding: 20px 22px 16px;
  display: flex;
  flex-direction: column;
  gap: 16px;
  position: relative;
  overflow: hidden;
  animation: dl-pop 280ms var(--ease-out-expo) both;
}

.dl-panel::before {
  content: "";
  position: absolute;
  inset: 0 0 auto;
  height: 2px;
  background: var(--moss);
}

.dl-panel__head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 14px;
}

.dl-panel__title {
  font-size: 20px;
  font-weight: 800;
  color: var(--walnut-deep);
  margin-top: 2px;
}

.dl-body {
  display: grid;
  grid-template-columns: 190px minmax(0, 1fr);
  gap: 14px;
  align-items: stretch;
}

/* 左侧大字指标 */
.dl-metrics {
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 16px 16px 14px;
  /* 弹窗内的信息块做成凹槽，和弹窗外框的凸起形成对照 */
  border-radius: 2px;
  border-style: solid;
  border-width: 1px 1px 2px;
  border-top-color: color-mix(in srgb, var(--line-strong) 45%, var(--ink));
  border-left-color: color-mix(in srgb, var(--line-strong) 45%, var(--ink));
  border-right-color: color-mix(in srgb, var(--line-strong) 58%, #fff);
  border-bottom-color: color-mix(in srgb, var(--line-strong) 58%, #fff);
  background: var(--panel-2);
  box-shadow: inset 0 1px 3px rgba(40, 28, 16, 0.1);
}

.dl-metric {
  display: flex;
  flex-direction: column;
  gap: 7px;
  padding-bottom: 12px;
  border-bottom: 1px solid var(--line);
}

.dl-metric:last-child {
  padding-bottom: 0;
  border-bottom: none;
}

.dl-metric__label {
  font-size: 11px;
  letter-spacing: 0.1em;
  color: var(--muted);
}

.dl-metric__value {
  font-family: var(--font-display);
  font-size: 27px;
  line-height: 1;
  color: var(--walnut-deep);
}

.dl-metric__value em {
  font-size: 13px;
  font-style: normal;
  color: var(--muted);
  margin-left: 2px;
}

.dl-metric__value--small {
  font-family: var(--font-mono);
  font-size: 17px;
}

/* 右侧任务卡片 */
.dl-task {
  display: flex;
  flex-direction: column;
  gap: 10px;
  min-width: 0;
  padding: 14px 16px;
  border-radius: 2px;
  border-style: solid;
  border-width: 1px 1px 2px;
  border-top-color: color-mix(in srgb, var(--line-strong) 45%, var(--ink));
  border-left-color: color-mix(in srgb, var(--line-strong) 45%, var(--ink));
  border-right-color: color-mix(in srgb, var(--line-strong) 58%, #fff);
  border-bottom-color: color-mix(in srgb, var(--line-strong) 58%, #fff);
  background: var(--panel-2);
  box-shadow: inset 0 1px 3px rgba(40, 28, 16, 0.1);
}

.dl-task__head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
}

.dl-task__label {
  font-size: 11px;
  letter-spacing: 0.1em;
  color: var(--muted);
}

.dl-task__state {
  font-size: 11px;
  font-weight: 700;
  color: var(--moss);
}

.dl-task__state--err {
  color: var(--danger);
}

.dl-steps {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 7px;
  max-height: 170px;
  overflow-y: auto;
}

.dl-step {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 12px;
  color: var(--muted);
}

.dl-step__mark {
  width: 16px;
  height: 16px;
  border-radius: 50%;
  display: grid;
  place-items: center;
  flex-shrink: 0;
  border: 1px solid var(--line-strong);
  background: var(--panel);
}

.dl-step--done .dl-step__mark {
  color: var(--moss);
  border-color: #d3dfc8;
  background: var(--moss-soft);
}

.dl-step--current .dl-step__mark {
  border-color: var(--moss);
}

.dl-step__dot {
  width: 4px;
  height: 4px;
  border-radius: 50%;
  background: var(--muted);
}

.dl-step__label {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.dl-step--done .dl-step__label {
  color: var(--ink-soft);
}

.dl-step--current .dl-step__label {
  color: var(--walnut-deep);
  font-weight: 600;
}

.dl-step__pct {
  flex-shrink: 0;
  font-size: 10.5px;
  color: var(--moss);
}

.dl-task__bytes {
  font-size: 10.5px;
  color: var(--muted);
}

/* 安装状态用图标表示，避免长文本挤坏标题栏 */
.dl-status {
  width: 26px;
  height: 26px;
  border-radius: 50%;
  display: grid;
  place-items: center;
  flex-shrink: 0;
  border: 1px solid var(--line);
  background: var(--panel-2);
  color: var(--muted);
}

.dl-status--small {
  width: 20px;
  height: 20px;
}

.dl-status--ok {
  color: var(--moss);
  background: var(--moss-soft);
  border-color: #d3dfc8;
}

.dl-status--err {
  color: var(--danger);
  background: var(--danger-soft);
  border-color: #ecd3ca;
}

.dl-status--busy {
  color: var(--walnut);
}

.dl-tip .dl-status {
  background: color-mix(in srgb, var(--paper) 10%, transparent);
  border-color: color-mix(in srgb, var(--paper) 18%, transparent);
  color: var(--paper);
}

.dl-tip .dl-status--ok {
  color: color-mix(in srgb, var(--moss-bright) 60%, var(--paper));
}

.dl-tip .dl-status--err {
  color: color-mix(in srgb, var(--danger) 45%, var(--paper));
}

.dl-error {
  padding: 9px 11px;
  border-radius: 0;
  border: 1px solid color-mix(in srgb, var(--danger) 34%, var(--line-strong));
  background: var(--danger-soft);
  box-shadow: inset 0 1px 2px rgba(90, 40, 30, 0.12);
  color: var(--danger);
  font-size: 12px;
}

.dl-panel__foot {
  display: flex;
  justify-content: flex-end;
  gap: 10px;
  padding-top: 4px;
}

/* 悬浮球 */
.dl-ball-wrap {
  position: fixed;
  right: 24px;
  bottom: calc(var(--statusbar) + 20px);
  z-index: 92;
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 10px;
  animation: dl-ball-float 4.6s ease-in-out infinite;
}

.dl-ball {
  /* 与项目实体按键同一套材质：偏白正面 + 左上亮边 / 右下暗边，底边留厚度 */
  --face: var(--panel);
  --bevel-hi: color-mix(in srgb, var(--face) 52%, #fff);
  --bevel-lo: color-mix(in srgb, var(--face) 74%, #000);
  /* 水位配色，默认苔绿；失败 / 部分完成在下面覆盖 */
  --water-top: color-mix(in srgb, var(--moss) 46%, var(--moss-soft));
  --water-bottom: color-mix(in srgb, var(--moss) 30%, var(--moss-soft));
  --water-line: color-mix(in srgb, var(--moss-bright) 62%, #fff);
  position: relative;
  width: 56px;
  height: 56px;
  padding: 0;
  border-radius: 2px;
  border-style: solid;
  border-width: 1px 1px 3px;
  border-top-color: var(--bevel-hi);
  border-left-color: var(--bevel-hi);
  border-right-color: var(--bevel-lo);
  border-bottom-color: var(--bevel-lo);
  background: var(--face);
  box-shadow: 0 12px 26px rgba(40, 28, 16, 0.24), 0 2px 4px rgba(40, 28, 16, 0.12);
  transition: transform var(--dur-2) var(--ease-out-expo), box-shadow var(--dur-2) ease,
    background 150ms ease;
}

.dl-ball:hover {
  transform: translateY(-3px);
  box-shadow: 0 18px 32px rgba(40, 28, 16, 0.28);
}

.dl-ball:active {
  transform: translateY(2px);
  border-top-color: var(--bevel-lo);
  border-left-color: var(--bevel-lo);
  border-right-color: var(--bevel-hi);
  border-bottom-color: var(--bevel-hi);
  box-shadow: inset 0 2px 4px rgba(0, 0, 0, 0.24);
}

/* 外侧柔光：下载中缓慢呼吸，完成/失败时静止 */
.dl-ball__glow {
  position: absolute;
  inset: -7px;
  z-index: -1;
  border-radius: 4px;
  background: radial-gradient(closest-side, rgba(var(--accent-rgb), 0.4), transparent 74%);
  filter: blur(7px);
  opacity: 0.3;
  pointer-events: none;
}

.dl-ball--busy .dl-ball__glow {
  animation: dl-ball-breathe 2.6s ease-in-out infinite;
}

/* 内凹的方槽：进度水从槽底往上灌 */
.dl-ball__face {
  position: absolute;
  inset: 2px;
  overflow: hidden;
  border-radius: 1px;
  background: var(--panel-2);
  box-shadow: inset 0 2px 4px rgba(70, 48, 26, 0.16),
    inset 0 0 0 1px color-mix(in srgb, var(--line-strong) 55%, transparent);
}

.dl-ball__water {
  position: absolute;
  left: 0;
  right: 0;
  bottom: 0;
  height: var(--fill, 0%);
  background: linear-gradient(180deg, var(--water-top), var(--water-bottom));
  transition: height 360ms var(--ease-out-quart), background 240ms ease;
}

/* 水线：一道受光细边，像水面反光 */
.dl-ball__water::before {
  content: "";
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  height: 2px;
  background: var(--water-line);
}

.dl-ball--busy .dl-ball__water::before {
  animation: dl-ball-wave 2.2s ease-in-out infinite;
}

/* 表面扫过的高光，让方块看起来是玻璃材质 */
.dl-ball__sheen {
  position: absolute;
  top: -45%;
  left: -70%;
  width: 48%;
  height: 190%;
  transform: rotate(20deg);
  background: linear-gradient(90deg, transparent, rgba(255, 255, 255, 0.42), transparent);
  pointer-events: none;
  animation: dl-ball-sheen 3.4s ease-in-out infinite;
}

/* 图标与百分比浮在水面之上 */
.dl-ball__content {
  position: absolute;
  inset: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 1px;
  color: var(--walnut-deep);
  text-shadow: 0 1px 0 rgba(255, 255, 255, 0.6);
}

.dl-ball__icon {
  display: flex;
  align-items: center;
  justify-content: center;
  color: inherit;
}

.dl-ball--busy .dl-ball__icon {
  animation: dl-ball-icon 1.8s ease-in-out infinite;
}

.dl-ball__pct {
  font-size: 10.5px;
  font-weight: 700;
  line-height: 1.1;
}

.dl-ball--done {
  animation: dl-ball-pop 520ms var(--ease-out-expo);
}

.dl-ball--done .dl-ball__content {
  color: var(--moss);
}

.dl-ball--failed {
  --water-top: color-mix(in srgb, var(--danger) 40%, var(--danger-soft));
  --water-bottom: color-mix(in srgb, var(--danger) 26%, var(--danger-soft));
  --water-line: color-mix(in srgb, var(--danger) 24%, #fff);
  animation: dl-ball-shake 460ms ease;
}

.dl-ball--failed .dl-ball__content {
  color: var(--danger);
}

.dl-ball--partial {
  --water-top: color-mix(in srgb, var(--amber) 44%, var(--panel-2));
  --water-bottom: color-mix(in srgb, var(--amber) 28%, var(--panel-2));
  --water-line: color-mix(in srgb, var(--amber) 34%, #fff);
}

.dl-ball--partial .dl-ball__content {
  color: color-mix(in srgb, var(--amber) 62%, var(--walnut-deep));
}

.dl-ball__dismiss {
  width: 22px;
  height: 22px;
  border-radius: 2px;
  background: var(--panel);
  border-style: solid;
  border-width: 1px 1px 2px;
  border-top-color: color-mix(in srgb, var(--panel) 52%, #fff);
  border-left-color: color-mix(in srgb, var(--panel) 52%, #fff);
  border-right-color: color-mix(in srgb, var(--panel) 74%, #000);
  border-bottom-color: color-mix(in srgb, var(--panel) 74%, #000);
  color: var(--muted);
  font-size: 14px;
  line-height: 1;
  display: grid;
  place-items: center;
  margin-right: 4px;
  box-shadow: 0 4px 10px rgba(40, 28, 16, 0.14);
  animation: dl-ball-pop 360ms var(--ease-out-expo);
}

.dl-ball__dismiss:hover {
  color: var(--danger);
  border-color: var(--danger);
}

@keyframes dl-ball-float {
  0%,
  100% {
    transform: translateY(0);
  }
  50% {
    transform: translateY(-4px);
  }
}

@keyframes dl-ball-breathe {
  0%,
  100% {
    opacity: 0.28;
    transform: scale(0.96);
  }
  50% {
    opacity: 0.72;
    transform: scale(1.06);
  }
}

@keyframes dl-ball-sheen {
  0% {
    left: -70%;
  }
  55%,
  100% {
    left: 120%;
  }
}

/* 水线轻微起伏，读作水面在晃 */
@keyframes dl-ball-wave {
  0%,
  100% {
    transform: translateY(0);
    opacity: 0.85;
  }
  50% {
    transform: translateY(-1px);
    opacity: 1;
  }
}

@keyframes dl-ball-icon {
  0%,
  100% {
    transform: translateY(1px);
  }
  50% {
    transform: translateY(-2px);
  }
}

@keyframes dl-ball-pop {
  0% {
    transform: scale(0.86);
  }
  55% {
    transform: scale(1.08);
  }
  100% {
    transform: scale(1);
  }
}

@keyframes dl-ball-shake {
  0%,
  100% {
    transform: translateX(0);
  }
  25% {
    transform: translateX(-4px);
  }
  50% {
    transform: translateX(4px);
  }
  75% {
    transform: translateX(-2px);
  }
}

.dl-tip {
  width: 260px;
  padding: 12px 13px;
  border-radius: 0;
  background: color-mix(in srgb, var(--ink) 94%, transparent);
  color: var(--paper);
  box-shadow: 0 14px 34px rgba(0, 0, 0, 0.24);
  border: 1px solid color-mix(in srgb, var(--paper) 12%, transparent);
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.dl-tip__head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  font-size: 12.5px;
}

.dl-tip__bar {
  height: 5px;
  border-radius: 0;
  background: color-mix(in srgb, var(--paper) 18%, transparent);
  overflow: hidden;
}

.dl-tip__bar span {
  display: block;
  height: 100%;
  border-radius: 0;
  background: linear-gradient(90deg, var(--moss-bright), var(--amber));
  transition: width 240ms var(--ease-out-quart);
}

.dl-tip__rows {
  display: flex;
  flex-wrap: wrap;
  gap: 4px 12px;
  font-size: 11px;
  color: color-mix(in srgb, var(--paper) 72%, transparent);
}

.dl-tip__err {
  font-size: 11px;
  color: color-mix(in srgb, var(--danger) 45%, var(--paper));
}

.dl-tip__open {
  height: 28px;
  border-radius: 0;
  background: color-mix(in srgb, var(--paper) 12%, transparent);
  color: var(--paper);
  font-size: 11.5px;
  border-style: solid;
  border-width: 1px 1px 2px;
  border-top-color: color-mix(in srgb, var(--paper) 22%, transparent);
  border-left-color: color-mix(in srgb, var(--paper) 22%, transparent);
  border-right-color: color-mix(in srgb, #000 26%, transparent);
  border-bottom-color: color-mix(in srgb, #000 34%, transparent);
  transition: background var(--dur-1) ease, transform var(--dur-1) ease;
}

.dl-tip__open:hover {
  background: color-mix(in srgb, var(--paper) 20%, transparent);
  transform: translateY(-1px);
}

.dl-tip__open:active {
  transform: translateY(1px);
  box-shadow: inset 0 2px 3px rgba(0, 0, 0, 0.35);
}

@keyframes dl-fade-in {
  from {
    opacity: 0;
  }
}

@keyframes dl-pop {
  from {
    opacity: 0;
    transform: translateY(16px) scale(0.97);
  }
}

.dl-fade-enter-active,
.dl-fade-leave-active {
  transition: opacity 160ms ease;
}

.dl-fade-enter-from,
.dl-fade-leave-to {
  opacity: 0;
}

.dl-tip-enter-active,
.dl-tip-leave-active {
  transition: opacity 160ms ease, transform 160ms var(--ease-out-expo);
}

.dl-tip-enter-from,
.dl-tip-leave-to {
  opacity: 0;
  transform: translateY(6px);
}
</style>
