<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { api, openExternal, saveSkinDialog } from "../api";
import AppIcon from "../components/AppIcon.vue";
import IconButton from "../components/IconButton.vue";
import PlayerAvatar from "../components/PlayerAvatar.vue";
import SkinModel from "../components/SkinModel.vue";
import WoodButton from "../components/WoodButton.vue";
import WoodInput from "../components/WoodInput.vue";
import WoodModal from "../components/WoodModal.vue";
import SkinEditorView from "./SkinEditorView.vue";
import {
  beginMsLogin,
  createOffline,
  loadSelectedSkin,
  notify,
  refreshAccounts,
  refreshConfig,
  removeAccount,
  selectAccount,
  selectedAccount,
  store,
} from "../store";

const name = ref("");
const confirmId = ref<string | null>(null);
/// 添加账户弹窗：type 选择 → 离线表单，正版直接走微软登录
const addOpen = ref(false);
const offlineOpen = ref(false);

/// 第三方皮肤站（authlib-injector）登录
const thirdOpen = ref(false);
const thirdRoot = ref("https://littleskin.cn/api/yggdrasil");
const thirdUser = ref("");
const thirdPass = ref("");
const thirdBusy = ref(false);
const thirdServerName = ref("");

/// 常用皮肤站预设，点一下填入地址
const thirdPresets = [
  { label: "LittleSkin", root: "https://littleskin.cn/api/yggdrasil" },
];

async function checkThirdServer() {
  if (!thirdRoot.value.trim()) {
    notify("请填写认证服务器地址", "err");
    return;
  }
  try {
    const meta = await api.authlibMeta(thirdRoot.value.trim());
    thirdRoot.value = meta.root;
    thirdServerName.value = meta.name;
    notify(`已连接：${meta.name}`);
  } catch (err) {
    thirdServerName.value = "";
    notify(String(err), "err");
  }
}

async function loginThird() {
  if (thirdBusy.value) return;
  if (!thirdRoot.value.trim() || !thirdUser.value.trim() || !thirdPass.value) {
    notify("请填写服务器地址、邮箱与密码", "err");
    return;
  }
  thirdBusy.value = true;
  try {
    const account = await api.authlibLogin(
      thirdRoot.value.trim(),
      thirdUser.value.trim(),
      thirdPass.value
    );
    await refreshAccounts();
    await refreshConfig();
    thirdOpen.value = false;
    thirdPass.value = "";
    notify(`已登录 ${account.name}`);
  } catch (err) {
    notify(String(err), "err");
  } finally {
    thirdBusy.value = false;
  }
}

const loginVisible = computed(() => store.msLogin !== null);
const code = computed(() => store.msLogin?.code ?? null);
const status = computed(() => store.msLogin?.status ?? "loading");

const skinSource = computed(() => store.accountSkin?.data_url ?? "");
const skinModel = computed(
  () => store.accountSkin?.model ?? selectedAccount.value?.skin_model ?? "classic"
);
const skinSourceLabel = computed(() => {
  switch (store.accountSkin?.source) {
    case "local":
      return "本地自定义";
    case "official":
      return "Mojang 官方";
    case "authlib":
      return "皮肤站";
    default:
      return "默认皮肤";
  }
});

/// 截图/录屏时把 UUID 换成圆点，避免公开真实账号标识
const privacy = computed(() => import.meta.env.DEV && store.maskIds);

function shortUuid(uuid: string) {
  return privacy.value ? "••••••••" : uuid.slice(0, 8);
}

function fullUuid(uuid: string) {
  return privacy.value ? "••••••••••••••••••••••••••••••••" : uuid;
}

function addOffline() {
  const value = name.value.trim();
  if (!value) {
    notify("请输入离线用户名", "err");
    return;
  }
  createOffline(value);
  name.value = "";
  offlineOpen.value = false;
  notify(`已添加离线账号 ${value}`);
}

function pickAccountType(kind: "offline" | "microsoft" | "authlib") {
  addOpen.value = false;
  if (kind === "offline") {
    offlineOpen.value = true;
  } else if (kind === "authlib") {
    thirdOpen.value = true;
  } else {
    beginMsLogin();
  }
}

function requestDelete(id: string) {
  if (confirmId.value === id) {
    confirmId.value = null;
    removeAccount(id);
  } else {
    confirmId.value = id;
    window.setTimeout(() => {
      if (confirmId.value === id) confirmId.value = null;
    }, 3000);
  }
}

async function pickAccount(id: string) {
  if (id === selectedAccount.value?.id) return;
  await selectAccount(id);
}

