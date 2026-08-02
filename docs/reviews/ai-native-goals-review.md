# 三大产品目标对齐评审

> 评审对象：`claude/ai-native-system-review-71e0ee`（基于 `main` @ `d27cefa`）
>
> 评审标尺：项目自述的三条第一性目标 ——
> **(1) 作为 AI 原生系统设计；(2) 兼容 Mac 对于开发的鲁棒性和适配性；
> (3) 兼容 Windows 的驱动能力和游戏能力。**
>
> 方法：三路并行只读探索（Rust 控制面 / OS 构建与 CI / 战略研究文档）+ 本机
> `cargo test --workspace --locked`（130 passed）与
> `cargo clippy --workspace --all-targets --locked -- -D warnings`（零告警）。
>
> 与既有 [`docs/reviews/`](./README.md) 五份分维度评审的关系：那五份问"实现是否可靠、
> 契约形状是否正确"；本篇问"**离三个产品目标还有多远**"。因此本篇不重复它们已
> 记录且已修复的条目（见 §6），只记录**目标差距**与**本轮新发现**。

## 0. 总评

**设计层面成熟，交付层面刚起步；三个目标目前大约九成是文档与契约，一成是可运行能力。**

当前真实交付物是：一个测试严密的 Fedora bootc 44 + KDE Plasma 镜像，加一个**休眠的、
无鉴权的**任务决策守护进程。镜像里没有模型、没有执行器、没有沙箱、没有游戏栈
（无 Steam/Proton/Wine）、没有开发工具链。

这不是否定。`andromeda-policy` 与 `andromeda-runtime/store.rs` 的质量确实高，文档
对"未实现"的标注在同类项目里罕见地诚实。问题有两层：

1. **能力兑现距离**：三个目标都还没有一条端到端可信闭环；
2. **两处代码直接违背它自己反复声明的安全承诺**（§1.2），这两处在执行器落地时
   会立刻从"文档问题"变成"真实漏洞"。

| 目标 | 设计 | 交付 | 关键阻塞 |
|---|---|---|---|
| (1) AI 原生 | A | D | L3 确认死门、无证据门控、capability 自签发 + 无鉴权 |
| (2) Mac 级开发鲁棒性 | 可靠性 A−／开发体验 D+ | C− | 无自动更新与健康回滚、无故障注入、开发体验近乎无设计 |
| (3) Windows 级驱动与游戏 | 驱动 B+／游戏 B | D | 无 out-of-tree 通道、无游戏栈与游戏验证、CI 不覆盖 GPU |

## 1. 目标一：AI 原生系统设计

### 1.1 设计层面 —— 全项目最强、最有差异化的部分

[`research/reliability-update-ai-agent.md`](../research/reliability-update-ai-agent.md)
§5–§10 是这套文档的皇冠：结构化 capability（subject / 资源句柄 / 动作 / 工具 digest /
到期 / max_uses / purpose / parent_intent，broker 签发、模型不可伪造、可衰减）、
source-sink 污点跟踪、`read(secret) + network(send)` 组合需二次数据流授权、
"MCP 是互操作层而非信任层"、"验证不能由执行模型自证"。
[`research/desktop-platform-and-distribution.md`](../research/desktop-platform-and-distribution.md)
§5.3/§5.5 关于"什么绝不能进 KWin 进程"与 libei 输入约束同样到位。

§10 的"必须由 OS 提供、不能留给 agent 框架的能力"清单，是本项目真正回答
"凭什么不是 Linux + 一个 agent app"的地方。**这部分不需要修，需要的是兑现。**

### 1.2 两处代码与安全承诺直接矛盾（本轮新发现，最高优先级）

#### (a) L3「最终人工确认」在执行路径上是一道死门 —— 已在本轮修复，见 §6

`TaskService::guard_transition` 的 `Ready → Running` 分支把
`external_side_effect_confirmed` **硬编码为 `true`**，创建期的 `plan_fully_granted`
同样。而 `PolicyDecision::ask` 只在该标志为 `false` 时产生
（`crates/andromeda-policy/src/lib.rs`）。

