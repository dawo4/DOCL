import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";

export interface GameDirEntry {
  name: string;
  path: string;
}

export interface GameDirsView {
  dirs: GameDirEntry[];
  active: string;
  official: string;
}

export interface ServerEntry {
  address: string;
  last_used: number;
}

export interface LanRoom {
  id: string;
  motd: string;
  host: string;
  port: number;
  address: string;
  last_seen: number;
}

export interface LanView {
  rooms: LanRoom[];
  error: string | null;
}

export interface ServerPingResult {
  motd: string;
  version_name: string;
  protocol: number;
  players_online: number;
  players_max: number;
  players_sample: string[];
  latency_ms: number;
  favicon: string | null;
}

export interface ServerVersionOption {
  id: string;
  game_version: string;
  java_major: number;
  loader: string;
  /// 服务端整合包的显示名；普通版本为空
  name: string;
  /// 服务端包不含启动文件，首次开服需要自动安装加载器
  needs_install: boolean;
}

export interface ServerPack {
  id: string;
  name: string;
  dir: string;
  game_version: string;
  java_major: number;
  loader: string;
  imported_at: number;
  /// 包里没有启动文件，开服时需要自动安装加载器
  needs_install: boolean;
  loader_version: string;
}

export interface ServerOptions {
  port: number;
  motd: string;
  max_players: number;
  view_distance: number;
  online_mode: boolean;
  white_list: boolean;
  accept_eula: boolean;
}

export interface ServerSnapshot {
  version_id: string;
  game_version: string;
  pid: number;
  port: number;
  lan_address: string;
  local_address: string;
  started_at: number;
  online: boolean;
  attached: boolean;
  loader: string;
  dir: string;
  java: string;
  log: string[];
}

export interface TcProfile {
  machine_id: string;
  name: string;
  vendor: string;
  kind: string;
}

export interface TcState {
  state: string;
  index: number;
  room?: string;
  url?: string;
  difficulty?: string;
  profiles?: TcProfile[];
  type?: number;
}

export interface TcStatus {
  running: boolean;
  port: number;
  version: string;
  error: string | null;
  state: TcState | null;
}

export interface HallRoom {
  room: string;
  name: string;
  motd: string;
  players: number;
  max_players: number;
  version: string;
  updated: number;
}

export interface Config {
  game_dir: string;
  game_dirs: GameDirEntry[];
  recent_servers: ServerEntry[];
  hall_url: string;
  update_url: string;
  update_mode: string;
  java_path: string | null;
  java_install_dir: string | null;
  memory_mb: number;
  memory_auto: boolean;
  memory_mode: string;
  java_args: string;
  game_args: string;
  pre_launch: string;
  disable_launch_wrapper: boolean;
  disable_lwjgl_unsafe: boolean;
  prefer_gpu: boolean;
  window_width: number;
  window_height: number;
  fullscreen: boolean;
  download_source: string;
  concurrency: number;
  /// 全局下载限速（MiB/s），0 表示不限速
  speed_limit: number;
  animations: boolean;
  version_isolation: boolean;
  selected_version: string | null;
  selected_account: string | null;
  theme: string;
  background_image: string | null;
  /// 背景图文件夹：设置后每次启动从中随机选一张（与 background_image 互斥）
  background_dir: string | null;
  /// 背景适应方式：cover / contain / stretch / center
  background_fit: string;
  background_blur: number;
  glass_opacity: number;
  glass_blur: number;
  background_dim: number;
  ui_radius: string;
  ui_density: string;
  window_vibrancy: boolean;
  /// 从 MC 百科补全模组中文名
  mcmod_names: boolean;
  /// 记住启动器窗口大小
  remember_window: boolean;
  launcher_window_w: number;
  launcher_window_h: number;
}

export interface BackgroundInfo {
  path: string;
  data_url: string;
}

