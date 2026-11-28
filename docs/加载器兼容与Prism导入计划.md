# 加载器兼容与 Prism 实例导入计划

目标：让「GTO 这类只推荐 Prism 的整合包」和「Cleanroom / LiteLoader 等小众加载器」在 DOCL 里能用。

按阶段推进，**不绑定版本号**：可以和其他计划并行，做完一项勾一项，随时发版。

- **阶段 A（打底）**：GTO 类标准 CurseForge 包跑通 + 未知加载器不再静默误判 + Java 25 路径打通（低风险，先解决真实用户）
- **阶段 B（攻坚）**：自定义加载器 profile 底座 + Cleanroom + Prism / MultiMC 实例导入

> 维护规则：每完成一项就打勾并更新「状态」，不要只依赖对话上下文。

> **本文档从 A3 起的写法**：每个步骤都写给「没读过这份代码的模型」看——改哪里（文件 + 函数）、怎么改、坑在哪、不做什么、怎么验收。
> 实施前先读 [调研-加载器兼容-A3与B阶段.md](./调研-加载器兼容-A3与B阶段.md)，行号和外部格式的事实都在那份文档里，本文只引用结论不重复展开。
> 行号会随编辑漂移：定位时以函数名为准，行号只作参考。
> **计划与代码对不上时停下来报告，不要自行改计划。**

---

## 〇、实施须知（A3 与 B 阶段通用）

**开工前核对**
- 读一遍调研报告对应章节，再读计划里点名的函数。发现签名、行为、行号与计划描述不符，先停下来报告。
- 当前工作区有大量未提交改动（A0–A2 的成果）。**不要 stash、reset、checkout 或提交**，只在其上继续编辑。

**环境坑（本机实测）**
- 360 会拦截并删除 `src-tauri\target\debug\deps` 下新编译出的测试 exe，表现为 `拒绝访问 (os error 5)`。跑测试用：
  ```powershell
  $env:CARGO_TARGET_DIR="$env:TEMP\docl-test"; cargo test --lib
  ```
- GitHub REST API 未登录时很快触发限流。查 release 用 `gh release view --repo <owner>/<repo> --json tagName,assets`，输出重定向到文件再用 PowerShell 读（直接管道给 `ConvertFrom-Json` 会因转义出问题）。
- 本机访问 GitHub 需要代理：`curl.exe -x http://127.0.0.1:7897 ...`。
- Prism meta 的单版本路径是 `/v1/<uid>/<版本号>.json`，**不带 MC 版本段**；带 MC 段的写法一律 404，而且返回的是 GitHub Pages 的 404 HTML 页，不是 JSON——解析前必须先判断 HTTP 状态码。

**代码红线**
- 禁止为了编译通过加 `#[allow(...)]`、`unsafe` 或 `unwrap()` 到新增的业务路径上（测试代码除外）。
- 禁止修改已有测试的断言。行为确实要变，新加一个测试，并在报告里说明。
- 禁止顺手重构、改名、调整无关代码的格式。
- 新增的 `pub fn` 必须带中文文档注释，说明「为什么」而不是「做什么」。代码注释一律中文。
- 路径拼接后落盘前，必须经过类似 `modpack.rs::safe_relative` 的校验（拒绝 `..`、绝对路径、盘符）。从压缩包里读出来的文件名一律当作不可信输入。

**验证命令（每步做完都跑）**
```powershell
$env:CARGO_TARGET_DIR="$env:TEMP\docl-test"; cargo test --lib
$env:CARGO_TARGET_DIR="$env:TEMP\docl-test"; cargo clippy --lib --tests --message-format=short
pnpm exec vue-tsc --noEmit
pnpm test
pnpm build
```
基线：Rust 220 passed / 8 ignored；vitest 28 passed；主 chunk 约 623 KB（gzip 204 KB）。主 chunk 增幅超过 50 KB 要停下来报告。
clippy 有约 50 条**存量**告警，不用管；但要确认自己改过的文件里没有**新增**的告警。

**卡住时的报告格式**：计划的第几步、第一处报错的原文（不要贴整段日志）、该步骤的 `git diff`。同一处卡住两次就停，不要绕路硬凑。

---

## 一、调研结论（2026-11）

| 对象 | 事实 | 对 DOCL 的含义 |
| --- | --- | --- |
| GTO（GregTech Odyssey） | 仓库 `pack.toml` 为 `minecraft = "1.20.1"`、`forge = "47.4.20"`；Release 包是标准 CurseForge 格式（`manifest.json` + `overrides/`），169 个文件条目、约 420 个 config 文件 | **不是** Cleanroom、**不是** 1.12.2；现有 CF 导入链路理论上可直接导入，缺的是实测与兜底 |
| GTO 官方说明 | 「强烈推荐 Prism，其他启动器可能导致资源加载异常或原版配方丢失」 | 没有给出技术原因，需实测；暂不据此改核心逻辑 |
| Cleanroom 发布物 A | `cleanroom-<ver>.zip`（约 13 KB）：`instance.cfg` + `mmc-pack.json` + `patches/*.json` + `libraries/*-999999.0-empty.jar` | 面向 MultiMC 系；`mmc-pack.json` 复用 `net.minecraftforge` uid。**uid 不可信，只能靠 patch 的 mainClass 识别**（2026-11-26 实测 0.7.16-alpha） |
| Cleanroom 发布物 B | `cleanroom-<ver>-installer.jar`（约 15 MB）：内嵌完整 `version.json`，无 `inheritsFrom`，`mainClass = top.outlands.foundation.boot.Foundation`、217 个库（LWJGL 3.4.3，**不含 LWJGL2**）、`javaVersion = 25`、`assets = 1.12`、`minecraftArguments` 带 `--tweakClass FMLTweaker` | **不必跑 GUI 安装器**，解包 `version.json` 即可按自包含版本安装。版本号会动（0.7.14 → 0.7.16），**代码里不要写死** |
| Cleanroom assets | 索引 `1.12` 实测**没有** `virtual` / `map_to_resources` 字段（1305 个 objects） | 非虚拟布局，DOCL 现有「写 index + 下 objects」流程直接适用，无需特殊处理 |
| Prism meta | 根 `/v1/index.json`；组件 `/v1/<uid>/index.json`；单版本 **`/v1/<uid>/<版本号>.json`（uid 级，不带 MC 段）**。有 `com.mumfrey.liteloader`，没有 cleanroom | MMC 导入的通用组件走 Prism meta；Cleanroom 走官方 installer |
| Prism forge patch | `mainClass = io.github.zekerzhayard.forgewrapper.installer.Main`，带 `mavenFiles`，无 `+tweakers` | Prism 的 Forge 走 **ForgeWrapper**。B3 **不照抄**：Forge 只从 patch 里读版本号，安装走 DOCL 自己的 `install_forge_like` |
| Prism patch 合并语义 | `+libraries` / `+jvmArgs` / `+tweakers` 为**追加**；`+tweakers` 每项由启动器转成一条 `--tweakClass <t>` 追加到游戏参数；`libraries` / `minecraftArguments` 等不带 `+` 的键为**整体覆盖**；`999999.0-empty` 是「移除原库」的占位 | DOCL 的 `merge_child` 对旧式参数是整体覆盖（install.rs `merge_child`），B3 合并时要自己把 `+tweakers` 烘进最终的 `minecraftArguments` |
| LiteLoader | 仅 1.12.2 及更早（meta 上 28 个版本，最新 `1.12.2-SNAPSHOT`）；patch 的 `mainClass = net.minecraft.launchwrapper.Launch`，`+tweakers = [LiteLoaderTweaker]` | 与 Forge 组合时组件顺序为 minecraft → forge → liteloader，tweaker 依序追加（FML 在前） |

### 现有能力基线

| 能力 | 位置 |
| --- | --- |
| 加载器识别（forge / neoforge / fabric / quilt / cleanroom / liteloader，对不上的记 unknown；A2 已改） | `install.rs` `loader_of_profile` / `is_supported_loader` |
| 加载器安装（Fabric / Quilt / Forge / NeoForge，含镜像与重试） | `install.rs` `install_loader` / `install_forge_like` |
| 版本合并、继承链、natives 解压、classpath 构建 | `install.rs` `resolve_detail` / `merged_classpath_entries`，`launch.rs` |
| CurseForge 清单解析与下载（curse.tools + MCIM + cursemaven 兜底） | `modpack.rs` / `curseforge.rs` |
| 拖入识别 CF / mrpack / 通用压缩包 | `dropzone.rs` |
| 实例 config 浏览与编辑（UTF-8 / GBK） | `instance.rs` `config_tree` / `read_config_text` / `write_config_text` |
| Java 运行时（官方目录 + Temurin / Zulu / Corretto，含 25） | `java.rs` |

### 缺口

1. ~~未知加载器（Cleanroom / LiteLoader）静默判成 vanilla~~ —— A2 已解决，现在如实报出加载器名。
2. 没有「自定义加载器 profile」安装路径。
3. 没有 `mmc-pack.json` / `instance.cfg` 实例导入。
4. GTO 的 169 个 CF 文件、403 文件、Java 25 未做真包回归。

---

## 二、阶段 A：兼容性打底

### A0. 存量改动（已完成，待发布）

- [x] 一键开服闭环：白名单自动写入、离线默认值、控制台 JVM 参数拦截、日志注解、诊断报告
- [x] 崩溃弹窗底栏换行修复；`authlib-login` 误判修复
- [x] 实例「配置文件」编辑器
- [x] 玻璃不透明度下限 12%

### A1. GTO 真包回归（状态：代码侧已完成，真包实测待手动执行）