**后果**：一个 L3 计划（发邮件 / 发布 / 购买 / 改系统设置）可以直接以 `Ready`
创建，并**无任何确认地**转入 `Running`。guard 里对 `Ask` 的判断成为死代码。

这与 [`README.md`](../../README.md) 安全边界"L3 外部副作用需要 host broker 和最终确认"、
[`SECURITY.md`](../../SECURITY.md)、
[`development/task-control-plane.md`](../development/task-control-plane.md)
"任一 action 为 Deny 或 Ask 即拒绝转换" **正面冲突**。

#### (b) `Verifying → Succeeded` 无任何证据门控 —— 已在本轮修复，见 §6

`guard_transition` 只守 `AwaitingApproval → Ready` 与 `Ready → Running`，其余
全部落进 `_ => {}`。任何调用方连续 POST `{to:"verifying"}`、`{to:"succeeded"}`
即可把任务标记成功。

更根本的是：`TaskRecord` 原本**没有任何 outcome/evidence 字段**，而
`andromeda-core` 的 `Evidence` / `ActionOutcome` / `OutcomeStatus` /
`RecoverySemantics` 四个类型**在整个 workspace 里从未被构造过**。

因此 README 组件表里 `andromeda-core` 的"Evidence、恢复语义"在本轮修复前是
**死类型**——这是 README 唯一的实质性过度声称。

### 1.3 三个信任边界问题（既有安全评审已列，部分本轮加固）

| # | 问题 | 状态 |
|---|---|---|
| (c) | **capability 自签发 + taskd 完全无鉴权。** `CreateTaskRequest` 同时携带 plan 与 capabilities，唯一绑定是 `issued_to == plan.task_id`，而 `task_id` 也由调用方自选。任何能访问 loopback 的本地进程（包括未来的 agent 进程本身）可以给自己签发 `Files{root:"/", ReadWrite}` 并被判 Allow。策略引擎实际强制的只剩两条硬编码 deny 列表。 | 本轮加固了绑定面（§6），**能力签发与评估的分离仍是执行器前置项** |
| (d) | **risk 等级由不可信 planner 自选。** floor 映射是宿主拥有的，但 `ActionKind` 是模型提出的 `ActionSpec` 的字段。把不可信压缩包声明成 `ReadFile`(L1) 而非 `ParseUntrustedContent`(L2) 是**合法且被允许**的计划——没有独立的目标分类器。"模型不可绕过的风险"实际只意味着"不能低于它自选那一类的地板"。 | 未修，需独立分类器 |
| (e) | **deny root 纯词法、对 symlink 盲、默认清单薄。** 只 deny `/boot /etc /usr /System C:\Windows`；usrmerge 下 `/bin/sh` 不在 `/usr` 之内，macOS 上 `/private/etc/sudoers` 不在 `/etc` 之内，`~/.ssh`、`/dev/*`、`/proc/sys/*` 全在圈外。 | 未修，执行层须以 `realpath`/`openat` 复核 |

### 1.4 镜像里的 AI 面：零

镜像装入 `/usr/bin/andromeda` 与 `andromeda-taskd`（后者是硬化 systemd 服务，
`DynamicUser` + `IPAddressAllow=localhost` + `ProtectSystem=strict`），但：

- `os/files/` 里**没有任何 `.desktop`、没有 polkit/udev 规则、没有 GUI**；
- 全仓 grep `ollama|llama|inference|openai|anthropic|onnx|whisper` **零命中**；
- 装好的系统对用户而言**没有任何 AI 入口**。

项目自己的 skill 说得最准确
（`skills/andromeda-os-engineering/references/research-and-product.md`）：应描述为
"contract/policy/persistence infrastructure, not an autonomous privileged OS agent"。

### 1.5 缺失的地基规格

