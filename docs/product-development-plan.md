# Andromeda OS 产品开发计划

> 状态：Draft 0.3
>
> 日期：2026-07-30
>
> 本计划建立在 `docs/research/` 专题研究与 [操作系统全景调研](./os-landscape-and-andromeda-architecture.md) 之上。

## 2026-07-28 工程检查点

首个可安装 **Daily Driver Candidate（虚拟硬件）** 已完成：Fedora bootc 44 +
KDE Plasma Wayland 镜像可从离线 ISO 安装到空盘，并在真实桌面会话中验证
PipeWire、Flatpak、Discover、LibreOffice DOCX/XLSX/PPTX/PDF、Firefox、
打印、防火墙、OOM 保护、zram、trim 和用户数据持久性；随后完成 revision 2
原子更新和 revision 1 回滚。完整证据见
[日用消费版端到端验收](./development/daily-driver-e2e.md#已验证运行)。

该检查点提前验证了 Stage 2 的发行与可靠性骨架，不代表 Stage 2 或消费版已经
整体完成。真实 GPU、Wi‑Fi、蓝牙、摄像头、待机/休眠、固件、Steam/Proton、
Windows Workspace、Microsoft Office 原版和 PC/Mac 机型认证仍按本计划推进。

## 2026-07-30 硬件普适性检查点

Hardware Enablement Phase 1 已实现：通用 x86-64 镜像加入长尾内核模块、显式
GPU/无线/音频固件、Wi‑Fi/WWAN、摄像头、触控、存储文件系统、打印扫描和硬件
诊断工具。`andromeda hardware diagnose` 会阻止关键设备无驱动的机器进入支持
等级；HCM v2 要求 Supported 及以上固定 artifact、提供未过期测试证据和声明
到期时间。

虚拟硬件验收从单一 VirtIO profile 扩为 NVMe/SATA/IDE、e1000e/e1000、
XHCI/UHCI、HDA/AC97 与不同 CPU topology 的 pairwise 矩阵。它只证明模拟设备
驱动路径，实体 PC、Intel Mac、T2 Mac 与 Apple Silicon 仍必须进入独立实机队列。
详细边界见[硬件普适性工程](./development/hardware-enablement.md)。

## 1. 产品定义

### 1.1 愿景

Andromeda 是一个面向现有 PC 与 Mac 硬件的 AI 原生个人计算操作系统：

- 保留 Windows 最有价值的游戏、Office 和文件兼容能力；
- 达到接近 macOS 的系统一致性、可靠更新和低维护体验；
- 把 Codex、Claude Code 式“理解目标—使用工具—验证结果”的 agent 模型扩展到整个操作系统；
- 让用户可以从 Windows 或 macOS 迁入，也能在需要时安全调用原系统能力；
- 不要求用户成为 Linux 系统管理员。

### 1.2 一句话定位

> 能运行用户现有工作与娱乐负载、不会因日常维护轻易损坏、并能安全替用户完成电脑任务的 AI 原生桌面 OS。

### 1.3 三条产品支柱

#### A. Compatibility：现有数字生活可以带过来

- PC 游戏；
- Microsoft Office 文档与复杂办公流程；
- 常见和专业文件格式；
- Linux、Windows、Web 应用；
- Windows/macOS 的用户数据、设置和凭据迁移；
- 必要时通过隔离 VM 保留原系统工作环境。

#### B. Reliability：系统不把维护成本转嫁给用户

- 原子更新；
- 自动回滚；
- 系统与用户数据分离；
- 清楚的磁盘空间预算；
- 应用完整卸载；
- 后台任务可归因；
- GUI、CLI 与 AI 共用一个状态模型。

#### C. Agency：AI 是受约束的执行主体

- 能理解目标并规划；
- 能调用系统与应用的语义工具；
- 权限最小、按任务签发；
- 重要动作在提交点确认；
- 所有变更可预览、验证和审计；
- 技术上可逆的本地变更提供精确回滚；邮件、发布、购买、密钥泄露等不可逆外部动作必须在提交前确认，并使用明确标注的补偿或事件响应流程；
- 模型和云供应商可替换。

## 2. 非目标

首个正式版本不追求：

- 从零编写内核；
- 运行每一款 Windows 游戏或专业软件；
- 在非 Apple 硬件上产品化运行 macOS；
- 直接运行未移植的 macOS AppKit/Metal 应用；
- 自研浏览器引擎、Office 套件或游戏商店；
- 同时支持所有 CPU 架构与历史设备；
- 让 AI 默认获得管理员权限；
- 把 Linux 发行版的包管理和配置复杂度暴露给普通用户。

## 3. 目标用户与首要场景

### 3.1 首发用户

1. **AI 开发者与技术创作者**
   - 需要 Unix 开发环境、本地模型、容器和 GPU；
   - 同时需要 Windows 游戏或 Office；
   - 能容忍开发者预览期的少量兼容缺口。

2. **跨 Windows/macOS 的个人专业用户**
   - 文档、表格、PDF、图片、视频格式多；
   - 希望换设备或换系统时不重建全部工作环境；
   - 重视稳定性、隐私和长期可维护性。

3. **PC 游戏玩家中的尝鲜用户**
   - Steam 游戏为主；
   - 接受少量反作弊游戏暂时保留 Windows；
   - 愿意贡献硬件和游戏兼容报告。

### 3.2 暂不作为首发核心用户

- 强依赖 Adobe 全家桶、AutoCAD、特定财务税务软件且不能接受 VM 的用户；
- 只玩内核级反作弊不支持 Linux 的竞技游戏用户；
- 依赖专有采集卡、工业控制或医疗设备驱动的用户；
- 要求零学习成本的大规模企业桌面客户。

### 3.3 北极星任务

首个开发者预览必须完整跑通：

1. 从 Windows 导入用户目录、浏览器数据和应用清单；
2. 安装并启动一个已验证的 Steam/Proton 游戏；
3. 打开、编辑并导出一份复杂 DOCX/XLSX/PPTX；
4. 对不兼容 Office 流程，一键启动隔离 Windows Workspace；
5. 从 macOS 导入用户文件、照片/媒体库的可迁移部分和应用清单；
6. 系统升级失败后自动回到上一个可启动版本；
7. AI 整理下载目录，先展示计划，执行后可一键撤销；
8. AI 安装一个应用，只获得完成该任务所需的权限；
9. 明确展示每个后台进程、容器、VM 和 agent 的 CPU/内存/磁盘/网络占用；
10. 在两台 Andromeda 设备间恢复用户工作空间，但不复制硬件专属配置。

## 4. 产品原则

1. **兼容优于纯粹，但兼容层不得污染系统核心。**
2. **默认可靠优于默认可修改。** 开发模式明确解锁低层能力。
3. **更新是状态切换，不是现场改造。**
4. **AI 输出是建议；权限和系统事实由确定性组件决定。**
5. **所有高影响动作都有提交点。**
6. **所有自动化都有可见主体、原因和终止入口。**
7. **用户数据从不因系统回滚而回滚丢失。**
8. **先提供语义 API，再允许 agent 模拟点击。**
9. **没有实测通过就不承诺兼容。**
10. **硬件支持是产品能力，不是社区 Wiki 上的一句“应该能用”。**
11. **上游优先，减少永久私有补丁。**
12. **模型、应用商店和云服务均不得成为系统单点。**

## 5. 总体技术架构

```text
┌───────────────────────────────────────────────────────────────┐
│ Human Experience                                             │
│ Shell · Task Center · Settings · Files · Store · Migration   │
├───────────────────────────────────────────────────────────────┤
│ AI Control Plane                                             │
│ Intent · Planner · Capability Broker · Verifier · Audit      │
├───────────────────────────────────────────────────────────────┤
│ Application & Compatibility                                  │
│ Native/Flatpak · Web · OCI · Wine/Proton · Android? · VM     │
├───────────────────────────────────────────────────────────────┤
│ System Services                                              │
│ Identity · Secrets · Portals · Update · Backup · Telemetry    │
├───────────────────────────────────────────────────────────────┤
│ Desktop Platform                                             │
│ Wayland/Xwayland · Plasma/KWin · Mesa/Vulkan · PipeWire       │
├───────────────────────────────────────────────────────────────┤
│ Isolation                                                    │
│ namespaces/cgroups/LSM · KVM/QEMU · virtio · optional VFIO   │
├───────────────────────────────────────────────────────────────┤
│ Hardware Enablement                                          │
│ Linux stable channels · firmware · platform boot providers   │
└───────────────────────────────────────────────────────────────┘
```

### 5.1 基础系统

初步选择：

- 按 HCM cohort 管理内核：认证 PC 使用 LTS/稳定内核；新硬件使用有退出期限的 hardware-enable 稳定分支；Apple silicon 使用 Asahi 维护的内核/Mesa/固件组合；
- Fedora/RHEL 系软件与硬件生态；
- bootc/OCI 描述、构建和交付基础系统镜像；
- OSTree 管理多 deployment 与回滚，近期可复用 rpm-ostree 实现，但不把产品 API 绑定到它；
- 只读或不可变 `/usr`；
- `/etc` 采用受控三方合并；
- `/var` 与用户数据独立；
- 平台化启动档案，而不是把 UEFI/TPM 强加给所有 Mac：
  - PC：UEFI + UKI + Secure Boot + TPM2；
  - Intel Mac：Apple EFI，按机型定义可信启动、恢复与回滚能力；
  - Apple silicon：Apple boot policy + m1n1/U-Boot + 配对固件；
- 所有平台提供等价的启动尝试计数、`mark-good` 健康判定和 last-known-good 回退语义；
- 独立恢复环境，并始终保留至少两个可启动 deployment；
- Btrfs 作为首版状态层、用户数据快照与 agent 撤销候选，但不把文件系统快照当作 OS 更新机制；
- systemd 管理系统服务、资源与启动；
- fwupd/LVFS 处理可支持的固件升级。

选择门：

- 在第 12 周前，用同一参考镜像验证 bootc + OSTree，并以 rpm-ostree、固定 A/B 和 NixOS 原型作为对照；
- 必须证明断电安全、启动健康自动回滚、磁盘占用可预测、驱动扩展、数据 schema 兼容和离线恢复；
- 必须评估并补齐 bootc 当前在完整可信启动链、通用基础镜像与非 RPM 组件方面的缺口；
- 不因“声明式很优雅”直接选择会显著增加桌面应用兼容成本的方案。

### 5.2 桌面与显示

首版采用 Wayland + Xwayland + KDE Plasma 6/KWin，不开发 X11 桌面会话，也不自研 compositor。锁定一个发行版维护分支，默认不引入永久私有 KWin 补丁。

选择 Plasma/KWin 是为了直接获得并持续继承：

- 多 GPU；
- HDR/色彩管理；
- 可变刷新率；
- 分数缩放；
- 输入法；
- 屏幕共享；
- 无障碍；
- Xwayland；
- NVIDIA 特殊路径。

Andromeda 的原创界面放在 Task Center、权限面板、统一设置和空间/工作域。`andromeda-taskd`、Capability Broker、事务服务与验证器独立运行；Plasma/KWin 只通过 KRunner、Plasmoid、Kirigami、公开 D-Bus 和受限 KWin Script/Effect adapter 提供入口、窗口语境和呈现。模型不得进入 compositor 进程，不得直接读取输入设备或调用任意 D-Bus。

自动化优先使用应用语义工具，其次 AT-SPI，再次是经 RemoteDesktop portal 授权的 libei 合成输入。COSMIC/Smithay 作为季度 Pilot；只有经过至少两个产品版本，确认三个以上核心需求无法通过标准 Wayland、KWin 公共接口或上游改进解决，并且图形团队和硬件 CI 能承担多年维护时，才重新评估自研 compositor。

### 5.3 应用模型

- 系统应用：与系统镜像一起发布，使用受控内部 API；
- 第三方 Linux GUI：优先 Flatpak；
- Steam/Proton 进入专用受管游戏域，使用内容寻址 Steam Runtime、宿主图形驱动注入和逐游戏 profile，不简单套用普通 Flatpak 生命周期；
- 开发工具：OCI/Toolbox/Distrobox 类开发容器；
- CLI：受管开发环境，不默认修改基础系统；
- Web/PWA：系统 Web runtime；
- Windows：Wine/Proton 前缀或 Windows VM；
- Android：验证需求后再决定；
- macOS：不提供非授权兼容承诺。

### 5.4 Windows 兼容

建立 `Compatibility Orchestrator`：

- 识别安装包、应用和游戏；
- 查找签名兼容 recipe；
- 选择 Wine/Proton 版本；
- 配置 DXVK/VKD3D、字体、运行库和前缀；
- 应用升级前创建前缀快照；
- 自动执行烟雾测试；
- 失败时回滚 recipe；
- 必要时建议 Windows Workspace；
- 从不伪装“不支持”为“可能能用”。

Windows VM 提供：

- KVM/QEMU + virtio；
- 合法的用户自有 Windows 许可证；
- 完整 RDP 桌面作为可靠基线；
- 单窗口/RemoteApp、剪贴板、文件 portal、音频与通知桥仅作为 Pilot，必须验证 Windows SKU、RDS/CAL、Office virtualization rights、企业激活和主机端技术能力；
- 动态资源分配；
- 可选 GPU/设备直通，仅限通过认证的硬件；
- 凭据代理，不把 Andromeda 主密钥复制进 VM；
- 网络、USB 和共享目录最小授权。

Windows VM 不是游戏的通用后备。以下场景默认保留经过清楚引导的原生 Windows 启动项或独立 Windows PC：

- Xbox App、PC Game Pass、Microsoft Store/UWP 的完整安装与授权链；
- 要求 Windows 内核驱动、受保护启动链或明确禁止 VM 的反作弊；
- 发行商未启用 Proton 的 EAC/BattlEye 游戏；
- 专有 VR runtime、低延迟 direct mode 与部分追踪硬件；
- 无法在 Linux 安全升级固件或配置完整功能的游戏外设；
- 单 GPU 笔记本无法安全共享或直通 GPU 的高性能游戏。

Andromeda 不绕过 DRM/反作弊，不把 GPU 直通宣传为所有机器可用，也不把“反作弊中间件支持 Proton”误写成使用该中间件的每款游戏都受支持。

单窗口 Windows Application Bridge 在进入产品承诺前必须完成阻断性验证：

1. Windows Pro 完整 RDP 桌面；
2. Windows Server/RDS RemoteApp + CAL；
3. Azure Virtual Desktop/Windows 365；
4. 自有 guest agent 窗口桥。

在技术和授权路径确认前，完整 RDP 桌面是唯一可靠承诺。

### 5.5 Office 与文件格式

采用多引擎路由：

- LibreOffice/Collabora 或 OnlyOffice 处理常规 OOXML/ODF；
- Microsoft 365 Web 处理用户授权的云端流程；
- Windows Workspace 处理完整 VBA/ActiveX/OLE、COM/VSTO/XLL、Power Query/Power Pivot、Access、复杂排版、企业插件和 IRM/MIP 等高风险兼容场景；
- PDF、图像、媒体、电子书、压缩包由经过沙箱的专用开源库处理；
- 文件预览与索引永不自动执行宏、脚本或嵌入对象。

建立 `Format Confidence`：

```text
L0 Identify    只识别类型、来源、签名与风险
L1 Preview     在隔离解析器中生成安全预览
L2 Read        可提取和读取主要内容
L3 Edit        可基本编辑，保存前必须比较损失
L4 Round-trip  受支持语义通过结构和视觉往返测试
L5 Original-runtime  由原厂应用在其受支持运行时执行完整语义
```

复杂文档保存前可生成视觉/结构 diff，提示字体替换、公式、宏、外链和布局变化。

无论等级如何：

- 原文件永远保留；
- 转换输出写成带来源、工具版本和哈希的派生文件；
- 缺少 Microsoft/商业字体时必须展示替换及分页影响；
- DRM、加密、损坏或恶意文件可以被阻止，系统不承诺“保证打开”。

### 5.6 AI 控制平面

核心数据结构：

```text
Intent
  id
  issuer
  objective
  constraints
  expected_outputs
  risk_class

Capability
  subject
  resource_selector
  actions
  purpose_intent_id
  issued_at / expires_at
  usage_budget
  egress_policy
  confirmation_policy

ActionRecord
  tool
  inputs_digest
  capability_id
  before_state
  result
  evidence
  reversible
  recovery_semantics  # rollback | compensate | rotate-secret | none
  compensation_action
```

`rollback` 和 `compensate` 不得在数据模型或 UI 中混为一谈：前者恢复精确旧状态，后者只是执行相反业务动作。`reversible`、`recovery_semantics` 和不可逆原因是每条 ActionRecord 的强制字段。

执行顺序：

1. 用户提出目标；
2. Intent Router 形成结构化意图；
3. Planner 生成任务图和权限需求；
4. Policy Engine 确定可自动执行部分；
5. Capability Broker 签发短期权限；
6. 风险引擎将任务确定性映射到隔离层，工具在对应环境中执行；
7. Verifier 使用独立断言检查结果；
8. 用户确认高影响提交；
9. Transaction Manager 提交或撤销；
10. Audit Ledger 保存可理解记录。

隔离映射：

| 层 | 任务 | 强制约束 |
|---|---|---|
| L0 | 无工具推理或用户已经提供的文本 | 无外部数据和副作用 |
| L1 | 低风险本地任务 | namespace/cgroup + seccomp + Landlock + SELinux |
| L2 | 未知仓库、构建脚本、网页下载、第三方 MCP、压缩包/PDF 等不可信内容 | KVM microVM、quarantine、无宿主永久凭据 |
| L3 | 邮件、发布、购买、设备设置等真实外部副作用 | 宿主 typed tool broker 单次调用、参数绑定确认 |

额外规则：

- `read(secret) + network(send)` 等组合需要新的数据流授权，即使两个动作分别可用；
- 下载与迁入文件先进入 quarantine；
- 外部文档、网页和邮件不能直接写入长期 AI memory；
- L2 尚未交付时，相应任务必须禁用、只读降级或由用户手工完成，不能退化成宽权限 L1。

模型不直接持有：

- 用户主密码；
- SSH 私钥；
- 浏览器 cookie 数据库；
- TPM 解封密钥；
- 无限期文件系统权限；
- 任意网络访问；
- 无上限购买或云算力权限。

### 5.7 迁移与无缝切换

“无缝切换”拆为四层：

#### Layer 1：数据

- 文档、桌面、下载、照片、音乐、视频；
- 保留时间、标签、来源和校验哈希；
- OneDrive、iCloud Drive 等通过用户授权同步或导出；
- 导入前估算空间，导入后逐项校验；
- 源盘默认只读。

#### Layer 2：身份和偏好

- 语言、时区、键盘、网络、壁纸、无障碍；
- 浏览器书签、历史和可合法导出的密码；
- 邮件、日历和联系人账户；
- 不复制硬件绑定密钥或违反平台安全边界的凭据。

#### Layer 3：应用与工作流

- 扫描 Windows/macOS 应用清单；
- 映射为 Andromeda 原生、Linux、Web、Wine 或 VM 方案；
- 标出无法迁移项；
- 导入文件关联和默认处理偏好；
- 对专业应用生成迁移报告，不假装自动解决。

#### Layer 4：保留原环境

- P2V 仅作为实验选项；首版可靠后备是“新建受管 Windows VM + 数据导入”；
- P2V 必须按 Windows SKU/许可证、OEM 激活、UEFI/BIOS、BitLocker、VBS、Secure Boot、Windows Hello、物理 TPM→vTPM、驱动清理、VSS 一致性和磁盘身份逐项验证；
- macOS 仅在授权和硬件允许的范围保留；
- 双系统场景不直接写入休眠状态下的 NTFS/APFS；
- 原系统工作空间通过 portal 共享指定目录；
- 用户确认正常运行一段时间后才建议释放旧分区。

迁移不是一次性安装器页面，而是可暂停、可恢复、可重复校验的系统任务。

#### Layer 5：安全共存与卸载

- 默认缩小而非抹除原系统，所有分区修改先做恢复点；
- 检测 BitLocker、FileVault、Windows Fast Startup、休眠 NTFS、APFS container 和 Recovery；
- 在断电故障注入下验证安全缩盘；
- 保留 Windows Recovery、macOS Recovery/1TR/DFU 路径；
- 验证 EFI/NVRAM 启动项恢复；
- 提供卸载 Andromeda 并恢复原启动布局的工具；
- 清楚区分“迁移读取默认只读”和“用户明确授权的分区修改”。

### 5.8 备份、恢复与灾难恢复

本地 Btrfs snapshot 只负责快速 checkpoint，不是备份。Andromeda 必须另外提供：

- 加密的异机、离线或用户选择的云端备份；
- 版本保留和勒索软件隔离；
- 应用一致性与数据库 quiesce；
- 用户文件、Wine prefix、Windows VM、设置、密钥恢复材料和隐私脱敏审计的明确范围；
- 单文件、单应用、整机和跨设备恢复；
- 裸机恢复介质；
- 定期执行真实 restore drill，而不是只检查“上传成功”；
- 用户可验证的恢复清单和校验哈希。

磁盘加密至少提供用户凭据、TPM/平台密钥和离线恢复材料三条恢复路径，避免固件变化或设备损坏永久锁死数据。

## 6. 硬件支持计划

### 6.1 首发架构

- 第一优先：x86-64 UEFI PC；
- 第二优先：选定、不带 T2 的 Intel Mac，按机型白名单纳入 x86-64 支持；
- 并行预览：选定 M1/M2 Apple silicon Mac，完全服从 Asahi 上游真实成熟度；
- 实验支持：带 T2 的 Intel Mac，以及标准化 UEFI+ACPI 的通用 arm64 PC；
- M3 及更新 Apple silicon 只有在对应 Asahi 官方机型页、安装器与核心硬件支持可用后才立项；
- 32 位 x86、PowerPC 和其他历史架构不进入正式支持。

Mac 不能作为一个支持类别。无 T2 Intel Mac、T2 Mac、M1/M2、M3/M4，以及尚未进入 Asahi 官方功能表的更新代际（如 M5），都是不同硬件路线。某项硬件没有稳定驱动时，UI 必须显示缺口，不能以软件模拟“支持”。

M1/M2 游戏 Preview 的明确依赖是：

- Asahi 维护的内核、Mesa 和配对固件；
- 16 KiB host + 4 KiB 轻量游戏 VM；
- FEX + Wine/Proton + DXVK/vkd3d 的整套锁定版本；
- 单独的存储、内存、性能和功耗门槛。

任何一项未达标都只能降级为普通桌面 Preview，不能承诺 x86 游戏能力。

### 6.2 硬件 Tier

#### CI-0：虚拟参考平台

- QEMU x86-64；
- QEMU arm64；
- virtio GPU、网络、块设备、输入；
- 每次提交执行安装、升级、回滚、迁移和 agent 安全测试。

CI-0 用于持续集成，不代表真实整机支持等级。

#### Tier 0：Blocked

- 安装器发现缺少启动、安全、存储或关键恢复条件；
- 默认阻止安装，并给出具体原因；
- 用户不能通过普通确认框绕过会导致数据丢失的否决项。

#### Tier 1：Community

- Live 环境可启动并完成硬件探测；
- 社区报告有机型、固件、系统版本和测试证据；
- 不承诺休眠、功耗、摄像头、指纹或全部外设。

#### Tier 2：Supported

- 指定整机和固件组合通过正式测试；
- 已知降级能力公开；
- 系统更新对该组合有持续回归和修复承诺。

#### Tier 3：Andromeda Certified

全部通过：

- 安装和恢复；
- Secure Boot/TPM；
- GPU 与外接显示；
- Wi-Fi、蓝牙；
- 扬声器、麦克风、耳机；
- 摄像头；
- USB/Thunderbolt；
- 触控板/键盘/功能键；
- suspend/resume；
- 电池、风扇和温控；
- 固件更新；
- 连续两个正式版本升级；
- 关键 Windows 兼容负载。

#### Tier 4：Reference

- Andromeda 与 OEM 共同控制 BOM、固件、恢复镜像和生命周期；
- 除 Tier 3 全部要求外，提供固件 SLA、出厂 HCM 与长期更新承诺；
- 这是达到 macOS 式软硬件一致性的最终硬件产品线。

### 6.3 Hardware Compatibility Manifest（HCM）

每台机器产生签名清单，包含：

- 主板/机型/修订版；
- CPU、IOMMU、虚拟化；
- GPU、显示输出与驱动；
- NPU 与推理后端；
- 网络、音频、摄像头；
- 输入、传感器、生物识别；
- 电源状态；
- 固件来源与版本；
- 已通过的测试集；
- 已知问题；
- 支持 Tier。

系统、应用商店和 agent 使用同一份清单判断能力。

术语固定：

- **HCM = Hardware Compatibility Manifest**：整机、固件、Andromeda 版本、能力、测试证据和支持 Tier；
- **HEP = Hardware Enablement Pack**：可部署的内核、固件、模块和用户态驱动组合；
- capability 只是 HCM 中的能力字段，不作为另一种 manifest 名称。

### 6.4 硬件实验室

最小设备池：

- Intel 核显笔记本；
- AMD APU 笔记本；
- NVIDIA 独显游戏本；
- AMD 独显台式机；
- 多屏/高刷/HDR；
- 不带 T2 的 Intel Mac 3–5 个候选机型；
- 带 T2 的 Intel Mac 代表机型，仅用于实验通道；
- M1/M2 各 2–3 个候选机型；
- M3 及更新 Apple silicon 仅作为 Watch 样机；未出现在 Asahi 官方功能表的代际不得进入安装测试，更不形成发布承诺；
- 常用蓝牙、USB 音频、摄像头、打印机和扩展坞。

自动测试：

- 1000 次 suspend/resume；
- 冷启动/热重启；
- 断电更新恢复；
- HDMI/DP/USB-C 热插拔；
- Wi-Fi 漫游与断网恢复；
- 蓝牙睡眠后重连；
- 音频设备切换；
- GPU 压力与崩溃恢复；
- 电池闲置与视频播放；
- 固件升级/降级安全路径；
- Windows VM 和 Wine GPU 回归。

## 7. 工作流与团队

### 7.1 核心工作流

| 工作流 | 主要职责 |
|---|---|
| Platform | 内核、启动、镜像、更新、恢复、系统服务 |
| Hardware | 驱动、固件、实验室、认证、功耗 |
| Desktop | Plasma/KWin 集成、Task Center adapter、设置、文件、通知、无障碍 |
| Compatibility | Wine/Proton、VM、Office、格式、应用数据库 |
| Agent Runtime | intent、tools、capability、sandbox、audit、model routing |
| Migration | Windows/macOS 扫描、导入、P2V、账户和数据 |
| Security | threat model、签名、供应链、红队、隐私 |
| Developer Ecosystem | SDK、portals、商店、文档、CI |
| Quality | 硬件矩阵、端到端、性能、故障注入 |

### 7.2 初期最小团队能力

早期不一定每项都是独立全职岗位，但必须覆盖：

- Linux 内核/驱动；
- Wayland/Mesa 桌面；
- 系统更新/启动/文件系统；
- 虚拟化；
- Wine/Windows 兼容；
- Rust 系统服务；
- AI agent 与安全；
- 产品设计/用户研究；
- QA 自动化；
- 开源与许可证管理。

缺少内核、图形、虚拟化或安全经验时，不应把问题交给模型自动生成关键特权代码后直接发布。

## 8. 分阶段交付

### Stage 0：研究与架构候选冻结（当前—第 6 周）

交付：

- 系统、兼容、硬件、可靠性和 AI 专题研究；
- 开源采用矩阵；
- 威胁模型 v0；
- 产品需求文档；
- 10–20 台目标设备清单；
- Top 100 游戏、Top 30 办公/创作应用、Top 100 文件格式清单；
- 20 个北极星任务；
- 关键 ADR。

退出门：

- 每个关键依赖按“实际 artifact + 编译选项 + 链接方式 + 发布地区 + 商业 SKU”记录许可证、专利、源码和维护状态；
- 对 ONLYOFFICE、Ghostscript/MuPDF、NVIDIA、linux-firmware、codec 地区包、Microsoft/Apple 字体与固件形成可分发/商业授权/用户本机导入决策；
- 产品承诺可转换为测试；
- 不存在“之后再考虑安全/更新/驱动”的核心空白。

Stage 0 只冻结候选，不提前宣称架构完成。第 12 周设置正式 `Architecture Baseline Gate`。

### Stage 1：可行性原型（第 7–18 周）

交付：

- 可启动 x86-64 镜像；
- 原子更新与回滚；
- Plasma/KWin 上的 Task Center 与公开 adapter；
- Capability Broker v0；
- Flatpak 应用；
- Steam/Proton；
- LibreOffice/OnlyOffice 候选；
- KVM Windows Workspace；
- 完整 RDP Windows 桌面；
- RemoteApp/RDS/自有 guest bridge 技术与许可证 spike；
- Windows/macOS 只读迁移扫描器；
- QEMU CI。

必须证明：

- Windows 更新式磁盘占满问题不会发生；
- 更新前后磁盘预算可解释；
- 删除旧部署不会影响当前应用；
- agent 不能越过目录/网络 capability；
- 至少 10 个目标游戏和 10 份复杂文档流程通过；
- Windows VM 能在无 GPU 直通条件下完成 Office 工作。
- RemoteApp 若未通过技术与授权门，产品可靠路径仍为完整 RDP 桌面；
- L2 microVM 未交付的未知内容任务保持禁用或只读；
- 磁盘加密、账户恢复和 secret store 原型通过恢复测试。

### Stage 2：Andromeda Developer Preview（第 19–36 周）

交付：

- 安装器与恢复环境；
- Andromeda shell alpha；
- 统一设置；
- 应用/格式兼容数据库；
- Wine recipe 签名与回滚；
- Windows seamless window Pilot；
- 迁移任务可暂停/恢复；
- 审计和撤销 UI；
- 本地/云模型路由；
- 5 款 Tier 2 Supported 候选硬件；
- 安全共存安装器：BitLocker/FileVault/Fast Startup/APFS/Recovery 检测、缩盘、断电和卸载恢复；
- L2 microVM、quarantine 和 source-sink 数据流策略；
- 加密备份与单文件/整机恢复 alpha。

退出门：

- 30 天开发者日用；
- P0 数据丢失为零；
- 更新成功或自动回滚率达到本 Stage 的量化 SLO；
- 所有高影响 agent 动作有提交点；
- 安全团队完成首次提示注入与插件供应链红队。

### Stage 3：Public Alpha（第 37–60 周）

交付：

- 10–15 款 Tier 2 Supported 机型和少量 Tier 3 Certified 候选；
- Intel/AMD/NVIDIA 代表硬件；
- 目标 Intel Mac；
- Apple silicon 预览通道（以驱动成熟度为准）；
- 兼容性公开门户；
- 崩溃、功耗、更新和硬件遥测的本地可见面板；
- SDK 和语义 action API；
- 插件/工具签名；
- 用户恢复与备份。

退出门：

- 自动回滚覆盖不可启动更新；
- Supported 机型 100 次、Certified 候选 500 次 suspend/resume 无阻断错误；
- 冷启动成功率 Supported ≥99.5%/200 次，Certified 候选 ≥99.9%/1000 次；
- OTA 成功或自动回滚率 Supported ≥99.5%，Certified 候选 ≥99.9%；
- Top 100 游戏给出真实评级；
- Top 30 办公/创作应用有明确运行路径；
- Top 100 格式至少可以安全识别与预览；
- 迁移报告不会隐藏任何未迁移数据。
- 备份通过异机单文件、整机和裸机真实恢复演练。

### Stage 4：Beta 与硬件合作（第 61–90 周）

交付：

- OEM/ODM 参考设计；
- Apple silicon 支持范围冻结；
- 企业策略预览；
- 应用商店与更新基础设施；
- 差分更新 CDN；
- 安全响应和组件吊销；
- 无障碍与国际化完成；
- 性能、电量和兼容优化。

退出门：

- 外部安全审计；
- 加密、账户、secret store、备份与灾难恢复演练；
- SBOM 和源码义务流程；
- 更新基础设施灾备；
- 支持与硬件退换边界明确；
- Beta 用户核心任务成功率达到发布目标。

### Stage 5：1.0

1.0 的定义不是“所有功能完成”，而是：

- 一组明确认证的硬件可靠日用；
- 目标游戏、Office 和文件工作流有可信兼容路径；
- 更新、回滚、备份、恢复已通过故障注入；
- agent 权限和审计模型可以公开解释并经外部评估；
- 从 Windows/macOS 迁移不会静默丢数据；
- 不支持项公开且不误导。

## 9. 发布与质量门

### 9.1 更新

- 构建可复现；
- 镜像、内核、initramfs 和扩展全部签名；
- 分 Canary、Preview、Stable 通道；
- 分批发布并自动停止异常 rollout；
- 保留至少两个已知可启动部署；
- 旧部署空间由系统预算器管理；
- 清理不会触碰当前部署或用户应用数据；
- 离线恢复介质可重装系统而保留用户数据。

### 9.2 兼容

每次更新运行：

- 游戏启动与关键帧；
- DirectX/Vulkan 回归；
- Office 文档渲染像素 diff；
- 表格公式和值 diff；
- PDF 解析与恶意样本；
- Wine 前缀升级/回滚；
- Windows VM 启动、文件 portal 和剪贴板；
- Flatpak 权限与 portal。

### 9.3 AI 安全

发布阻断条件：

- 未授权文件读取；
- 凭据进入模型上下文；
- 不可信网页文本升级为系统指令；
- 插件越过声明权限；
- 高影响动作绕过提交确认；
- 审计记录缺失主体或工具；
- 撤销声称成功但状态未恢复；
- 云端路由违反本地数据策略。

## 10. 指标

### 10.1 北极星指标

**Verified Task Success Rate（VTSR）**：

用户委托的任务完成、通过确定性或人工验收、没有越权，并且产生足够证据的比例。

### 10.2 可靠性

- 成功启动率：必须记录分母、机型、固件、版本和启动截止时间；
- 更新成功或自动回滚率：不能只统计更新命令返回值；
- 自动回滚成功率；
- 更新产生的不可用时间；
- 数据损坏/丢失：发布阻断目标为零，同时按设备时长和操作次数记录；
- 休眠恢复成功率与循环次数；
- 系统后台资源可归因率；
- 从故障到恢复的中位时间。

最低硬件发布门：

| 等级 | 冷启动 | OTA 成功或自动回滚 | suspend/resume |
|---|---|---|---|
| Tier 2 Supported | ≥99.5% / 200 次 | ≥99.5% | 100 次无阻断错误 |
| Tier 3 Certified | ≥99.9% / 1000 次 | ≥99.9% | 500 次无阻断错误 |

### 10.3 兼容

- 游戏核心流程通过率；
- Office 文档视觉与结构保真率；
- 格式安全预览覆盖率；
- 应用升级后回归率；
- Wine 到 VM 降级比例；
- 迁移数据校验通过率；
- 用户迁移后需要手工重建的关键工作流数量。

### 10.4 AI

- agent 任务成功率；
- 平均授权次数；
- 用户取消率；
- 越权尝试拦截率；
- 撤销成功率；
- 证据覆盖率；
- 本地完成比例；
- 每任务延迟、能耗和云成本；
- 提示注入评测通过率。

VTSR 必须绑定版本化任务集、环境、成功断言和人工裁决规则；不同版本的任务集不得直接合并为趋势。

## 11. 主要风险

| 风险 | 应对 |
|---|---|
| 兼容范围吞噬全部开发能力 | Top N 清单、公开等级、Wine/VM 双路径 |
| Apple silicon 驱动节奏不可控 | 上游合作、单独通道、按能力发布 |
| NVIDIA/反作弊造成游戏承诺破裂 | 认证组合、厂商合作、明确保留 Windows |
| Office 往返损坏用户文档 | 格式信心等级、视觉/结构 diff、原件保留 |
| 自研桌面拖延产品 | 先复用 compositor，原创放在 AI 控制面 |
| 不可变系统妨碍驱动和开发工具 | 驱动扩展机制、开发容器、明确解锁模式 |
| AI agent 造成数据或财务损失 | capability、提交点、事务、审计、保险式限制 |
| 提示注入通过网页/文档攻击系统 | 数据/指令分离、egress、独立验证、红队 |
| 多模型带来行为不一致 | 标准工具协议、能力测试、模型适配层 |
| 云服务中断导致 OS 失能 | 本地降级、核心功能不依赖云 |
| 上游分叉难以维护 | 补丁预算、upstream-first、定期 rebase |
| 许可证阻止分发 | SBOM、法律评审、替代组件、源码流程 |

## 12. 近期 12 周执行清单

### 第 1–2 周

- 完成专题研究和采用矩阵；
- 冻结北极星任务；
- 建立 ADR 模板；
- 选择参考硬件采购清单；
- 建立 QEMU x86-64/arm64 CI；
- 定义 Hardware Compatibility Manifest（HCM）和 Hardware Enablement Pack（HEP）；
- 定义兼容等级与格式信心等级。

### 第 3–4 周

- 构建首个镜像；
- 实现安装/回滚烟雾测试；
- 集成 Wayland/Xwayland + Plasma/KWin，并建立零永久私有补丁门；
- 启动硬件探测器；
- 建立 Top 游戏/应用/格式数据库 schema；
- Capability/Intent/ActionRecord schema 评审。

### 第 5–6 周

- Steam/Proton 最小链路；
- Flatpak 最小链路；
- KVM Windows Workspace 最小链路；
- 完整 RDP Windows 桌面最小链路；
- Office 候选引擎文档往返测试；
- Windows/macOS 迁移扫描器；
- agent 目录整理 dry-run。

### 第 7–8 周

- 更新断电和空间不足故障注入；
- Wine 前缀快照与回滚；
- 文件 portal；
- agent 网络域 capability；
- 审计查看器；
- 第一批三台 PC 硬件测试。

### 第 9–10 周

- Windows P2V 授权、激活、BitLocker/vTPM 和 VSS 一致性实验，不进入首版关键路径；
- RDS RemoteApp、自有 guest window bridge、seamless window/剪贴板实验；
- 复杂 DOCX/XLSX/PPTX 基准；
- 10 款游戏基准；
- 提示注入测试集；
- 本地模型隐私过滤实验。

### 第 11–12 周：Architecture Baseline Gate

- 架构复盘；
- 基础镜像技术选择；
- Plasma/KWin adapter contract test 复盘，并确定 COSMIC/Smithay 季度 Pilot 范围；
- 文件系统选择；
- 启动平台 Provider、内核 channel 和 HEP 策略选择；
- Windows Workspace 的可靠桌面路径与单窗口 Pilot 路径选择；
- 备份、加密和账户恢复架构评审；
- 原型演示；
- 发布 Developer Preview 详细 backlog；
- 对失败假设明确终止或降级。

## 13. 必须建立的后续规格

1. `andromeda-threat-model.md`
2. `agent-runtime-spec.md`
3. `hardware-compatibility-manifest.schema.json`
4. `compatibility-database.schema.json`
5. `migration-manifest.schema.json`
6. `update-and-recovery-spec.md`
7. `windows-workspace-spec.md`
8. `format-safety-spec.md`
9. `telemetry-and-privacy-policy.md`
10. [`hardware-certification-test-plan.md`](./development/hardware-certification-test-plan.md)
11. `backup-restore-and-disaster-recovery-spec.md`
12. `identity-and-session-spec.md`
13. `storage-encryption-and-key-recovery-spec.md`
14. `credential-broker-and-secret-store-spec.md`
15. `installer-coexistence-and-uninstall-spec.md`
16. `boot-platform-provider-spec.md`

## 14. 当前建议

立即投入的主线：

1. x86-64 PC 上的不可变 Linux 原型；
2. Steam/Proton、Office 多引擎和 Windows VM；
3. Capability Broker、审计与事务撤销；
4. Windows/macOS 迁移扫描器；
5. QEMU + 真实硬件 CI。

保持探索、尚不作产品承诺：

- Apple silicon 的完整硬件覆盖；
- Android 应用运行时；
- GPU 直通默认体验；
- 自研 Wayland compositor（至少两个产品版本后才可重新评估）；
- 自研内核；
- 非 Apple 硬件 macOS 应用兼容。

Andromeda 最早应证明的不是“我们做了一个新桌面”，而是下面这个闭环：

> 用户把现有 Windows/macOS 数字生活带入系统，继续运行关键工作和游戏；系统经历更新、应用安装和 AI 自动化后仍保持可解释、可恢复和可控制。
