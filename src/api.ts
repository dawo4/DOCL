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
  /// 开服人自己的游戏 ID，开启白名单时自动写入
  player_name: string;
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
  /// 服务端是否开启正版验证
  online_mode: boolean;
  /// 当前白名单里的玩家名
  white_list: string[];
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
  /// 房主加载器，大厅源可留空
  loader?: string;
  /// 房间需要的 Mod 文件名或 mod id，大厅源可留空
  mods?: string[];
  updated: number;
}

/// 进房前的 Mod / 版本一致性校验结果
export interface RoomModCheck {
  game_version: string;
  loader: string;
  missing: string[];
  extra: string[];
  problems: string[];
}

export interface CustomTheme {
  id: string;
  name: string;
  /// 页面底色
  paper: string;
  /// 卡片底色
  panel: string;
  /// 主色（按钮、链接）
  primary: string;
  /// 强调色（选中态、进度条）
  accent: string;
  /// 标题文字
  title: string;
  /// 正文文字
  text: string;
  /// 次要文字
  muted: string;
  /// 边框线
  line: string;
}

export interface Config {
  game_dir: string;
  game_dirs: GameDirEntry[];
  recent_servers: ServerEntry[];
  hall_url: string;
  /// 附加的 EasyTier 公共节点，逗号或空格分隔
  terracotta_nodes: string;
  update_url: string;
  update_mode: string;
  java_path: string | null;
  /// 手动添加过的 Java（自动检测不到的自定义安装）
  extra_java_paths: string[];
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
  /// 分来源偏好：留空表示跟随 download_source
  source_metadata: string;
  source_files: string;
  source_modrinth: string;
  source_curseforge: string;
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
  /// 版本页「游玩档案」卡片底图，空表示内置
  profile_hero_image: string | null;
  /// 版本页「最常玩版本」卡片底图，空表示内置
  profile_card_image: string | null;
  /// 版本页卡片文字颜色，空表示跟随主题
  profile_text_color: string | null;
  /// 用户创建的主题，theme 为 `custom:<id>` 时启用
  custom_themes: CustomTheme[];
  ui_font: string;
  ui_density: string;
  window_vibrancy: boolean;
  /// 从 MC 百科补全模组中文名
  mcmod_names: boolean;
  /// 记住启动器窗口大小
  remember_window: boolean;
  launcher_window_w: number;
  launcher_window_h: number;
  /// 上次保存时便携版根目录（exe 所在目录），用于整包搬家后重写绝对路径
  portable_root: string | null;
  /// 低内存模式：降低 WebView2 缓存与 JS 堆上限，并关闭毛玻璃/动效（重启后生效）
  low_memory: boolean;
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

export interface MemoryRelease {
  /// 优化前后系统可用内存的差值（MB），可能为 0
  freed_mb: number;
  /// 成功清理工作集的进程数
  processes: number;
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
  /// JDK 或 JRE
  kind: string;
  /// 发行版名称，如 Eclipse Temurin、Azul Zulu
  vendor: string;
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

/// 崩溃规则的来源与版本信息
export interface CrashRuleStatus {
  /// bundled（内置）/ cache（缓存副本）
  source: string;
  count: number;
  /// 缓存更新时间（Unix 秒），0 表示使用内置规则
  updated: number;
  /// 是否可以回滚到上一版
  has_previous: boolean;
}

/// 一键自愈的结果
export interface RepairReport {
  version_json: boolean;
  files_restored: number;
  files_failed: string[];
  natives: boolean;
  disabled_mods: string[];
  /// 因与当前游戏/加载器不兼容而被禁用的模组
  incompatible_mods: string[];
  duplicate_mods: string[];
  missing_deps: string[];
  /// mods 目录里无法正常打开的 jar
  broken_mods: string[];
  issues: string[];
  /// 修复前的备份目录，null 表示没有改动任何文件
  backup_dir: string | null;
  /// 自动补装成功的前置
  deps_installed: string[];
  /// 自动切换后的 Java
  java_switched: string | null;
  /// 是否重置了画质选项
  graphics_reset: boolean;
  /// 补全的资源文件数量
  assets_restored: number;
  /// 补全失败的资源文件数量
  assets_failed: number;
  /// 是否清空过 config 目录
  config_reset: boolean;
  /// 从 mods 移到 mods-disabled 的解压残留目录
  quarantined_mods: string[];
  /// 因文件名非法而重命名的 Mod
  renamed_mods: string[];
  /// 自动安装的修复类 Mod
  installed_fixes: string[];
  /// 因文件已存在冲突而被定点清理的路径
  removed_paths: string[];
  /// 被移除的无效 -javaagent 参数
  removed_java_agents: string[];
  /// 是否重装过加载器
  loader_reinstalled: boolean;
}

/// 实例的光影加载器检测结果
export interface ShaderSupport {
  loader: string;
  game_version: string;
  /// 已装的光影加载器，null 表示没有
  provider: string | null;
  /// 推荐安装的 Modrinth 项目 id
  recommend: string | null;
  recommend_label: string | null;
}

/// 一键修复的可选动作，默认全部关闭
export interface RepairOptions {
  install_deps?: boolean;
  reset_graphics?: boolean;
  repair_assets?: boolean;
  reinstall_loader?: boolean;
  /// 崩溃分析命中的规则 id，用于针对性修复
  rules?: string[];
  /// 崩溃证据行，用于定位具体配置文件
  evidence?: string[];
}

/// 修复计划里的一项动作，files 是相对游戏根目录的路径
export interface RepairPreviewAction {
  id: string;
  title: string;
  detail: string;
  files: string[];
  /// 会改动的文件总数，files 可能被截断
  count: number;
  risk: "低" | "中";
}

export interface RepairPreview {
  actions: RepairPreviewAction[];
  /// 不会改动文件但需要提醒的问题
  issues: string[];
}

/// 一次修复留下的备份
export interface RepairBackup {
  /// 备份目录名，按次还原时使用
  id: string;
  /// 创建时间（Unix 秒）
  created: number;
  /// 备份的文件数
  files: number;
  /// 改动过的文件示例
  samples: string[];
}

/// 清理游戏垃圾的结果
export interface JunkReport {
  /// 删除的文件数
  files: number;
  /// 释放的字节数
  freed: number;
  /// 删除失败的项数（通常正被占用）
  failed: number;
  /// 失败示例
  failed_samples: string[];
}

/// 启动前自检发现的单个问题
export interface PreflightIssue {
  id: string;
  title: string;
  detail: string;
  /// 是否阻止启动
  blocking: boolean;
  /// 是否能交给一键修复处理
  fixable: boolean;
}

export interface PreflightReport {
  version_id: string;
  issues: PreflightIssue[];
}

/// 下载源测速结果
export interface SourceProbe {
  id: string;
  name: string;
  /// mojang = 游戏文件与加载器；mod = CurseForge / Modrinth 接口
  group: string;
  url: string;
  ok: boolean;
  latency_ms: number;
  bytes: number;
  error: string;
}

/// 整合包清单里没装上的单个文件
export interface ImportFailure {
  /// 所属的 CurseForge 项目；Modrinth 清单不带项目信息时为 0
  project_id: number;
  file_id: number;
  /// 给用户认的名字：项目名，拿不到时退回文件名或 ID
  name: string;
  reason: string;
}

/// 重试失败文件后的结果
export interface RetryDownloads {
  /// 文本摘要（名字：原因）
  failed: string[];
  failures: ImportFailure[];
}

/// 本地游玩画像
export interface VersionPlayStat {
  launches: number;
  total_seconds: number;
  first_played: number | null;
  last_played: number | null;
}

export interface PlaySession {
  version: string;
  started: number;
  seconds: number;
}

export interface PlayStats {
  versions: Record<string, VersionPlayStat>;
  sessions: PlaySession[];
}

/// 游戏设置预设（options.txt 模板）
export interface GameOptionsPreset {
  id: string;
  name: string;
  saved_at: number;
  size: number;
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
  /// 1 环境/安装、2 Mod 归因、3 兼容、4 兜底
  phase: number;
  /// high / medium / low
  confidence: string;
  category: string;
  suspects: CrashSuspect[];
}

export interface WindowsCrashEvent {
  event_id: number;
  provider: string;
  time_created: string;
  message: string;
}

export interface ModChange {
  kind: string;
  file_name: string;
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
  mod_changes: ModChange[];
  windows_events: WindowsCrashEvent[];
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

/// config 目录里的一个条目，path 为相对 config 的 `/` 分隔路径
export interface ConfigEntry {
  path: string;
  name: string;
  is_dir: boolean;
  size: number;
}

export interface ConfigText {
  content: string;
  /// "utf-8" 或 "gbk"
  encoding: string;
  size: number;
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

export interface WorldBackup {
  /// 存档文件夹名
  world: string;
  path: string;
  size: number;
  /// 备份时间（Unix 秒）
  created: number;
}

export interface WorldRule {
  key: string;
  value: string;
}

export interface WorldSettings {
  name: string;
  /// survival / creative / adventure / spectator
  game_mode: string;
  /// peaceful / easy / normal / hard
  difficulty: string | null;
  difficulty_locked: boolean;
  hardcore: boolean;
  allow_commands: boolean | null;
  seed: string | null;
  version_name: string | null;
  game_rules: WorldRule[];
}

export interface WorldSettingsPatch {
  name?: string;
  game_mode?: string;
  difficulty?: string;
  allow_commands?: boolean;
  seed?: string;
  game_rules?: WorldRule[];
}

export interface GameRuleLang {
  name: string | null;
  description: string | null;
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

/// 某个版本导入后是否使用独立目录
export interface PclIsolation {
  version_id: string;
  /// true 表示隔离（mods 等在该版本目录里），false 表示与根目录共用
  isolated: boolean;
  /// 判定依据：pcl（PCL 设置） / content（按文件位置推断）
  source: string;
}

/// 一个 PCL 游戏文件夹
export interface PclGameDirInfo {
  path: string;
  /// 是否为 PCL 当前选中的文件夹
  active: boolean;
  /// 清单齐全的版本数
  version_count: number;
  /// 各版本导入后应使用的隔离状态
  isolation: PclIsolation[];
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
  /// PCL 里配置的全部游戏文件夹，第一个是当前选中的
  game_dirs: PclGameDirInfo[];
  findings: string[];
}

/// HMCL 里的一个启动配置（对应一个游戏文件夹）
export interface HmclGameDirInfo {
  path: string;
  /// HMCL 的启动配置名称
  name: string;
  /// 是否为 HMCL 上次使用的配置
  active: boolean;
  /// 清单齐全的版本数
  version_count: number;
  /// 各版本导入后应使用的隔离状态
  isolation: PclIsolation[];
}

export interface HmclPreview {
  root: string;
  config_path: string | null;
  game_dir: string | null;
  game_dirs: HmclGameDirInfo[];
  java_path: string | null;
  memory_auto: boolean | null;
  memory_mb: number | null;
  window_width: number | null;
  window_height: number | null;
  fullscreen: boolean | null;
  download_source: string | null;
  java_args: string | null;
  game_args: string | null;
  pre_launch: string | null;
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

/// 用新版整合包文件更新实例时的单个文件
export interface PackUpdateFile {
  path: string;
  /// added / changed / removed
  kind: string;
}

export interface PackUpdatePreview {
  format: string;
  name: string;
  /// 旧清单里的版本，找不到时为空
  old_version: string;
  new_version: string;
  game_version: string;
  loader: string;
  loader_version: string | null;
  added: number;
  changed: number;
  removed: number;
  /// 覆盖文件数量（overrides / client-overrides）
  overrides: number;
  download_bytes: number;
  files: PackUpdateFile[];
  warnings: string[];
}

export interface PackUpdateResult {
  downloaded: number;
  removed: number;
  overrides: number;
  failed: string[];
  /// 解析/准备阶段跳过的问题，没有重试入口
  warnings: string[];
  /// 更新前的文件备份目录，null 表示没有覆盖旧文件
  backup_dir: string | null;
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
  /// 清单里标记为可选的文件数；默认不装，导入表单里可以勾选
  optional_count: number;
  /// 加载器启动器能否自动安装；false 时按原版导入并保留全部文件
  loader_supported: boolean;
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
  /// 清单里没装上的文件：可以逐个补下，也可以去项目页手动下载
  failures: ImportFailure[];
  /// 不属于单个文件的提醒：加载器暂不支持、清单写入失败等
  warnings: string[];
  /// 导入完成后的使用建议，例如内存分配
  hints: string[];
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

export interface LocalCape {
  name: string;
  saved_at: number;
  data_url: string;
}

export interface LittleSkinStatus {
  configured: boolean;
  authorized: boolean;
  name: string;
  uid: number;
  redirect_uri: string;
  scope: string;
}

export interface LittleSkinClosetItem {
  tid: number;
  name: string;
  kind: string;
  preview: string;
  texture: string;
}

export interface LittleSkinClosetPage {
  total: number;
  page: number;
  last_page: number;
  items: LittleSkinClosetItem[];
}

export interface LittleSkinPlayer {
  pid: number;
  name: string;
  tid_skin: number;
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

/// 本地收藏的社区项目
export interface Favorite {
  id: string;
  source: string;
  /// 资源分类：mods / modpacks / resourcepacks 等
  kind: string;
  name: string;
  slug: string;
  summary: string;
  website_url: string;
  logo_url: string;
  author: string;
  download_count: number;
  class_id: number;
  updated: string;
  categories: string[];
  /// 收藏时间（Unix 秒）
  added_at: number;
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
  /// 第三方账号在皮肤站上的全部角色
  profiles: AuthlibProfile[];
}

/// 皮肤站上的角色档案
export interface AuthlibProfile {
  id: string;
  name: string;
}

export interface AuthlibLoginResult {
  /// 账号下有多个角色时需要前端先选择
  needs_selection: boolean;
  profiles: AuthlibProfile[];
  /// 站点已经绑定/默认的角色
  selected: AuthlibProfile | null;
  account: AccountView | null;
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
  /// 该步骤的完成百分比；历史步骤为 100，当前步骤为整体进度
  percent?: number;
}

export interface ProgressSnapshot {
  /// 所属任务 ID；下载中心按它把进度分派到对应任务
  job: string;
  stage: string;
  message: string;
  steps: ProgressStep[];
  files_done: number;
  files_total: number;
  bytes_done: number;
  bytes_total: number;
  elapsed_ms: number;
  /// 整个任务的总进度（0-100），跨阶段累加，不会每步归零
  overall: number;
  active: boolean;
  paused: boolean;
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
  optimizeMemory: () => invoke<MemoryRelease>("optimize_memory"),
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
  installVersion: (versionId: string, jobId?: string) =>
    invoke<InstalledVersion>("install_version", { versionId, jobId }),
  cancelInstall: (jobId?: string) => invoke<void>("cancel_install", { jobId }),
  pauseJob: (jobId: string) => invoke<void>("pause_job", { jobId }),
  resumeJob: (jobId: string) => invoke<void>("resume_job", { jobId }),
  activeJobs: () => invoke<string[]>("active_jobs"),
  loaderOptions: (versionId: string) =>
    invoke<LoaderOption[]>("loader_options", { versionId }),
  loaderVersions: (versionId: string, loader: string) =>
    invoke<LoaderVersion[]>("loader_versions", { versionId, loader }),
  installLoader: (versionId: string, loader: string, loaderVersion?: string, jobId?: string) =>
    invoke<InstalledVersion>("install_loader", { versionId, loader, loaderVersion, jobId }),
  reinstallLoader: (versionId: string, jobId?: string) =>
    invoke<InstalledVersion>("reinstall_loader", { versionId, jobId }),
  instanceContext: (versionId: string) =>
    invoke<InstanceContext>("instance_context", { versionId }),
  checkInstanceFiles: (versionId: string) =>
    invoke<InstanceCheck>("check_instance_files", { versionId }),
  repairInstance: (versionId: string, options?: RepairOptions, jobId?: string) =>
    invoke<RepairReport>("repair_instance", { versionId, options: options ?? null, jobId }),
  previewRepair: (versionId: string, options?: RepairOptions) =>
    invoke<RepairPreview>("repair_preview", { versionId, options: options ?? null }),
  repairBackups: (versionId: string) =>
    invoke<RepairBackup[]>("repair_backups", { versionId }),
  restoreRepairBackupAt: (versionId: string, backupId: string) =>
    invoke<number>("repair_restore_at", { versionId, backupId }),
  repairRestore: (versionId: string) =>
    invoke<number>("repair_restore", { versionId }),
  shaderSupport: (versionId: string) =>
    invoke<ShaderSupport>("shader_support", { versionId }),
  installShaderLoader: (versionId: string, jobId?: string) =>
    invoke<string>("install_shader_loader", { versionId, jobId }),
  preflightCheck: (versionId: string) =>
    invoke<PreflightReport>("preflight_check", { versionId }),
  gameOptionsPresets: () => invoke<GameOptionsPreset[]>("game_options_presets"),
  saveGameOptionsPreset: (versionId: string, name: string) =>
    invoke<GameOptionsPreset>("save_game_options_preset", { versionId, name }),
  applyGameOptionsPreset: (versionId: string, id: string) =>
    invoke<string>("apply_game_options_preset", { versionId, id }),
  deleteGameOptionsPreset: (id: string) =>
    invoke<void>("delete_game_options_preset", { id }),
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
  instanceConfigTree: (versionId: string) =>
    invoke<ConfigEntry[]>("instance_config_tree", { versionId }),
  instanceReadText: (versionId: string, path: string) =>
    invoke<ConfigText>("instance_read_text", { versionId, path }),
  instanceWriteText: (
    versionId: string,
    path: string,
    content: string,
    encoding: string
  ) => invoke<void>("instance_write_text", { versionId, path, content, encoding }),
  instanceThumb: (versionId: string, kind: string, name: string) =>
    invoke<string | null>("instance_thumb", { versionId, kind, name }),
  instanceModMeta: (versionId: string) =>
    invoke<InstanceModMeta[]>("instance_mod_meta", { versionId }),
  instanceImportFiles: (versionId: string, kind: string, paths: string[]) =>
    invoke<number>("instance_import_files", { versionId, kind, paths }),
  instanceImportMods: (versionId: string, paths: string[], jobId?: string) =>
    invoke<ModImportOutcome>("instance_import_mods", { versionId, paths, jobId }),
  dropInspect: (paths: string[]) => invoke<DroppedItem[]>("drop_inspect", { paths }),
  instancePaths: (versionId: string) =>
    invoke<InstancePaths>("instance_paths", { versionId }),
  instanceWorlds: (versionId: string) =>
    invoke<WorldSummary[]>("instance_worlds", { versionId }),
  instanceWorldSeed: (versionId: string, folder: string) =>
    invoke<WorldSeed>("instance_world_seed", { versionId, folder }),
  instanceBackups: (versionId: string) =>
    invoke<WorldBackup[]>("instance_backups", { versionId }),
  instanceBackupsDir: (versionId: string) =>
    invoke<string>("instance_backups_dir", { versionId }),
  instanceBackupWorld: (versionId: string, folder: string) =>
    invoke<WorldBackup>("instance_backup_world", { versionId, folder }),
  instanceRestoreBackup: (versionId: string, path: string) =>
    invoke<string>("instance_restore_backup", { versionId, path }),
  instanceImportWorld: (versionId: string, path: string) =>
    invoke<string>("instance_import_world", { versionId, path }),
  instanceDeleteBackup: (versionId: string, path: string) =>
    invoke<void>("instance_delete_backup", { versionId, path }),
  instanceWorldSettings: (versionId: string, folder: string) =>
    invoke<WorldSettings>("instance_world_settings", { versionId, folder }),
  instanceSaveWorldSettings: (
    versionId: string,
    folder: string,
    patch: WorldSettingsPatch
  ) =>
    invoke<WorldSettings>("instance_save_world_settings", {
      versionId,
      folder,
      patch,
    }),
  instanceGameRuleLang: (versionId: string) =>
    invoke<Record<string, GameRuleLang>>("instance_game_rule_lang", {
      versionId,
    }),
  appVersion: () => invoke<string>("app_version"),
  appPortable: () => invoke<boolean>("app_portable"),
  pclImportPreview: (path: string) =>
    invoke<PclPreview>("pcl_import_preview", { path }),
  pclImportApply: (path: string) => invoke<Config>("pcl_import_apply", { path }),
  hmclImportPreview: (path: string) =>
    invoke<HmclPreview>("hmcl_import_preview", { path }),
  hmclImportApply: (path: string) => invoke<Config>("hmcl_import_apply", { path }),
  probeDownloadSources: () => invoke<SourceProbe[]>("probe_download_sources"),
  retryFailedDownloads: (sourceJob?: string, jobId?: string) =>
    invoke<RetryDownloads>("retry_failed_downloads", { sourceJob, jobId }),
  modpackRetryFile: (sourceJob: string, projectId: number, fileId: number, jobId?: string) =>
    invoke<RetryDownloads>("modpack_retry_file", { sourceJob, projectId, fileId, jobId }),
  playStats: () => invoke<PlayStats>("play_stats"),
  updateCheck: (url: string) => invoke<UpdateInfo>("update_check", { url }),
  updateDownload: (url: string, version: string) =>
    invoke<string>("update_download", { url, version }),
  updateInstall: (path: string) => invoke<void>("update_install", { path }),
  portableUpdateInstall: (path: string) =>
    invoke<void>("portable_update_install", { path }),
  exportSettings: (path: string) => invoke<void>("export_settings", { path }),
  importSettings: (path: string) => invoke<Config>("import_settings", { path }),
  cleanJunk: () => invoke<JunkReport>("clean_junk"),
  crashAnalyze: (versionId: string, exitCode?: number | null) =>
    invoke<CrashReport>("crash_analyze", { versionId, exitCode: exitCode ?? null }),
  crashExport: (versionId: string) =>
    invoke<string>("crash_export", { versionId }),
  crashUndoAddedMods: (versionId: string) =>
    invoke<string[]>("crash_undo_added_mods", { versionId }),
  crashExportFeedback: (versionId: string, ruleIds: string[]) =>
    invoke<string>("crash_export_feedback", { versionId, ruleIds }),
  crashRuleStatus: () => invoke<CrashRuleStatus>("crash_rule_status"),
  crashRuleRollback: () => invoke<number>("crash_rule_rollback"),
  crashShare: (versionId: string) =>
    invoke<string>("crash_share", { versionId }),
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
  modpackDownload: (url: string, fileName: string, jobId?: string) =>
    invoke<string>("modpack_download", { url, fileName, jobId }),
  modpackTempRemove: (path: string) => invoke<void>("modpack_temp_remove", { path }),
  modpackImport: (path: string, options: ModpackImportOptions, jobId?: string) =>
    invoke<ModpackImportResult>("modpack_import", { path, options, jobId }),
  modpackExportPreview: (versionId: string) =>
    invoke<ExportPreview>("modpack_export_preview", { versionId }),
  modpackExport: (versionId: string, options: ExportOptions, jobId?: string) =>
    invoke<ExportResult>("modpack_export", { versionId, options, jobId }),
  modpackUpdatePreview: (versionId: string, path: string) =>
    invoke<PackUpdatePreview>("modpack_update_preview", { versionId, path }),
  modpackUpdateApply: (
    versionId: string,
    path: string,
    installOptional?: boolean,
    jobId?: string
  ) =>
    invoke<PackUpdateResult>("modpack_update_apply", {
      versionId,
      path,
      installOptional: installOptional ?? null,
      jobId,
    }),
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
  favoritesList: () => invoke<Favorite[]>("favorites_list"),
  favoritesToggle: (favorite: Favorite) => invoke<boolean>("favorites_toggle", { favorite }),
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
  cfInstallFile: (versionId: string, kind: string, url: string, fileName: string, jobId?: string) =>
    invoke<InstallOutcome>("cf_install_file", { versionId, kind, url, fileName, jobId }),
  /// 重新检查并补齐某个已装模组的前置
  retryContentDeps: (instanceId: string, path: string, jobId?: string) =>
    invoke<DependencyReport>("retry_content_deps", { instanceId, path, jobId }),
  cfInstallWorld: (versionId: string, url: string, fileName: string, jobId?: string) =>
    invoke<string>("cf_install_world", { versionId, url, fileName, jobId }),
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
    invoke<AuthlibLoginResult>("authlib_login", { root, username, password }),
  /// 多角色账号：选择要使用的角色并完成建号
  authlibLoginSelect: (profileId: string) =>
    invoke<AuthlibLoginResult>("authlib_login_select", { profileId }),
  /// 在已保存的第三方账号里切换角色
  authlibSwitchProfile: (accountId: string, profileId: string) =>
    invoke<AccountView>("authlib_switch_profile", { accountId, profileId }),
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
  exportSkinToCsl: (accountId: string, versionId: string, pngBase64?: string | null) =>
    invoke<{ path: string }>("export_skin_to_customskinloader", {
      accountId,
      versionId,
      pngBase64: pngBase64 ?? null,
    }),
  uploadAccountSkin: (accountId: string, pngBase64: string, model: string) =>
    invoke<AccountView>("upload_account_skin", { accountId, pngBase64, model }),
  skinLibraryList: () => invoke<SkinLibraryItem[]>("skin_library_list"),
  capeLibraryList: () => invoke<LocalCape[]>("cape_library_list"),
  capeLibrarySave: (name: string, pngBase64: string) =>
    invoke<LocalCape>("cape_library_save", { name, pngBase64 }),
  capeLibraryRemove: (name: string) => invoke<void>("cape_library_remove", { name }),
  capeImport: (path: string) => invoke<string>("cape_import", { path }),
  capeExport: (path: string, pngBase64: string) =>
    invoke<void>("cape_export", { path, pngBase64 }),
  uploadAccountCape: (accountId: string, pngBase64: string) =>
    invoke<void>("account_cape_upload", { accountId, pngBase64 }),
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
  littleskinStatus: () => invoke<LittleSkinStatus>("littleskin_status"),
  littleskinSetApp: (clientId: string, clientSecret: string) =>
    invoke<LittleSkinStatus>("littleskin_set_app", { clientId, clientSecret }),
  littleskinAuthorize: () => invoke<LittleSkinStatus>("littleskin_authorize"),
  littleskinLogout: () => invoke<void>("littleskin_logout"),
  littleskinCloset: (page: number) =>
    invoke<LittleSkinClosetPage>("littleskin_closet", { page }),
  littleskinPlayers: () => invoke<LittleSkinPlayer[]>("littleskin_players"),
  littleskinApply: (pid: number, tid: number) =>
    invoke<void>("littleskin_apply", { pid, tid }),
  accountCapes: (accountId: string) =>
    invoke<CapeView[]>("account_capes", { accountId }),
  capeSetActive: (accountId: string, capeId: string | null) =>
    invoke<void>("cape_set_active", { accountId, capeId }),
  msLoginBegin: () => invoke<DeviceCode>("ms_login_begin"),
  detectJava: () => invoke<JavaInfo[]>("detect_java"),
  addJava: (path: string) => invoke<JavaInfo>("add_java", { path }),
  javaRuntimeMajors: (source: string) =>
    invoke<number[]>("java_runtime_majors", { source }),
  installJava: (major: number, source: string, setAsDefault?: boolean, jobId?: string) =>
    invoke<JavaInfo>("install_java", { major, source, setAsDefault, jobId }),
  deleteJava: (path: string) => invoke<boolean>("delete_java", { path }),
  launchVersion: (versionId: string) => invoke<number>("launch_version", { versionId }),
  launchServer: (versionId: string, address: string) =>
    invoke<number>("launch_server", { versionId, address }),
  lanRooms: () => invoke<LanView>("lan_rooms"),
  hallRooms: () => invoke<HallRoom[]>("hall_rooms"),
  roomModCheck: (
    versionId: string,
    roomVersion: string,
    roomLoader: string,
    roomMods: string[]
  ) =>
    invoke<RoomModCheck>("room_mod_check", {
      versionId,
      roomVersion,
      roomLoader,
      roomMods,
    }),
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
  serverDiagnostic: () => invoke<string>("server_diagnostic"),
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

export async function pickJavaExe(): Promise<string | null> {
  const result = await open({
    directory: false,
    multiple: false,
    title: "选择 java.exe（JDK/JRE 的 bin 目录下）",
    filters: [{ name: "Java 运行时", extensions: ["exe"] }],
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

export async function pickWorldZip(): Promise<string | null> {
  const result = await open({
    directory: false,
    multiple: false,
    title: "选择存档压缩包",
    filters: [{ name: "Minecraft 存档", extensions: ["zip", "mcworld"] }],
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

export async function saveCapeDialog(defaultName = "cape.png"): Promise<string | null> {
  const result = await save({
    title: "导出披风",
    defaultPath: defaultName,
    filters: [{ name: "Minecraft 披风", extensions: ["png"] }],
  });
  return typeof result === "string" ? result : null;
}

export async function pickCapePng(): Promise<string | null> {
  const result = await open({
    title: "选择披风 PNG（64×32）",
    multiple: false,
    directory: false,
    filters: [{ name: "Minecraft 披风", extensions: ["png"] }],
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