[`product-development-plan.md`](../product-development-plan.md) §13 列为第 1、2 项的
`andromeda-threat-model.md` 与 `agent-runtime-spec.md` **至今不存在**。所有 AI 原生
声称都压在这两份未写的规格上。

## 2. 目标二：兼容 Mac 的开发鲁棒性与适配性

### 2.1 可靠性设计 —— 强

[`research/reliability-update-ai-agent.md`](../research/reliability-update-ai-agent.md)
本质就是"向 macOS 看齐"文档，且做得诚实：§2.2 拆解 APFS COW/快照、Signed System
Volume Merkle 封印、系统/数据卷分离，并直言 Andromeda 缺 Apple 的闭环，必须靠 HCM +
按硬件类分的内核 channel + 可回滚驱动组合补偿；§4.2 给出完整更新状态机
（预检空间/电源、tries/mark-good、固件级 fallback）。

### 2.2 交付层面的硬缺口

1. **装好的系统不会自更新，坏更新也不会自动回滚。** 镜像未装 greenboot，未启用
   `bootc-fetch-apply-updates.timer`。一个"能引导进坏桌面"的更新会**留在那里**。
2. **断电 / 磁盘满 / 坏更新的故障注入测试一个都不存在。** "1000 次随机断电更新测试"
   只写在产品计划里，`os/scripts/` 无对应脚本。这恰是"像 macOS 一样可靠"最需要
   证据的地方。
3. **供应链未固定。** payload 基础镜像 `fedora-bootc:44` 是滚动 tag（构建不可复现）；
   安装写入的 `--target-imgref` 指向**可变、未签名**的 `ghcr.io/oratis/andromeda:edge`，
   而 CI 从不发布它——没有生产者、没有签名策略。

### 2.3 开发体验 —— 相对目标最薄弱的一环

面向产品计划 §3.1 点名的首发用户 #1"AI 开发者"，但：

- **镜像里零开发工具链**：无 git / gcc / clang / make / cmake / python3 / nodejs /
  cargo / vim / gdb / strace / distrobox / toolbox。比原版 Fedora Workstation 装得还少。
  sshd 装了但被强制关闭且被验收脚本断言关闭。
- **整个开发体验设计只有一段 bullet**（产品计划 §5.3："OCI/Toolbox/Distrobox 类容器 +
  受管 CLI"）。没有 Homebrew 等价物（`brew install jq` 映射到什么未定义）、没有语言
  工具链策略（rustup/node/uv）、没有 `/usr/local` 可写性答案、没有 dotfiles 迁移、
  没有开发容器里的 GPU/CUDA 访问路径。
- **"开发者模式"被提及三次、定义零次**——解锁什么、与 Secure Boot/MOK/attestation
  如何交互、对 HCM tier 有何影响、是否可逆，全未指定。而不可变桌面通常依赖的
  rpm-ostree 分层逃生阀被明确标为 Watch/限制使用，却没给替代设计。

### 2.4 Mac 硬件策略 —— 清醒，但完全受制于上游

[`research/hardware-drivers-and-migration.md`](../research/hardware-drivers-and-migration.md)
§4 对三条 Mac 路线的边界划得很准：无 T2 Intel Mac（白名单 Supported）、T2（仅
Experimental，骑 `apple-bce`，Sonoma 固件后 suspend 坏）、Apple Silicon（M1/M2
Developer Preview，**M3/M4 = Watch/不支持**因 Asahi 安装器尚无，M5 不得从 CPU 名推断）。

代价是：**任何"PC and Mac"定位在相当长时间内实际是 PC-only**；且缺少"Apple 固件 /
macOS 更新打坏已装 Andromeda"的事件处置流程，Boot Camp/固件的法律结论也已 defer。

## 3. 目标三：兼容 Windows 的驱动能力和游戏能力

### 3.1 驱动策略（policy 层）—— 成熟

