<script setup lang="ts">
import { getCurrentWindow } from "@tauri-apps/api/window";
import AppIcon from "./AppIcon.vue";
import IconButton from "./IconButton.vue";
import { store, type Page } from "../store";

const items: { key: Page; label: string }[] = [
  { key: "home", label: "启动台" },
  { key: "versions", label: "版本" },
  { key: "download", label: "下载" },
  { key: "multiplayer", label: "联机" },
  { key: "accounts", label: "账号" },
  { key: "skins", label: "皮肤库" },
  { key: "settings", label: "设置" },
];

const win = getCurrentWindow();

function go(page: Page) {
  store.page = page;
}

function minimize() {
  win.minimize();
}
function toggleMaximize() {
  win.toggleMaximize();
}
function close() {
  win.close();
}
</script>

<template>
  <header class="topbar">
    <div class="topbar__brand">
      <span class="brand__mark" aria-hidden="true">
        <span class="brand__letter">D</span>
      </span>
      <strong class="brand__name">DOCL</strong>
    </div>

    <nav class="topnav">
      <button
        v-for="item in items"
        :key="item.key"
        class="topnav__item"
        :class="{ 'topnav__item--active': store.page === item.key }"
        @click="go(item.key)"
      >
        {{ item.label }}
      </button>
    </nav>

    <div class="topbar__drag" data-tauri-drag-region @dblclick="toggleMaximize" />

    <div class="topbar__right">
      <IconButton
        icon="log"
        label="运行日志"
        :active="store.logPanel"
        :size="32"
        @click="store.logPanel = true"
      />

      <div class="window-controls">
        <button class="window-btn" title="最小化" @click="minimize">
          <AppIcon name="minimize" :size="14" />
        </button>
        <button class="window-btn" title="最大化" @click="toggleMaximize">
          <AppIcon name="maximize" :size="14" />
        </button>
        <button class="window-btn window-btn--close" title="关闭" @click="close">
          <AppIcon name="close" :size="14" />
        </button>
      </div>
    </div>
  </header>
</template>

<style scoped>
.topbar {
  height: 54px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  gap: 8px;
  padding-left: 16px;
  background: var(--bar-a);
  border-bottom: 2px solid var(--frame);
  position: relative;
  z-index: 20;
}

.topbar__brand {
  display: flex;
  align-items: center;
  gap: 9px;
  height: 32px;
  padding-right: 16px;
  border-right: 1px solid var(--line);
  flex-shrink: 0;
}

/* 品牌标记：苔藓绿到木色的一块铜牌，中间压一个 D */
.brand__mark {
  width: 30px;
  height: 30px;
  display: grid;
  place-items: center;
  border-radius: 2px;
  background: linear-gradient(160deg, var(--moss) 0 46%, var(--walnut) 46% 100%);
  border: 1px solid var(--walnut-deep);
  box-shadow:
    0 2px 7px rgba(70, 48, 26, 0.26),
    0 1px 0 rgba(255, 255, 255, 0.35) inset;
  position: relative;
  flex-shrink: 0;
  transition: transform var(--dur-3) var(--ease-out-expo);
}

.brand__mark::after {
  content: "";
  position: absolute;
  inset: 4px;
  border: 1px solid rgba(255, 253, 246, 0.26);
  border-radius: 4px;
}

.brand__letter {
  position: relative;
  z-index: 1;
  font-family: var(--font-display);
  font-size: 16px;
  font-weight: 700;
  line-height: 1;
  color: #fff8ea;
  text-shadow: 0 1px 2px rgba(0, 0, 0, 0.38);
}

.topbar__brand:hover .brand__mark {
  transform: rotate(-3deg) scale(1.04);
}

.brand__name {
  font-family: var(--font-display);
  font-size: 17px;
  letter-spacing: 0.08em;
  color: var(--walnut-deep);
}

.topnav {
  display: flex;
  align-items: center;
  height: 100%;
  gap: 4px;
}

.topnav__item {
  position: relative;
  display: flex;
  align-items: center;
  height: 32px;
  padding: 0 14px;
  border-radius: 2px;
  border: 1px solid transparent;
  color: var(--muted);
  font-size: 13px;
  font-weight: 500;
  transition: color 150ms ease, background 150ms ease,
    border-color 150ms ease, transform 100ms ease, box-shadow 100ms ease;
}