export interface SystemMemory {
  total_mb: number;
  available_mb: number;
  /// 按当前占用情况推荐的自动分配值（参考 PCL，随可用内存实时变化）
  recommended_mb: number;
}

export interface NewsItem {
  id: string;
  title: string;
  category: string;
  tag: string;
  date: string;
  summary: string;
  image_url: string | null;
  url: string | null;
  source: string;
  version: string | null;
  kind: string | null;
  detail_path: string | null;
}

export interface NewsFeed {
  items: NewsItem[];
  updated_at: number;
  stale: boolean;
  errors: string[];
}

export interface VersionSummary {
  id: string;
  kind: string;
  release_time: string | null;
  installed: boolean;
}

export interface InstalledVersion {
  id: string;
  kind: string;
  jar_size: number;
  has_jar: boolean;
  last_modified: number;
  java_major: number;
  /// 游戏版本与加载器（整合包也能拿到真实值）
  game_version: string;
  loader: string;
  /// 仅作为整合包父版本存在
  dependency: boolean;
}

export interface LoaderOption {
  loader: string;
  label: string;
  version: string | null;
  installed: boolean;
  installed_id: string | null;
  supported: boolean;
  note: string;
}

export interface LoaderVersion {
  version: string;
  recommended: boolean;
}

export interface JavaInfo {
  path: string;
  version: string;
  major: number;
  source: string;
}

export interface InstanceContext {
  game_version: string;
  loader: string;
}

export interface BrokenFile {
  kind: string;
  name: string;
  size: number;
}

export interface InstanceCheck {
  scanned: number;
  broken: BrokenFile[];
}

export interface CrashLink {
  label: string;
  url: string;
}

export interface CrashSuspect {
  name: string;
  /// mods 目录里的文件名，未匹配到时为空
  file_name: string;
  /// 命中的包名（堆栈分析时）
  package: string;
  /// stack / reported
  reason: string;
}

export interface CrashContext {
  game_version: string;
  loader: string;
  java_major: number;
  java_path: string;
  memory_mb: number;
  mod_count: number;
  problematic_frame: string | null;
}

export interface CrashMatch {
  id: string;
  title: string;
  summary: string;
  advice: string[];
  evidence: string[];
  links: CrashLink[];
  /// 90 高 / 70 中 / 50 低
  priority: number;
  category: string;
  suspects: CrashSuspect[];
}

export interface CrashReport {
  version_id: string;
  found: boolean;
  source: string;
  source_path: string;
  source_kind: string;
  modified: number;
  headline: string;
  description: string | null;
  matches: CrashMatch[];
  excerpt: string[];
  scanned: string[];
  suspects: CrashSuspect[];
  context: CrashContext;
}

export interface ModUpdate {
  file_name: string;
  mod_name: string;
  current_version: string;
  latest_version: string;
  latest_file: string;
  download_url: string;
  sha1: string | null;
  project_id: string;
  slug: string;
  icon_url: string | null;
  version_type: string;
  game_versions: string[];
  source: string;
  project_url: string;
}

export interface ModUpdateReport {
  version_id: string;
  game_version: string;
  loader: string;
  scanned: number;
  matched: number;
  up_to_date: number;
  updates: ModUpdate[];
  notes: string[];
}

export interface InstanceMeta {
  display_name: string | null;
  description: string;
  icon: string;
  category: string;
  favorite: boolean;
  memory_mb: number | null;
  memory_mode: string | null;
  java_path: string | null;
  java_args: string | null;
  game_args: string | null;
  pre_launch: string | null;
  disable_launch_wrapper: boolean | null;
  disable_lwjgl_unsafe: boolean | null;
  prefer_gpu: boolean | null;
  window_width: number | null;
  window_height: number | null;
  fullscreen: boolean | null;
}

export interface InstanceCounts {
  mods: number;
  resourcepacks: number;
  shaderpacks: number;
  saves: number;
  screenshots: number;
}

