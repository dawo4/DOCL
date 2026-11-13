<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { openFolder } from "../api";
import AppIcon from "../components/AppIcon.vue";
import IconButton from "../components/IconButton.vue";
import NewsPanel from "../components/NewsPanel.vue";
import PlayerAvatar from "../components/PlayerAvatar.vue";
import SkinHead from "../components/SkinHead.vue";
import {
  launchVersion,
  openInstance,
  selectedAccount,
  selectedVersion,
  selectVersion,
  stopGame,
  store,
} from "../store";
import { kindLabel } from "../utils";

const version = selectedVersion;
const account = selectedAccount;

const pickerOpen = ref(false);
const pickerRoot = ref<HTMLElement | null>(null);

const isInstalling = computed(
  () => !!store.installingId && store.installingId === version.value?.id
);

const versionLabel = computed(() => {
  const current = version.value;
  if (!current) return "";
  const name = store.metas[current.id]?.display_name;
  return name && name.trim() ? name : current.id;
});

const skinSource = computed(
  () => store.accountSkin?.data_url || store.defaultSkin
);

const javaLabel = computed(() => {
  const v = version.value;
  if (!v) return "—";
  const java = store.javaList.find((j) => j.major === v.java_major);
  return java ? `Java ${java.major}` : `缺少 Java ${v.java_major}`;
});

const allocatedGb = computed(() => {
  const mb = store.config?.memory_mb ?? 0;
  return mb ? (mb / 1024).toFixed(1) : "—";
});

const statusLabel = computed(() => {
  if (store.gameRunning) return "游戏运行中";
  if (store.installingId) return `正在安装 ${store.installingId}`;
  return version.value ? "已就绪" : "未安装版本";
});

const accountType = computed(() => {
  if (!account.value) return "未登录";
  return account.value.kind === "microsoft" ? "微软账号" : "离线账号";
});

function openGameDir() {
  if (store.config?.game_dir) openFolder(store.config.game_dir);
}

function pickVersion(id: string) {
  selectVersion(id);
  pickerOpen.value = false;
}

function versionName(id: string) {
  const name = store.metas[id]?.display_name;
  return name && name.trim() ? name : id;
}

function onDocClick(event: MouseEvent) {
  if (pickerRoot.value && !pickerRoot.value.contains(event.target as Node)) {
    pickerOpen.value = false;
  }
}

onMounted(() => document.addEventListener("mousedown", onDocClick));
onBeforeUnmount(() => document.removeEventListener("mousedown", onDocClick));
</script>

