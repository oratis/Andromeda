# PC 与 macOS 操作系统全景及 Andromeda 架构建议

> 状态：研究初稿
>
> 检索日期：2026-07-26
>
> 目标：为“可运行在尽可能广泛硬件上的 AI 原生 Andromeda OS”建立技术、产品和许可决策基线。
>
> 说明：本文是最早的全景研究基线；与[产品开发计划](./product-development-plan.md)不一致之处以后者为准。本文的硬件 Tier 与桌面 compositor 相关章节已按产品开发计划 §6.2、§5.2 修订。

## 0. 执行摘要

### 核心判断

1. **不要从零写内核作为第一阶段。** “广泛硬件支持”和“从零内核”在早期目标上直接冲突。Linux 已拥有最广的 PC 驱动、图形栈、网络栈、KVM、容器和软件生态，应作为 Andromeda v1 的硬件使能层。
2. **Andromeda 应是 AI 原生系统产品，不是换皮 Linux 发行版。** 差异化应位于系统服务、权限、任务运行时、桌面 shell、应用意图接口和可审计执行层，而不是壁纸、启动器或聊天侧栏。
3. **兼容性要采用多路径组合。** Linux 原生应用直接运行；Windows 应用优先 Wine/Proton 类 API 兼容层，必要时进入 KVM 虚拟机；Web/PWA 作为通用路径；macOS 应用仅在授权允许的 Apple 硬件上通过虚拟化探索，不能承诺跨任意 PC。
4. **AI agent 必须是“受控主体”，而不是超级管理员。** 借鉴 Codex/Claude Code 的范式：上下文、工具、计划、执行、验证、恢复；但把权限从简单弹窗升级为 OS 原生 capability、作用域、预算、审计、回滚和来源证明。
5. **“所有硬件”必须产品化为等级。** 采用与产品开发计划 §6.2 一致的等级体系：CI-0 虚拟参考平台、Tier 0 Blocked、Tier 1 Community、Tier 2 Supported、Tier 3 Certified、Tier 4 Reference，而非作无法验证的绝对承诺。
6. **首个可交付产品应是可安装的 Linux-based OS。** 先证明 AI 原生交互和兼容编排，再评估是否将更多驱动、文件系统和网络服务移出内核，或长期采用更小的可信计算基。

### 推荐架构一句话

**Linux LTS 内核 + systemd/最小用户态 + Wayland + KDE Plasma 6/KWin（不自研 compositor）+ Flatpak/OCI + KVM/QEMU + Wine + AI capability broker + 可审计 agent runtime。**

这是一条现实的起点，不是不可改变的终局。Andromeda 的稳定 API 应在内核之上，从而保留未来更换底层的可能。

---

## 1. 范围与方法

### 1.1 “所有系统”的可操作定义

PC 操作系统数量极多，Linux 发行版和历史系统尤其无法穷举。本文按“系统家族 + 代表实现 + 可借鉴机制”覆盖：

- 主流闭源桌面：Windows、macOS、ChromeOS；
- 主流开源桌面底座：Linux、FreeBSD、OpenBSD、NetBSD；
- 兼容性导向：ReactOS、Wine、Darling（仅作路线识别）；
- 安全隔离导向：Qubes OS、ChromeOS；
- 新内核/研究导向：Fuchsia/Zircon、Redox、SerenityOS、Haiku；
- 商用 UNIX 与后继：Solaris/illumos；
- 相邻的大规模硬件抽象案例：Android/AOSP；
- 历史上影响现代桌面设计的系统：经典 Mac OS、BeOS、AmigaOS、OS/2、NeXTSTEP、Plan 9、MINIX 等。

评价重点不是“谁功能最多”，而是：

- 硬件与驱动覆盖；
- 应用兼容；
- 安全隔离与权限；
- 更新和回滚；
- 可观测性；
- 桌面体验；
- AI agent 能否成为一等公民；
- 许可证与商业可行性。

### 1.2 事实、推论和建议

- **事实**：由文末一手资料支持。
- **推论**：从多个事实推导出的工程判断，会明确标注。
- **建议**：针对 Andromeda 的产品选择，不代表相关项目官方观点。

---

## 2. 系统家族全景

