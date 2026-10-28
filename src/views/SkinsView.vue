<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import {
  api,
  openExternal,
  type AccountView,
  type CapeView,
  type SkinLibraryItem,
} from "../api";
import AppIcon from "../components/AppIcon.vue";
import IconButton from "../components/IconButton.vue";
import SkinModel from "../components/SkinModel.vue";
import WoodButton from "../components/WoodButton.vue";
import WoodInput from "../components/WoodInput.vue";
import WoodModal from "../components/WoodModal.vue";
import WoodSelect from "../components/WoodSelect.vue";
import WoodToggle from "../components/WoodToggle.vue";
import { loadSelectedSkin, notify, refreshAccounts, store } from "../store";

const items = ref<SkinLibraryItem[]>([]);
const loading = ref(true);
const error = ref("");

const importOpen = ref(false);
const importName = ref("");
const importModel = ref("classic");
const importPath = ref("");
const importing = ref(false);

const harvestOpen = ref(false);
const harvestName = ref("");
const harvestAccount = ref("");
const harvesting = ref(false);

const playerOpen = ref(false);
const playerName = ref("");
const playerFetching = ref(false);

const applyTarget = ref<SkinLibraryItem | null>(null);
const applyAccount = ref("");
const applyUpload = ref(false);
const applying = ref(false);

const renameTarget = ref<SkinLibraryItem | null>(null);
const renameName = ref("");
const renaming = ref(false);

const deleteTarget = ref<SkinLibraryItem | null>(null);
const deleting = ref(false);

const capeAccount = ref("");
const capes = ref<CapeView[]>([]);
const capesLoading = ref(false);
const capesError = ref("");
const capeBusy = ref("");

const accountOptions = computed(() =>
  store.accounts.map((account) => ({ value: account.id, label: account.name }))
);
/// 披风：微软账号可切换，第三方账号只读展示皮肤站下发的披风
const capeAccountOptions = computed(() =>
  store.accounts
    .filter((account) => account.kind === "microsoft" || account.kind === "authlib")
    .map((account) => ({ value: account.id, label: account.name }))
);
const capeAccountView = computed<AccountView | null>(
  () => store.accounts.find((account) => account.id === capeAccount.value) ?? null
);
const applyAccountView = computed<AccountView | null>(
  () => store.accounts.find((account) => account.id === applyAccount.value) ?? null
);

function sourceLabel(source: string) {
  if (source === "account") return "来自账号";
  if (source === "player") return "玩家抓取";
  if (source === "default") return "内置";
  return "导入";
}

async function loadLibrary() {
  loading.value = true;
  error.value = "";
  try {
    items.value = await api.skinLibraryList();
  } catch (err) {
    error.value = String(err);
  } finally {
    loading.value = false;
  }
}

async function pickImportFile() {
  const picked = await openDialog({
    multiple: false,
    filters: [{ name: "Minecraft 皮肤", extensions: ["png"] }],
  });
  if (typeof picked === "string") {
    importPath.value = picked;
    if (!importName.value.trim()) {
      const file = picked.split(/[\\/]/).pop() ?? "";
      importName.value = file.replace(/\.png$/i, "");
    }
  }
}

async function confirmImport() {
  if (importing.value) return;
  if (!importPath.value) {
    notify("请先选择皮肤文件", "err");
    return;
  }
  importing.value = true;
  try {
    await api.skinLibraryImport(
      importName.value.trim() || "未命名皮肤",
      importModel.value,
      importPath.value
    );
    importOpen.value = false;
    importName.value = "";
    importPath.value = "";
    importModel.value = "classic";
    await loadLibrary();
    notify("皮肤已入库");
  } catch (err) {
    notify(`入库失败：${String(err)}`, "err");
  } finally {
    importing.value = false;
  }
}

function openHarvest() {
  if (!store.accounts.length) {
    notify("还没有账号，请先到账号页添加", "err");
    return;
  }
  harvestAccount.value = store.config?.selected_account ?? store.accounts[0].id;
  const account = store.accounts.find((a) => a.id === harvestAccount.value);
  harvestName.value = account ? `${account.name} 的皮肤` : "";
  harvestOpen.value = true;
}

async function confirmPlayer() {
  const name = playerName.value.trim();
  if (!name || playerFetching.value) return;
  playerFetching.value = true;
  try {
    const item = await api.skinLibraryFromPlayer(name);
    playerOpen.value = false;
    playerName.value = "";
    await loadLibrary();
    notify(`已抓取 ${item.name} 的皮肤`);
  } catch (err) {
    notify(`抓取失败：${String(err)}`, "err");
  } finally {
    playerFetching.value = false;
  }
}