export interface InstanceInfo extends InstanceMeta {
  id: string;
  game_version: string;
  loader: string;
  kind: string;
  java_major: number;
  path: string;
  installed_at: number;
  counts: InstanceCounts;
}

export interface InstanceFile {
  name: string;
  display_name: string;
  size: number;
  modified: number;
  enabled: boolean;
  is_dir: boolean;
}

export interface InstanceModMeta {
  file_name: string;
  name: string;
  project_id: string;
  /// modrinth / curseforge
  source: string;
  logo_url: string;
  website_url: string;
  version_number: string;
}

export interface InstancePaths {
  version: string;
  mods: string;
  resourcepacks: string;
  shaderpacks: string;
  saves: string;
  screenshots: string;
  datapacks: string;
  schematics: string;
  game_dir: string;
}

export interface WorldSummary {
  folder: string;
  name: string;
  modified: number;
  datapacks: string;
}

export interface WorldSeed {
  folder: string;
  name: string;
  seed: string | null;
  version: string | null;
}

export interface UpdateInfo {
  current: string;
  latest: string;
  available: boolean;
  url: string;
  portable_url: string;
  notes: string;
}

export interface PclPreview {
  root: string;
  game_dir: string | null;
  java_path: string | null;
  memory_auto: boolean | null;
  memory_mb: number | null;
  window_width: number | null;
  window_height: number | null;
  update_mode: string | null;
  download_source: string | null;
  selected_version: string | null;
  findings: string[];
}

export interface UpdateProgress {
  downloaded: number;
  total: number;
}

export interface PackFilePreview {
  path: string;
  size: number;
  url: string | null;
  required: boolean;
}

export interface PackPreview {
  format: string;
  name: string;
  version: string;
  game_version: string;
  loader: string;
  loader_version: string | null;
  detected_parent: string | null;
  file_count: number;
  download_bytes: number;
  override_bytes: number;
  files: PackFilePreview[];
  warnings: string[];
  suggested_id: string;
  /// 服务端安装时会跳过的仅客户端文件数
  server_skips: number;
  /// 压缩包特征更像服务端整合包
  server_like: boolean;
}

export interface ModpackImportOptions {
  instance_name?: string | null;
  instance_id?: string | null;
  loader_version?: string | null;
  parent_version?: string | null;
  install_optional?: boolean;
  /// 安装为服务端：按清单跳过仅客户端文件
  server?: boolean;
}

export interface ModpackImportResult {
  instance_id: string;
  display_name: string;
  installed_files: number;
  failed: string[];
}

export interface ExportEntry {
  key: string;
  label: string;
  file_count: number;
  bytes: number;
}

export interface ExportPreview {
  instance_id: string;
  name: string;
  version: string;
  game_version: string;
  loader: string;
  loader_version: string | null;
  entries: ExportEntry[];
  total_bytes: number;
  /// 检测到的常用自定义目录（tacz 枪包等），可在导出时勾选
  suggested_extras: ExportEntry[];
}

export interface ExportOptions {
  name: string;
  version: string;
  format: string;
  groups: string[];
  target: string;
  /// 额外打包的自定义文件夹（相对实例目录，如 tacz）
  extra_dirs?: string[];
}

export interface ExportResult {
  paths: string[];
  bytes: number;
  file_count: number;
}

export interface DefaultSkin {
  id: string;
  name: string;
  model: string;
  data_url: string;
}

export interface SkinLibraryItem {
  id: string;
  name: string;
  model: string;
  source: string;
  created_at: number;
  data_url: string;
}

export interface CapeView {
  id: string;
  alias: string;
  url: string;
  active: boolean;
}

export interface CfProject {
  source: "curseforge" | "modrinth" | string;
  id: string;
  name: string;
  slug: string;
  /// 官方项目网页地址（Modrinth / CurseForge 项目页）
  website_url: string;
  summary: string;
  download_count: number;
  class_id: number;
  logo_url: string;
  author: string;
  updated: string;
  categories: string[];
}

