# 调研报告：加载器兼容 A3 与 B 阶段

日期：2026-11-26。只读调研，未改任何代码。供写 A3 / B1–B4 实施计划使用。

> 引用格式 `文件:行号` 以当前工作区为准。清单里预想的函数名与实际对不上的地方已注明。
> 样本文件留存：`%TEMP%\cleanroom\`（installer.jar、version.json、MMC 实例解包）、`%TEMP%\prism-meta\`（各组件 index 与 patch json）。

---

## 一、Java 25（A3）

### 1. java.rs 现状

- `JavaInfo`（java.rs:17-26）：path / version / major / source / kind（JDK/JRE）/ vendor。
- `inspect_java`（java.rs:38-77）：跑 `java -version` 解析版本；`1.x` 取第二段，否则取第一段。
- `detect`（java.rs:245-313）扫描顺序：
  1. `cfg.extra_java_paths`（手动添加）
  2. `JAVA_HOME`
  3. **运行时根目录一层子目录**的 `bin/java.exe`（java.rs:273-282）。根目录 = `config::runtime_dir()`（= `data_dir()/runtime`，config.rs:256-258）与 `cfg.java_runtime_root()`（用户自定义 `java_install_dir`，config.rs:516-521）
  4. 注册表（java.rs:178-203：JavaSoft JRE/JDK、Adoptium、Microsoft、Zulu、Liberica）
  5. 常见安装目录（java.rs:205-228）→ `where java`（PATH）→ 旧 `cfg.java_path`
- `pick_for_major`（java.rs:354-366）：先精确匹配；`required <= 8` 不允许高版本顶替；否则取「最小的更高版本」。
- `available_runtime_majors`（java.rs:385-409）：拉 `RUNTIME_ALL_URL`（java.rs:13-14，launchermeta all.json），只认组件名以 `jre-` / `java-runtime` 开头且不含 `snapshot` 的条目，按 `parse_major` 归并出大版本列表。
- `install_runtime`（java.rs:420-492）：all.json → 取**第一个** `parse_major == major` 的条目（java.rs:433-443）→ 下载该组件 manifest → 逐文件 `FileTask`（带 sha1）→ 装到 `java_runtime_root()/<component>`。
- 第三方发行版：`vendor_majors`（java.rs:412-418）temurin/zulu = `[8,11,16,17,21,25]`、corretto = `[8,11,17,21,25]`——**25 都在列**。`install_runtime_vendor`（java.rs:528-541）按 source 分发；Temurin 走 adoptium API（java.rs:543-571，zip + sha256 校验），Zulu 走 azul API（java.rs:573-609），Corretto 固定 URL（java.rs:611-639）；统一 `install_zip_runtime`（java.rs:651-702）解压后找含 `bin/java.exe` 的一层（`find_runtime_root`，java.rs:768-777）。

**⚠️ 隐患（A3 计划要处理）**
1. `install_runtime` 选组件用 `best.is_none()` 取第一个匹配（java.rs:439），而 `windows` 是 `HashMap<String, Vec<RuntimeEntry>>`——**迭代顺序不确定**，同一大版本可能这次装 `java-runtime-beta` 下次装别的。应优先 `java-runtime`（稳定）组件。
2. `available_runtime_majors` 同样受 HashMap 顺序影响，但只取 major 集合，无影响。

### 2. required_java

`VersionDetail::required_java()`（models.rs:75-81）= `java_version.majorVersion`，缺省 8。任意大版本（含 25）都能正确返回，无需改动。`JavaVersion` 结构（models.rs:176-180）：`component` + `majorVersion`。

### 3. find_java 的兼容规则

`launch::find_java`（launch.rs:132-150）：
- 手动 `cfg.java_path` 优先：`info.major >= required && (required > 8 || info.major == required)` 才接受。
- 否则 `java::detect` + `pick_for_major`（精确优先，`required<=8` 不顶替）。
- 找不到 → `Err("LAUNCH_NO_JAVA:当前版本需要 Java {required}，本机未找到可用运行时")`。

**结论**：Java 21 不能顶替 Java 25（25 需要精确匹配或更高）。缺失时的报错码 `LAUNCH_NO_JAVA` 前端已有处理（见第 5 条）。`find_java_exact`（launch.rs:154-166）用于加载器安装，要求精确版本。

**已有对高版本 Java 的适配**（对 Cleanroom 直接受益，无需新做）：
- Java ≥ 24 自动加 `--sun-misc-unsafe-memory-access=allow`（launch.rs:450-452，受 `disable_lwjgl_unsafe` 开关控制）。
- ZGC 参数按 Java ≥ 21 用分代 ZGC（launch.rs:88-96）。

### 4. crash.rs 的 Java 相关规则（误报评估）

| 规则 id | 位置 | 触发文本 | 1.20.1 + Java 25 会误报吗 |
| --- | --- | --- | --- |
| `java-version` | crash.rs:402-418 | `UnsupportedClassVersionError` / `class file major version` / `has been compiled by a more recent version` | **不会**。高版本跑低版本类文件不产生这些错误；25 ≥ 17 无风险 |
| `forge-java-incompatible` | crash.rs:927-938 | 旧 Forge + Java 8u321+ | 不涉及 |
| `mod-needs-java11` | crash.rs:955-971 | `class file version 55.0` 等 | 不涉及（只有 Java 8 才会缺 55） |
| `java-32bit-exact` | crash.rs:974-989 | Invalid maximum heap size 等 | 不涉及 |
| 其余 java 类 | out-of-memory(419+)、vm-options(441+)、java-agent-failed(1106+)、insufficient-memory(1213)、startup-too-short(1451-1467) | — | 不涉及 |

`expected_java`（crash.rs:2333-2360）：1.20.5+/21.x+/26+ → 21；1.17–1.20.4 → 17；其余 → 8。**`1.20.1` → 17**。
`apply_context` 对 `java-version`（crash.rs:2413-2435）会插入「当前实例：1.20.1，需要 Java 17；启动器目前选择的是 Java 25」这类建议。规则本身在高 Java 下不触发；**仅当**它因其他原因触发时，硬编码的「1.20.5+ 需要 Java 21…」建议文案（crash.rs:411）对 Java 25 场景略显过时——A3 顺手把这句改成查 `expected_java` 的动态文案即可，非必须。

`build_context`（crash.rs:2362-2373）里的 `java_major` 来自 `launch::find_java(cfg, required.max(8))` 的实际结果，不是实例声明的版本——诊断里显示的「实际 Java」是启动时会用的那个，语义正确。

### 5. 前端 Java 流程

- 安装：store.ts:1136-1152 `installJava(major, source)` → enqueueJob（stage `java`）→ `api.installJava(major, source, jobId)` → `refreshJava()` + `refreshConfig()`。
- 命令：commands.rs:2622-2646 `install_java` → `java::install_runtime_vendor`；**成功后把 `cfg.java_path` 全局设为新装的运行时**（commands.rs:2640）。⚠️ 注意：给 GTO 装 25 后，其他需要 17 的实例也会优先用 25（25 ≥ 17 会被 `find_java` 接受）。实例级 `java_path` 覆盖已存在（instance.rs:211-213 `apply()`），A3 可考虑装完只提示、由用户决定是否全局切。
- 版本下拉：SettingsView.vue:764-770（mojang/temurin/zulu/corretto 四源）、773-778 `fallbackMajors`（mojang 已含 25）、784-800 `loadJavaOptions` 走 `api.javaRuntimeMajors`（commands.rs:2609-2619：mojang → `available_runtime_majors`，其他 → `vendor_majors`）。
- 启动前检查：store.ts:1226-1235 `javaMismatchFor`（只在手动指定 java_path 且低于要求时提示，读 `installed[].java_major`）；1246-1267 `preflightGate`（后端 preflight，非阻断确认一次）；`LAUNCH_NO_JAVA` 报错 → `store.javaPrompt` 弹窗（store.ts:1329-1332、1397-1400）。
- Java 列表/删除：SettingsView.vue:1486-1491、commands.rs:2651+ `delete_java`（只允许删 runtime 目录内的）。
- preflight.rs 目前**没有**「要求的 Java 大版本本机没有」这一条（只有 32 位检查 preflight.rs:264-295）。A3 要把「缺 Java 25 + 一键安装入口」放进 preflight 或 javaPrompt。

### 6. 外部核实

实测 `https://launchermeta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json`（windows-x64，过滤规则同 java.rs:394-399）：**可装大版本 = 8, 16, 17, 21, 25**。Java 25 官方可装 ✅（Mojang component 名 `java-runtime-epsilon`，与 Cleanroom `compatibleJavaName` 一致）。