**目标**：用真实 `GregTech.Odyssey-0.5.6-beta.zip` 完整走一遍导入 + 启动，形成可复现的验收记录。

**任务**

- [x] 导入回归：`manifest.json` 解析 + `overrides/` 落盘。用 GTO 形态的夹具（169 个必需 + 3 个可选、`config/openloader`、GBK 配置）做了单元测试；**真包下载后的完整回归未做**，见下方「待手动验证」
- [x] 可选模组：策略确认为**默认不装**，预览标注「可选模组 N 个」，拖入导入弹窗补上了开关。
      与原计划的「默认安装」不同：可选项常是互斥的备选（两种性能模组二选一），全装可能直接冲突，缺一两个可选模组则不影响进游戏
- [x] 403 兜底：链路核对通过（双 API 源 → cursemaven → 按 fileId 拼直链 → 官方 CDN 403/404 → MCIM 镜像）。
      顺带修了直链文件名不转义的问题（空格、`#`、`?`、中文会被截断成无效地址）
- [x] 失败聚合：失败项结构化（项目名 + 原因 + 项目/文件 ID），任务卡片按项目列出，每项有「重试」和「项目页」。
      「补下失败文件」现在也覆盖**解析阶段**的失败（以前这类失败没有下载地址，只能重新导入整个包）
- [x] Forge 47.4.20：实测安装器在 BMCLAPI、CERNET 镜像均返回 200（8,835,110 字节）；官方 maven 本机直连超时，属预期，走镜像
- [x] 导入结果提示：按模组数量给内存建议（60 / 100 / 150 个模组对应 4 / 6 / 8 GB）；自动分配只做说明，手动分配不足才提醒。Java 相关提示归入 A3
- [ ] **真包回归（手动）**：下载 GTO 真包，走完导入 + 启动，记录实际耗时与失败清单

**计划外修复**：`fabric-loader-0.15.11` / `quilt-loader-0.21.0` 这类加载器 ID 的版本号解析错误，
会把 `loader-0.15.11` 当版本号去请求加载器配置，导致 **CurseForge 的 Fabric / Quilt 整合包导入直接失败**（本地压缩包与在线下载两条路径都中招）。已修并有测试。

**涉及**：`src-tauri/src/modpack.rs`（`detect_loader` / `parse_pack` / `import` / `install_specs`）、`src-tauri/src/curseforge.rs`（`file_download` / `cdn_url`）、`src-tauri/src/commands.rs`（`retry_failed_downloads` / `modpack_retry_file`）、`src/components/ImportFailureList.vue`、`DownloadTasks.vue` / `DownloadCenter.vue`

**验收**：全新实例导入 GTO 成功；失败文件有明确原因与重试入口；记录实际耗时与失败清单。

### A2. 未知加载器不再静默当原版（状态：已完成）

**目标**：认不出的加载器给出结论，而不是伪装成原版后启动失败。

- [x] 识别规则集中到 `install::loader_of_profile`：先看主类（最明确），再看库坐标。新增 Cleanroom（`top.outlands.foundation.boot.Foundation`）、LiteLoader；主类不是原版又对不上任何已知加载器的记为 `unknown`
- [x] 如实返回加载器名：CurseForge 清单、Modrinth 索引、通用压缩包（`versions/*.json` 的主类，以及 Prism / MultiMC 的 `mmc-pack.json` + `patches/*.json`）三条路都不再归成原版
- [x] 导入预览、拖入弹窗、版本列表、实例页统一显示「Cleanroom（暂不支持）」；实例描述同样标注
- [x] 启动器装不了的加载器不安装：实例挂在原版上，文件照常全部导入并保留，加载器版本输入框隐藏
- [x] 只警告不阻断：启动前预检新增 `unsupported-loader`（非阻断，本会话确认一次）。只在「实例确实是原版底座」时提示；从别的启动器导入、自带完整加载器的实例不打扰。存量被误判为原版的实例不受影响
- [x] 社区搜索：不支持的加载器不作为筛选条件（源站没有对应筛选项，带上会筛成 0 条）

**涉及**：`src-tauri/src/install.rs`（`loader_of_profile` / `is_supported_loader` / `loader_display_name`）、`src-tauri/src/modpack.rs`、`src-tauri/src/preflight.rs`、`src/utils.ts`（`loaderLabel`）、`src/views/CommunityView.vue` / `VersionsView.vue` / `InstanceView.vue`、`src/components/DropImportDialog.vue`

**验收**：导入 Cleanroom 实例包不再显示「原版」；启动前有明确提示。单元测试已覆盖识别与提示逻辑，界面效果待手动过一遍。

**明确没做（留给 B 阶段）**：Prism / MultiMC 实例的组件合并与依赖安装（B3）。当前只做到「认出加载器并如实显示」，这类实例仍需用户选父版本、只导入文件。

### A3. Java 25 路径打通（状态：代码侧已完成，真机确认待手动执行）

**目标**：GTO 推荐 Java 25。用户没装 Java 25 时，能在启动前看到提示并一键装好，然后选中启动。

**调研结论（决定本步范围）**
- Java 25 **已经能装**：Mojang 官方清单实测含 25（组件名 `java-runtime-epsilon`）；`java.rs::vendor_majors` 里 temurin / zulu / corretto 也都有 25。
- `launch.rs::find_java` **已经允许高版本**：手动指定的 Java 只要 `major >= required`（且 required > 8）就接受；自动选择在无精确版本时取「最小的更高版本」。
- 崩溃规则**不会**对 1.20.1 + Java 25 误报：`java-version` 等规则只在「Java 偏低」时命中。
- 所以原计划的前两项（验证可装、验证可启动）不需要改代码，只做一次真机确认。真正缺的是下面三件事。

**步骤**

#### A3-1. `install_runtime` 选组件的顺序不确定（必修）

- **位置**：`src-tauri/src/java.rs`，`install_runtime`（约 420–492 行）
- **现状**：`windows` 的类型是 `HashMap<String, Vec<RuntimeEntry>>`。代码用 `best.is_none()` 取「第一个」`parse_major == major` 的条目（约 439 行）。HashMap 迭代顺序每次进程都不同，所以同样装 Java 21，这次可能装 `java-runtime-alpha`（稳定），下次装 `java-runtime-beta`。
- **改成**：先把所有匹配的 `(component, manifest_url)` 收集起来，再按固定优先级挑一个：
  1. 组件名 `java-runtime-alpha` / `java-runtime-beta` / `java-runtime-gamma` / `java-runtime-delta` / `java-runtime-epsilon` 这类**不带后缀**的稳定组件优先；
  2. 其余按组件名字典序，取第一个（保证每次结果一样）。
  - `available_runtime_majors` 只取 major 的集合，不受顺序影响，**不用改**。
- **测试**：新增一个单元测试。构造一个只含两个同 major 条目的 `RuntimeAll`（需要把 `RuntimeAll` / `RuntimeEntry` 的构造暴露给测试，或在函数里抽出「挑选」的纯函数再测它——**推荐抽纯函数** `pick_runtime_component<'a>(windows: &'a HashMap<String, Vec<RuntimeEntry>>, major: u32) -> Option<(&'a String, &'a String)>`），断言选到的是稳定组件。
- **不做**：不要改下载和解压逻辑；不要动 `install_runtime_vendor` 及其子函数。

#### A3-2. 装完 Java 不要悄悄改全局设置（必修）

- **位置**：`src-tauri/src/commands.rs`，`install_java`（约 2622–2646 行）
- **现状**：安装成功后执行 `cfg.java_path = Some(info.path.clone())` 并保存（约 2640 行）。也就是说，用户给 GTO 装一个 Java 25，**所有实例**的全局 Java 都会被改成 25。需要 Java 8 的 1.12 实例会被 `find_java` 判为不匹配然后自动回退到别的运行时，行为变得难以解释。
- **改成**：命令增加一个参数 `set_as_default: Option<bool>`。为 `true` 时保持现有行为；为 `false` 或 `None` 时只装、不改 `java_path`。
  - `#[tauri::command]` 的参数名用 snake_case，前端传 camelCase 的 `setAsDefault`，Tauri 会自动转换。
- **前端配套**：`src/store.ts` 的 `installJava(major, source)`（约 1136 行）增加可选第三参 `setAsDefault = true`，透传给 `api.installJava`。`src/api.ts` 里 `installJava` 的签名同步加上该参数。**保持默认 `true`**——设置页里用户主动装 Java，设为默认是合理预期，这条路径的行为不变。
- **测试**：这条主要是接线，Rust 侧给 `install_java` 加测试成本高（要联网）。用 `vue-tsc` + 手动验证为准，不强制写测试。

#### A3-3. 启动前提示「缺 Java N」并提供一键安装（核心）

现状是：缺 Java 时 `find_java` 返回 `LAUNCH_NO_JAVA`，前端 `store.javaPrompt.kind = "missing"`，弹窗只有「前往设置」一个出口。用户要到设置页自己找对版本再装。改成弹窗里直接装。

- **后端**：`src-tauri/src/preflight.rs` 的 `check` 函数，在「9. 加载器」那条之后新增一条检查（编号 10）：
  - 读 `merged.detail.required_java()` 得到 `required`；调用 `crate::launch::find_java(cfg, required)`。
  - **`find_java` 返回 `Err`（错误串含 `LAUNCH_NO_JAVA`）时**，`push` 一条：
    - `id: "java-missing"`，`title: "缺少 Java {required}"`，`blocking: false`，`fixable: false`；
    - `detail`：`"当前实例需要 Java {required}，本机没有可用的运行时。可以在弹窗里一键安装 Java {required}。"`
  - **注意一个边界**：`required <= 8` 且本机只有高版本 Java 时，`find_java` 同样返回 `LAUNCH_NO_JAVA`。这条提示对它也成立（「缺 Java 8」），逻辑不用特判。
  - `find_java` 内部会跑 `java::detect`，要给每个 Java 进程起一次 `java -version`，**同步且偏慢**（本机十来个 Java 时约 1–2 秒）。`check` 本来就是同步函数、启动前只跑一次，可以接受；但**不要**在别的地方（比如版本列表）调用这条。