export interface CfSearchResult {
  items: CfProject[];
  total: number;
}

export interface CfFileView {
  id: string;
  name: string;
  file_name: string;
  file_date: string;
  size: number;
  download_url: string | null;
  game_versions: string[];
  release_type: number;
  /// 该文件本身是服务端整合包
  is_server_pack: boolean;
  /// 配套服务端包的文件 ID，可一键下载
  server_pack_file_id: string | null;
  /// 必需前置的项目 ID
  dependencies: string[];
}

/// 模组安装完成后，后端自动补齐前置的结果。
export interface DependencyReport {
  installed: string[];
  failed: string[];
  unresolved: string[];
}

export interface InstallOutcome {
  path: string;
  dependencies: DependencyReport;
}

export interface ModImportOutcome {
  imported: number;
  dependencies: DependencyReport;
}

/// 拖入窗口后由后端识别出的文件分类。
export interface DroppedItem {
  path: string;
  file_name: string;
  size: number;
  kind: "mods" | "resourcepacks" | "shaderpacks" | "datapacks" | "saves" | "schematics" | "modpack" | "unknown";
  label: string;
  detail: string;
}

export interface AccountView {
  id: string;
  kind: string;
  name: string;
  uuid: string;
  skin_path: string | null;
  skin_model: string;
  /// 第三方认证（authlib-injector）的 API 根
  authlib_url: string | null;
}

export interface AuthlibMeta {
  root: string;
  name: string;
  homepage: string;
}

export interface AccountSkin {
  data_url: string;
  model: string;
  source: "local" | "official" | "authlib" | "default";
}

export interface DeviceCode {
  user_code: string;
  verification_uri: string;
  message: string;
  expires_in: number;
  interval: number;
}

export interface ProgressStep {
  label: string;
  done: boolean;
}

export interface ProgressSnapshot {
  stage: string;
  message: string;
  steps: ProgressStep[];
  files_done: number;
  files_total: number;
  bytes_done: number;
  bytes_total: number;
  elapsed_ms: number;
  active: boolean;
}

export interface LogLine {
  stream: string;
  line: string;
}

export interface GameExit {
  code: number | null;
  pid: number;
}