async function confirmHarvest() {
  if (harvesting.value) return;
  harvesting.value = true;
  try {
    await api.skinLibraryFromAccount(
      harvestName.value.trim() || "未命名皮肤",
      harvestAccount.value
    );
    harvestOpen.value = false;
    await loadLibrary();
    notify("已收录到皮肤库");
  } catch (err) {
    notify(`收录失败：${String(err)}`, "err");
  } finally {
    harvesting.value = false;
  }
}

function openApply(item: SkinLibraryItem) {
  if (!store.accounts.length) {
    notify("还没有账号，请先到账号页添加", "err");
    return;
  }
  applyTarget.value = item;
  applyAccount.value = store.config?.selected_account ?? store.accounts[0].id;
  applyUpload.value = false;
}

async function confirmApply() {
  const target = applyTarget.value;
  if (!target || applying.value) return;
  applying.value = true;
  try {
    await api.skinLibraryApply(target.id, applyAccount.value, applyUpload.value);
    await refreshAccounts();
    await loadSelectedSkin();
    applyTarget.value = null;
    notify(
      applyUpload.value
        ? `已应用「${target.name}」并同步上传 Mojang`
        : `已应用「${target.name}」`
    );
  } catch (err) {
    notify(`应用失败：${String(err)}`, "err");
  } finally {
    applying.value = false;
  }
}

function openRename(item: SkinLibraryItem) {
  renameTarget.value = item;
  renameName.value = item.name;
}

async function confirmRename() {
  const target = renameTarget.value;
  if (!target || renaming.value) return;
  renaming.value = true;
  try {
    await api.skinLibraryRename(target.id, renameName.value.trim());
    renameTarget.value = null;
    await loadLibrary();
  } catch (err) {
    notify(`重命名失败：${String(err)}`, "err");
  } finally {
    renaming.value = false;
  }
}

async function confirmDelete() {
  const target = deleteTarget.value;
  if (!target || deleting.value) return;
  deleting.value = true;
  try {
    await api.skinLibraryRemove(target.id);
    deleteTarget.value = null;
    await loadLibrary();
    notify(`已删除「${target.name}」`);
  } catch (err) {
    notify(`删除失败：${String(err)}`, "err");
  } finally {
    deleting.value = false;
  }
}

async function loadCapes() {
  if (!capeAccount.value) {
    capes.value = [];
    return;
  }
  capesLoading.value = true;
  capesError.value = "";
  try {
    capes.value = await api.accountCapes(capeAccount.value);
  } catch (err) {
    capes.value = [];
    capesError.value = String(err);
  } finally {
    capesLoading.value = false;
  }
}

async function toggleCape(cape: CapeView) {
  if (capeBusy.value) return;
  capeBusy.value = cape.id;
  try {
    await api.capeSetActive(capeAccount.value, cape.active ? null : cape.id);
    await loadCapes();
    notify(cape.active ? "已卸下披风" : `已启用披风「${cape.alias}」`);
  } catch (err) {
    notify(`披风操作失败：${String(err)}`, "err");
  } finally {
    capeBusy.value = "";
  }
}

watch(capeAccount, () => {
  void loadCapes();
});

/// 第三方账号的披风只能在皮肤站切换，这里按 API 根推出站点首页
function openSkinSite() {
  const trimmed = (capeAccountView.value?.authlib_url ?? "").trim().replace(/\/+$/, "");
  if (!trimmed) return;
  const site = trimmed.endsWith("/api/yggdrasil")
    ? trimmed.slice(0, -"/api/yggdrasil".length)
    : trimmed;
  openExternal(site);
}

onMounted(async () => {
  await loadLibrary();
  if (capeAccountOptions.value.length) {
    const selected = store.config?.selected_account;
    capeAccount.value = capeAccountOptions.value.some((o) => o.value === selected)
      ? (selected as string)
      : capeAccountOptions.value[0].value;
  }
});
</script>