五级优先级（mainline → 发行版补丁集带 upstream 计划 → 厂商稳定 ABI 模块 → 社区
out-of-tree 仅 Experimental → 逆向 shim 永不入内核），每个下游补丁要求 upstream
URL + owner + 移除条件 + CVE 联系人；HEP（签名的 kernel+initramfs+firmware+模块
组合，产生**新 deployment** 而非改运行中系统）概念清晰。

### 3.2 与 Windows 驱动生态的结构性差距

- **完全没有第三方驱动认证生态。** 没有 IHV/ISV 提交门户、没有面向厂商的驱动签名
  服务、没有别人能申请的 logo/认证项目、没有 per-driver 崩溃遥测。所有认证都是
  Andromeda 实验室内部的。这是相对 WHQL 的**根本**差距，不是工程量差距。
- **镜像里没有任何 out-of-tree / DKMS 路径**（无 akmods / kmod-* / dkms / MOK 注册）。
  在只读 `/usr` 上这意味着 NVIDIA 专有、Xbox 手柄（xone/xpadneo）、厂商 RAID、ZFS
  **永久排除**。三 cohort NVIDIA 计划（nvk / open / legacy）只是文档，无代码、无构建
  目标、无实体 GPU 进过 CI。通用镜像只发 Nouveau/NVK，且 `diagnosis.rs` 把 NVK 判为
  `needs_review`，永不自动提升。
- Intel CPU microcode（`intel-ucode` / `microcode_ctl`）未在镜像中显式 pin，只 pin 了
  `amd-ucode-firmware`。

### 3.3 游戏能力 —— 有依赖、无栈、无验证

**镜像里有**：`gamescope`、`gamemode`、`mangohud`、`steam-devices`（仅 udev 规则）、
i686 的 mesa-dri / mesa-vulkan / vulkan-loader / alsa-lib / pipewire-libs，64 位
mesa/vulkan + `vulkan-tools`（含 `vkcube`/`vulkaninfo`）。

**镜像里没有**：Steam、Proton、Wine、Lutris/Heroic、umu-launcher、gamescope session、
`mesa-va-drivers`（AMD VA-API 解码都没有）。装的还是 Fedora **过滤版** Flathub remote，
用户须自行启用完整 Flathub 才能装 Steam。

**游戏验证测试：零。** `os/scripts/` 与 `os/files/` 中 grep
`vkcube|vulkaninfo|gamescope|steam` 在 Containerfile 之外零命中。daily-driver 验收只
`rpm -q` 了两个 i686 包，从不跑一次 Vulkan/GL 调用，也从不启动 gamescope。而 QEMU 下
全是 `virtio-vga` + llvmpipe 软件渲染，即便跑 `vkcube` 也证明不了真实 GPU。

**CI 硬件矩阵不变 GPU**：`modern-nvme` / `q35-sata` / `legacy-i440fx` 三档变的是
存储 / 网卡 / USB / 音频，GPU **全是 `virtio-vga`**。没有真实 GPU、Wi-Fi、蓝牙、
suspend、电池、固件覆盖，CI 里**从无实体硬件**。

### 3.4 未被设为门槛的玩家可见能力

**HDR、VRR、高刷、MUX 混合显卡、延迟都不是 Tier 3 Certified 门槛**，只是
[`development/hardware-certification-test-plan.md`](../development/hardware-certification-test-plan.md)
§7.5 的"能力声明"，而产品计划 §6.2 / §10.2 根本未列。对号称"Windows 级游戏"的产品，
这些恰是玩家一眼能看出的差异。

**反作弊只点名 EAC/BattlEye**（缺 Vanguard / nProtect / Denuvo AC / FACEIT），厂商合作
defer 到"18 个月后 Phase D"、无 owner；**非 Steam launcher**（Epic/GOG/EA/Ubisoft/
Battle.net）只有一行 Heroic Pilot；依赖的 `compatibility-database.schema.json` 尚不存在。
**没有任何 Stage 设过数值化的游戏通过率门槛**（Stage 1 = 10 款游戏，Stage 3 = "给出
真实评级"，都不是"通过率 ≥ N%"）。