---

## 二、自定义加载器 profile 底座（B1）与 Cleanroom（B2）

### 7. install.rs 可复用件

- `resolve_detail`（install.rs:998-1048）：沿 `inheritsFrom` 走链（带环检测）；`jar_id` = 链上第一个「有 `versions/<id>/<id>.jar` 或 `downloads.client`」的层级（install.rs:1010-1028）。合并规则 `merge_child`（install.rs:937-996）：
  - kind / main_class / asset_index / downloads / java_version / logging / assets / client_version / docl：**子版本覆盖**；
  - libraries：按**完整 name** 替换或追加（install.rs:966-972）——group:artifact 去重不在合并阶段做；
  - `arguments.game/jvm`：子版本**前插**（install.rs:974-986）；
  - `minecraftArguments`：子版本**整体覆盖**（install.rs:993-995）。
- 库去重（供 B1 的 `group:artifact` 语义直接参考）：
  - `maven_key_version`（install.rs:1052-1064）：解析 `group:artifact:version[:classifier][@ext]` → key = `group:artifact[:classifier]`；
  - `compare_versions`（install.rs:1068-1094）：数字段按数值、否则字典序；
  - `library_file_paths`（install.rs:1096-1153）：跳过 rules 不允许的与 native 库；按最终路径去重（PCL 清单重复条目），再按 maven key 折叠成**更高版本**。
  - ⚠️ **`999999.0-empty` 占位版本按数值比较会赢过任何真实版本**。B3 合并 MMC patch 时必须把占位库按「移除原库」处理，不能进这个去重流程。