<template>
  <div class="page skins">
    <section class="lib card">
      <header class="lib__head">
        <div class="lib__title">
          皮肤收藏
          <span class="lib__count mono">{{ items.length }}</span>
        </div>
        <div class="lib__actions">
          <WoodButton variant="ghost" size="sm" @click="openHarvest">
            <AppIcon name="user" :size="14" />
            从账号收录
          </WoodButton>
          <WoodButton variant="ghost" size="sm" @click="playerOpen = true">
            <AppIcon name="globe" :size="14" />
            按玩家名抓取
          </WoodButton>
          <WoodButton variant="primary" size="sm" @click="importOpen = true">
            <AppIcon name="plus" :size="14" />
            导入 PNG
          </WoodButton>
        </div>
      </header>

      <div v-if="loading" class="lib__empty">
        <span class="muted small">正在读取皮肤库…</span>
      </div>
      <div v-else-if="error" class="lib__empty">
        <span class="muted small">{{ error }}</span>
        <WoodButton variant="ghost" size="sm" @click="loadLibrary">重试</WoodButton>
      </div>
      <div v-else-if="!items.length" class="lib__empty">
        <span class="muted small">皮肤库还是空的，导入 PNG 或从账号收录一件吧</span>
      </div>

      <div v-else class="skin-grid">
        <article v-for="item in items" :key="item.id" class="skin">
          <div class="skin__preview">
            <SkinModel :skin="item.data_url" :seed="item.id" :model="item.model" :height="200" />
          </div>
          <div class="skin__meta">
            <strong class="skin__name">{{ item.name }}</strong>
            <span class="skin__tags">
              {{ item.model === "slim" ? "纤细" : "经典" }} · {{ sourceLabel(item.source) }}
            </span>
          </div>
          <footer class="skin__foot">
            <WoodButton variant="primary" size="sm" @click="openApply(item)">
              应用
            </WoodButton>
            <IconButton icon="settings" label="重命名" :size="26" @click="openRename(item)" />
            <IconButton
              icon="trash"
              label="删除"
              :size="26"
              variant="danger-quiet"
              @click="deleteTarget = item"
            />
          </footer>
        </article>
      </div>
    </section>

    <section class="lib card">
      <header class="lib__head">
        <div class="lib__title">披风管理</div>
        <WoodSelect
          v-if="capeAccountOptions.length"
          v-model="capeAccount"
          :options="capeAccountOptions"
        />
      </header>

      <div v-if="!capeAccountOptions.length" class="lib__empty">
        <span class="muted small">登录微软或第三方账号后可以查看披风</span>
      </div>
      <div v-else-if="capesLoading" class="lib__empty">
        <span class="muted small">正在读取披风…</span>
      </div>
      <div v-else-if="capesError" class="lib__empty">
        <span class="muted small">{{ capesError }}</span>
        <WoodButton variant="ghost" size="sm" @click="loadCapes">重试</WoodButton>
      </div>
      <div v-else-if="!capes.length" class="lib__empty">
        <span class="muted small">
          {{
            capeAccountView?.kind === "authlib"
              ? "这个角色还没有在皮肤站设置披风"
              : "这个账号没有可用的披风"
          }}
        </span>
      </div>

      <div v-else class="cape-grid">
        <article
          v-for="cape in capes"
          :key="cape.id"
          class="cape"
          :class="{ 'cape--active': cape.active }"
        >
          <div class="cape__preview">
            <div
              class="cape-front"
              :style="{ backgroundImage: `url(${cape.url})` }"
              role="img"
              :aria-label="cape.alias"
            />
          </div>
          <div class="cape__meta">
            <strong class="cape__name">{{ cape.alias }}</strong>
            <span class="cape__state">{{ cape.active ? "使用中" : "未使用" }}</span>
          </div>
          <footer class="cape__foot">
            <WoodButton
              v-if="capeAccountView?.kind === 'authlib'"
              variant="soft"
              size="sm"
              @click="openSkinSite"
            >
              去皮肤站管理
            </WoodButton>
            <WoodButton
              v-else
              :variant="cape.active ? 'ghost' : 'primary'"
              size="sm"
              :disabled="capeBusy === cape.id"
              @click="toggleCape(cape)"
            >
              {{ capeBusy === cape.id ? "处理中…" : cape.active ? "卸下" : "启用" }}
            </WoodButton>
          </footer>
        </article>
      </div>
    </section>

    <WoodModal v-if="importOpen" title="导入皮肤 PNG" :width="440" @close="importOpen = false">
      <div class="form">
        <label class="field">
          <span class="field__label">名称</span>
          <WoodInput v-model="importName" placeholder="给皮肤起个名字" />
        </label>
        <label class="field">
          <span class="field__label">模型</span>
          <div class="seg">
            <button
              class="seg__btn"
              :class="{ 'seg__btn--active': importModel === 'classic' }"
              @click="importModel = 'classic'"
            >
              经典
            </button>
            <button
              class="seg__btn"
              :class="{ 'seg__btn--active': importModel === 'slim' }"
              @click="importModel = 'slim'"
            >
              纤细
            </button>
          </div>
        </label>
        <div class="field">
          <span class="field__label">皮肤文件</span>
          <div class="file-row">
            <span class="file-row__path mono">{{ importPath || "未选择文件" }}</span>
            <WoodButton variant="ghost" size="sm" @click="pickImportFile">选择文件</WoodButton>
          </div>
        </div>
      </div>
      <template #footer>
        <WoodButton variant="ghost" size="sm" @click="importOpen = false">取消</WoodButton>
        <WoodButton variant="primary" size="sm" :disabled="importing" @click="confirmImport">
          {{ importing ? "入库中…" : "确认入库" }}
        </WoodButton>
      </template>
    </WoodModal>

    <WoodModal v-if="harvestOpen" title="从账号收录皮肤" :width="440" @close="harvestOpen = false">
      <div class="form">
        <label class="field">
          <span class="field__label">账号</span>
          <WoodSelect v-model="harvestAccount" :options="accountOptions" />
        </label>
        <label class="field">
          <span class="field__label">名称</span>
          <WoodInput v-model="harvestName" placeholder="给皮肤起个名字" />
        </label>
        <p class="muted small">会把该账号当前使用的皮肤复制一份到皮肤库。</p>
      </div>
      <template #footer>
        <WoodButton variant="ghost" size="sm" @click="harvestOpen = false">取消</WoodButton>
        <WoodButton variant="primary" size="sm" :disabled="harvesting" @click="confirmHarvest">
          {{ harvesting ? "收录中…" : "确认收录" }}
        </WoodButton>
      </template>
    </WoodModal>

    <WoodModal v-if="playerOpen" title="按玩家名抓取皮肤" :width="440" @close="playerOpen = false">
      <div class="form">
        <label class="field">
          <span class="field__label">玩家名</span>
          <WoodInput v-model="playerName" placeholder="例如 Notch" @keyup.enter="confirmPlayer" />
        </label>
        <p class="muted small">
          通过 Mojang 官方接口抓取该正版玩家当前使用的皮肤，入库后会用玩家名命名。
        </p>
      </div>
      <template #footer>
        <WoodButton variant="ghost" size="sm" @click="playerOpen = false">取消</WoodButton>
        <WoodButton
          variant="primary"
          size="sm"
          :disabled="playerFetching || !playerName.trim()"
          @click="confirmPlayer"
        >
          {{ playerFetching ? "抓取中…" : "抓取入库" }}
        </WoodButton>
      </template>
    </WoodModal>

    <WoodModal
      v-if="applyTarget"
      :title="`应用皮肤「${applyTarget.name}」`"
      :width="440"
      @close="applyTarget = null"
    >
      <div class="form">
        <label class="field">
          <span class="field__label">目标账号</span>
          <WoodSelect v-model="applyAccount" :options="accountOptions" />
        </label>
        <label v-if="applyAccountView?.kind === 'microsoft'" class="toggle-row">
          <WoodToggle v-model="applyUpload" />
          <span>同步上传 Mojang（替换该账号的官方皮肤，游戏内生效）</span>
        </label>
        <p v-else-if="applyAccountView?.kind === 'authlib'" class="muted small">
          第三方账号的游戏内皮肤由皮肤站下发，启动器无法修改；请到皮肤站官网更换。这里应用后只影响启动器内的预览与导出。
        </p>
        <p v-else class="muted small">
          离线账号的皮肤只用于启动器预览与导出，不会改变游戏内显示。
        </p>
      </div>
      <template #footer>
        <WoodButton variant="ghost" size="sm" @click="applyTarget = null">取消</WoodButton>
        <WoodButton variant="primary" size="sm" :disabled="applying" @click="confirmApply">
          {{ applying ? "应用中…" : "确认应用" }}
        </WoodButton>
      </template>
    </WoodModal>

    <WoodModal
      v-if="renameTarget"
      title="重命名皮肤"
      :width="400"
      @close="renameTarget = null"
    >
      <div class="form">
        <label class="field">
          <span class="field__label">新名称</span>
          <WoodInput v-model="renameName" placeholder="皮肤名称" />
        </label>
      </div>
      <template #footer>
        <WoodButton variant="ghost" size="sm" @click="renameTarget = null">取消</WoodButton>
        <WoodButton variant="primary" size="sm" :disabled="renaming" @click="confirmRename">
          {{ renaming ? "保存中…" : "保存" }}
        </WoodButton>
      </template>
    </WoodModal>

    <WoodModal
      v-if="deleteTarget"
      title="删除皮肤"
      :width="400"
      @close="deleteTarget = null"
    >
      <p class="muted small">
        确定要从皮肤库删除「{{ deleteTarget.name }}」吗？此操作不可恢复。
      </p>
      <template #footer>
        <WoodButton variant="ghost" size="sm" @click="deleteTarget = null">取消</WoodButton>
        <WoodButton variant="danger" size="sm" :disabled="deleting" @click="confirmDelete">
          {{ deleting ? "删除中…" : "确认删除" }}
        </WoodButton>
      </template>
    </WoodModal>
  </div>