async function copyCode() {
  if (!code.value) return;
  try {
    await navigator.clipboard.writeText(code.value.user_code);
    notify("验证码已复制");
  } catch {
    notify("复制失败，请手动选中复制", "err");
  }
}

function openLink() {
  if (code.value) openExternal(code.value.verification_uri);
}

function closeLogin() {
  store.msLogin = null;
}

async function refreshOfficial() {
  await loadSelectedSkin();
  notify("已重新获取皮肤");
}

async function exportSkin() {
  const account = selectedAccount.value;
  if (!account) return;
  if (!store.accountSkin?.data_url) {
    notify("当前没有可导出的皮肤", "err");
    return;
  }
  const path = await saveSkinDialog(`${account.name}-skin.png`);
  if (!path) return;
  try {
    await api.saveAccountSkin(account.id, store.accountSkin.data_url, skinModel.value);
    await api.exportAccountSkin(account.id, path);
    await refreshAccounts();
    await loadSelectedSkin();
    notify("皮肤已导出");
  } catch (err) {
    notify(`导出失败：${String(err)}`, "err");
  }
}

async function clearLocalSkin() {
  const account = selectedAccount.value;
  if (!account) return;
  try {
    await api.clearAccountSkin(account.id);
    await refreshAccounts();
    await loadSelectedSkin();
    notify("已清除本地皮肤覆盖");
  } catch (err) {
    notify(`清除失败：${String(err)}`, "err");
  }
}

function editSkin() {
  if (!selectedAccount.value) return;
  store.skinEditorOpen = true;
}

function closeEditor() {
  store.skinEditorOpen = false;
  void loadSelectedSkin();
}

watch(
  () => selectedAccount.value?.id,
  () => {
    void loadSelectedSkin();
  }
);

watch(
  () => store.msLogin?.status,
  (value) => {
    if (value === "ok") {
      window.setTimeout(() => {
        store.msLogin = null;
      }, 1400);
    }
  }
);

onMounted(() => {
  if (!store.accountSkin && selectedAccount.value) void loadSelectedSkin();
});
</script>