| 家族/代表 | 源码与许可概况 | 内核/隔离主线 | 优势 | 对 Andromeda 的主要启示 |
|---|---|---|---|---|
| Windows 10/11 | 闭源商业；部分组件开源 | NT 混合内核、AppContainer、Hyper-V、WSL | PC 商业软件与驱动生态最强，企业管理成熟 | 必须兼容其应用和用户习惯；不要复制 Registry、历史控制面板分裂和不可解释后台行为 |
| macOS | Darwin 部分开源，GUI/多数框架闭源 | XNU 混合内核、sandbox、签名、公证、DriverKit | 软硬件一体、交互一致、功耗与媒体工作流强 | 学习垂直整合和默认值；不能把闭源框架或非 Apple 硬件虚拟化当作通用兼容基础 |
| Linux 桌面 | 内核 GPLv2；用户态多许可证 | 单体模块化内核、namespaces/cgroups、LSM、KVM | 驱动、架构、容器、服务器与开发生态最广 | 最适合 v1 硬件底座；必须主动解决发行版碎片、打包割裂和桌面一致性 |
| ChromeOS/ChromiumOS | ChromeOS 混合闭源；ChromiumOS 开源 | Linux、verified boot、只读/分区更新、容器/VM | 安全启动、自动恢复、低维护、Web/Android/Linux 应用组合 | 学习不可变系统、A/B 更新、恢复和分层应用运行时 |
| FreeBSD | BSD 许可 | 单体内核、Jails、Capsicum、bhyve、ZFS | 基础系统一致性、网络、存储、文档 | 学习“内核+用户态作为整体产品”、Jails、ZFS 管理体验 |
| OpenBSD | ISC/BSD 类 | 单体内核、pledge/unveil、W^X、强默认值 | 主动安全、代码审计、简洁一致 | 权限接口应小而清楚，安全默认值优先于无限兼容 |
| NetBSD | BSD 许可 | 可移植单体内核 | 跨架构可移植性与清晰抽象 | “可移植”来自严格接口与工程纪律，不来自营销口号 |
| illumos/Solaris | illumos 开源 CDDL；Oracle Solaris 闭源 | 单体内核、Zones、ZFS、DTrace | 可观测性、存储、服务隔离 | AI 运维需要系统级事件、追踪与可回放状态 |
| Qubes OS | 开源 | Xen + 分区 VM + 统一 GUI | 强隔离、模板 VM、设备域、一次性环境 | agent 与不可信应用应按任务/身份/数据域隔离 |
| ReactOS | 开源 GPL/LGPL/BSD 组合 | NT 兼容实现 | Windows API/驱动兼容研究价值 | API 行为兼容成本巨大，不能低估“bug-for-bug”兼容 |
| Wine | 开源 LGPL | Windows API 到 POSIX 的翻译层 | 低于完整 VM 的运行开销，桌面集成好 | Windows 应用的第一兼容路径，但必须维护应用兼容数据库和回归测试 |
| Haiku | 开源 MIT | 模块化单体内核，BeOS API | 一致、轻量、响应式桌面 | 小团队应控制系统范围，追求端到端一致性 |
| Fuchsia/Zircon | 开源 | 微内核风格、组件、用户态驱动/网络/文件系统 | 最小权限、组件化、独立更新 | 长期架构参考；不适合替代 v1 的成熟 PC 驱动底座 |
| Redox | 开源 MIT | Rust 微内核 | 内存安全语言、用户态服务 | 新组件优先 Rust；但语言安全不能替代硬件/应用生态 |
| SerenityOS | 开源 BSD-2 | 自研类 Unix 内核与完整桌面 | 垂直学习价值、统一 UI | 适合研究完整 OS 工程，不适合作为近期兼容底座 |
| Android/AOSP | AOSP 开源，设备常含闭源 BSP/GMS | Linux + Binder + HAL + SELinux | 大规模厂商 HAL、应用沙箱、稳定接口 | 借鉴版本化 HAL；也警惕厂商 BSP 与更新碎片 |

### 2.1 Windows：最大的兼容目标，也是历史包袱样本

Windows 的价值不是单一 UI，而是 Win32/COM/.NET/DirectX、Office/Adobe/专业软件、游戏、企业域管理和 OEM 驱动共同形成的网络效应。NT 内核具有对象管理器、I/O 管理器、内存管理器、PnP 和电源管理器等完整分层。Windows 也用 Hyper-V、WSL2 和 WSLg 证明：一个商业桌面系统可以把另一个 OS 的内核与 GUI 应用融合进本地体验。

对 Andromeda 最重要的不是复刻 NT，而是承认兼容工作的规模：

- Win32 行为包含大量历史细节和应用依赖的 quirks；
- 驱动不能通过 Wine 获得兼容；
- 内核反作弊、DRM、专业硬件和企业安全软件常要求真实 Windows；
- DirectX 到 Vulkan 的翻译能覆盖大量游戏，但不是所有软件；
- Windows 虚拟机的授权仍由 Microsoft 条款约束。

**推论：** Windows 兼容需要三级策略：

1. Wine/Proton 风格的 API 翻译；
2. 隔离的 Windows VM，以完整 RDP 桌面为可靠基线；seamless window/文件门户仅作为需通过许可证与技术门的 Pilot（见产品开发计划 §5.4）；
3. 对不能可靠运行的软件明确标注“不支持”，而不是制造模糊承诺。

### 2.2 macOS：垂直整合标杆，但不是可自由移植的兼容层

macOS 由 XNU、Darwin 用户态、专有图形与应用框架、签名/公证、安全启动、Apple silicon 共同构成。DriverKit 将多类驱动移到用户空间，以改善稳定性与安全性。Apple 的优势来自对芯片、固件、驱动、功耗、显示、输入和应用框架的共同控制。

限制同样关键：

- macOS 和 Apple SDK 的许可对 Apple 品牌硬件/授权使用有明确限制；
- 许多关键框架和图形栈不是开源 Darwin 的一部分；
- Apple silicon 应用依赖 arm64、Metal、系统框架、签名与硬件特性；
- “Darwin 能启动”不等于“macOS 应用兼容”。

**建议：**