- `maven_path`（install.rs:217-230）：标准 Maven 仓库路径；`library_artifact`（install.rs:242-267）支持 `downloads.artifact` 与 Fabric 式 `name + url` 两种来源；`library_native_download`（install.rs:270-290）按 `natives.windows` classifier 取 natives jar。
- `library_is_native`（install.rs:232-240）：`natives` 含 `windows` 键或带 `extract` 即 native（旧式）；1.19+ 的 natives classifier 库走普通 classpath。
- `merged_classpath_entries`（install.rs:1155-1176）：库 + jar（去重）。
- `natives_dir` / `ensure_natives`（install.rs:1194-1230）：natives 解压到 `versions/<id>/natives`（含 java/jna/lwjgl/netty 子目录），排除 `extract.exclude`（缺省 META-INF/）。**启动时若 natives 缺失会自动补**（launch.rs:287-291）——但只解压已在 `libraries/` 下的 jar，**不会下载缺失的 natives jar**。
- 资源（assets）：**只在 `install_version` 里下载**（install.rs:332 起；assetIndex 部分 422-489）：竞速拉索引 → 写 `assets/indexes/<id>.json` → 按 `resources.download.minecraft.net/<2>/<hash>` 批量下载（失败只汇总不致命）。log4j 配置 491-505。⚠️ **没有独立的「按已有 version.json 补库 + 补资源」函数**：`install_loader`（install.rs:2622 起）只下载合并后清单的普通库（2678-2696，跳过 native！），natives jar 依赖父原版装好。B1 需要抽出或新写「库（含 natives classifier）+ client jar + assetIndex/objects」的安装函数；`install_version` 对 `inheritsFrom` 直接 bail（install.rs:354-356）。
- `install_version` 其余流程（客户端 jar 363-371、库 373-408、natives 解压 507-520+）可整体参考。

### 8. VersionDetail / Library 字段（models.rs）