<template>
  <div class="accounts-root">
    <SkinEditorView
      v-if="store.skinEditorOpen && selectedAccount"
      :account="selectedAccount"
      @close="closeEditor"
    />

    <div v-else class="page">
    <div class="accounts-frame card">
      <section class="acc-list">
        <div class="acc-list__head">
          <span class="acc-list__title">账号</span>
          <span class="acc-list__count mono">{{ store.accounts.length }}</span>
        </div>

        <div v-if="!store.accounts.length" class="acc-empty">
          还没有账号，先在下方创建或登录
        </div>

        <div v-else class="acc-rows">
          <button
            v-for="(account, index) in store.accounts"
            :key="account.id"
            class="acc-row"
            :class="{ 'acc-row--active': account.id === selectedAccount?.id }"
            :style="{ '--i': index }"
            @click="pickAccount(account.id)"
          >
            <PlayerAvatar :account="account" :size="34" :radius="2" />
            <span class="acc-row__body">
              <span class="acc-row__name">{{ account.name }}</span>
              <span class="acc-row__meta mono">
                {{
                  account.kind === "microsoft"
                    ? "MSA"
                    : account.kind === "authlib"
                      ? "SKIN"
                      : "OFFLINE"
                }} ·
                {{ shortUuid(account.uuid) }}
              </span>
            </span>
            <IconButton
              icon="trash"
              class="acc-del"
              :label="confirmId === account.id ? '确认移除账号' : '移除账号'"
              variant="ghost"
              :size="26"
              @click.stop="requestDelete(account.id)"
            />
          </button>
        </div>

        <div class="acc-add">
          <WoodButton variant="primary" block sheen @click="addOpen = true">
            <AppIcon name="plus" :size="15" />
            添加账户
          </WoodButton>
        </div>
      </section>

      <section v-if="selectedAccount" class="acc-detail">
        <header class="acc-detail__head">
          <div class="acc-detail__id">
            <span class="acc-detail__label mono">当前账号</span>
            <h2 class="acc-detail__name">{{ selectedAccount.name }}</h2>
          </div>
          <div class="acc-detail__tags">
            <span class="tag" :class="{ 'tag--grass': selectedAccount.kind === 'microsoft' }">
              {{
                selectedAccount.kind === "microsoft"
                  ? "微软账号"
                  : selectedAccount.kind === "authlib"
                    ? "第三方账号"
                    : "离线账号"
              }}
            </span>
            <span class="tag">{{ skinSourceLabel }}</span>
          </div>
        </header>

        <div class="acc-detail__uuid mono">{{ fullUuid(selectedAccount.uuid) }}</div>

        <div class="acc-model">
          <div v-if="store.skinLoading" class="acc-model__loading">
            <span class="skeleton" />
            <span class="muted small">正在获取皮肤…</span>
          </div>
          <SkinModel
            v-else
            :skin="skinSource"
            :seed="selectedAccount.uuid"
            :model="skinModel"
            :height="360"
          />
        </div>

        <div class="acc-actions">
          <WoodButton variant="primary" size="sm" sheen @click="editSkin">
            <AppIcon name="image" :size="14" />
            编辑皮肤
          </WoodButton>
          <WoodButton variant="soft" size="sm" @click="exportSkin">
            <AppIcon name="download" :size="14" />
            导出 PNG
          </WoodButton>
          <WoodButton
            v-if="selectedAccount.kind !== 'offline'"
            variant="soft"
            size="sm"
            @click="refreshOfficial"
          >
            <AppIcon name="refresh" :size="14" />
            刷新皮肤
          </WoodButton>
          <WoodButton
            v-if="selectedAccount.kind !== 'offline' && store.accountSkin?.source === 'local'"
            variant="ghost"
            size="sm"
            class="acc-actions__danger"
            @click="clearLocalSkin"
          >
            <AppIcon name="trash" :size="13" />
            清除覆盖
          </WoodButton>
        </div>

        <p class="acc-note">
          离线账号的皮肤只用于启动器内预览与导出，游戏内原版不会显示；第三方账号会自动读取皮肤站上的角色皮肤；微软账号可在编辑页选择上传到 Mojang。
        </p>
      </section>

      <section v-else class="acc-detail acc-detail--empty">
        选择或创建一个账号后，这里会显示 3D 皮肤
      </section>
    </div>

    <WoodModal v-if="addOpen" title="添加账户" :width="440" @close="addOpen = false">
      <p class="add-type__hint muted">选择要添加的账户类型。</p>
      <button class="add-type" @click="pickAccountType('offline')">
        <AppIcon name="user" :size="22" />
        <span class="add-type__body">
          <b>离线账户</b>
          <span>使用本地用户名进入游戏。</span>
        </span>
      </button>
      <button class="add-type" @click="pickAccountType('microsoft')">
        <AppIcon name="star" :size="22" />
        <span class="add-type__body">
          <b>正版账户</b>
          <span>通过 Microsoft 账户登录并获取皮肤头像。</span>
        </span>
      </button>
      <button class="add-type" @click="pickAccountType('authlib')">
        <AppIcon name="globe" :size="22" />
        <span class="add-type__body">
          <b>第三方账号</b>
          <span>LittleSkin 等皮肤站，用邮箱密码登录。</span>
        </span>
      </button>
    </WoodModal>

    <WoodModal v-if="thirdOpen" title="第三方账号" :width="460" @close="thirdOpen = false">
      <p class="offline-hint muted">
        通过 authlib-injector 接入皮肤站，登录后启动游戏会自动带上认证参数。
      </p>
      <div class="third-presets">
        <button
          v-for="preset in thirdPresets"
          :key="preset.root"
          class="third-preset"
          :class="{ 'third-preset--active': thirdRoot === preset.root }"
          @click="thirdRoot = preset.root"
        >
          {{ preset.label }}
        </button>
      </div>
      <label class="offline-field">
        <span class="offline-field__label">认证服务器</span>
        <div class="third-server">
          <WoodInput v-model="thirdRoot" mono placeholder="https://example.com/api/yggdrasil" />
          <WoodButton variant="ghost" @click="checkThirdServer">校验</WoodButton>
        </div>
        <span v-if="thirdServerName" class="offline-field__hint muted small">
          已连接 {{ thirdServerName }}
        </span>
      </label>
      <label class="offline-field">
        <span class="offline-field__label">邮箱</span>
        <WoodInput v-model="thirdUser" placeholder="登录皮肤站用的邮箱" />
      </label>
      <label class="offline-field">
        <span class="offline-field__label">密码</span>
        <WoodInput v-model="thirdPass" type="password" placeholder="皮肤站密码" @keyup.enter="loginThird" />
      </label>
      <div class="offline-actions">
        <WoodButton variant="ghost" @click="thirdOpen = false">取消</WoodButton>
        <WoodButton variant="primary" sheen :disabled="thirdBusy" @click="loginThird">
          {{ thirdBusy ? "登录中…" : "登录" }}
        </WoodButton>
      </div>
    </WoodModal>

    <WoodModal v-if="offlineOpen" title="离线账户" :width="440" @close="offlineOpen = false">
      <p class="offline-hint muted">
        如果您尚未拥有正版 Minecraft，请始终优先考虑购入正版
        <a class="offline-hint__link" href="#" @click.prevent="openExternal('https://www.minecraft.net/zh-hans/store/minecraft-java-bedrock-edition-pc')">Minecraft</a>。
      </p>
      <label class="offline-field">
        <span class="offline-field__label">用户名</span>
        <WoodInput v-model="name" placeholder="例如 Steve" @keyup.enter="addOffline" />
      </label>
      <div class="offline-actions">
        <WoodButton variant="ghost" @click="offlineOpen = false">取消</WoodButton>
        <WoodButton variant="primary" sheen @click="addOffline">确定</WoodButton>
      </div>
    </WoodModal>

    <WoodModal v-if="loginVisible" title="微软账号登录" :width="470" @close="closeLogin">
      <div v-if="status === 'loading'" class="login-state">正在向微软申请登录码…</div>

      <div v-else-if="status === 'err'" class="login-state login-state--err">
        <p>{{ store.msLogin?.message }}</p>
        <WoodButton variant="soft" @click="beginMsLogin">重试</WoodButton>
      </div>

      <div v-else-if="status === 'ok'" class="login-state login-state--ok">
        登录成功，正在同步账号信息…
      </div>

      <div v-else class="login-code">
        <p class="muted">在浏览器中打开下面的地址，输入验证码：</p>
        <div class="login-code__row">
          <code class="login-code__code display">{{ code?.user_code }}</code>
          <IconButton icon="copy" label="复制验证码" variant="soft" :size="36" @click="copyCode" />
        </div>
        <p class="login-code__url mono">{{ code?.verification_uri }}</p>
        <WoodButton variant="primary" block sheen @click="openLink">
          <AppIcon name="external" :size="15" />
          打开登录页面
        </WoodButton>
        <p class="muted small">完成后请保持此窗口打开，启动器会自动完成登录。</p>
      </div>
    </WoodModal>
    </div>
  </div>