- 在 Apple 硬件上，把 Asahi Linux 路线的上游驱动成果视为重要输入；
- 对 macOS 应用优先推动 Web、跨平台或供应商原生移植；
- 只在许可允许的 Apple 硬件场景研究 macOS VM；
- 不将 Hackintosh 作为正式产品能力。

### 2.3 Linux：唯一现实的广覆盖起点

Linux 官方文档展示了跨架构支持、统一设备模型、驱动 API、KVM、VFIO、virtio 等成熟基础。它的关键优势：

- x86-64、arm64、RISC-V 等架构；
- GPU、Wi‑Fi、蓝牙、存储、USB、音频和各类 PC 外设；
- Mesa/Vulkan、Wayland、PipeWire 等现代桌面组件；
- namespaces、cgroups、seccomp、LSM；
- KVM/QEMU、VFIO、virtio；
- OCI、Flatpak、Distrobox 等应用分发/隔离选择；
- 工具链、云与 AI 开发生态。

但 Linux 桌面的典型问题也应由 Andromeda 主动解决：

- 发行版、包格式、桌面环境和配置入口碎片；
- GPU/休眠/外接显示器等硬件体验不一致；
- 权限多为低层机制，用户难以理解；
- GUI 与 CLI 状态可能脱节；
- 应用主题、文件选择器、通知、更新体验不统一；
- 传统 POSIX 权限不足以表达 agent 的目的、时间、预算和数据边界。

### 2.4 ChromeOS：不可变与多运行时的产品化

ChromiumOS 官方架构把系统分为固件、系统用户态和 Chromium/窗口管理器，并强调 verified boot 与恢复。ChromeOS 的启示不是“浏览器就是 OS”，而是：

- 收窄硬件矩阵可显著改善可靠性；
- 系统分区、验证启动、自动更新和恢复必须一起设计；
- Web、Android、Linux 容器/VM 可以在同一产品中由系统编排；
- 用户不应成为系统维护员。

### 2.5 BSD、illumos：一致性、安全与可观测性

FreeBSD 把内核和基础用户态作为同一项目交付；Jails 提供 OS 级虚拟化，bhyve 提供硬件虚拟化，ZFS 提供快照与数据完整性。OpenBSD 以主动审计、保守默认值和小接口著称。illumos/Solaris 的 Zones、ZFS、DTrace 展示了隔离、存储和全栈追踪如何成为系统原生能力。

对 Andromeda：

- “能执行”之外，agent 必须能说明执行链；
- 系统变更应自动形成快照和事件记录；
- 服务、任务和设备访问应该可单独撤销；
- 基础系统需要单一、版本化、可回滚的发布物。

### 2.6 Qubes：AI 时代最值得吸收的安全模型

Qubes 通过 Xen 将工作、个人、银行、网络、USB 等放入不同 qube，并把窗口统一呈现在桌面上。其代价是资源开销、硬件要求、学习成本和图形/设备兼容复杂度。

Andromeda 不必每个应用一台 VM，但应吸收：

- 数据域和身份域；
- 模板化执行环境；
- 一次性任务环境；
- 网络与 USB 的隔离服务；
- 跨域复制/文件传递必须显式；
- 窗口要显示其信任域；
- AI agent 默认在独立任务域中运行。

### 2.7 Fuchsia、Redox 等新架构：长期参考而非 v1 底座

Fuchsia 把驱动、文件系统、网络栈等放在用户态组件中，Zircon 只保留内存、调度、IPC 等核心能力。Redox 用 Rust 与微内核追求内存安全和隔离。这些方向适合 Andromeda 长期演进，但当前 PC 驱动和应用覆盖不足。

**策略：** v1 使用 Linux；自研服务通过稳定 IPC/capability API 与内核解耦；新代码优先 Rust；逐步把高风险服务移入用户态隔离域。这样可以获得部分微内核收益，而不先承担重写整个硬件生态的成本。

---

## 3. “运行在所有硬件”的工程定义

### 3.1 不能实现的字面目标

没有任何现代桌面 OS 能保证运行在“所有硬件”上，原因包括：

- 设备缺少公开规格或可再分发固件；
- GPU、NPU、Wi‑Fi、指纹、摄像头 ISP 等依赖厂商驱动；
- CPU 架构和启动固件不同；
- IOMMU、TPM、安全启动能力不一致；
- 睡眠、电源、风扇和传感器经常是机型专用；
- Apple 软硬件存在技术和许可边界；
- 过旧设备无法满足现代安全基线。

### 3.2 推荐支持等级

支持等级与产品开发计划 §6.2 及硬件研究 §5 保持同一套定义：

| 等级 | 定义 | 承诺 |
|---|---|---|
| CI-0 | QEMU/KVM 虚拟参考平台 | CI 必须通过；开发者基准环境，不代表真实整机支持等级 |
| Tier 0 Blocked | 缺少启动、安全、存储或关键恢复条件 | 默认阻止安装，并给出具体原因 |
| Tier 1 Community | Live 环境可启动并完成硬件探测，有社区测试证据 | 不承诺休眠、功耗、摄像头、指纹或全部外设 |
| Tier 2 Supported | 指定整机和固件组合通过正式测试 | 已知降级公开，系统更新有持续回归与修复承诺 |
| Tier 3 Certified | 安装恢复、GPU、Wi‑Fi、音频、摄像头、蓝牙、睡眠、升级全部通过认证门 | 达到认证发布门并持续保持 |
| Tier 4 Reference | Andromeda 与 OEM 共同控制 BOM、固件与生命周期 | 固件 SLA、出厂 HCM 与长期更新承诺 |