</template>

<style scoped>
.skins {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.lib {
  display: flex;
  flex-direction: column;
  border-radius: 3px;
  overflow: hidden;
}

.lib__head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
  padding: 10px 14px;
  border-bottom: 1px solid var(--line);
}

.lib__title {
  display: flex;
  align-items: baseline;
  gap: 8px;
  min-width: 0;
  font-size: 13px;
  font-weight: 700;
  color: var(--walnut-deep);
}

.lib__count {
  font-size: 10.5px;
  font-weight: 500;
  color: var(--muted);
}

.lib__actions {
  display: flex;
  gap: 6px;
}

.lib__empty {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 12px;
  padding: 36px 12px;
}

.skin-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(172px, 1fr));
  gap: 1px;
  background: var(--line);
}

.skin {
  display: flex;
  flex-direction: column;
  background: rgba(var(--glass-rgb), 0.72);
  transition: background var(--dur-2) ease;
}

.skin:hover {
  background: rgba(var(--glass-rgb), 0.86);
}

.skin__preview {
  display: grid;
  place-items: center;
  height: 214px;
  background: rgba(var(--glass-rgb), 0.42);
  border-bottom: 1px solid var(--line);
  overflow: hidden;
}

/* 预览浮起：去掉 SkinModel 自带的渐变底和边框，融入卡片底色 */
.skin__preview :deep(.skin-model) {
  border: none;
  border-radius: 0;
  background: transparent;
}