- **`PreflightIssue` 不需要加字段**：前端已经能从 `issue.id` 识别。
- **测试**：`preflight.rs` 的测试模块里新增一条。`Config::default()` 的游戏目录下没有装任何版本，`check("__docl_missing_version__")` 会在第 1 步就返回，测不到这条。需要一个已安装版本 json——**推荐做法**：把新检查的主体抽成纯函数 `java_missing_issue(required: u32, java_available: bool) -> Option<PreflightIssue>`，测试直接喂参数断言 `id` / `blocking` / `detail` 文案。

- **前端弹窗**：`src/App.vue` 里的 `store.preflightPrompt` 弹窗（约 371 行起）。在「仍然启动」旁边加一个按钮「安装 Java {N}」，条件是 `issues` 里有 `id === "java-missing"`：
  - 点击：`store.preflightPrompt = null`，然后调用 `store.ts` 里新加的 `installJavaFromPreflight(required)`。
  - `installJavaFromPreflight`：调 `installJava(required, "mojang", false)`（**不设为全局默认**，见 A3-2），成功后 `notify("Java {N} 已安装，正在重新启动…")`，再调用一次 `launchVersion(versionId, { skipJavaCheck: true, skipPreflight: false })`。
    - `skipPreflight` 用 `false`：装完重跑一次自检，让其余的警告正常出现；此时 `java-missing` 应已消失。
    - 弹窗关闭期间用户可能切了页面，重启动前检查 `store.gameRunning`，为真就不启动。
  - 「仍然启动」按钮对这条**保留**：有些实例的 `required_java` 只是清单默认值，用户可能明知能用。
- **`javaPrompt` 的 `missing` 弹窗**（App.vue 约 339–369 行）也加同一个按钮。`message` 里不带 required 数字，需要后端把数字带回来：把 `launch.rs::find_java` 的错误串保持不变（已有 `LAUNCH_NO_JAVA:` 前缀约定，别改），前端用正则从 message 里取 `Java (\d+)`。
- **不做**：不要改 `find_java` 的兼容规则；不要改 `javaMismatchFor`（那是手动指定不匹配的场景，已有流程）；不要在 `preflight.rs` 里做 Java 的下载或安装。

#### A3-4. 崩溃提示文案跟上动态值（小改，可选）

- **位置**：`src-tauri/src/crash.rs`，规则 `java-version` 的 `advice` 第一条（约 411 行）写死了「1.20.5+ 需要 Java 21，1.17+ 需要 17，1.16 及以下用 8」。
- **现状**：`apply_context`（约 2403–2435 行）在能拿到 `game_version` 时，会在最前面插入一条带 `expected_java()` 的动态建议，所以用户实际看到的已经是对的。静态那句只在拿不到版本号时露出。
- **改成**：把静态那句改成不含具体版本号的通用表述，例如「到「设置 → Java」安装或选择该游戏版本要求的 Java」。改完跑一下 `crash.rs` 里已有的断言文案的测试，**如果因此有测试失败，停下报告**，不要改断言。
- **不做**：不要动 `expected_java` 的映射表（1.20.1 → 17 是对的；GTO 用 25 是「高于要求」，不是「要求 25」）。

#### A3-5. 真机确认（手动）

- [ ] 在一台没装 Java 25 的环境（或先把 `java_path` 指到一个只有 Java 17 的位置）导入一个 Forge 1.20.1 实例，把它的实例设置里 Java 手动指定为一个不存在的路径，观察启动前是否出现「缺少 Java」并能一键安装。
- [ ] GTO 真包：装好 Java 25 后在实例设置里选中它，启动到主菜单。

**涉及**：`src-tauri/src/java.rs`（`install_runtime`、新增 `pick_runtime_component`）、`src-tauri/src/commands.rs`（`install_java`）、`src-tauri/src/preflight.rs`（`check`、新增 `java_missing_issue`）、`src-tauri/src/crash.rs`（文案）、`src/store.ts`（`installJava`、新增 `installJavaFromPreflight`）、`src/api.ts`、`src/App.vue`

**验收**
- `cargo test --lib` 全过，新增的测试至少覆盖：组件挑选的确定性、`java_missing_issue` 的文案与 `blocking = false`。
- 没装 Java 25 时启动 GTO 实例：弹窗出现「安装 Java 25」，点完自动装好并继续启动，期间全局 Java 设置**不被改动**。
- 在设置页主动装 Java 25：仍然被设为全局默认（行为不变）。

### A4. 发布检查（状态：待实施）

- [ ] 4 处版本号同步（`package.json` / `Cargo.toml` / `tauri.conf.json` / `update.json`）
- [ ] `scripts/build-release.ps1`、`scripts/build-portable.ps1`
- [ ] commit / push / tag / `gh release create`
- [ ] `scripts/purge-cdn.ps1`
- [ ] 校验 raw / jsDelivr / Release 资产一致
- [ ] 更新说明：支持 GTO 类 CF 整合包、配置文件编辑器、一键开服闭环

---

## 三、阶段 B：加载器攻坚

**依赖关系**：B1 → B2 → B3 → B4。**按顺序做**，后一步依赖前一步的产物。B2 和 B3 各自可以单独发版。

**本阶段新增一个 Rust 模块**：`src-tauri/src/profile.rs`（自定义 profile 安装）和 `src-tauri/src/mmcimport.rs`（MMC 实例导入）。记得在 `src-tauri/src/lib.rs` 的 `mod` 列表里登记，新命令加进 `generate_handler!`。

**从外部读进来的数据一律不可信**：压缩包里的文件名、`version.json` / patch 里的路径和 URL、`instance.cfg` 里的键值。URL 只允许 `https://`，下载前要过 `curseforge.rs::trusted_cdn_url` 同类的白名单思路（见 B1-1 的「库地址来源」）。

---

### B1. 自定义加载器 profile 底座（状态：待实施）

**目标**：给一份 `version.json`，把它装成可启动的版本。Forge 系（带 `inheritsFrom`）和自包含（不带）两种形态都要支持。

**为什么需要它**：DOCL 现在只有两个安装入口，都不适用：
- `install.rs::install_version`（约 332–565 行）是原版全家桶，遇到 `inheritsFrom` 直接 `bail!`（约 354–356 行）；
- `install.rs::install_loader`（约 2622 行起）只下载**普通库**，跳过了 natives（约 2681 行 `library_is_native` 判断），natives 依赖父原版已经装好。

**调研结论（决定本步范围）**
- 合并（`resolve_detail` + `merge_child`，install.rs 约 937–1048 行）、库去重（`library_file_paths`，约 1096–1153 行）、classpath、natives 解压（`ensure_natives`，约 1200–1230 行）、启动参数拼装（`launch.rs::build`）**全部现成，直接复用，不要重写**。
- 旧式 `minecraftArguments` 里的 `--tweakClass` 已经能透传（launch.rs 约 474–483 行，参数追加在主类之后），不需要特判。
- 唯一要新写的是「按清单装 client jar + 库（含 natives classifier）+ assets」这一段。

#### B1-1. 新模块 `profile.rs`：`install_profile`

- **新文件**：`src-tauri/src/profile.rs`
- **函数签名**：
  ```rust
  pub async fn install_profile(
      app: &AppHandle,
      client: &reqwest::Client,
      cfg: &Config,
      version_id: &str,
      detail: &VersionDetail,
      meta: &crate::models::DoclMeta,
      progress: Arc<ProgressState>,
  ) -> Result<install::InstalledVersion>
  ```