- `VersionDetail`（models.rs:46-72）：`id`、`kind(type)`、`main_class`、`assets`、`client_version`、`asset_index{id,sha1,size,url,total_size}`（84-91）、`downloads{client,server}`（93-98，`DownloadRef{sha1,size,url,path}` 100-107）、`libraries`、`arguments{game,jvm}`（150-156）、`minecraft_arguments`、`java_version{component,majorVersion}`、`logging{client{argument,file,kind}}`（182-193）、`inherits_from`、`docl(_docl，序列化时 None 跳过)`。**Deserialize + Serialize 双向派生**——`finalize_self_contained` 靠 Serialize 回写合并清单（modpack.rs:1741 起）。
- `Library`（models.rs:109-123）：`name`、`downloads{artifact,classifiers}`、`rules`（132-143，`OsRule{name,arch,version}`）、`natives(map)`、`extract{exclude}`、`url`（Fabric 式 maven 根）、`sha1`、`size`。
- `Rule`/`rules_allow`（install.rs:186-199）：最后一条匹配规则决定 allow/disallow，标准语义。
- `ArgValue` untagged（models.rs:158-173）：`Plain(String)` 或 `{rules, value: One|Many}`——自包含 Cleanroom 清单没有 `arguments`，不涉及。

### 9. 启动命令行拼装（launch.rs `build`，266-516）

- 游戏目录 = 实例目录（launch.rs:283，版本隔离）；首次启动写默认 options.txt（258-264）。
- natives 缺失 → 启动时 `ensure_natives` 补（287-291）。
- Java = `find_java(required)`（293-294）。
- jar 处理：`jar_id != version_id` 时把 jar 复制进实例目录（313-335）；主类为 `net.neoforged.fml.startup*`（FML 11）时 jar 不进 classpath（301-312）。
- classpath：`merged_classpath_entries` + 可能补 `version_meta_jar`（337-352，给 CSL 用的只含 version.json 小包）。
- 变量替换（359-408）：`assets_index_name = assetIndex.id || assets || "legacy"`（376-383）——Cleanroom 是 `1.12`，能对上；`library_directory` 也有（408）。
- JVM 参数：`arguments.jvm`（410-415）→ logging（416-429）→ Xmx/Xms（431-435）→ memory_args（436-438）→ UTF-8 编码三连（439-441）→ launcher brand（442-448）→ Java≥24 unsafe 放行（450-452）→ authlib injector（454-458）→ 自定义 java_args（460-462）→ `-Djava.library.path` 兜底（463-468）→ `-cp` 兜底（469-472）。
- 游戏参数：**`arguments.game` 或旧式 `minecraftArguments`（按空白切分，474-483）**，追加在主类之后（485-513）。旧式清单里的 `--tweakClass` 会原样作为游戏参数传给主类——launchwrapper / Foundation 这类「主类自己解析参数」的形态天然兼容 ✅。Cleanroom 的 `minecraftArguments` 带 `--tweakClass FMLTweaker`，DOCL 现有拼装即可传到位，无需特判。
- 主类缺省 `net.minecraft.client.main.Main`（486-490）。
- `spawn`（627-777）：日志双写 game.log、prefer_gpu 注册表、pre_launch、退出监听 → stats + game-exit 事件 + code 0 时存 mods 快照。

### 10. HMCL / PCL 导入的现状（可复用性有限）

两者都是「把 game root 指到外部启动器的目录」，**不复制文件**：
- hmclimport.rs `apply`（746 起）：解析 HMCL 配置 → 改 `cfg.game_dir` 等；共享工具来自 `crate::launcher_layout`（hmclimport.rs:12-15：`content_isolation, count_versions, detect_with_root, root_has_content, version_ids`）。
- pclimport.rs `apply`（663 起）：同样指 root；顺带原地清理版本清单重复库：`normalize_version_jsons`（pclimport.rs:497-533）→ `clean_library_versions`（450-491，按 group:artifact 保留更高版本，用 `install::compare_versions`），留 `.pcl-import.bak` 备份。⚠️ 同样的占位库问题：若外部目录里真有 `999999.0-empty` 也会被当成「更高版本」保留——B1/B3 若复用 `compare_versions` 需先滤占位。
- 对 B1 的可复用件：`ziputil.rs`（zip 修复/读取）、`modpack.rs::extract_entries` / `safe_relative`（modpack.rs:2410+ / 1667+）、`instance::save` 元数据、下载层（缓存 + `run_tasks`）。文件复制导入的现成范式是 modpack 的 generic 分支。