export const api = {
  getConfig: () => invoke<Config>("get_config"),
  systemMemory: () => invoke<SystemMemory>("system_memory"),
  newsFeed: (force = false) => invoke<NewsFeed>("news_feed", { force }),
  newsArticle: (path: string, wikiTitle?: string) =>
    invoke<string>("news_article", { path, wikiTitle: wikiTitle ?? null }),
  saveConfig: (config: Config) => invoke<Config>("save_config", { config }),
  gameDirs: () => invoke<GameDirsView>("game_dirs"),
  gameDirVersionCounts: () => invoke<Record<string, number>>("game_dir_version_counts"),
  addGameDir: (path: string) => invoke<Config>("add_game_dir", { path }),
  switchGameDir: (path: string) => invoke<Config>("switch_game_dir", { path }),
  removeGameDir: (path: string) => invoke<Config>("remove_game_dir", { path }),
  fetchVersions: (force = false) => invoke<VersionSummary[]>("fetch_versions", { force }),
  installedVersions: () => invoke<InstalledVersion[]>("installed_versions"),
  installVersion: (versionId: string) => invoke<InstalledVersion>("install_version", { versionId }),
  cancelInstall: () => invoke<void>("cancel_install"),
  loaderOptions: (versionId: string) =>
    invoke<LoaderOption[]>("loader_options", { versionId }),
  loaderVersions: (versionId: string, loader: string) =>
    invoke<LoaderVersion[]>("loader_versions", { versionId, loader }),
  installLoader: (versionId: string, loader: string, loaderVersion?: string) =>
    invoke<InstalledVersion>("install_loader", { versionId, loader, loaderVersion }),
  instanceContext: (versionId: string) =>
    invoke<InstanceContext>("instance_context", { versionId }),
  checkInstanceFiles: (versionId: string) =>
    invoke<InstanceCheck>("check_instance_files", { versionId }),
  instanceInfo: (versionId: string) =>
    invoke<InstanceInfo>("instance_info", { versionId }),
  instanceMetas: () => invoke<Record<string, InstanceMeta>>("instance_metas"),
  instanceSaveMeta: (versionId: string, meta: InstanceMeta) =>
    invoke<InstanceInfo>("instance_save_meta", { versionId, meta }),
  instanceSetIcon: (versionId: string, icon: string) =>
    invoke<void>("instance_set_icon", { versionId, icon }),
  instanceSetIconImage: (versionId: string, source: string) =>
    invoke<string>("instance_set_icon_image", { versionId, source }),
  instanceIconImage: (versionId: string) =>
    invoke<string | null>("instance_icon_image", { versionId }),
  instanceFiles: (versionId: string, kind: string) =>
    invoke<InstanceFile[]>("instance_files", { versionId, kind }),
  instanceToggleFile: (versionId: string, kind: string, name: string) =>
    invoke<void>("instance_toggle_file", { versionId, kind, name }),
  instanceDeleteFile: (versionId: string, kind: string, name: string) =>
    invoke<void>("instance_delete_file", { versionId, kind, name }),
  instanceOpenContent: (versionId: string, kind: string, name: string) =>
    invoke<void>("instance_open_content", { versionId, kind, name }),
  instanceThumb: (versionId: string, kind: string, name: string) =>
    invoke<string | null>("instance_thumb", { versionId, kind, name }),
  instanceModMeta: (versionId: string) =>
    invoke<InstanceModMeta[]>("instance_mod_meta", { versionId }),
  instanceImportFiles: (versionId: string, kind: string, paths: string[]) =>
    invoke<number>("instance_import_files", { versionId, kind, paths }),
  instanceImportMods: (versionId: string, paths: string[]) =>
    invoke<ModImportOutcome>("instance_import_mods", { versionId, paths }),
  dropInspect: (paths: string[]) => invoke<DroppedItem[]>("drop_inspect", { paths }),
  instancePaths: (versionId: string) =>
    invoke<InstancePaths>("instance_paths", { versionId }),
  instanceWorlds: (versionId: string) =>
    invoke<WorldSummary[]>("instance_worlds", { versionId }),
  instanceWorldSeed: (versionId: string, folder: string) =>
    invoke<WorldSeed>("instance_world_seed", { versionId, folder }),
  appVersion: () => invoke<string>("app_version"),
  appPortable: () => invoke<boolean>("app_portable"),
  pclImportPreview: (path: string) =>
    invoke<PclPreview>("pcl_import_preview", { path }),
  pclImportApply: (path: string) => invoke<Config>("pcl_import_apply", { path }),
  updateCheck: (url: string) => invoke<UpdateInfo>("update_check", { url }),
  updateDownload: (url: string, version: string) =>
    invoke<string>("update_download", { url, version }),
  updateInstall: (path: string) => invoke<void>("update_install", { path }),
  portableUpdateInstall: (path: string) =>
    invoke<void>("portable_update_install", { path }),
  exportSettings: (path: string) => invoke<void>("export_settings", { path }),
  importSettings: (path: string) => invoke<Config>("import_settings", { path }),
  crashAnalyze: (versionId: string) =>
    invoke<CrashReport>("crash_analyze", { versionId }),
  crashExport: (versionId: string) =>
    invoke<string>("crash_export", { versionId }),
  modUpdates: (versionId: string) =>
    invoke<ModUpdateReport>("mod_updates", { versionId }),
  modUpdateInstall: (
    versionId: string,
    url: string,
    fileName: string,
    oldName: string,
    sha1: string | null
  ) =>
    invoke<string>("mod_update_install", {
      versionId,
      url,
      fileName,
      oldName,
      sha1,
    }),
  modpackInspect: (path: string) => invoke<PackPreview>("modpack_inspect", { path }),
  modpackDownload: (url: string, fileName: string) =>
    invoke<string>("modpack_download", { url, fileName }),
  modpackTempRemove: (path: string) => invoke<void>("modpack_temp_remove", { path }),
  modpackImport: (path: string, options: ModpackImportOptions) =>
    invoke<ModpackImportResult>("modpack_import", { path, options }),
  modpackExportPreview: (versionId: string) =>
    invoke<ExportPreview>("modpack_export_preview", { versionId }),
  modpackExport: (versionId: string, options: ExportOptions) =>
    invoke<ExportResult>("modpack_export", { versionId, options }),
  cfSearch: (
    classId: number,
    query: string,
    gameVersion?: string | null,
    loader?: string | null,
    index = 0,
    sortField = 2,
    categoryId?: number | null
  ) =>
    invoke<CfSearchResult>("cf_search", {
      classId,
      query,
      gameVersion: gameVersion ?? null,
      loader: loader ?? null,
      categoryId: categoryId ?? null,
      index,
      sortField,
    }),
  cfGameVersions: () => invoke<string[]>("cf_game_versions"),
  cfCategories: (classId: number) =>
    invoke<{ id: number; name: string }[]>("cf_categories", { classId }),
  cfFiles: (modId: string, gameVersion?: string | null, loader?: string | null) =>
    invoke<CfFileView[]>("cf_files", {
      modId,
      gameVersion: gameVersion ?? null,
      loader: loader ?? null,
    }),
  modrinthSearch: (
    kind: string,
    query: string,
    gameVersion?: string | null,
    loader?: string | null,
    index = 0,
    sortField = 2,
    category?: string | null
  ) =>
    invoke<CfSearchResult>("modrinth_search", {
      kind,
      query,
      gameVersion: gameVersion ?? null,
      loader: loader ?? null,
      category: category ?? null,
      index,
      sortField,
    }),
  modrinthCategories: (kind: string) =>
    invoke<string[]>("modrinth_categories", { kind }),
  modrinthFiles: (
    projectId: string,
    gameVersion?: string | null,
    loader?: string | null
  ) =>
    invoke<CfFileView[]>("modrinth_files", {
      projectId,
      gameVersion: gameVersion ?? null,
      loader: loader ?? null,
    }),
  cfFileLink: (projectId: string, fileId: string) =>
    invoke<{ url: string; file_name: string }>("cf_file_link", { projectId, fileId }),
  cfInstallFile: (versionId: string, kind: string, url: string, fileName: string) =>
    invoke<InstallOutcome>("cf_install_file", { versionId, kind, url, fileName }),
  contentProjects: (source: string, ids: string[]) =>
    invoke<CfProject[]>("content_projects", { source, ids }),
  projectTranslations: (source: string, ids: string[]) =>
    invoke<Record<string, { summary: string }>>("project_translations", {
      source,
      ids,
    }),
  mcmodNames: (queries: { key: string; title: string; slug: string }[]) =>
    invoke<Record<string, { name: string; url: string }>>("mcmod_names", { queries }),
  deleteVersion: (versionId: string) => invoke<string[]>("delete_version", { versionId }),
  defaultSkin: () => invoke<string>("default_skin"),
  defaultSkins: () => invoke<DefaultSkin[]>("default_skins"),
  getAccounts: () => invoke<AccountView[]>("get_accounts"),
  createOfflineAccount: (name: string) => invoke<AccountView>("create_offline_account", { name }),
  authlibMeta: (root: string) => invoke<AuthlibMeta>("authlib_meta", { root }),
  authlibLogin: (root: string, username: string, password: string) =>
    invoke<AccountView>("authlib_login", { root, username, password }),
  removeAccount: (id: string) => invoke<void>("remove_account", { id }),
  selectAccount: (id: string) => invoke<void>("select_account", { id }),
  getAccountSkin: (accountId: string) =>
    invoke<AccountSkin>("get_account_skin", { accountId }),
  saveAccountSkin: (accountId: string, pngBase64: string, model: string) =>
    invoke<AccountView>("save_account_skin", { accountId, pngBase64, model }),
  clearAccountSkin: (accountId: string) =>
    invoke<AccountView>("clear_account_skin", { accountId }),
  exportAccountSkin: (accountId: string, targetPath: string) =>
    invoke<void>("export_account_skin", { accountId, targetPath }),
  uploadAccountSkin: (accountId: string, pngBase64: string, model: string) =>
    invoke<AccountView>("upload_account_skin", { accountId, pngBase64, model }),
  skinLibraryList: () => invoke<SkinLibraryItem[]>("skin_library_list"),
  skinLibraryImport: (name: string, model: string, path: string) =>
    invoke<SkinLibraryItem>("skin_library_import", { name, model, path }),
  skinLibrarySave: (name: string, model: string, pngBase64: string) =>
    invoke<SkinLibraryItem>("skin_library_save", { name, model, pngBase64 }),
  skinLibraryFromAccount: (name: string, accountId: string) =>
    invoke<SkinLibraryItem>("skin_library_from_account", { name, accountId }),
  skinLibraryFromPlayer: (name: string) =>
    invoke<SkinLibraryItem>("skin_library_from_player", { name }),
  skinLibraryRename: (id: string, name: string) =>
    invoke<void>("skin_library_rename", { id, name }),
  skinLibraryRemove: (id: string) => invoke<void>("skin_library_remove", { id }),
  skinLibraryApply: (id: string, accountId: string, upload: boolean) =>
    invoke<AccountView>("skin_library_apply", { id, accountId, upload }),
  accountCapes: (accountId: string) =>
    invoke<CapeView[]>("account_capes", { accountId }),
  capeSetActive: (accountId: string, capeId: string | null) =>
    invoke<void>("cape_set_active", { accountId, capeId }),
  msLoginBegin: () => invoke<DeviceCode>("ms_login_begin"),
  detectJava: () => invoke<JavaInfo[]>("detect_java"),
  installJava: (major: number) => invoke<JavaInfo>("install_java", { major }),
  deleteJava: (path: string) => invoke<boolean>("delete_java", { path }),
  launchVersion: (versionId: string) => invoke<number>("launch_version", { versionId }),
  launchServer: (versionId: string, address: string) =>
    invoke<number>("launch_server", { versionId, address }),
  lanRooms: () => invoke<LanView>("lan_rooms"),
  hallRooms: () => invoke<HallRoom[]>("hall_rooms"),
  serverVersions: () => invoke<ServerVersionOption[]>("server_versions"),
  serverStatus: () => invoke<ServerSnapshot | null>("server_status"),
  serverStart: (versionId: string, options: ServerOptions) =>
    invoke<ServerSnapshot>("server_start", { versionId, options }),
  serverPackImport: (
    path: string,
    name: string,
    slug: string,
    gameVersion?: string | null,
    loader?: string | null
  ) =>
    invoke<ServerPack>("server_pack_import", {
      path,
      name,
      slug,
      gameVersion: gameVersion ?? null,
      loader: loader ?? null,
    }),
  serverPackRemove: (id: string) => invoke<void>("server_pack_remove", { id }),
  serverCommand: (command: string) => invoke<void>("server_command", { command }),
  serverStop: () => invoke<void>("server_stop"),
  terracottaPrepare: () => invoke<TcStatus>("terracotta_prepare"),
  terracottaStatus: () => invoke<TcStatus>("terracotta_status"),
  terracottaHost: (room: string | null, player: string | null) =>
    invoke<void>("terracotta_host", { room, player }),
  terracottaJoin: (room: string, player: string | null) =>
    invoke<void>("terracotta_join", { room, player }),
  terracottaReset: () => invoke<void>("terracotta_reset"),
  terracottaShutdown: () => invoke<void>("terracotta_shutdown"),
  rememberServer: (address: string) => invoke<Config>("remember_server", { address }),
  forgetServer: (address: string) => invoke<Config>("forget_server", { address }),
  serverPing: (address: string) => invoke<ServerPingResult>("server_ping", { address }),
  killGame: () => invoke<void>("kill_game"),
  importBackground: (path: string) =>
    invoke<BackgroundInfo>("import_background", { path }),
  backgroundDataUrl: (path: string) =>
    invoke<string>("background_data_url", { path }),
  pickBackgroundRandom: (dir: string) =>
    invoke<string | null>("pick_background_random", { dir }),
};