- **做什么**（顺序固定）：
  1. **写清单**：`versions/<version_id>/<version_id>.json`，内容是 `detail` 序列化后的结果。**写之前先设 `detail.docl = Some(meta.clone())`**（`_docl` 字段，见 B1-3）。
     - ⚠️ 如果 `versions/<version_id>/` 已存在且里面有 `<version_id>.json`，**不要覆盖**，返回错误「版本 {id} 已存在」。用户自己的实例可能就叫这个名字。
  2. **原版底座**（仅当 `detail.inherits_from` 为 `Some(parent)`）：
     - 若 `install::is_installed(cfg, parent)` 为假，调用 `install::install_version(app, client, cfg, parent, progress.clone())`。
     - 若 `detail.asset_index` 为 `None`，**assets 跟随父版本**，第 5 步跳过。Forge 系 profile 通常如此。
  3. **client jar**（仅当 `detail.downloads.client` 为 `Some` 且 `inherits_from` 为 `None`）：
     - 目标 `versions/<version_id>/<version_id>.jar`，来源是 `downloads.client.url`，带 `sha1` 与 `size`，走 `download::run_tasks`（单任务，并发 1）。
     - 自包含 profile 的 jar 名就是版本名。`resolve_detail` 会据此把 `jar_id` 认成本版本（install.rs 约 1010–1028 行），启动时不会再去找别的 jar。
  4. **库**：遍历 `detail.libraries`，对每条：
     - `!install::rules_allow(lib.rules.as_ref())` → 跳过；
     - 普通库：`install::library_artifact(lib)` → `(DownloadRef, path)`；
     - native 库（`install::library_is_native(lib)` 为真）：`install::library_native_download(lib)`；
     - 两者都拿不到 → 跳过（Fabric 式 `name + url` 的库没有 `downloads`，由 `library_artifact` 的第二分支处理，已覆盖）；
     - 目标路径 = `cfg.game_root().join("libraries").join(path.replace('/', "\\"))`。**`path` 来自外部清单**，落盘前校验：不包含 `..` 段、不是绝对路径、不含盘符。可以复用 `modpack.rs::safe_relative` 的思路，但它目前是私有的——在 `profile.rs` 里写一个同语义的小函数，不要为此改 `modpack.rs` 的可见性。
     - 目标文件已存在且大小与 `size` 相符（`size == 0` 时不校验大小）→ 跳过，不重复下。
     - `DownloadRef.url` 必须以 `https://` 开头，否则整次安装 `bail!`，报错带上库的 `name`。
     - 汇总成 `Vec<FileTask>` 后 `download::run_tasks`，并发 `cfg.concurrency`。
     - ⚠️ **`library_artifact` 与 `library_native_download` 目前是 `pub(crate)`**（install.rs 约 242、270 行），`profile.rs` 在同一个 crate 内可以直接调，**不要改它们的可见性**。
  5. **assets**（仅当 `detail.asset_index` 为 `Some`）：逻辑照搬 `install_version` 的第 2 段（install.rs 约 422–489 行）：
     - `download::get_bytes_race` 拉索引（`3` 次尝试、`1500` ms 补位）→ 解析成 `models::AssetIndexFile` → 写 `assets/indexes/<id>.json` → 每个 object 一个 `FileTask`（URL `https://resources.download.minecraft.net/<2>/<hash>`，带 sha1 与 size）→ `download::run_tasks_reporting`。
     - **不要**把这段从 `install_version` 里抽出来复用——那会动到原版安装这条最核心的路径。在 `profile.rs` 里照着写一遍（约 60 行），并在注释里注明「与 install_version 的资源下载保持一致，改一处要同步另一处」。
     - 资源失败不致命，失败的 `label` 收集起来放进最终进度消息。
     - **`virtual` / `map_to_resources` 不用处理**：Cleanroom 用的 `1.12` 索引实测没有这两个字段。
     - 索引里 `objects` 为空 → `bail!("资源索引 {id} 为空")`。
  6. **日志配置**（`detail.logging.client` 为 `Some` 时）：同 `install_version` 第 3 段（约 491–505 行），下到 `assets/log_configs/<文件名>`。
  7. **natives**：调用 `install::ensure_natives(cfg, version_id, detail)`。它只解压已在 `libraries/` 下的 jar，第 4 步已经把 natives jar 下好了。
     - `ensure_natives` 的可见性确认是 `pub`（install.rs 约 1200 行）。
  8. **返回**：`context_from(version_id, detail, &[version_id.to_string()])` 得到 `(game_version, loader)`，组装 `InstalledVersion`。字段照 `install_version` 结尾（install.rs 约 547–564 行）。`kind` 用 `detail.kind`（空串时写 `"release"`）。
- **取消**：每个阶段开头检查 `progress.is_cancelled()`，为真就 `bail!("安装已取消")`。
- **不做**：不要改 `install_version` / `install_loader`；不要动 `resolve_detail`；不要在这个函数里下载 Java。

#### B1-2. 从 jar / zip 里取 `version.json`

- **新函数**（放在 `profile.rs`）：
  ```rust
  pub fn read_version_json(archive_path: &Path) -> Result<VersionDetail>
  ```
- 用 `crate::ziputil::open(path)` 打开（它处理了非 UTF-8 文件名等 zip 怪问题，**不要直接用 `zip::ZipArchive::new`**）。
- 读 **根目录下**名为 `version.json` 的条目（精确匹配，不要用 `ends_with`，避免命中 `META-INF/.../version.json`）。读成 `Vec<u8>` 后 `serde_json::from_slice::<VersionDetail>`。
- 条目大小上限 8 MB，超过 `bail!`。
- **`VersionDetail` 的 `id` 字段是必填**（models.rs 约 48 行，没有 `#[serde(default)]`）。缺失时解析直接失败，报错文案写「version.json 缺少 id」。
- **测试**：用 `zip::ZipWriter` 打三个测试包：正常包；`version.json` 在子目录里的包（应报找不到）；超过 8 MB 的包（应报超大）。

#### B1-3. `_docl` 元信息与 `DoclMeta`

- **现状**：`models.rs::DoclMeta`（约 39–44 行）只有 `game_version` 和 `loader` 两个字段，对应 `VersionDetail.docl`（`#[serde(rename = "_docl")]`）。
- **改成**：新增一个可选字段：
  ```rust
  pub struct DoclMeta {
      pub game_version: String,
      pub loader: String,
      /// 这个版本由哪份外部 profile 装出来的，例如 "cleanroom"。仅用于界面展示与重装，
      /// 不参与加载器识别
      #[serde(default, skip_serializing_if = "Option::is_none")]
      pub source: Option<String>,
  }
  ```
  - **所有构造 `DoclMeta { .. }` 的地方都要补上 `source`**。已知有一处：`modpack.rs` 的 `import` 函数结尾（搜 `DoclMeta {`）。用 `rg "DoclMeta \{" src-tauri/src` 找全，补 `source: None`。
  - `context_from` 读的是 `meta.game_version` 与 `meta.loader`，不受影响。
- **为什么要它**：B2 的「重装 / 更新 Cleanroom」需要知道这个版本是 Cleanroom 装出来的。`loader` 字段也可以承载这个信息，但 `loader` 参与 A2 的「是否支持」判断，语义不同，分开更安全。

#### B1-4. 命令与前端接线

- **命令**（`src-tauri/src/commands.rs`）：
  ```rust
  #[tauri::command]
  pub async fn install_profile_archive(
      app: AppHandle, state: State<'_, AppState>,
      path: String, version_id: Option<String>, job_id: Option<String>,
  ) -> CmdResult<InstalledVersion>
  ```
  - 调 `profile::read_version_json` → 取 `detail.id`（`version_id` 参数为空时用它，否则用传入的）；`meta` 里 `game_version` 先留空，由 `context_from` 的回退逻辑推断；`loader` 用 `install::loader_of_profile(detail.main_class.as_deref(), |prefix| ...)`（这个函数的签名是 `(Option<&str>, impl Fn(&str) -> bool) -> String`，见 install.rs 约 1480 行；**闭包里遍历 `detail.libraries` 判断前缀**）。
  - 包一层 `begin_job` / `finish_job` 与 `download::reset_failed_batch`，照 `install_loader` 命令的写法（commands.rs 约 510–546 行）。成功后设 `selected_version`。
  - `lib.rs` 注册命令。
- **前端**（`src/api.ts`）：加 `installProfileArchive: (path, versionId?, jobId?) => invoke<InstalledVersion>("install_profile_archive", { path, versionId, jobId })`。
- **这一步不做界面入口**，只是让命令可用。界面在 B2 里做。

#### B1-5. 测试

- `profile.rs` 测试模块：
  - `read_version_json` 的三个用例（B1-2）。
  - 路径校验函数：`../../x.jar`、`C:\x.jar`、`/etc/x` 都被拒；`com/example/a/1.0/a-1.0.jar` 通过。
  - `install_profile` 本身需要联网和 `AppHandle`，**不写单元测试**，用 B1-6 的手动验收。
- **现有测试不能坏**：`cargo test --lib` 220 个全过。

#### B1-6. 手动验收

- [ ] 下载 Cleanroom installer.jar（`gh release view --repo CleanroomMC/Cleanroom --json tagName,assets` 查最新 tag，用 curl 加代理下 `cleanroom-<tag>-installer.jar`）。
- [ ] 用一个临时入口（可以在 `#[ignore]` 的集成测试里，或临时在界面加个按钮，验完删掉）调用 `install_profile_archive`。
- [ ] 确认：`versions/<id>/` 下有 json 和 jar；`libraries/` 下 217 个库齐全（含 `lwjgl-3.4.3-natives-windows.jar`）；`assets/indexes/1.12.json` 存在；`assets/objects/` 有 1305 个文件。
- [ ] **此时先不要启动**——Java 25 可能没装。装好 Java 25 后启动到主菜单（这一条可以在 B2 做完后再回来验）。

**涉及**：新文件 `src-tauri/src/profile.rs`；`src-tauri/src/lib.rs`（`mod profile;` + 命令注册）；`src-tauri/src/models.rs`（`DoclMeta.source`）；`src-tauri/src/commands.rs`（新命令）；`src/api.ts`；`src-tauri/src/modpack.rs`（仅补 `source: None`）

**验收**：单元测试全过；手动验收的文件数对得上。

---

### B2. Cleanroom 支持（状态：待实施）

**目标**：用户在 DOCL 里选「Cleanroom」→ 选版本 → 一键装好并能启动。

#### B2-1. Cleanroom 版本列表

- **位置**：`profile.rs`，新增
  ```rust
  pub async fn cleanroom_versions() -> Result<Vec<install::LoaderVersion>>
  ```
- **数据源**：`https://github.com/CleanroomMC/Cleanroom/releases` 的 Atom feed：`https://github.com/CleanroomMC/Cleanroom/releases.atom`。**不要用 GitHub REST API**（未登录限流很紧，调研时实测触发过）。Atom 是公开的、不需要令牌。
  - 解析每个 `<entry>`：`<updated>` 的时间戳 + `<title>` 或 `<id>` 里的 tag。tag 形如 `0.7.16-alpha`、`v0.7.14-alpha`，**前缀不固定**，去掉开头的 `v`。
  - 只要 tag 能解析成「数字.数字.数字」开头的版本号；其他（例如 `nightly` 之类）丢弃。
  - 按版本号降序；`recommended` 标在第一个**不含 `-alpha` / `-beta` / `-rc`** 的版本上，没有就标第一个。
  - XML 解析：项目里**没有** xml 解析依赖。不要为这个加依赖——用简单的字符串扫描：按 `<entry>` 切分，每段里找 `<title>` 和 `<updated>` 的内容。写一个纯函数 `parse_release_feed(text: &str) -> Vec<(String /*version*/, String /*time*/)>` 便于测试。