<template>
  <div class="home">
    <div class="home__inner">
      <div class="home__top">
        <div class="home__heading">
          <span class="home__title">启动台</span>
          <span class="home__sub">
            {{ version ? `${version.id} · ${kindLabel(version.kind)}` : "还没有可用版本" }}
          </span>
        </div>
        <span class="home__state" :class="{ 'home__state--busy': store.gameRunning }">
          <span class="home__dot" />
          {{ statusLabel }}
        </span>
      </div>

      <div class="home__grid">
        <section class="cover" aria-label="当前版本">
          <div class="cover__art">
            <div class="cover__figure">
              <SkinHead
                :skin="skinSource"
                :seed="account?.uuid ?? 'steve'"
                :height="168"
              />
            </div>
          </div>

          <div class="cover__meta">
            <strong class="cover__name">
              {{ version ? versionLabel : "空白存档" }}
            </strong>
            <span class="cover__id mono">
              {{ version ? version.id : "去下载页装一个版本" }}
            </span>
            <span class="cover__facts mono">
              {{ allocatedGb }} GB 内存 · {{ javaLabel }} · {{ store.installed.length }} 个版本
            </span>
          </div>

          <button v-if="!version" class="play" @click="store.page = 'download'">
            <AppIcon name="download" :size="16" />
            去下载版本
          </button>
          <button
            v-else
            class="play"
            :class="{ 'play--stop': store.gameRunning }"
            :disabled="isInstalling"
            @click="store.gameRunning ? stopGame() : launchVersion()"
          >
            <AppIcon :name="store.gameRunning ? 'stop' : 'play'" :size="16" />
            {{ store.gameRunning ? "停止游戏" : "启动游戏" }}
          </button>

          <div class="cover__row">
            <div ref="pickerRoot" class="vpick">
              <button class="vpick__trigger" @click="pickerOpen = !pickerOpen">
                <span class="vpick__text">
                  选择版本
                  <em>{{ store.installed.length }} 个可用</em>
                </span>
                <AppIcon
                  name="chevron-down"
                  :size="14"
                  class="vpick__chev"
                  :class="{ 'vpick__chev--open': pickerOpen }"
                />
              </button>

              <Transition name="drop">
                <div v-if="pickerOpen" class="vpick__menu">
                  <button
                    v-for="item in store.installed"
                    :key="item.id"
                    class="vpick__item"
                    :class="{ 'vpick__item--active': item.id === version?.id }"
                    @click="pickVersion(item.id)"
                  >
                    <span class="vpick__name">{{ versionName(item.id) }}</span>
                    <span class="vpick__kind">{{ kindLabel(item.kind) }}</span>
                    <AppIcon
                      v-if="item.id === version?.id"
                      name="check"
                      :size="13"
                      class="vpick__check"
                    />
                  </button>
                  <button class="vpick__manage" @click="store.page = 'versions'">
                    <AppIcon name="layers" :size="14" />
                    管理版本
                  </button>
                </div>
              </Transition>
            </div>

            <IconButton
              v-if="version"
              icon="cube"
              label="版本设置"
              variant="soft"
              :size="38"
              @click="openInstance(version.id)"
            />
          </div>

          <div class="cover__foot">
            <button class="account" @click="store.page = 'accounts'">
              <PlayerAvatar :account="account" :size="26" :radius="7" />
              <span class="account__text">
                <strong>{{ account?.name ?? "未登录" }}</strong>
                <span>{{ accountType }}</span>
              </span>
            </button>
            <div class="cover__quick">
              <IconButton icon="folder" label="打开游戏目录" variant="soft" :size="36" @click="openGameDir" />
              <IconButton icon="layers" label="版本管理" variant="soft" :size="36" @click="store.page = 'versions'" />
              <IconButton icon="download" label="下载中心" variant="soft" :size="36" @click="store.page = 'download'" />
              <IconButton icon="settings" label="设置" variant="soft" :size="36" @click="store.page = 'settings'" />
            </div>
          </div>
        </section>

        <section class="feed" aria-label="动态">
          <NewsPanel />
        </section>
      </div>
    </div>
  </div>
</template>

<style scoped>
.home {
  position: relative;
  height: 100%;
  max-width: none;
  padding: 18px 26px 22px;
  gap: 0;
  overflow: hidden;
}

.home__inner {
  position: relative;
  z-index: 1;
  display: flex;
  flex-direction: column;
  gap: 12px;
  height: 100%;
  max-width: 1240px;
  margin: 0 auto;
}

.home__top {
  display: flex;
  align-items: flex-end;
  justify-content: space-between;
  gap: 16px;
  flex-shrink: 0;
}

.home__heading {
  display: flex;
  align-items: baseline;
  gap: 10px;
  min-width: 0;
}

.home__title {
  font-size: 17px;
  font-weight: 800;
  letter-spacing: -0.01em;
  color: var(--walnut-deep);
}