export function onInstallProgress(cb: (p: ProgressSnapshot) => void): Promise<UnlistenFn> {
  return listen<ProgressSnapshot>("install-progress", (e) => cb(e.payload));
}

export function onGameLog(cb: (l: LogLine) => void): Promise<UnlistenFn> {
  return listen<LogLine>("game-log", (e) => cb(e.payload));
}

export function onGameExit(cb: (e: GameExit) => void): Promise<UnlistenFn> {
  return listen<GameExit>("game-exit", (e) => cb(e.payload));
}

export function onMsLoginAccount(cb: (a: AccountView) => void): Promise<UnlistenFn> {
  return listen<AccountView>("ms-login-account", (e) => cb(e.payload));
}

export function onMsLogin(
  cb: (p: { ok: boolean; message?: string }) => void
): Promise<UnlistenFn> {
  return listen<{ ok: boolean; message?: string }>("ms-login", (e) => cb(e.payload));
}

export async function pickDirectory(title = "选择游戏目录"): Promise<string | null> {
  const result = await open({ directory: true, multiple: false, title });
  return typeof result === "string" ? result : null;
}

export async function pickImage(): Promise<string | null> {
  const result = await open({
    directory: false,
    multiple: false,
    title: "选择背景图片",
    filters: [
      { name: "图片", extensions: ["png", "jpg", "jpeg", "webp", "bmp", "gif"] },
    ],
  });
  return typeof result === "string" ? result : null;
}