- **`LoaderVersion` 结构**（install.rs 约 1234 行起附近，搜 `pub struct LoaderVersion`）：确认字段是 `version: String` + `recommended: bool`，照着填。
- **超时**：整个请求 15 秒，失败返回错误文案「获取 Cleanroom 版本列表失败，请检查网络或代理」。
- **测试**：`parse_release_feed` 喂一段仿造的 Atom（3 条，其中一条 tag 不合法），断言输出 2 条且顺序正确。

#### B2-2. 安装 Cleanroom

- **位置**：`profile.rs`，新增
  ```rust
  pub async fn install_cleanroom(
      app: &AppHandle, client: &reqwest::Client, cfg: &Config,
      version: &str, progress: Arc<ProgressState>,
  ) -> Result<install::InstalledVersion>
  ```
- **流程**：
  1. 下载 `https://github.com/CleanroomMC/Cleanroom/releases/download/<tag>/cleanroom-<tag>-installer.jar` 到 `crate::config::data_dir().join("temp").join("cleanroom")`。用 `download::download_stream` 或一个单任务 `FileTask` + `download::run_tasks`。**tag 要原样使用 feed 里给的**（去掉 `v` 前缀后的值），不要自己拼 `v`。实测 URL 是 `.../download/0.7.16-alpha/cleanroom-0.7.16-alpha-installer.jar`（**不带 `v`**）。
     - 下完检查大小在 5 MB 到 60 MB 之间，否则当作下载失败。
  2. `profile::read_version_json(&jar_path)` 得到 `detail`。
  3. 版本 ID 用 `detail.id`（实测是 `Cleanroom-0.7.16-alpha`）。若已安装（`install::is_installed`），且用户不是要重装，直接返回已有版本的信息。
  4. **Java 检查（安装前）**：`detail.required_java()` 是 25。调 `crate::launch::find_java(cfg, 25)`，若返回 `Err`：
     - **不要自动装 Java**。返回一个带固定前缀的错误：`NEED_JAVA:25`。前端据此弹 A3-3 的安装弹窗。
     - 原因：装 Java 是几十到一百多 MB 的下载，应该让用户明确触发，而不是装加载器时悄悄带上。
  5. `meta = DoclMeta { game_version: "1.12.2".into(), loader: "cleanroom".into(), source: Some("cleanroom".into()) }`。
     - **`game_version` 不要写死**：从 `detail` 推断。自包含清单没有 `inheritsFrom`，`context_from` 的回退链会依次试 `clientVersion` → 库坐标 → `assets`。Cleanroom 的 `assets` 是 `"1.12"`、库坐标里没有 forge 版本串，推断不出 `1.12.2`。**需要用 `java.rs` 里没有的新办法**：读 jar 里 `install_profile.json` 的 `minecraft` 字段（Forge 安装器格式，Cleanroom 沿用）。在 `read_version_json` 旁边加 `read_install_profile_minecraft(archive) -> Option<String>`。取不到时退回 `detail.assets`（`"1.12"`），再不行留空。
     - 把这个推断单独做成纯函数 `cleanroom_game_version(detail: &VersionDetail, install_profile_mc: Option<&str>) -> String`，方便测试。
  6. 调 `B1-1` 的 `install_profile(app, client, cfg, &detail.id, &detail, &meta, progress)`。
  7. 删除下载的 jar（失败不影响结果）。
- **`is_supported_loader` 要认 cleanroom**：`install.rs::is_supported_loader`（约 1502 行）目前 `cleanroom` 返回 `false`。**B2 做完后 Cleanroom 是可安装的**，把它加进支持的列表。
  - ⚠️ 这会改变 A2 的行为：A2 里「整合包声明 cleanroom → 按原版导入并保留文件」的逻辑在 `modpack.rs::ensure_pack_loader`，它先问 `is_supported_loader`，为真才会走 `ensure_loader` → `install_loader`。**而 `install_loader` 不认识 cleanroom**（它会走到「不支持的加载器」分支）。
  - **所以这一步要一起改**：`modpack.rs::ensure_pack_loader` 里，在 `is_supported_loader` 判断之后加一个分支：`loader == "cleanroom"` 时调 `install_cleanroom`（版本号取清单里的 `pack_version`；没有就取 feed 的推荐版本），返回 `(version_id, Some(version), None)`。
  - **A2 的测试会受影响**：`modpack.rs` 测试 `keeps_unknown_loader_names` 里有 `assert!(!crate::install::is_supported_loader("cleanroom"))`，`install.rs` 测试 `vanilla_and_common_loaders_keep_working` 里也有同样的断言。**这两条断言的行为确实要变**——按红线，不改旧断言：把这两行**删掉**，在同一个测试里新增 `assert!(crate::install::is_supported_loader("cleanroom"))`，并在提交说明里写清楚原因。其余断言不动。
  - `utils.test.ts` 里 `loaderLabel("cleanroom")` 断言是 `"Cleanroom（暂不支持）"`，同理：改这一条断言为 `"Cleanroom"`，并新增一条 `liteloader` 仍是 `"LiteLoader（暂不支持）"` 的断言。
  - `src/utils.ts::loaderLabel` 的 `SUPPORTED_LOADERS` 集合加 `"cleanroom"`。
- **测试**：`parse_release_feed`；`cleanroom_game_version` 的三个分支。

#### B2-3. 界面入口

- **加载器选项列表**：`install.rs::loader_options`（约 2573–2607 行）的 `loaders` 数组，目前是 `("optifine", "OptiFine", false)` 这样带 `supported` 标志。新增 `("cleanroom", "Cleanroom", true)`。
  - **限制 1.12.2**：`loader_options` 有 `game_version` 参数。Cleanroom 只支持 1.12.2——`game_version != "1.12.2"` 时这一项 `supported` 设 `false`、`note` 写「仅支持 1.12.2」。
- **版本列表命令**：`commands.rs` 的 `loader_versions`（约 499–508 行）调 `install::loader_versions`，后者对未知加载器 `bail!("不支持的加载器")`（install.rs 约 1646 行）。在 `install::loader_versions` 开头加分支：`loader == "cleanroom"` → `profile::cleanroom_versions().await`。
- **安装命令**：`commands.rs::install_loader`（约 510–546 行）调 `install::install_loader`。在 `install::install_loader` 开头加分支：`loader == "cleanroom"` → `profile::install_cleanroom(...)`。**注意** `install_loader` 的开头会先确保原版已装（约 2632–2635 行）——Cleanroom 是自包含的，**不要**走这一步，分支要放在它**前面**。
- **前端**：`src/views/` 里加载器选择的界面搜 `loaderOptions` 和 `LOADER` 找到渲染列表的地方（`DownloadView.vue` / `InstanceView.vue`），确认它是按后端返回的列表渲染、并且对 `supported === false` 的项禁用。**大概率不需要改**——只是多了一项。跑起来看一眼确认。
- **Java 缺失的接线**：前端调 `api.installLoader` 失败、错误串含 `NEED_JAVA:` 时，解析后面的数字，打开 `store.javaPrompt`（`kind: "missing"`），并把 A3-3 加的「安装 Java N」按钮带出来。装完 Java 后**不要自动重试装 Cleanroom**——让用户再点一次，避免连着两次大下载。

#### B2-4. 启动前的 Java 提示

- A3-3 的 `java-missing` 预检项已经覆盖：Cleanroom 清单的 `required_java()` 是 25，`find_java(cfg, 25)` 失败就会出提示。**不需要新代码**。
- A2 加的 `unsupported-loader` 预检项（`preflight.rs::unsupported_loader_issue`）：只在「实例是原版底座」时提示。B2 之后 `is_supported_loader("cleanroom")` 为真，这一项对 Cleanroom **自动不再触发**。确认一下即可。

#### B2-5. 手动验收

- [ ] 在版本 1.12.2 上选 Cleanroom → 选版本 → 安装。没装 Java 25 时应弹「需要 Java 25」而不是直接失败。
- [ ] 装好 Java 25 后安装成功，启动到主菜单。
- [ ] 往 `mods/` 放一个 1.12.2 的 Forge 模组（例如 JEI 1.12.2），确认游戏里能看到。

**涉及**：`src-tauri/src/profile.rs`（`cleanroom_versions` / `install_cleanroom` / `parse_release_feed` / `read_install_profile_minecraft` / `cleanroom_game_version`）；`src-tauri/src/install.rs`（`loader_options`、`loader_versions`、`install_loader`、`is_supported_loader`）；`src-tauri/src/modpack.rs`（`ensure_pack_loader`）；`src/utils.ts`；相关测试文件

**验收**：1.12.2 + Cleanroom 安装成功并能进主菜单；`liteloader` 仍显示「暂不支持」。

---

### B3. Prism / MultiMC 实例导入（状态：待实施）

**目标**：把 Prism / MultiMC 导出的实例 zip（或实例文件夹）拖进来，变成一个能启动的 DOCL 实例。