.home__sub {
  font-family: var(--font-mono);
  font-size: 10px;
  letter-spacing: 0.05em;
  color: var(--muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.home__state {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  padding: 4px 10px;
  border: 1px solid var(--line);
  border-radius: 0;
  font-size: 11px;
  color: var(--ink-soft);
  background: rgba(255, 255, 255, 0.03);
  flex-shrink: 0;
}

.home__dot {
  width: 6px;
  height: 6px;
  border-radius: 0;
  background: var(--moss-bright);
  box-shadow: 0 0 8px rgba(var(--accent-rgb), 0.55);
}

.home__state--busy .home__dot {
  background: var(--amber);
  box-shadow: 0 0 8px rgba(201, 162, 106, 0.6);
}

.home__grid {
  display: grid;
  grid-template-columns: 362px minmax(0, 1fr);
  gap: 14px;
  flex: 1;
  min-height: 0;
}

/* ---------- 左栏：当前版本 ---------- */
.cover {
  display: flex;
  flex-direction: column;
  gap: 10px;
  min-height: 0;
  padding: 16px;
  /* 启动台主卡：和按键同源的 2px 浮雕外框 */
  border: 2px solid var(--frame);
  border-top-color: color-mix(in srgb, var(--frame) 66%, #fff);
  border-left-color: color-mix(in srgb, var(--frame) 66%, #fff);
  border-right-color: color-mix(in srgb, var(--frame) 74%, #000);
  border-bottom-color: color-mix(in srgb, var(--frame) 74%, #000);
  border-radius: var(--radius);
  background: linear-gradient(180deg, var(--panel-2), var(--panel));
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--panel) 46%, #fff);
  overflow: hidden;
}

.cover__art {
  position: relative;
  display: flex;
  align-items: center;
  justify-content: center;
  flex: 1;
  min-height: 120px;
  border-radius: var(--radius-sm);
  overflow: hidden;
}

.cover__figure {
  position: relative;
  z-index: 1;
  transform: translateY(2px);
  filter: drop-shadow(0 14px 20px rgba(0, 0, 0, 0.4));
}

.cover__meta {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
  flex-shrink: 0;
}

.cover__name {
  font-size: 19px;
  font-weight: 800;
  line-height: 1.2;
  letter-spacing: -0.01em;
  color: var(--walnut-deep);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.cover__id {
  font-size: 10px;
  letter-spacing: 0.04em;
  color: var(--muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.cover__facts {
  margin-top: 2px;
  font-size: 10px;
  letter-spacing: 0.03em;
  color: var(--ink-soft);
}

/* 主操作：唯一高饱和元素 */
.play {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  width: 100%;
  height: 44px;
  flex-shrink: 0;
  border: 1px solid var(--btn-border);
  border-bottom-width: 5px;
  border-radius: 2px;
  background: var(--btn-b);
  border-bottom-color: color-mix(in srgb, var(--btn-b) 56%, #000);
  color: var(--btn-fg);
  font-size: 14px;
  font-weight: 800;
  letter-spacing: 0.04em;
  text-shadow: 0 1px 0 rgba(0, 0, 0, 0.32);
  cursor: pointer;
  transition:
    background var(--dur-1) ease,
    border-color var(--dur-1) ease,
    transform var(--dur-1) ease;
}

.play:hover:not(:disabled) {
  background: var(--btn-a);
  border-bottom-color: color-mix(in srgb, var(--btn-a) 56%, #000);
}

.play:active:not(:disabled) {
  transform: translateY(4px);
  border-bottom-width: 1px;
}

.play:disabled {
  opacity: 0.55;
  cursor: default;
}

.play--stop {
  border-color: #6d3a33;
  border-bottom-color: #7c382e;
  background: #bd5c4e;
  color: #fff5f2;
}

.play--stop:hover:not(:disabled) {
  background: #c96a5b;
  border-bottom-color: #7c382e;
}

.cover__row {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
}

/* 版本选择 */
.vpick {
  position: relative;
  flex: 1;
  min-width: 0;
}

.vpick__trigger {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  width: 100%;
  height: 38px;
  padding: 0 11px;
  border: 1px solid var(--line);
  border-radius: var(--radius-sm);
  background: rgba(255, 255, 255, 0.03);
  color: var(--ink-soft);
  cursor: pointer;
  transition:
    border-color var(--dur-2) ease,
    background var(--dur-2) ease;
}

.vpick__trigger:hover {
  border-color: var(--line-strong);
  background: rgba(255, 255, 255, 0.055);
}

.vpick__text {
  display: flex;
  align-items: baseline;
  gap: 7px;
  font-size: 11.5px;
}

.vpick__text em {
  font-style: normal;
  font-family: var(--font-mono);
  font-size: 9px;
  color: var(--muted);
}

.vpick__chev {
  color: var(--muted);
  transition: transform var(--dur-2) var(--ease-out-expo);
}

.vpick__chev--open {
  transform: rotate(180deg);
}

.vpick__menu {
  position: absolute;
  left: 0;
  right: 0;
  bottom: calc(100% + 8px);
  z-index: 30;
  max-height: 280px;
  overflow-y: auto;
  padding: 5px;
  border: 2px solid var(--frame);
  border-top-color: color-mix(in srgb, var(--frame) 62%, #fff);
  border-left-color: color-mix(in srgb, var(--frame) 62%, #fff);
  border-right-color: color-mix(in srgb, var(--frame) 76%, #000);
  border-bottom-color: color-mix(in srgb, var(--frame) 76%, #000);
  border-radius: var(--radius-sm);
  background: var(--panel-2);
  box-shadow: 0 18px 40px rgba(0, 0, 0, 0.42);
}

.vpick__item {
  display: flex;
  align-items: center;
  gap: 9px;
  width: 100%;
  height: 34px;
  padding: 0 9px;
  border-radius: 3px;
  color: var(--ink);
  text-align: left;
  transition: background var(--dur-1) ease;
}

.vpick__item:hover {
  background: rgba(255, 255, 255, 0.05);
}

.vpick__item--active {
  background: var(--moss-soft);
}

.vpick__name {
  flex: 1;
  font-size: 12.5px;
  font-weight: 700;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.vpick__kind {
  font-size: 10px;
  color: var(--muted);
}

.vpick__check {
  color: var(--moss-bright);
}

.vpick__manage {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  height: 32px;
  margin-top: 4px;
  padding: 0 9px;
  border-top: 1px solid var(--line);
  color: var(--muted);
  font-size: 11.5px;
  text-align: left;
  transition: color var(--dur-1) ease, background var(--dur-1) ease;
}

.vpick__manage:hover {
  color: var(--moss-bright);
  background: rgba(255, 255, 255, 0.04);
}

/* 底部：账号与快捷操作 */
.cover__foot {
  display: flex;
  align-items: center;
  gap: 10px;
  padding-top: 10px;
  border-top: 1px solid var(--line);
  flex-shrink: 0;
}

.account {
  display: flex;
  align-items: center;
  gap: 9px;
  flex: 1;
  min-width: 0;
  padding: 5px 8px;
  border: 1px solid transparent;
  border-radius: var(--radius-sm);
  text-align: left;
  cursor: pointer;
  transition:
    background var(--dur-2) ease,
    border-color var(--dur-2) ease;
}

.account:hover {
  background: rgba(255, 255, 255, 0.045);
  border-color: var(--line);
}

.account__text {
  display: flex;
  flex-direction: column;
  gap: 1px;
  min-width: 0;
}

.account__text strong {
  font-size: 12.5px;
  font-weight: 700;
  color: var(--walnut-deep);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.account__text span {
  font-size: 10px;
  color: var(--muted);
}

.cover__quick {
  display: flex;
  gap: 6px;
  flex-shrink: 0;
}

/* ---------- 右栏：动态 ---------- */
.feed {
  display: flex;
  min-height: 0;
  overflow: hidden;
}

.feed :deep(.news) {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-height: 0;
  margin: 0;
  padding: 14px 16px;
  border: 2px solid var(--frame);
  border-top-color: color-mix(in srgb, var(--frame) 66%, #fff);
  border-left-color: color-mix(in srgb, var(--frame) 66%, #fff);
  border-right-color: color-mix(in srgb, var(--frame) 74%, #000);
  border-bottom-color: color-mix(in srgb, var(--frame) 74%, #000);
  border-radius: var(--radius);
  background: var(--panel);
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--panel) 46%, #fff);
  overflow: hidden;
}

.feed :deep(.news__body) {
  min-height: 0;
}

/* ---------- 响应式 ---------- */
@media (max-width: 980px) {
  .home {
    padding: 16px 18px 18px;
  }

  .home__grid {
    grid-template-columns: 320px minmax(0, 1fr);
  }
}
</style>