每个机型生成机器可读的 `Hardware Compatibility Manifest`（HCM，术语见产品开发计划 §6.3）：

```yaml
machine_id: vendor-model-revision
arch: x86_64
boot: uefi
iommu: true
tpm: 2.0
gpu:
  driver: amdgpu
  vulkan: true
npu:
  runtime: null
power:
  suspend_s2idle: pass
  suspend_s3: unavailable
overall_tier: supported
```

AI 助手读取该清单后，只能承诺本机真实具备的能力。

### 3.3 驱动策略

1. 优先上游 Linux 驱动，避免长期维护巨大 out-of-tree 补丁集；
2. 固件采用可审计的许可清单和哈希；
3. GPU 优先 Mesa/Vulkan，专有驱动作为受控可选项；
4. AI 加速通过统一推理抽象适配 CPU/GPU/NPU；
5. 新的 Andromeda 专用驱动尽可能用户态化；
6. 建立硬件实验室与自动化 suspend/resume、显示、网络、音频测试；
7. 安装器先执行硬件探测并展示明确的支持矩阵。

---

## 4. 应用兼容架构

### 4.1 兼容路径

| 应用类型 | 首选路径 | 后备路径 | 主要限制 |
|---|---|---|---|
| Andromeda 原生 | 稳定 SDK + portal + sandbox | Web | 早期生态小 |
| Linux GUI/CLI | Flatpak/原生容器 | OCI/Distrobox | 发行版依赖、GUI 一致性 |
| Windows Win32 | Wine + DXVK/VKD3D 类翻译 | Windows KVM VM | DRM、驱动、反作弊、专业软件 |
| Windows 内核/设备软件 | Windows VM + 合法许可 + 可选设备直通 | 无 | IOMMU、设备独占、安全 |
| Web/PWA | 系统 Web runtime | 浏览器 | 离线/系统集成差异 |
| Android | 容器或 VM 中的 AOSP 运行时 | Web | ARM 转译、GMS 许可、输入/UI |
| macOS AppKit/Metal | 供应商移植 | 授权 Apple 硬件上的 macOS VM（研究） | 法律、框架、GPU、签名 |
| DOS/经典系统 | DOSBox/全系统模拟 | VM | 主要用于遗留场景 |

### 4.2 统一体验层

无论应用来自哪种运行时，系统都应提供统一的：

- 窗口、通知、剪贴板和输入法；
- 文件 portal，而非直接暴露整个主目录；
- 凭据代理，而非复制密钥进 VM/容器；
- 打印、摄像头、麦克风和屏幕共享授权；
- 每应用网络策略；
- 生命周期、资源和电量管理；
- 更新来源与签名显示；
- 任务栏上的运行时/信任域标识；
- agent 可调用的语义动作。

### 4.3 兼容性不是二元值

建立公开兼容数据库，每个应用记录：

- 安装、启动、核心流程、GPU、音频、输入、打印；
- 测试的版本与硬件；
- 所用运行时和配置；
- 已知风险；
- 可重复的自动化测试；
- “原生 / 良好 / 降级 / 实验 / 不支持”评级。

不要用 agent 自动修改大量 Wine 前缀、注册表或系统库后隐藏结果。所有兼容修复应成为版本化 recipe，可审计、可回滚、可复现。

---

## 5. AI 原生：从“聊天功能”到 OS 执行模型

### 5.1 Codex 与 Claude Code 的共同范式

官方资料显示，两者的共同核心不是聊天框，而是：

1. 接收目标与上下文；
2. 读取持久项目指令；
3. 通过工具观察真实环境；
4. 形成或维护计划；
5. 执行文件、命令、浏览器或连接器操作；
6. 在危险边界请求授权；
7. 运行测试并检查产物；
8. 保留会话，可继续、恢复或自动化；
9. 通过 MCP 等协议连接外部工具与数据。

Codex 还强调 workspace sandbox、approval policy、`AGENTS.md`、skills、plugins、MCP、自动化和多 agent。Claude Code 提供工具 allow/deny、permission mode、MCP、非交互输出和恢复会话等机制。

**对 Andromeda 的意义：** OS 的主抽象应从“打开应用”扩展为“委托可验证任务”。

### 5.2 建议的 Agent Runtime

```text
用户意图
  ↓
Intent Router（理解但不直接越权）
  ↓
Planner（生成任务图、风险与成本估算）
  ↓
Capability Broker（签发最小权限、时间限制、数据域限制）
  ↓
Task Sandbox / VM（执行工具和应用）
  ↓
Verifier（断言、测试、视觉检查、策略检查）
  ↓
Commit / Rollback（提交状态或完整撤销）
  ↓
Audit Ledger（记录输入、授权、工具、差异、结果）
```

关键组件：