文档对这些边界其实很诚实（`daily-driver-e2e.md` 明说不覆盖 Steam/Proton 性能、DRM、
反作弊）。**问题不是它撒谎，而是目标三目前只交付了一套"部分依赖包"，游戏能力实质为零。**

## 4. 跨目标结构性问题

### 4.1 签名能力已具备，但 `hardware check` 并未使用它（已实机复现）

> **本节已随 `main` 的演进更新。** 评审初稿写于 PR #19（`followup/hcm-signing`）合并前，
> 当时的判断是"schema 根本没有 manifest 级签名字段"。该判断**现已过时**：`main` 上
> `crates/andromeda-hardware/src/signing.rs` 已实现 fail-closed 的 ed25519 detached
> 签名验证（`TrustedKeyring`、`ManifestSignatureStatus`，只有 `Verified` 是接受，
> 空 keyring 谁都不信），schema 也已加入 `signature` 字段。

**但结论没有变，只是根因变了。** 构造一份 selector 命中本机、`requirements: []`、
含任意 `sha256` 与"passed / 2099 到期"证据、**且完全不带 `signature` 字段**的清单，
`andromeda hardware check --require-tier certified` 仍返回
`effective_tier: certified`、退出码 0。

新的根因是**接线缺失**，而非能力缺失：

1. CLI 的 `hardware check` 调用的是不验签的 `evaluate_manifest`，
   而不是 `evaluate_manifest_verified`（`crates/andromeda-cli/src/main.rs`）；
2. 未签名的清单在"无 keyring"路径上被当作合法输入，而不是 fail-closed；
3. `ArtifactPin` 的哈希在不带 `--artifact-root` 时仍被原样采信（本轮已加校验与警告，见 §6）。

**这是比原判断更值得记录的一类问题**：安全能力已经写好、测试也齐全，却没有被
唯一的用户入口消费——库是安全的，产品不是。修复成本极低（把 CLI 接到已存在的
verified 路径，并让"无签名"在要求 supported 以上时 fail-closed），收益极高。

今天 HCM 判定仍只是"咨询性"（安装器 preflight 用镜像 label），但 `hardware check` 的
退出码显然是为脚本门控设计的，一旦用于放行安装/驱动即升级为严重。README 架构图
"signed HCM"在库层面现已成立，在 **CLI 层面尚未成立**。

### 4.2 HCM schema 表达不了它要认证的东西

`hardware-certification-test-plan.md` §2 把 cohort 定义为 OEM + 型号 + 主板 revision +
BIOS/EC/TB/PD/SSD 固件 + 设备 ID + 镜像 digest；但 schema 的 selector 只有
`manufacturer_contains` + `model_prefix`：

| 认证需要 | schema 是否可表达 |
|---|---|
| 主板名/revision、产品 SKU | ✗ |
| BIOS/EC/Thunderbolt/PD/SSD 固件版本或区间 | ✗ |
| ACPI HID/CID/UID、DT `compatible` | ✗ |
| Apple `model_identifier`/`board_id`/`soc` | ✗ |
| 排除（负向）选择器 | ✗ |
| `degraded` / `unknown` 证据结果 | ✗（enum 只有 `passed`/`failed`） |
| 撤销（CRL）对象 | ✗ |

**最要命的是倒数第二行**：cert plan §7 要求每项测试产出
`pass/degraded/unsupported/blocked/unknown`，而 schema 只接受 `passed|failed`——
**恰恰是整套策略赖以生存的"降级/未知"状态无法进入 HCM**。

### 4.3 安装器比文档更具破坏性

- `os/installer/andromeda-ci.ks` 是 `clearpart --all` 抹**所有**盘，而
  `os/README.md` 说"第一块安装盘"（VM-only 守卫缓解了风险，但文档描述不准）；