### 11. 外部事实：Cleanroom 发布物实测

最新 release **0.7.16-alpha**（gh CLI 查询；计划文档里的 0.7.14-alpha 已过时，版本号会动，代码里别写死）：

**A. installer.jar（15,336,798 B）**——自包含，**不必跑 GUI 安装器**：
- jar 内含 `install_profile.json` + `version.json`（121,260 B）+ 自带 maven 库条目（共 1903 个 zip 条目）。
- `version.json` 关键内容：
  - `id: "Cleanroom-0.7.16-alpha"`，**无 `inheritsFrom`**，无 `arguments{}`；
  - `mainClass: "top.outlands.foundation.boot.Foundation"`；
  - `javaVersion: {component: "java-runtime-epsilon", majorVersion: 25}`；
  - `assets: "1.12"`，`assetIndex: {id: "1.12", sha1: a21e1ded1a24ea1548dd8db0cf30b6acb02655a9, url: piston-meta .../1.12.json, totalSize≈129.8 MB}`；
  - `downloads.client` = **1.12.2 原版 client.jar**（piston-data，sha1 `0f275bc1547d01fa5f56ba34bdc87d981ee12daf`，10,180,113 B）；
  - `minecraftArguments`（旧式，含 `--tweakClass net.minecraftforge.fml.common.launcher.FMLTweaker --versionType Cleanroom`）；
  - `libraries` 217 条：`org.lwjgl:lwjgl:3.4.3`（本体 + 各平台 natives classifier）、**完全没有 LWJGL2**（`lwjgl-platform` 2.9.x 不存在）、maven 源 = repo.maven.apache.org / maven.cleanroommc.com / libraries.minecraft.net。

**结论**：解包 version.json → 写 `versions/<id>/<id>.json` → 走 B1 自包含安装（client jar + 217 库含 natives + assets 1.12）→ 启动。不需要运行安装器，也不需要 LWJGL2 特殊处理。

**B. MMC 实例包（cleanroom-0.7.16-alpha.zip，13,786 B）**：
- 内容：`instance.cfg`（`InstanceType=OneSix` / `name` / `iconKey`，无内存等设置）、`mmc-pack.json`、`patches/net.minecraftforge.json`、`patches/org.lwjgl.json`、`libraries/*.jar` 共 **8 个 `999999.0-empty` 占位 jar**（icu4j-core-mojang、jutils、lwjgl、lwjgl-platform、lwjgl_util、oshi-core、patchy、platform）。
- `mmc-pack.json` components：`net.minecraft=1.12.2`、`org.lwjgl=3.4.3`、**`net.minecraftforge=0.7.16-alpha`（Cleanroom 挂在 Forge 的 uid 下）**。
- `patches/net.minecraftforge.json`：`name: "Cleanroom"`、`requires: [net.minecraft equals 1.12.2, org.lwjgl equals 3.4.3]`、`mainClass: Foundation`、**`compatibleJavaMajors: [25]`**、`compatibleJavaName: java-runtime-epsilon`、**`+tweakers: [FMLTweaker]`**、`+traits: [FirstThreadOnMacOS]`、`libraries` 128 条（带完整 `downloads.artifact`）。
- **⚠️ uid 不可信**：`net.minecraftforge` 这个 uid 在 Cleanroom 实例里装的是 Cleanroom——判断加载器必须读 patch 的 `mainClass`（现有 `generic_loader` / `profile_loader`，modpack.rs:1469-1556 已经这么做 ✅）。
- **⚠️ `999999.0-empty`**：见第 7 条，合并时按「移除被占位的原库」处理。

**assets 1.12 补充核实**：`https://piston-meta.mojang.com/v1/packages/a21e1ded.../1.12.json`（1305 个 objects）**没有 `virtual` 也没有 `map_to_resources` 字段** → 属于 1.7.3+ 的非虚拟内容寻址布局：游戏自己读 `assets/indexes/1.12.json` 做映射，DOCL 现有「写 index + 下 objects」流程直接适用。计划文档里「1.12 虚拟资产映射需实测」的风险**降级**——只有 `legacy` / `pre-1.6` 索引才需要 virtual 目录，Cleanroom 不涉及。