export async function saveSettingsDialog(): Promise<string | null> {
  const result = await save({
    title: "导出设置",
    defaultPath: "docl-settings.json",
    filters: [{ name: "DOCL 设置", extensions: ["json"] }],
  });
  return typeof result === "string" ? result : null;
}

export async function pickSettingsFile(): Promise<string | null> {
  const result = await open({
    directory: false,
    multiple: false,
    title: "导入设置",
    filters: [{ name: "DOCL 设置", extensions: ["json"] }],
  });
  return typeof result === "string" ? result : null;
}

export async function saveSkinDialog(defaultName = "skin.png"): Promise<string | null> {
  const result = await save({
    title: "导出皮肤",
    defaultPath: defaultName,
    filters: [{ name: "Minecraft 皮肤", extensions: ["png"] }],
  });
  return typeof result === "string" ? result : null;
}

export async function savePackDialog(
  defaultName: string,
  format: string
): Promise<string | null> {
  const mrpack = format === "mrpack";
  const result = await save({
    title: "导出整合包",
    defaultPath: defaultName,
    filters: [
      {
        name: mrpack ? "Modrinth 整合包" : "整合包 ZIP",
        extensions: [mrpack ? "mrpack" : "zip"],
      },
    ],
  });
  return typeof result === "string" ? result : null;
}

function reportOpenError(err: unknown) {
  window.dispatchEvent(
    new CustomEvent("docl-open-error", { detail: String(err) })
  );
}

export function revealFile(path: string) {
  return invoke<void>("reveal_file", { path }).catch(reportOpenError);
}

export function openFolder(path: string) {
  return invoke<void>("open_folder", { path }).catch(reportOpenError);
}

export function openExternal(url: string) {
  return openUrl(url);
}