- **Intent Router**：将“整理下载目录”“安装软件”“修复网络”映射为版本化意图；
- **Planner**：输出结构化 DAG，而非仅自然语言思考；
- **Tool Registry**：工具具备签名、schema、风险级别和幂等说明；
- **Capability Broker**：能力令牌绑定主体、资源、动作、时间、次数与预算；
- **Context Service**：提供经过权限过滤的文件、设置、设备与个人数据；
- **Task Sandbox**：每个高风险任务在容器/微 VM/VM 中执行；
- **Verifier**：执行测试、读取系统状态、比较前后差异；
- **Transaction Manager**：结合快照、包管理与配置声明实现回滚；
- **Audit Ledger**：本地优先、可搜索、可导出、对敏感数据可脱敏；
- **Model Router**：本地/云端、多供应商、能力与隐私策略路由；
- **Human Control Plane**：展示计划、权限、进度、差异和终止按钮。

### 5.3 权限模型：超越传统 UAC/sudo

传统权限只问“是否允许管理员操作”，信息不足。Andromeda 应表达：

```text
允许 Agent A
在未来 10 分钟内
读取 ~/Projects/Andromeda
写入该目录但不能删除 Git 历史
访问 docs.kernel.org
最多下载 500 MB
运行 cargo test
禁止读取 SSH 密钥和浏览器 cookie
在提交最终 diff 前再次确认
```

权限维度：

- 主体：用户、agent、模型、工具、插件；
- 资源：文件集合、设备、凭据、联系人、网络域；
- 动作：读、写、删除、执行、发送、购买、发布；
- 目的：当前任务 ID；
- 时间与次数；
- 金额/算力/流量预算；
- 数据离开设备的边界；
- 是否需要二次确认；
- 回滚和补偿动作。

所有“发送消息、转账、公开发布、删除不可恢复数据、改变安全设置”属于高影响动作，默认在最终提交点确认。

### 5.4 语义系统 API

Agent 不应靠截图和模拟点击完成所有事情。每个系统应用提供双界面：

- 面向人的 GUI；
- 面向 agent 的版本化、可发现、强类型 action API。

示例：

```json
{
  "action": "files.organize",
  "version": "1.0",
  "scope": ["~/Downloads"],
  "dry_run": true,
  "policy": {
    "never_delete": true,
    "preserve_timestamps": true
  }
}
```

只有缺少语义 API 时才回退到 accessibility tree，再回退到视觉操作。每次回退都会提高风险等级并降低可验证性。

### 5.5 本地 AI 与云端 AI

不应把“AI 原生”绑定到单一模型或始终联网：

- 小模型：离线意图分类、搜索、OCR、隐私过滤；
- 本地中型模型：敏感文档摘要、低延迟控制；
- 云端模型：复杂规划、编码、长上下文；
- 专用模型：语音、视觉、嵌入、恶意内容检测；
- 用户/组织策略决定哪些数据可出设备；
- NPU 不存在时必须回退到 GPU/CPU；
- 模型输出永远不是权限依据，策略引擎才是。

### 5.6 防御提示注入和工具供应链

桌面 agent 会读取网页、邮件、文档，这些内容可能包含恶意指令。系统必须区分：

- 用户指令；
- 系统/组织策略；
- 工具返回的数据；
- 不可信内容中的文本。

必要控制：

- 数据不能自行升级为指令；
- 工具与插件签名、来源和权限清单；
- MCP/插件按服务器和具体工具授权；
- 凭据由 broker 代用，不直接进入模型上下文；
- 网络 egress 域名白名单；
- 高风险动作采用独立验证器；
- 将“模型建议”和“系统事实”分开显示；
- 可一键终止任务并撤销临时 capability。

---

## 6. Andromeda v1 参考架构

### 6.1 分层

| 层 | 推荐选择 | Andromeda 自研重点 |
|---|---|---|
| 固件/启动 | UEFI、Secure Boot、TPM 2.0，可选 coreboot 支持 | 测量启动、恢复、设备声明 |
| 内核/驱动 | Linux LTS，尽量接近上游 | 安全配置、补丁最小化、硬件 CI |
| 基础系统 | 不可变 root、声明式镜像、A/B 或快照更新 | 原子更新、回滚、设备策略 |
| 服务管理 | systemd + D-Bus/自研强类型 IPC 边界 | capability broker、任务与审计服务 |
| 显示/媒体 | Wayland、Mesa/Vulkan、PipeWire、KDE Plasma 6/KWin | Task Center 等 Plasma 定制层、跨运行时门户（不自研 compositor） |
| 存储 | 初期 Btrfs（桌面快照便利）或经验证的其他方案 | 用户数据版本、任务事务、备份 |
| 应用 | Flatpak + OCI + 原生系统组件 | 商店、签名、权限 manifest、兼容 recipe |
| 虚拟化 | KVM/QEMU、virtio、必要时 VFIO | seamless apps（Pilot，须过许可证/技术门）、凭据/文件/通知 portal |
| Windows 兼容 | Wine、DXVK/VKD3D 路线 | 应用画像、自动测试、可回滚前缀 |
| AI | 多模型 runtime + MCP/工具协议 | agent runtime、意图 API、验证、权限 UX |
| 桌面 | KDE Plasma 6/KWin，不自研 compositor，也不 fork 完整 DE 后换主题 | 任务中心、空间/数据域、统一设置（作为 Plasma 定制层） |