---

## 三、Prism / MultiMC 实例导入（B3、B4）

### 12. 现在 MMC 实例包拖进来会发生什么

- **拖入识别**（dropzone.rs `classify_names`，dropzone.rs:88-150）：依次查 `modrinth.index.json`、`manifest.json`（读内容确认 CF 格式）、`mods/` + config/scripts/libraries/server.properties（无清单整合包）、`level.dat`、`pack.mcmeta`、shaders、mod 标记。**`mmc-pack.json` / `instance.cfg` 完全没被识别** → Cleanroom 实例包（无 mods/ 目录）会被判成 `unknown`。
- **强行按整合包导入**：`parse_pack` generic 分支（modpack.rs:1180 起，generic 兜底 1297-1465）：
  - `generic_loader`（modpack.rs:1469-1526）会读 `patches/*.json` 的主类 → 能认出 cleanroom ✅（A2 的成果）；
  - 但 `detected_parent` 为空（没有 `versions/*.json`），提示仍是「通用压缩包没有加载器信息，需要选择父版本」（即使已经认出加载器，这句也会出现——小的 UX 瑕疵）；
  - 导入（modpack.rs:3503-3560）：**必须有已安装的父版本**（modpack.rs:3516 bail），实例 `inheritsFrom` 父版本；解压走 `excluded_generic`（modpack.rs:1674-1681：logs/、crash-reports/、screenshots/、**versions/**、usercache.json、launcher_profiles、session.lock、.part）；
  - 加载器装不了时实例描述/预检按 A2 的语义标注。
- **更新路径**：generic 包不能用于更新，直接 bail（modpack.rs:733-735）。

**B3 需要补的**：dropzone 认 `mmc-pack.json`/`instance.cfg`；独立的 `mmcimport.rs` 做组件合并；MMC 实例「没有父版本可选」的兜底（比如自动装原版 1.12.2 + 提示装 Cleanroom，或按 B1 走 installer）。

### 13. 外部事实：Prism meta v1（实测）

- **根 index** `GET /v1/index.json` → `{formatVersion:1, packages:[{name, uid, sha256}]}`，现有 uid：
  `com.azul.java`、`com.ibm.java`、`com.mumfrey.liteloader`、`net.adoptium.java`、`net.fabricmc.fabric-loader`、`net.fabricmc.intermediary`、`net.minecraft`、`net.minecraft.java`、`net.minecraftforge`、`net.neoforged`、`org.lwjgl`、`org.lwjgl3`、`org.quiltmc.quilt-loader`。
- **组件 index** `GET /v1/<uid>/index.json` → `{formatVersion, name, uid, versions:[{version, releaseTime, requires:[{uid, equals}], sha256, recommended?, type?}]}`。
  - forge index 有 **5093** 个版本；每条 `requires[].equals` 标明对应 MC 版本（如 `47.4.29` requires `net.minecraft equals 1.20.1`）。
  - liteloader 28 个版本，最新 `1.12.2-SNAPSHOT`（只到 1.12.2）。
- **单版本 patch** `GET /v1/<uid>/<version>.json`——**注意路径是 uid 级，不带 MC 版本段**（带 MC 段的写法全部 404）。实测 200：`/v1/net.minecraftforge/47.4.29.json`、`/v1/com.mumfrey.liteloader/1.12.2-SNAPSHOT.json`、`/v1/net.minecraft/1.20.1.json`。
- `net.minecraft` patch（1.20.1）顶层键：`+jvmArgs, +traits, assetIndex, compatibleJavaMajors, compatibleJavaName, formatVersion, libraries(41), logging, mainClass, mainJar, minecraftArguments, name, order, releaseTime, requires, type, uid, version`。`mainClass: net.minecraft.client.main.Main`，`assetIndex.id: 5`（1.20.1 的索引叫 `5`）。
- **forge patch（47.4.29）顶层键**：`formatVersion, libraries(30), mainClass, mavenFiles, minecraftArguments, name, order, releaseTime, requires, uid, version`：
  - `mainClass: io.github.zekerzhayard.forgewrapper.installer.Main`——**Prism 走 ForgeWrapper**，不是 DOCL 现有的 installer 路径；
  - `minecraftArguments` 带 `--launchTarget forgeclient --fml.forgeVersion 47.4.29 --fml.mcVersion 1.20.1 --fml.forgeGroup net.minecraftforge --fml.mcpVersion ...`；
  - **无 `+tweakers`**、`mavenRepositories` 为空、有 `mavenFiles`（ForgeWrapper 需要的额外 maven 产物）。
  - ⚠️ 对 B3 的设计含义：直接采用 Prism 的 forge patch 意味着引入 ForgeWrapper 及其 mavenFiles 依赖；另一条路是**只从实例读出 MC 版本 + Forge 版本号，然后走 DOCL 自己的 `install_forge_like`**（不用 meta 的 forge patch）。后者与 DOCL 现有安装/修复/重装链路一致，更省事。这个取舍留给计划阶段定。
- **liteloader patch（1.12.2-SNAPSHOT）**：`mainClass: net.minecraft.launchwrapper.Launch`，**`+tweakers: [com.mumfrey.liteloader.launch.LiteLoaderTweaker]`**，`libraries: [net.minecraft:launchwrapper:1.12, org.ow2.asm:asm-all:5.2, com.mumfrey:liteloader:1.12.2-SNAPSHOT]`，无 `minecraftArguments`。
- **合并语义（MMC OneSix）**：patch 按 `order` / 组件顺序叠加；`+libraries` 追加、`+jvmArgs` 追加、**`+tweakers` 追加且每个元素由启动器转成一条 `--tweakClass <t>` 追加到游戏参数**（Cleanroom 实例的 Forge patch 里没有 minecraftArguments，`--tweakClass` 全靠 +tweakers 生成——印证该语义）。**Forge + LiteLoader 顺序**：组件顺序 net.minecraft → net.minecraftforge → liteloader，tweakers 依序追加 → `--tweakClass FMLTweaker --tweakClass LiteLoaderTweaker`（FML 在前）。DOCL 的旧式参数是整体覆盖而非叠加（install.rs:993-995），B3 合并成最终 json 时要把 +tweakers 直接烘进 minecraftArguments，绕开这个差异。
- `requires` / `compatibleJavaMajors`：patch 里的 `requires` 是组件依赖声明（如 Cleanroom patch requires lwjgl equals 3.4.3），`compatibleJavaMajors` 直接对应 Java 版本要求（Cleanroom = [25]）。

---

## 四、给计划阶段的要点汇总

1. **A3 几乎全是接线活**：Java 25 Mojang / Temurin / Zulu / Corretto 都能装（vendor_majors 已含 25，Mojang 清单实测含 25）；缺的是「required Java 本机没有」的启动前提示 + 一键安装入口（preflight 或 javaPrompt），以及 install_runtime 选组件的确定性（优先稳定组件）。崩溃规则不会因 1.20.1 + Java 25 误报。
2. **B1 的核心工作量**是抽一个「按已有 version.json 装 client jar + 库（含 natives classifier）+ assets」的安装函数（现在只有 install_version 全家桶和 install_loader 的库-only 两个形态），其余（合并、去重、classpath、启动）全部现成。
3. **B2 走 installer.jar 自包含路径**，不需要 LWJGL2 处理；MMC 补丁包路径（B3）才有占位库和 uid 陷阱。
4. **`999999.0-empty` 与 `net.minecraftforge` uid 是 B3 的两个深坑**，测试必须覆盖。
5. **B3 的 Forge 建议复用 DOCL 自己的安装链**，Prism meta 只用来读版本号与补丁（fabric/quilt/liteloader 的 patch 可直接用；forge patch 绑定 ForgeWrapper）。
6. **assets 1.12 无虚拟布局问题**，现有流程适用；计划文档里对应风险可降级。
7. 旧式 `minecraftArguments` 里的 `--tweakClass` 已能透传（launch.rs:474-483 + 主类后追加），B2 的 Cleanroom 启动参数链路不需要新代码。