- `install-uefi-fallback.sh` 在**交互模式**也无条件覆写目标 ESP 的
  `BOOTX64.EFI`/`grubx64.efi`——双系统机器的既有 fallback loader 会被砸，且无
  dual-boot 测试；
- 没有 dual-boot / 缩盘 / LUKS / 迁移 / 恢复 UI（`installable-preview.md` 已承认）。

### 4.4 CI 时间预算不自洽

job 上限 `timeout-minutes: 150`，但 `test-gcp-nested.sh` 内部超时加总为 100 + 60 + 30 =
**190 分钟**；`test-install.sh` 单独就允许 45m install + 2700s boot ≈ 90m。且两个
harness 无 `/dev/kvm` 即硬失败，而仓库**没有任何 nightly/scheduled 触发**。

### 4.5 文档内部不一致

| 位置 | 问题 |
|---|---|
| `research/hardware-drivers-and-migration.md` §5 | 仍把 "Tier 4 Reference" 当**最高**级，与代码权威阶梯 `blocked < community < reference < supported < certified`（`reference` 是**次低**）矛盾。其他所有文档都已加消歧注，唯独这份没有 |
| research §6.2 / os-landscape §3.2 / `schemas/*.json` | **三套互不兼容的 HCM 形状**同时存在（`andromeda.hcm/v1` YAML、另一套 YAML、实际交付的 v2 JSON），无一份文档做归一 |
| `research/open-source-adoption-matrix.md` §2.1 vs §8 | 同一文件内 osbuild 一处 Pilot、一处 Adopt |
| reliability §15.1 / desktop §8.1 / matrix §2.1 | bootc 的 radar 级别三处不同（Pilot / Adopt / Adopt-Pilot），而仓库已在发 bootc 44 镜像 |
| 游戏/采用矩阵 vs desktop 文档 | Gamescope 在前者是 Adopt，在**拥有显示架构话语权**的后者完全不出现；无 gamescope↔KWin 嵌套/切换/HDR 归属设计 |
| `hardware-certification-test-plan.md` §11 | 仍引用不存在的 `os/scripts/test-daily-driver.sh` |
| `product-development-plan.md` §6.4 vs §10.2 | suspend 循环 1000 次 vs 发布门 100/500 次，§6.4 无 stretch-goal 注 |

## 5. 建议的优先级

### 执行器落地前必须（越晚改越贵，且直接违背安全叙事）

1. 修 L3 确认死门 —— 确认不能由请求方硬编码，必须成为显式、可审计、默认关闭的门。
2. 给 `Verifying → Succeeded` 加证据门控，让 `ActionOutcome`/`Evidence` 成为活类型。
3. taskd 本地鉴权 + capability 签发与策略评估分离 —— 不接受调用方自带裸能力。

### 尽快

4. **把 `hardware check` 接到已存在的 `evaluate_manifest_verified`**，并让"要求
   supported 以上却无有效签名"fail-closed。签名能力已在 `main` 上具备（§4.1），
   缺的只是接线；在接好之前，文档须继续声明 `hardware check` 结果**不可作信任门控**。
5. 装好的系统补 greenboot 健康门控 + 自动回滚；`:edge` 与基础镜像按 digest + 签名固定
   （`os/signing/policy.json.example` 已提供未启用的模板，需要落地启用路径）。
6. 目标二/三各补一条"最小可信闭环"证明：一台机器装完能 `git clone && cargo build`
   （开发）；一个真实 GPU 上跑通 `vkcube` + 一款 Steam/Proton 游戏（游戏）。
   **当前这两条闭环都不存在。**

### 补设计（docs 层缺口）

7. `agent-runtime-spec.md`、`andromeda-threat-model.md`（产品计划 §13 第 1、2 项，
   所有 AI 原生声称的地基）。
8. HCM schema v3：cohort key 字段、`degraded`/`unknown` 证据结果、撤销对象、
   排除选择器（§4.2）。manifest 级签名字段已在 `main` 上落地，不再属于此项。