**调研结论（决定本步范围）**
- MMC 实例的「版本」是**多个 patch 叠加**出来的：`patches/net.minecraft.json` 是底座，`patches/<其他 uid>.json` 依次覆盖/追加。`mmc-pack.json` 里的 `components` 数组就是叠加顺序。
- 包里**不一定**带全 patch（很多人导出时只带 `mmc-pack.json`）。缺的要去 Prism meta 取。
- **两个深坑**（实测）：
  1. Cleanroom 实例的 patch 文件名是 `net.minecraftforge.json`，`mmc-pack.json` 里 uid 也写 `net.minecraftforge`——**uid 不可信**。判断加载器一律看 patch 的 `mainClass`（A2 的 `modpack.rs::profile_loader` 已经这么做了）。
  2. 包里带 `libraries/*-999999.0-empty.jar`，patch 里对应库的版本写成 `999999.0-empty`。这是 MMC「把原版自带的库替换掉」的手段。DOCL 的 `compare_versions("999999.0-empty", "3.4.3")` 会判前者更大——**必须先把占位库识别出来并按「移除」处理**，不能让它进去重流程。
- **Forge 的取舍**：Prism 的 forge patch 用 ForgeWrapper（`mainClass = io.github.zekerzhayard.forgewrapper.installer.Main`），依赖 `mavenFiles`。**不照抄**。Forge 组件只从 patch 里读出 Forge 版本号，然后调 DOCL 自己的 `install_forge_like`。
- **拖入识别现状**：`dropzone.rs::classify_names`（约 88–150 行）完全不认识 `mmc-pack.json` / `instance.cfg`，Cleanroom 实例包（没有 `mods/`）会被判成 `unknown`。

#### B3-1. 拖入识别

- **位置**：`src-tauri/src/dropzone.rs`，`classify_names`（约 88 行）与 `classify_dir`（约 152 行）
- **压缩包**：在 `modrinth.index.json` 判断**之前**加：
  ```rust
  // MultiMC / Prism 实例：按任意层级找 mmc-pack.json（包外层常套一个实例名文件夹）
  if has_file(names, "mmc-pack.json") {
      return "mmcinstance".into();
  }
  ```
- **文件夹**：`classify_dir` 开头加 `if path.join("mmc-pack.json").is_file() || path.join(".minecraft").is_dir() && path.join("instgroups.json").is_file() { ... }`——**只认 `mmc-pack.json`**，第二个条件不要加（`instgroups.json` 是上层目录的东西）。
- **`label_of`**（约 17 行）加 `"mmcinstance" => "Prism / MultiMC 实例"`。
- **测试**：`dropzone.rs` 测试模块里加一条：`["MyInstance/mmc-pack.json", "MyInstance/instance.cfg", "MyInstance/.minecraft/mods/a.jar"]` → `"mmcinstance"`；并确认原来的 `classifies_client_pack_without_manifest` 等三条不变。

#### B3-2. 新模块 `mmcimport.rs`：预览

- **新文件**：`src-tauri/src/mmcimport.rs`
- **数据结构**：
  ```rust
  #[derive(Debug, Clone, Serialize)]
  pub struct MmcComponentView {
      /// mmc-pack.json 里的 uid
      pub uid: String,
      pub version: String,
      /// 识别出的加载器名：vanilla / forge / neoforge / fabric / quilt / cleanroom / liteloader / 其他原样
      pub loader: String,
      /// 这份 patch 的来源：包内 / Prism meta / 缺失
      pub source: String,
      /// 能不能由 DOCL 安装
      pub supported: bool,
  }

  #[derive(Debug, Clone, Serialize)]
  pub struct MmcPreview {
      pub name: String,
      pub game_version: String,
      pub loader: String,
      pub loader_version: Option<String>,
      pub components: Vec<MmcComponentView>,
      /// .minecraft 下的文件数（不含 libraries/ 与 versions/）
      pub file_count: u32,
      pub warnings: Vec<String>,
      pub suggested_id: String,
  }
  ```
- **`pub fn inspect(archive_or_dir: &str) -> Result<MmcPreview>`**：
  - 同时支持 zip 和已解压的目录。建议内部统一成「按相对路径读字节」的枚举：
    ```rust
    enum Source<'a> { Zip { path: &'a str }, Dir { root: &'a Path } }
    impl Source<'_> { fn read(&self, rel: &str) -> Result<Option<Vec<u8>>>; fn list(&self) -> Result<Vec<String>>; }
    ```
    zip 用 `crate::ziputil::open`；目录用 `std::fs`。目录模式下 `rel` 先过路径校验（同 B1-1）。
  - 找前缀：包外层可能套一层文件夹。用 `modpack.rs::generic_prefix` 同样的思路（先找含 `.minecraft/` 的路径，再看第一层是否单一目录）。**那个函数是私有的**——在 `mmcimport.rs` 里写一个只认 `mmc-pack.json` 位置的简化版：找到任一 `mmc-pack.json` 的路径，它所在的目录就是实例根。
  - 读 `mmc-pack.json` → `components`。对每个组件：
    - **`net.minecraft`**：记下 `version` 作 `game_version`。patch 不用读（DOCL 自己装原版）。
    - **`org.lwjgl` / `org.lwjgl3`**：跳过，不进 `components` 列表（它们只替换 LWJGL 库，DOCL 走 Forge 安装器时不需要）。
      - ⚠️ 但 Cleanroom 的 patch 里 `requires` 了 `org.lwjgl = 3.4.3`，且 patch 自己的 `libraries` 里带了完整的 LWJGL 3。**对 cleanroom 组件，LWJGL 由 patch 自带，不受这条影响**。
    - **其他 uid**：先在包内找 `patches/<uid>.json`。找到 → 读出 `mainClass`，用 `install::loader_of_profile` 识别加载器；找不到 → 记为 `source: "missing"`，`loader` 先按 uid 粗略映射（`net.minecraftforge` → `forge`、`net.neoforged` → `neoforge`、`net.fabricmc.fabric-loader` → `fabric`、`org.quiltmc.quilt-loader` → `quilt`、`com.mumfrey.liteloader` → `liteloader`，其余原样）。**这个粗略映射只用于预览显示**，真正导入时 `source == "missing"` 的组件一律去 meta 取，取回后再按 `mainClass` 重新识别。
  - `supported` 的判定做成纯函数：
    ```rust
    fn component_supported(loader: &str, source: &str) -> bool
    ```
    规则：`vanilla` → true；`forge` / `neoforge` / `fabric` / `quilt` → true；`cleanroom` → **仅当 patch 在包内**（`source == "bundle"`）时 true（走 B1 自包含安装需要完整清单，meta 上没有 cleanroom）；`liteloader` → 见 B4，B3 阶段先返回 `false`；其余 → false。
  - `file_count`：`.minecraft/` 下（递归）除了 `libraries/`、`versions/` 之外的文件数。
  - `instance.cfg`：INI 格式，`name=xxx` 一行。读出来作 `name`，没有就用实例根的文件夹名。`MaxMemAlloc` / `JavaPath` / `iconKey` 也读出来存进 `MmcPreview` 的新字段（`memory_mb: Option<u32>`、`java_path: Option<String>`、`icon_key: Option<String>`），B3-3 用。
    - 写一个纯函数 `parse_instance_cfg(text: &str) -> HashMap<String, String>`：按行、`=` 切、去首尾空白、跳过 `[section]` 行与 `#` 注释。
  - `warnings`：
    - 有 `source == "missing"` 的组件 →「有 N 个组件需要联网从 Prism 获取」
    - 有不支持的组件 →「该实例使用了 X，启动器暂不支持，将只导入文件」（X 用 `install::loader_display_name`）
    - 没有 `net.minecraft` 组件 →「实例清单缺少 Minecraft 版本」，并且这是**硬错误**，直接 `bail!`。
- **测试**（都在 `mmcimport.rs` 测试模块，用 `ZipWriter` 打包夹具）：
  - Cleanroom 官方实例的形态（`mmc-pack.json` 三个组件 + `patches/net.minecraftforge.json` 里 mainClass 是 Foundation + `patches/org.lwjgl.json` + 8 个 `-999999.0-empty.jar`）→ `loader == "cleanroom"`、`supported == true`、`game_version == "1.12.2"`。
  - 只有 `mmc-pack.json`、没有 `patches/` 的 Forge 实例 → forge 组件 `source == "missing"`，warning 里出现「联网」。
  - `instance.cfg` 的解析。
  - 路径穿越：zip 里条目名是 `../../evil.json` 时被跳过，不报 panic。

#### B3-3. `mmcimport.rs`：导入