### 6.2 为什么不是直接 fork Ubuntu/Fedora 后换 UI

可以复用发行版构建技术，但产品边界要更严格：

- 基础镜像由 Andromeda 统一签名；
- 系统设置只有一个权威数据模型；
- 所有修改都走声明式 API；
- shell、portal、agent runtime 和更新是同一版本契约；
- 避免让用户同时面对 apt/dnf、Flatpak、Snap、手工脚本等多个互相覆盖的状态源；
- 开发模式可以开放低层能力，但必须标识“离开受管状态”。

### 6.3 IPC 与 ABI

稳定性从 ABI 开始：

- 内部服务优先强类型、版本化 schema；
- capability 随 IPC 调用传递，不依赖全局 root；
- 不向第三方承诺 Linux 内核内部 ABI；
- 对应用承诺 Andromeda SDK/portal ABI；
- 对虚拟设备采用 virtio 等开放标准；
- 对模型/工具采用供应商中立的协议与 JSON Schema；
- 所有废弃接口有遥测（本地可见）与迁移周期。

### 6.4 桌面交互原则

1. 默认界面围绕“目标、任务、结果”，而不仅是应用图标；
2. 每个 agent 操作持续显示主体、范围和状态；
3. 系统设置可搜索、可解释、可撤销；
4. GUI 与 CLI 操作写入同一状态模型；
5. 通知按任务聚合，降低 Windows 式后台噪音；
6. 更新默认原子化，不在关机时突然长时间阻塞；
7. 错误信息给出原因、证据和可执行修复；
8. 任何自动优化都可关闭、可查看历史、可恢复默认。

---

## 7. 需要避免复制的 Windows 类问题

具体问题清单将在后续规格 `windows-pain-points.md`（见产品开发计划 §13）中逐项整理；本节先建立归类框架，使每个问题进入架构决策而非零散修补。

| 问题域 | 常见表现 | Andromeda 设计约束 |
|---|---|---|
| 更新 | 强制重启、进度不透明、失败恢复差 | 原子镜像、后台预取、明确维护窗、一键回滚 |
| 设置 | 新旧面板并存、入口重复 | 单一 schema，GUI/CLI/agent 共用 |
| 后台任务 | 难解释的 CPU/磁盘/网络活动 | 每项资源消耗归因到主体和任务 |
| 权限 | UAC 只有“允许/拒绝”，范围过大 | capability + 资源/动作/时间/目的范围 |
| 预装与推荐 | 广告、推荐、默认应用被重置 | 商业内容与系统功能分离，明确 opt-in |
| 隐私 | 遥测边界不清 | 本地仪表板、数据分类、可验证出口策略 |
| 卸载 | 残留服务、注册表、文件关联 | 沙箱与声明式安装，完整卸载清单 |
| 搜索 | 本地/云/广告混合，结果不稳定 | 来源分栏、离线优先、用户控制索引 |
| 文件占用 | 无法删除但不说明占用者 | 展示句柄主体，提供安全释放/延后事务 |
| 故障排查 | 错误码不可操作 | 结构化事件、因果链、可回放诊断包 |
| 多设备 | 驱动安装不透明 | 签名来源、兼容级别、回退驱动 |
| 默认应用 | 关联修改繁琐或被覆盖 | 一次明确选择、可导入导出、升级不重置 |

后续每条问题建议使用模板：

```markdown
### 问题：……
- 复现路径：
- 用户损失：
- 根因层：产品策略 / UI / 服务 / 内核 / OEM / 生态
- Andromeda 原则：
- v1 方案：
- 长期方案：
- 验收指标：
- 兼容与迁移风险：
```

---

## 8. 开源、闭源与治理

### 8.1 推荐开放边界

建议开源：

- 系统核心服务与 capability 模型；
- shell 的基础部分与 SDK；
- 安装器、更新客户端、硬件探测；
- agent 工具协议、权限 manifest、审计格式；
- 兼容 recipe 与测试；
- 可复现构建配置。

可以闭源或单独授权：

- 云端模型路由与托管服务；
- 商业应用商店服务；
- 企业管理控制台；
- 受第三方合同限制的 codec、固件或集成；
- 特定品牌体验层。

如果核心权限 broker 闭源，社区很难验证 AI 没有越权；这会削弱“AI 原生 OS”的信任基础。

### 8.2 许可证风险

- Linux 内核修改受 GPLv2 约束；
- Wine 为 LGPL，链接和分发方式需审查；
- Mesa、systemd、QEMU、Flatpak 等各有许可证义务；
- ZFS 的 CDDL 与 Linux 内核组合需要专门法律评估；
- Microsoft/Apple/Google 服务、商标、codec、固件、应用商店均可能有额外条款；
- macOS/Xcode 的使用存在 Apple 硬件限制；
- Windows VM 需要合法许可证；
- Android 的 AOSP 开源不等于可自由捆绑 Google Mobile Services。

发布前必须生成 SBOM、许可证清单、源码提供流程和第三方 notice。本文不构成法律意见。

### 8.3 项目治理