.skin__meta {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 8px 10px;
}

.skin__name {
  font-size: 12.5px;
  font-weight: 700;
  color: var(--walnut-deep);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.skin__tags {
  font-size: 10px;
  color: var(--muted);
}

.skin__foot {
  display: flex;
  align-items: center;
  gap: 4px;
  margin-top: auto;
  padding: 6px 8px 8px;
  border-top: 1px solid var(--line);
}

.skin__foot .btn {
  flex: 1;
}

.cape-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(172px, 1fr));
  gap: 1px;
  background: var(--line);
}

.cape {
  display: flex;
  flex-direction: column;
  background: rgba(var(--glass-rgb), 0.72);
  transition: background var(--dur-2) ease;
}

.cape:hover {
  background: rgba(var(--glass-rgb), 0.86);
}

.cape--active {
  box-shadow: inset 2px 0 0 var(--moss);
}

.cape__preview {
  display: grid;
  place-items: center;
  height: 150px;
  background: rgba(var(--glass-rgb), 0.42);
  border-bottom: 1px solid var(--line);
  overflow: hidden;
}

/* 披风纹理是 64x32 的拼接图，只裁外侧面（x=1..11, y=1..17）放大展示，
   避免露出内侧与侧边拼接 */
.cape-front {
  width: 70px;
  height: 112px;
  background-size: 448px 224px;
  background-position: -7px -7px;
  background-repeat: no-repeat;
  image-rendering: pixelated;
}

.cape__meta {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
  padding: 8px 10px;
}

.cape__name {
  font-size: 12.5px;
  font-weight: 700;
  color: var(--walnut-deep);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.cape__state {
  font-size: 10px;
  color: var(--muted);
}

.cape__foot {
  display: flex;
  align-items: center;
  margin-top: auto;
  padding: 6px 8px 8px;
  border-top: 1px solid var(--line);
}

.cape__foot .btn {
  flex: 1;
}

.form {
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.field {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.field__label {
  font-size: 12px;
  font-weight: 600;
  color: var(--ink-soft);
}

.seg {
  display: flex;
  gap: 2px;
  padding: 3px;
  border-radius: 4px;
  border: 1px solid var(--line);
  background: var(--panel-2);
  align-self: flex-start;
}

.seg__btn {
  height: 28px;
  padding: 0 14px;
  border-radius: 3px;
  font-size: 12px;
  font-weight: 600;
  color: var(--ink-soft);
  transition: background var(--dur-1) ease, color var(--dur-1) ease;
}

.seg__btn--active {
  background: var(--panel);
  color: var(--walnut-deep);
  box-shadow: var(--shadow-1);
}

.file-row {
  display: flex;
  align-items: center;
  gap: 10px;
}

.file-row__path {
  flex: 1;
  min-width: 0;
  font-size: 11.5px;
  color: var(--muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.toggle-row {
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: 12.5px;
  color: var(--ink-soft);
}
</style>