- **`pub async fn import(app, client, cfg, source: &str, instance_id: &str, progress) -> Result<ImportOutcome>`**
- **流程**：
  1. `inspect` 重新跑一遍（拿到 `components` 与 `instance.cfg`）。
  2. **确定加载器并安装底座**，按组件顺序：
     - `net.minecraft`：`install::install_version`（若未装）。
     - **Forge / NeoForge 组件**：从 patch 取版本号。patch 的 `version` 字段就是 Forge 版本（如 `47.4.20`）；**不要**用 Prism 的 forge patch 做安装。调用 `install::install_loader(app, client, cfg, &game_version, "forge", Some(&forge_version), progress)`，拿到一个 `parent_id`。
       - patch 里可能有 `requires` 指定更细的约束，忽略。
     - **Fabric / Quilt 组件**：同上，`install::install_loader(..., "fabric", Some(&version), ...)`。
     - **Cleanroom 组件（patch 在包内）**：这个 patch **本身就是一份几乎完整的 version.json**（有 mainClass、libraries，但缺 `assetIndex` / `downloads` / `javaVersion`）。**不能直接当 version.json 用**。做法：调 `B2-2` 的 `install_cleanroom(app, client, cfg, patch.version, progress)`——按版本号走官方 installer。版本号形如 `0.7.16-alpha`，与 release tag 一致。
       - 所以「Cleanroom 的 MMC 实例导入」实际上等于「装 Cleanroom + 拷文件」。这是有意为之：installer 路径经过 B1 验证，最稳。
     - **LiteLoader 组件**：B3 阶段**不处理**，记一条 warning「LiteLoader 暂不支持，已跳过」。B4 再做。
     - **未知组件**：warning，跳过。
     - 多个加载器组件同时存在（如 Forge + LiteLoader）：LiteLoader 跳过后，Forge 照常装。
  3. **实例落盘**：
     - `version_id = instance_id`。`write_instance_json`（`modpack.rs` 里的私有函数）的思路：写 `versions/<instance_id>/<instance_id>.json`，内容 `{"id", "inheritsFrom": parent_id, "type": "release"}`。**那个函数是私有的**，在 `mmcimport.rs` 里自己写（十几行），不要改可见性。
     - 没有 `parent_id`（纯原版实例）时 `inheritsFrom` 指向 `game_version`。
     - `instance::save(cfg, &instance_id, &InstanceMeta { display_name, description: "从 Prism / MultiMC 实例导入 · {game_version} · {loader}", icon: "grass", category: "modpack", isolated: Some(true), memory_mb: cfg 里的 `MaxMemAlloc`, java_path: `JavaPath`, ..Default::default() })`。
       - `InstanceMeta` 的字段名核对一下（instance.rs 约 17–45 行）：`memory_mb: Option<u32>`、`java_path: Option<String>`。`MaxMemAlloc` 是 MB 字符串，解析失败就留 `None`。`JavaPath` 是指向 java 的路径，**只在文件存在时才写入**，否则留 `None`（Prism 实例搬机器后路径多半失效）。
     - natives：`install::natives_dir(cfg, &parent_id)` 存在时复制到实例（照 `modpack.rs` 约 3537–3540 行）。
  4. **拷贝 `.minecraft/`**：遍历实例根下 `.minecraft/` 的所有文件，跳过 `libraries/`、`versions/` 子树；目标 = `instance::game_dir(cfg, &instance_id)` + 相对路径；目标路径校验同 B1-1；用 `spawn_blocking` + `std::fs::copy`。**已存在的目标文件不覆盖**（保留玩家数据）——但这是新实例，正常情况下不会有冲突。
     - **跳过的目录还要加**：`logs/`、`crash-reports/`、`screenshots/`（与 `modpack.rs::excluded_generic` 保持一致）。
  5. **不做自包含合并**。MMC 导入的实例保留 `inheritsFrom` 结构，与 DOCL 自己装的加载器版本共用底座。
     - ⚠️ `finalize_self_contained`（modpack.rs 约 1741 行，**会删父版本目录**）**绝对不要调用**。
- **返回**：
  ```rust
  #[derive(Debug, Clone, Serialize)]
  pub struct MmcImportOutcome { pub instance_id: String, pub display_name: String, pub installed_files: u32, pub warnings: Vec<String> }
  ```
- **命令**：`commands.rs` 加 `mmc_inspect(path) -> MmcPreview` 与 `mmc_import(path, instance_id, instance_name, job_id) -> MmcImportOutcome`，照 `modpack_inspect` / `modpack_import` 的写法（含 `begin_job` / `finish_job`、设 `selected_version`）。`lib.rs` 注册。

#### B3-4. 前端

- `src/api.ts`：`MmcPreview` / `MmcImportOutcome` 接口，`mmcInspect` / `mmcImport` 两个调用。
- **`src/components/DropImportDialog.vue`**：
  - `packs` 现在是 `kind === "modpack"`。新增 `mmcInstances = items.filter(kind === "mmcinstance")`。
  - 对每个实例调 `api.mmcInspect`，结果存进一个 `mmcMeta` 的 ref（结构与现有 `packMeta` 一致）。
  - 每个实例条目下方显示：`游戏版本 · 加载器`、组件数、文件数、warnings（复用现有 `.warn` 样式）。
  - 有不支持的组件时，导入按钮文案改成「仅导入文件」。
  - 点导入：`api.mmcImport(path, suggestedId, name)`，用 `enqueueJob` 包起来（照 `importModpack` 的写法，`key: "mmc-import:" + path`），完成后 `refreshInstalled()` / `refreshInstanceMetas()` / `store.contentRevision += 1`。
  - 实例名可编辑（照现有 `packNames` 的做法）。
- **社区页的导入入口**（`CommunityView.vue` 的 `pickPack`，约 935 行）不用改——那条路径只接 `.zip` / `.mrpack` 的整合包，MMC 实例走拖入即可。

#### B3-5. 测试与验收

- **单元测试**：B3-2 列的四组。另外在 `mmcimport.rs` 里给「占位库识别」加一条纯函数测试：
  ```rust
  fn is_placeholder_version(version: &str) -> bool  // "999999.0-empty" → true
  ```
  虽然 B3 的导入流程不会把包内 patch 直接合并（Forge 走安装器、Cleanroom 走 installer），**不碰占位库**；但把这个函数写上并测掉，留给 B4 的 LiteLoader 合并用，同时在注释里说明「当前导入路径不消费它」。
  - ⚠️ 这与「不为假想需求写代码」有冲突。**决定：不写这个函数**。B4 真要用时再加。在这里记一笔是为了防止实施时误以为漏了。
- **手动验收**：
  - [ ] Cleanroom 官方 MMC 实例 zip 拖入 → 识别为「Prism / MultiMC 实例」，显示 Cleanroom · 1.12.2 → 导入 → 启动到主菜单。
  - [ ] 一个 Prism 导出的 Forge 1.20.1 实例（只有 `mmc-pack.json`，无 `patches/`）→ 导入 → 启动。
  - [ ] 一个带 LiteLoader 的 1.12.2 实例 → Forge 部分正常，LiteLoader 提示「暂不支持，已跳过」。

**涉及**：新文件 `src-tauri/src/mmcimport.rs`；`src-tauri/src/dropzone.rs`；`src-tauri/src/commands.rs`；`src-tauri/src/lib.rs`；`src/api.ts`；`src/components/DropImportDialog.vue`

**验收**：三种实例形态各导一次，行为符合上面的描述；单元测试全过。

---

### B4. LiteLoader（状态：待评估，B3 完成后再定）

**现状**：B3 把 LiteLoader 组件跳过了。

**调研结论**
- Prism meta 有 `com.mumfrey.liteloader`，单版本 patch 路径 `/v1/com.mumfrey.liteloader/<版本>.json`。最新 `1.12.2-SNAPSHOT`。
- patch 内容简单：`mainClass: net.minecraft.launchwrapper.Launch`，`+tweakers: [com.mumfrey.liteloader.launch.LiteLoaderTweaker]`，`libraries` 三条（`net.minecraft:launchwrapper:1.12`、`org.ow2.asm:asm-all:5.2`、`com.mumfrey:liteloader:1.12.2-SNAPSHOT`），`requires: net.minecraft = 1.12.2`。
- **只有 1.12.2 及更早**，且与现代 Forge 的 launchwrapper 顺序是关键。

**评估要回答的问题**（做之前先花半小时实测，结论写回本节）
1. 纯 LiteLoader（无 Forge）的 1.12.2 实例：把 patch 转成一份 `VersionDetail`（`inheritsFrom: "1.12.2"`，`main_class` 取 patch 的，`minecraft_arguments` = 原版 1.12.2 的参数 + `--tweakClass com.mumfrey.liteloader.launch.LiteLoaderTweaker`，`libraries` 取 patch 的三条），走 B1 的 `install_profile`，能不能启动？
2. Forge + LiteLoader：用 DOCL 装好 Forge 1.12.2 后，再叠一份 LiteLoader 的 profile（`inheritsFrom` 指向 Forge 版本），`minecraft_arguments` 里 `--tweakClass FMLTweaker` 在前、`--tweakClass LiteLoaderTweaker` 在后，能不能启动？
   - ⚠️ `merge_child` 对 `minecraftArguments` 是**子版本整体覆盖**（install.rs 约 993–995 行）。所以 LiteLoader 这层 profile 的 `minecraftArguments` 必须是**完整的**（含 `--tweakClass FMLTweaker --tweakClass LiteLoaderTweaker`），不能只写增量。原版 1.12.2 的参数模板要从 `versions/1.12.2/1.12.2.json` 里读出来拼。
   - 这一步会**第一次真正用到** `999999.0-empty` 的处理吗？——不会，因为不合并 MMC 的 patch，而是自己拼 profile。

**结论二选一，写回本节**
- 两个问题都是「能」 → B4 做成：`loader_options` 加 `("liteloader", "LiteLoader", true)`（仅 1.12.2），版本列表读 Prism meta 的 liteloader index（`/v1/com.mumfrey.liteloader/index.json`，`versions[].requires[].equals == "1.12.2"` 的最新一个），安装走 B1。`is_supported_loader` 加 `liteloader`，B3-2 的 `component_supported` 对 `liteloader` 放开。
- 有「不能」 → **不做**，在 B3 的 warning 文案里保留「LiteLoader 暂不支持」，本节标注「已评估，不实施」及原因。

---

### B5. 发布检查（状态：待实施）

流程同 A4。更新说明写：Cleanroom、Prism / MultiMC 实例导入、Java 缺失一键安装；LiteLoader 视 B4 结论。

发版前额外确认：
- [ ] Fabric / Forge / NeoForge 的**现有**实例启动不受影响（`is_supported_loader` 和 `loader_options` 都被改过）。
- [ ] 原版安装（`install_version`）没有被改动过——`git diff src-tauri/src/install.rs` 里不应出现 `install_version` 函数体内的变更。

---
## 四、测试策略