- 上游优先：驱动与通用修复尽量回到 upstream；
- 架构决策记录（ADR）公开；
- 安全公告和响应时限公开；
- 硬件认证标准公开；
- 遥测 schema 与默认值公开；
- 模型供应商可替换；
- 关键格式和 API 避免供应商锁定。

---

## 9. 分阶段路线图

### Phase 0：问题验证与技术样机（0–3 个月）

- 确定 3–5 台 x86-64 参考机和 QEMU CI-0 虚拟参考平台；
- 制作可启动不可变 Linux 镜像；
- 集成 Wayland + Plasma/KWin 最小桌面会话；
- 实现 agent 任务中心原型；
- capability broker v0：目录、命令、网络域、时间；
- 集成一种 Linux 应用格式、Wine 和 KVM；
- 定义硬件/应用兼容 manifest；
- 用 10 个北极星任务（见产品开发计划 §3.3）做端到端评测。

退出条件：

- 更新失败可自动回滚；
- agent 无权读取未授权目录；
- 每个操作可定位到任务和工具；
- 参考机的安装、网络、音频、显示、睡眠通过。

### Phase 1：开发者预览（3–9 个月）

- 基于 Plasma/KWin 的 Andromeda 定制层（Task Center、统一设置）达到日用基础；
- 文件、终端、设置、软件中心提供语义 action API；
- Windows 应用兼容数据库与自动回归；
- 容器/VM 文件和剪贴板 portal（seamless window 仅作为须过许可证/技术门的 Pilot）；
- 本地小模型 + 云模型路由；
- Secure Boot、TPM 密钥、恢复介质；
- 公布 SDK、权限模型和威胁模型。

### Phase 2：Alpha 硬件计划（9–18 个月）

- Tier 3 Certified 候选机型；
- GPU/多屏/休眠/蓝牙/摄像头自动化实验室；
- A/B 或快照式 OTA；
- 企业策略与多用户；
- 插件签名、商店审核和恶意工具吊销；
- 无障碍、国际化、输入法；
- 性能、电量、崩溃与兼容指标。

### Phase 3：生态与规模化（18 个月以后）

- arm64 参考硬件；
- Android 运行时（若商业需求成立）；
- 更多 NPU 后端；
- 远程任务与设备间 agent 迁移；
- 更细的设备域与微 VM；
- 评估将特定驱动/服务移出内核；
- 仅在数据证明有收益时研究替代内核。

---

## 10. 成功指标

### 硬件

- Supported/Certified 机型冷启动、安装成功率；
- suspend/resume 循环成功率（发布门与产品开发计划 §10.2 一致：Supported 100 次、Certified 500 次无阻断错误；1000 次连续循环仅作为实验室 stretch 目标，不是发布门）；
- GPU/显示/音频/网络回归通过率；
- 每瓦性能与待机耗电；
- 驱动崩溃是否可隔离恢复。

### 兼容

- Top N 目标应用的核心流程通过率；
- Wine 路径与 VM 路径的启动时间、内存、电量；
- 升级后兼容回归数；
- 配置 recipe 的可复现率。

### AI

- 任务成功率，而非回答满意度；
- 无授权访问率必须为零目标；
- 用户干预次数；
- 错误操作回滚成功率；
- 每个结果的证据覆盖率；
- 提示注入评测通过率；
- 本地/云端数据边界违规率。

### 体验

- 新机到可用时间；
- 设置任务完成步骤数；
- 更新导致的不可用时间；
- 错误自助解决率；
- 系统后台资源归因覆盖率。

---

## 11. 关键风险与应对

| 风险 | 严重度 | 应对 |
|---|---:|---|
| 目标过宽，长期没有可用产品 | 极高 | 固定 Supported/Certified 目标硬件和 Top N 工作流，按阶段交付 |
| AI agent 越权或受提示注入 | 极高 | capability、任务沙箱、数据/指令分离、提交点确认、红队 |
| Windows 兼容低于预期 | 高 | Wine + VM 双路径、公开评级、聚焦目标应用 |
| macOS 兼容承诺违法或不可行 | 极高 | 不承诺非 Apple 硬件 macOS；以移植/Web 为主 |
| Linux 桌面碎片泄漏给用户 | 高 | 单一受管镜像、portal、统一设置和打包政策 |
| 上游补丁分叉失控 | 高 | upstream-first、补丁预算与定期 rebase |
| GPU/NPU 专有栈造成不稳定 | 高 | 标准 API、后端可替换、CPU fallback、认证矩阵 |
| 云模型成本/中断/政策变化 | 高 | 多模型路由、本地能力、离线降级、预算控制 |
| 日志泄露敏感数据 | 高 | 本地优先、字段级脱敏、保留期、用户可见导出 |
| “自动化”降低用户控制感 | 高 | dry-run、差异预览、撤销、显式主体与进度 |

---

## 12. 建议立即作出的架构决策

建议批准：

1. Andromeda v1 基于 Linux LTS；
2. x86-64 为首发，arm64 为第二架构；
3. Wayland + KDE Plasma 6/KWin，不自研 compositor（详见产品开发计划 §5.2）；
4. 不可变基础系统与原子更新；
5. Flatpak/OCI 为主要应用隔离，KVM 为强隔离；
6. Wine + Windows VM 双兼容路径；
7. macOS 不作为跨 PC 兼容承诺；
8. Rust 为自研特权服务首选语言；
9. capability broker 和 audit ledger 在 UI 之前先定义数据模型；
10. AI 模型供应商中立，本地能力是降级基线；
11. 所有系统应用提供 GUI + 语义 action API；
12. 建立硬件 Tier 和应用兼容数据库。