</template>

<style scoped>
.accounts-frame {
  display: grid;
  grid-template-columns: 300px minmax(0, 1fr);
  min-height: calc(100vh - 168px);
  border-radius: 3px;
  overflow: hidden;
}

.acc-list {
  display: flex;
  flex-direction: column;
  min-height: 0;
  border-right: 1px solid var(--line);
}

.acc-list__head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-shrink: 0;
  height: 42px;
  padding: 0 14px;
  border-bottom: 1px solid var(--line);
}

.acc-list__title {
  font-size: 13px;
  font-weight: 700;
  color: var(--walnut-deep);
}

.acc-list__count {
  font-size: 11px;
  color: var(--muted);
}

.acc-empty {
  padding: 28px 14px;
  color: var(--muted);
  font-size: 12px;
}

.acc-rows {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
}

.acc-row {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  padding: 9px 12px;
  border-bottom: 1px solid var(--line);
  text-align: left;
  transition: background var(--dur-1) ease;
}

.acc-row:hover {
  background: rgba(var(--accent-rgb), 0.05);
}

.acc-row--active {
  background: rgba(var(--accent-rgb), 0.08);
  box-shadow: inset 2px 0 0 var(--moss);
}

.acc-row__body {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 1px;
}

.acc-row__name {
  font-size: 12.5px;
  font-weight: 700;
  color: var(--walnut-deep);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.acc-row__meta {
  font-size: 9.5px;
  letter-spacing: 0.04em;
  color: var(--muted);
}

.acc-add {
  display: flex;
  flex-direction: column;
  gap: 8px;
  flex-shrink: 0;
  padding: 12px;
  border-top: 1px solid var(--line);
}

/* 删除按钮平时完全隐形，悬停行时才浮现红色 */
.acc-del {
  opacity: 0;
  color: var(--muted);
  transition: opacity var(--dur-1) ease, color var(--dur-1) ease, background var(--dur-1) ease;
}

.acc-row:hover .acc-del,
.acc-del:focus-visible {
  opacity: 1;
}

.acc-del:hover:not(:disabled) {
  color: var(--danger);
  background: rgba(190, 92, 70, 0.12);
}

/* ---------- 添加账户弹窗 ---------- */

.add-type__hint {
  margin-bottom: 10px;
  font-size: 12.5px;
}

.add-type {
  display: flex;
  align-items: center;
  gap: 12px;
  width: 100%;
  padding: 13px 14px;
  margin-bottom: 8px;
  border: 1px solid var(--line);
  border-radius: 4px;
  background: rgba(var(--glass-rgb), 0.4);
  color: var(--ink);
  text-align: left;
  transition: border-color var(--dur-1) ease, background var(--dur-1) ease;
}

.add-type:hover {
  border-color: var(--moss);
  background: rgba(var(--accent-rgb), 0.07);
}

.add-type > svg {
  color: var(--walnut);
  flex-shrink: 0;
}

.add-type__body {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.add-type__body b {
  font-size: 13px;
  color: var(--walnut-deep);
}

.add-type__body span {
  font-size: 11.5px;
  color: var(--muted);
}

/* ---------- 离线账户表单 ---------- */

.offline-hint {
  margin-bottom: 14px;
  font-size: 12.5px;
  line-height: 1.7;
}

.offline-hint__link {
  color: var(--moss);
  text-decoration: underline;
}

.offline-field {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.offline-field__label {
  font-size: 12px;
  font-weight: 600;
  color: var(--ink-soft);
}

.offline-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 18px;
}

/* ---------- 第三方皮肤站 ---------- */

.offline-field + .offline-field {
  margin-top: 12px;
}

.offline-field__hint {
  font-size: 11.5px;
}

.third-presets {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-bottom: 14px;
}

.third-preset {
  padding: 5px 12px;
  border: 1px solid var(--line);
  border-radius: 3px;
  background: var(--panel-2);
  color: var(--walnut);
  font-size: 12px;
  font-weight: 700;
  transition: border-color var(--dur-1) ease, background var(--dur-1) ease;
}

.third-preset:hover {
  border-color: var(--moss);
}

.third-preset--active {
  border-color: var(--moss);
  background: var(--moss-soft);
  color: var(--moss);
}

.third-server {
  display: flex;
  align-items: center;
  gap: 8px;
}

.third-server > :first-child {
  flex: 1;
  min-width: 0;
}

.acc-detail {
  display: flex;
  flex-direction: column;
  gap: 12px;
  min-width: 0;
  padding: 18px 20px;
}

.acc-detail--empty {
  display: grid;
  place-items: center;
  color: var(--muted);
  font-size: 12.5px;
}

.acc-detail__head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 16px;
}

.acc-detail__id {
  display: flex;
  flex-direction: column;
  gap: 3px;
  min-width: 0;
}

.acc-detail__label {
  font-size: 9.5px;
  letter-spacing: 0.14em;
  color: var(--muted);
}

.acc-detail__name {
  font-size: 26px;
  font-weight: 800;
  line-height: 1.1;
  letter-spacing: -0.02em;
  color: var(--walnut-deep);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.acc-detail__tags {
  display: flex;
  gap: 6px;
  flex-shrink: 0;
}

.acc-detail__uuid {
  padding-bottom: 8px;
  border-bottom: 1px solid var(--line);
  color: var(--muted);
  font-size: 10px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.acc-model {
  display: flex;
  align-items: center;
  justify-content: center;
  flex: 1;
  min-height: 260px;
}

/* BlockHelm 式：预览浮在页面上，不套框不铺底 */
.acc-model :deep(.skin-model) {
  border: none;
  border-radius: 0;
  background: transparent;
}

.acc-model__loading {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 10px;
  width: 100%;
  height: 360px;
  border: 1px solid var(--line);
  background: rgba(var(--glass-rgb), 0.3);
}

.acc-model__loading .skeleton {
  width: 120px;
  height: 160px;
}

.acc-actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
  padding-top: 10px;
  border-top: 1px solid var(--line);
}

/* 破坏性操作推到最右，平时和普通按钮一样轻，悬停才变红 */
.acc-actions__danger {
  margin-left: auto;
}

.acc-actions__danger:hover {
  color: var(--danger);
}

.acc-note {
  color: var(--muted);
  font-size: 11.5px;
  line-height: 1.6;
}

.login-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 14px;
  padding: 20px 6px;
  text-align: center;
  color: var(--ink-soft);
}

.login-state--err {
  color: var(--danger);
}

.login-state--ok {
  color: var(--moss);
  font-weight: 700;
}

.login-code {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.login-code__row {
  display: flex;
  align-items: center;
  gap: 10px;
}

.login-code__code {
  flex: 1;
  height: 54px;
  display: grid;
  place-items: center;
  border-radius: 4px;
  background: var(--panel-2);
  border: 1px dashed var(--line-strong);
  font-size: 26px;
  letter-spacing: 0.24em;
  color: var(--walnut-deep);
  user-select: text;
}

.login-code__url {
  font-size: 12px;
  color: var(--ink-soft);
  word-break: break-all;
  user-select: text;
}

@media (max-width: 1080px) {
  .accounts-frame {
    grid-template-columns: 264px minmax(0, 1fr);
  }
}
</style>