| 层级 | 内容 |
| --- | --- |
| 单元测试（A 阶段已有） | GTO manifest 解析；403 回退；未知加载器识别；加载器 ID 拆分；CDN 直链转义 |
| 单元测试（A3 新增） | `pick_runtime_component` 的确定性；`java_missing_issue` 的文案与 `blocking = false` |
| 单元测试（B1 新增） | `read_version_json`（正常 / 子目录 / 超大）；库路径校验 |
| 单元测试（B2 新增） | `parse_release_feed`；`cleanroom_game_version` 三分支；`is_supported_loader("cleanroom")` 翻转 |
| 单元测试（B3 新增） | 拖入识别；MMC 预览（Cleanroom 形态 / 无 patches 形态）；`parse_instance_cfg`；zip 路径穿越 |
| 集成测试（`#[ignore]`） | 下载真实 GTO / Cleanroom 包，断言文件数与启动参数（未写，按需补） |
| 手动回归 | GTO 进存档；Cleanroom 进主界面；三种 Prism 实例导入后启动；Fabric / Forge / NeoForge 既有实例不受影响 |
| 兼容回归 | 旧实例（未识别加载器）仍能按原方式启动，不因 A2 被阻断 |

**写测试的约定**
- 涉及文件系统的测试用 `std::env::temp_dir().join(format!("docl-<主题>-{}", uuid::Uuid::new_v4()))` 建目录，结束时 `let _ = std::fs::remove_dir_all(&dir);`（各模块已有同名 `temp_dir` 辅助函数的照用）。
- zip 夹具用 `zip::ZipWriter::new(std::fs::File::create(..))` + `SimpleFileOptions::default()`，写完 `finish()`。需要写非 UTF-8 字节的条目时用 `&[u8]` 重载（`modpack.rs` 测试里的 `make_zip_bytes` 可参考）。
- 纯函数优先：把「判定」从「IO」里拆出来测，不要为了测一条文案去构造网络或 `AppHandle`。

## 五、风险与对策

| 风险 | 对策 |
| --- | --- |
| GTO「仅 Prism」的真实原因未知 | 先实测；若真出现配方 / 资源异常，对比 `config/openloader` 与 Prism 实例结构，再定位 |
| CF 403 文件不可得 | 明确错误 + 项目页手动下载 + 拖入导入，不把整个包判失败（A1 已做） |
| ~~Cleanroom legacy assets~~ | **已排除**：索引 `1.12` 实测无 `virtual` / `map_to_resources`，现有下载流程适用 |
| Cleanroom 版本号会动 | 版本列表从 releases.atom 读，代码里不写死版本；tag 原样用于拼下载地址 |
| GitHub API 限流 | 不用 REST API，用 Atom feed；查 release 用 `gh` CLI |
| 装完 Java 25 悄悄改掉全局 Java | A3-2 给 `install_java` 加 `set_as_default`，预检触发的安装不设默认 |
| `is_supported_loader("cleanroom")` 翻转牵连 A2 | B2-2 同步改 `ensure_pack_loader` 的分支；相关断言按红线处理（删旧 + 加新，提交说明写清） |
| MMC uid 不可信 | 判断加载器一律看 patch 的 `mainClass`，uid 只用于显示和找文件 |
| `999999.0-empty` 占位库 | B3 的导入路径**不合并**包内 patch（Forge 走安装器、Cleanroom 走 installer），天然避开；B4 若要合并再处理 |
| Prism 的 Forge patch 绑定 ForgeWrapper | 不照抄；只读版本号，安装走 DOCL 自己的 `install_forge_like` |
| MMC 导入范围膨胀 | 只解析实例里实际出现的组件；未知组件降级为「仅导入文件」 |
| 误判改变存量行为 | A2 只警告不阻断；B5 发版前回归现有 Fabric / Forge / NeoForge 实例 |

## 六、明确不做

- 自动内网穿透 / 自动公网开服
- 自动替换服务端核心
- 全量 MultiMC 组件市场（只处理实例里实际出现的组件）
- 照搬 Prism 的 ForgeWrapper 安装路径
- 虚拟资源布局（`legacy` / `pre-1.6` 索引）的兼容
- LiteLoader + Forge 组合的深度适配（B4 实测后定，做不动就标注限制）

---

## 七、进度总览

| 步骤 | 内容 | 状态 |
| --- | --- | --- |
| A0 | 存量改动（开服 / 崩溃 / 配置编辑器） | 已完成，待发布 |
| A1 | GTO 真包回归 | 代码侧已完成；真包实测待手动执行 |
| A2 | 未知加载器提示 | 已完成 |
| A3 | Java 25 路径 | 代码侧已完成（A3-1 ~ A3-4）；A3-5 真机确认待手动 |
| A4 | 发布检查 | 待实施 |
| B1 | 自定义 profile 底座 | 计划已细化（B1-1 ~ B1-6），待实施 |
| B2 | Cleanroom | 计划已细化（B2-1 ~ B2-5），待实施 |
| B3 | Prism / MMC 导入 | 计划已细化（B3-1 ~ B3-5），待实施 |
| B4 | LiteLoader | 待评估（B3 完成后做两个实测再定） |
| B5 | 发布检查 | 待实施 |

## 八、变更记录

| 日期 | 变更 |
| --- | --- |
| 2026-11-25 | 建立计划；完成调研结论与阶段划分 |
| 2026-11-25 | 取消版本绑定，改为按步骤推进；允许多个计划并行、随时发布 |
| 2026-11-26 | A1 代码侧完成：失败项结构化 + 单项/整批补下、CDN 直链转义、内存建议、可选模组计数；修复 `fabric-loader-x` 版本号解析导致 Fabric / Quilt 包导入失败 |
| 2026-11-26 | A1 策略调整：可选模组默认不装（原计划是默认安装），原因是可选项常为互斥备选，全装可能冲突 |
| 2026-11-26 | A2 完成：Cleanroom / LiteLoader / unknown 识别，预览与版本页如实标注「暂不支持」，按原版导入并保留文件，启动前非阻断提示 |
| 2026-11-26 | 完成只读调研（见 [调研-加载器兼容-A3与B阶段.md](./调研-加载器兼容-A3与B阶段.md)）。据实测修正：Cleanroom 最新 0.7.16-alpha；Prism meta 单版本路径为 uid 级；assets `1.12` 非虚拟布局；Prism forge patch 绑定 ForgeWrapper |
| 2026-11-26 | 计划重写为「可交给零上下文模型实施」的格式：新增第〇节实施须知；A3 细化为 A3-1 ~ A3-5；B1–B5 全部细化到文件 / 函数 / 坑 / 不做 / 验收。**策略变更**：B3 的 Forge 不照搬 Prism 的 ForgeWrapper patch，改走 DOCL 自己的安装链；Cleanroom 的 MMC 实例导入等同「装 Cleanroom + 拷文件」，不合并包内 patch |
| 2026-11-26 | A3 代码侧完成：`pick_runtime_component` 修选组件的不确定性（含排除 minecraft-java-exe）；`install_java` 增加 `set_as_default`（弹窗一键安装不再改全局 Java）；预检新增 `java-missing`（非阻断）+ 两个弹窗的「安装 Java N」按钮与装完自动重启；`java-version` 崩溃建议改通用表述。A3-5 真机确认与 GTO 真包回归待手动 |
| 2026-11-26 | 发布前审查：修复 `retry_failed_downloads` 在「解析失败 + 下载失败」同时存在时丢弃下载失败批次的 bug（两个池子合并补下、取消时分类回写）；`add_pending_specs` 拒收空列表、`remove_pending_spec` 清理空壳条目 |
| 2026-11-27 | 用户反馈「材质导入了、地图没了」：修复整合包地图丢失的四个环节——①地图特征（level.dat / session.lock）优先于 pack.mcmeta 判断，带内置资源包的冒险地图不再被误判成资源包；②下载后兜底搬移：错放进 mods / 资源包目录的地图 zip（接口被拦、镜像缺 classId、modules 为空时的形态）挪回 saves 并同步整合包清单，挂进 sync_content_dirs 让旧实例更新时自愈；③存档根目录搜索加深到三层，套三层目录的地图 zip 能正常解压；④同名文件落盘名带文件 ID，两张同名地图不再被下载层静默去重丢掉 |
| 2026-11-27 | 新增「更新加载器」：实例详情页可为 Forge / NeoForge / Fabric / Quilt 实例切换加载器版本（默认推荐版）。实现方式：先按目标版本安装，再把新清单与补丁 jar 接管到原实例目录（改写清单 id），mods / 存档 / 配置都不搬动；整合包实例与非标准目录拒绝更新。新增 `install::update_loader`、`update_loader` 命令、`InstanceInfo.loader_version` 与实例页更新弹窗 |
| 2026-11-28 | 加载器更新功能审查：修复 ①`check_existing_loader` 用 `contains` 比对版本，47.4.2 会命中 47.4.28 导致静默复用旧实例（改为精确比对，并支持清单里的游戏版本前缀）；②更新后页面仍显示旧版本号、重装会按旧版本降级（改为从清单库坐标读真实版本，`InstanceInfo.loader_version` 同源）；③更新过的实例再重装会落到新目录并孤立原实例（补接管回原 id）。另：先换补丁 jar 再写清单，避免游戏运行时锁文件留下半套状态；切换实例自动关闭更新弹窗；整合包/自包含/非标准目录拒绝更新 |
| 2026-11-28 | 1.7.6 发布内容：皮肤编辑器正确补全 64×32 皮肤四肢并保留帽子（兼容 RGB / 索引色 PNG）；陶瓦联机状态轮询不再堆叠、复用其他启动器的陶瓦实例、进入房间页不再自动下载启动、断线可一键重新加入；安装器下载改为多源并发竞速，并支持 PCL 等自包含清单原地更新 / 重装加载器；实例图标缩略图兼容 16 位 PNG |