.topnav__item:hover {
  color: var(--ink);
  background: rgba(var(--accent-rgb), 0.06);
  transform: translateY(-1px);
  border-top-color: color-mix(in srgb, var(--panel-2) 52%, #fff);
  border-left-color: color-mix(in srgb, var(--panel-2) 52%, #fff);
  border-right-color: color-mix(in srgb, var(--panel-2) 42%, #000);
  border-bottom-color: color-mix(in srgb, var(--panel-2) 42%, #000);
}

/* 选中态是一个压进面板的方块，和按钮、分段切换统一 */
.topnav__item--active,
.topnav__item--active:hover {
  color: var(--walnut-deep);
  font-weight: 700;
  background: var(--panel);
  border-top-color: color-mix(in srgb, var(--moss) 42%, #000);
  border-left-color: color-mix(in srgb, var(--moss) 42%, #000);
  border-right-color: color-mix(in srgb, var(--moss) 34%, #fff);
  border-bottom-color: color-mix(in srgb, var(--moss) 34%, #fff);
  box-shadow: inset 0 2px 3px rgba(0, 0, 0, 0.12);
}

.topbar__drag {
  flex: 1;
  height: 100%;
  min-width: 20px;
}

.topbar__right {
  display: flex;
  align-items: center;
  gap: 6px;
  height: 100%;
}

.window-controls {
  display: flex;
  align-items: stretch;
  gap: 0;
  height: 100%;
  margin-left: 2px;
}

.window-controls::before {
  content: "";
  width: 1px;
  height: 18px;
  margin: auto 6px;
  background: var(--line);
}

.window-btn {
  /* 点击区保持全宽，实体键面由 ::before 画在中心，和图标按钮同一种浮雕 */
  position: relative;
  isolation: isolate;
  width: 46px;
  height: 100%;
  display: grid;
  place-items: center;
  color: var(--ink-soft);
  transition: color var(--dur-1) ease;
}

.window-btn::before {
  --face: var(--panel-2);
  content: "";
  position: absolute;
  inset: 0;
  margin: auto;
  width: 28px;
  height: 28px;
  border-radius: 2px;
  border-style: solid;
  border-width: 1px 1px 2px;
  border-color: transparent;
  background: transparent;
  transition: background var(--dur-1) ease, border-color var(--dur-1) ease,
    transform var(--dur-1) ease, box-shadow var(--dur-1) ease;
  z-index: -1;
}

.window-btn:hover {
  color: var(--walnut-deep);
}

.window-btn:hover::before {
  background: var(--face);
  border-top-color: color-mix(in srgb, var(--face) 52%, #fff);
  border-left-color: color-mix(in srgb, var(--face) 52%, #fff);
  border-right-color: color-mix(in srgb, var(--face) 72%, #000);
  border-bottom-color: color-mix(in srgb, var(--face) 72%, #000);
  transform: translateY(-1px);
}

.window-btn:active::before {
  background: var(--face);
  transform: translateY(1px);
  border-top-color: color-mix(in srgb, var(--face) 72%, #000);
  border-left-color: color-mix(in srgb, var(--face) 72%, #000);
  border-right-color: color-mix(in srgb, var(--face) 52%, #fff);
  border-bottom-color: color-mix(in srgb, var(--face) 52%, #fff);
  border-bottom-width: 1px;
  box-shadow: inset 0 2px 3px rgba(0, 0, 0, 0.2);
}

.window-btn--close:hover {
  color: #fffdf6;
}

.window-btn--close:hover::before {
  --face: #b4523a;
  background: var(--face);
  border-top-color: color-mix(in srgb, var(--face) 62%, #fff);
  border-left-color: color-mix(in srgb, var(--face) 62%, #fff);
  border-right-color: color-mix(in srgb, var(--face) 66%, #000);
  border-bottom-color: color-mix(in srgb, var(--face) 66%, #000);
}

.window-btn--close:active::before {
  --face: #b4523a;
  background: #96432f;
  transform: translateY(1px);
  border-top-color: color-mix(in srgb, var(--face) 66%, #000);
  border-left-color: color-mix(in srgb, var(--face) 66%, #000);
  border-right-color: color-mix(in srgb, var(--face) 62%, #fff);
  border-bottom-color: color-mix(in srgb, var(--face) 62%, #fff);
  border-bottom-width: 1px;
  box-shadow: inset 0 2px 3px rgba(0, 0, 0, 0.24);
}

@media (max-width: 1040px) {
  .topnav__item {
    padding: 0 11px;
  }
}
</style>