9. `developer-experience-spec.md`（**目前完全缺**）：开发容器 + GPU/CUDA 直通、语言
   工具链策略、Homebrew 等价物、`/usr/local` 可写性、"开发者模式"的完整定义。
10. Gamescope ↔ KWin 集成设计（会话切换、HDR/VRR/缩放/帧限归属、GPU reset 处理）。

### 一致性/清理

11. 补断电故障注入测试；修 CI 时间预算与 `/dev/kvm` 前置断言；修 §4.5 全部文档不一致。

## 6. 本轮已落地的修复

以下在本次评审后随同提交。测试从 130 增至 **143**，`cargo clippy -D warnings` 保持零告警。

| 修复 | 对应发现 |
|---|---|
| **L3 确认成为真实的门**：`StateTransitionRequest` 新增 `external_side_effect_confirmed`（默认 `false`），`Ready → Running` 用调用方显式提供的值重跑策略，并把 `Ask` 与 `Deny` 分开报告；未确认的 L3 动作以新的 `TransitionGuardError::ExternalConfirmationRequired` 被拒；确认值记入 `StateChanged` 事件、与 actor 一起留痕。CLI 加 `--confirm-external` | §1.2 (a) |
| **`Verifying → Succeeded` 证据门控**：`TaskRecord` 新增 `outcomes`（每 action 至多一条、append-only），新增 `TaskService::record_outcome`、`POST /v1/tasks/{id}/outcomes`、`TaskEventKind::OutcomeRecorded` 与 CLI `task record-outcome`；转换要求每个 action 都有 outcome，状态为 `succeeded`/`skipped`，且**至少一条 evidence**。`ActionOutcome`/`Evidence`/`OutcomeStatus` 由死类型变为强制契约 | §1.2 (b) |
| **taskd 拒绝非 loopback 绑定**：新增 `ensure_loopback_bind`，启动时校验监听地址，非回环地址直接拒绝启动；需显式 `ANDROMEDA_ALLOW_NON_LOOPBACK=1` 才能越过并打印醒目警告。把文档里的"禁止绑定 loopback 之外"从约定变成机制 | §1.3 (c) |
| **CLI 接入制品校验**：`hardware check` 新增 `--artifact-root` 与 `--trusted-key`，实际用 `DirectoryArtifactVerifier` 重算并比对 `sha256`；未提供时在 stderr 打印显式警告。`hardware-compatibility.md` 新增"信任边界"小节，明确 `hardware check` 退出码**不得**用于放行安装或提升等级 | §4.1 |
| 文档一致性：`Tier 4 Reference` → `OEM Reference Design` 并加消歧注、失效脚本引用、osbuild 自相矛盾、`os/README.md` 的"抹第一块盘"更正为"抹所有盘" | §4.3、§4.5 |

新增回归测试锁定的行为：未确认的 L3 计划不能进入 `Running`（且被拒后 revision 不前进）、
仅 L1 的计划无需确认、确认值被写入事件、无 outcome/无证据/失败 outcome 三种情况均不能
`Succeeded`、outcome 的未知 action / 重复记录 / 错误状态均被拒、非回环绑定被拒而回环被放行。

**已验证但仍未修的关键项**：不带 `--artifact-root` 时，一份**未签名**的伪造清单
**仍可**获得 `certified`（已在本机复现）。`main` 已具备 fail-closed 的 ed25519 验签能力，
但 CLI 尚未接线到 `evaluate_manifest_verified`（§4.1），因此当前仍以强制警告与
文档声明兜底。这是 §5 第 4 项。

未修项均已在 §5 按优先级列出，其中"能力签发与评估分离""`hardware check` 接线验签""greenboot
自动回滚""故障注入"是执行器与消费版落地前的阻塞级前置项。

---

*本篇 `file:line` 与结论基于评审时的分支 HEAD；`cargo test`/`clippy` 结果为本机实跑。*

*Reviewed by Claude Code (three-goal alignment dimension).*