需要通过原型再决定：

- Btrfs、ZFS 或其他文件系统的最终选择；
- systemd 保留范围；
- Flatpak 是否为唯一 GUI 包格式；
- Android 运行时优先级；
- 微 VM 的默认粒度；
- Plasma 定制层（Task Center 等）使用 Rust/C++/QML 的具体组合；
- capability IPC 的具体协议；
- 商店与系统核心的开源边界。

明确暂缓：

- 从零内核；
- 自研浏览器引擎；
- 自研完整 Office 套件；
- 非 Apple 硬件上的 macOS 产品化；
- “支持所有 Windows 软件/所有硬件”的营销承诺。

---

## 13. 一手资料与延伸阅读

### 主流系统与硬件

- Microsoft Learn：[Windows kernel-mode components and driver architecture](https://learn.microsoft.com/en-us/windows-hardware/drivers/kernel/)
- Microsoft Learn：[WSL and open-source component overview](https://learn.microsoft.com/en-us/windows/wsl/opensource)
- Microsoft Learn：[Windows AI APIs hardware requirements](https://learn.microsoft.com/en-us/windows/ai/apis/get-started)
- Apple Developer：[DriverKit — user-space drivers](https://developer.apple.com/documentation/driverkit)
- Apple Legal：[Xcode and Apple SDKs Agreement](https://www.apple.com/legal/sla/docs/xcode.pdf)
- Linux Kernel：[Kernel documentation index](https://docs.kernel.org/)
- Linux Kernel：[Device model](https://www.kernel.org/doc/html/latest/driver-api/driver-model/overview.html)
- Linux Kernel：[KVM documentation](https://docs.kernel.org/virt/kvm/index.html)
- Linux Kernel：[VFIO and IOMMU-protected device access](https://docs.kernel.org/driver-api/vfio.html)
- Linux Kernel：[Virtio](https://docs.kernel.org/driver-api/virtio/virtio.html)
- ChromiumOS：[Software architecture and verified boot](https://www.chromium.org/chromium-os/chromiumos-design-docs/software-architecture/)

### BSD、隔离与新架构

- FreeBSD：[Architecture Handbook](https://docs.freebsd.org/en/books/arch-handbook/)
- FreeBSD：[Jails and Containers](https://docs.freebsd.org/en/books/handbook/jails/)
- FreeBSD：[Virtualization](https://docs.freebsd.org/en/books/handbook/virtualization/)
- OpenBSD：[Project overview](https://www.openbsd.org/)
- Qubes OS：[Architecture](https://dev.qubes-os.org/en/latest/developer/system/architecture.html)
- Qubes OS：[Introduction and compartment model](https://www.qubes-os.org/intro/)
- Qubes OS：[Hardware requirements](https://www.qubes-os.org/doc/installation-guide/)
- Fuchsia：[System architecture](https://fuchsia.dev/fuchsia-src/get-started/learn/intro/architecture)
- Fuchsia：[Zircon fundamentals](https://fuchsia.dev/fuchsia-src/get-started/learn/intro/zircon)
- Android Open Source Project：[HAL overview](https://source.android.com/docs/core/architecture/hal)
- OmniOS/illumos：[Zones, Linux-branded zones and VM-branded zones](https://omnios.org/setup/zones)

### 兼容层

- WineHQ：[Wine is an API compatibility layer](https://www.winehq.org/about/)
- ReactOS：[ReactOS architecture and NT compatibility](https://reactos.org/architecture/)

### AI agent 范式

- OpenAI Codex：[Codex developer documentation](https://developers.openai.com/codex/)
- OpenAI Codex：[Agent approvals and security](https://learn.chatgpt.com/docs/agent-approvals-security)
- Anthropic：[Claude Code setup and execution environment](https://docs.anthropic.com/en/docs/claude-code/getting-started)
- Anthropic：[Claude Code CLI, tool and permission controls](https://docs.anthropic.com/en/docs/claude-code/cli-usage)
- Anthropic：[Model Context Protocol](https://docs.anthropic.com/en/docs/mcp)

---

## 14. 下一轮研究建议

本文是架构基线，不是最终规格。下一轮应拆为四份可执行文档：

1. `windows-pain-points.md`：整理 Windows 痛点清单，逐项追根因并形成验收指标（已列入产品开发计划 §13 后续规格）；
2. `andromeda-threat-model.md`：资产、主体、信任边界、提示注入、插件供应链；
3. `hardware-support-matrix.md`：参考机、驱动、固件、功耗和自动化测试；
4. `agent-runtime-spec.md`：intent、tool schema、capability、audit、transaction API。

第一轮原型应优先回答一个问题：**用户是否愿意把真实电脑任务交给一个能够展示计划、最小授权、执行证据和完整撤销能力的系统级 agent？**

只有这个答案成立，Andromeda 才不仅是另一个 Linux 桌面。
