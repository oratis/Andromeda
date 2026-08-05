# Andromeda 实施缺口总审计（2026-08-05）

> 对账基线：`main` SHA `03aab0ebf3713255e3f0964ee960de3735dd0522`，PR #1–#42 中
> #1–#26、#28–#42 已合并，远端无 open PR / open issue。本表以当前代码、schema 和 exact-head
> CI 证据为准；`docs/reviews/` 是历史评审快照，不能直接当作当前 backlog。

## 结论

项目已经是“可安装、可更新/回滚、能进入真实 Plasma 桌面的 x86-64 虚拟日用候选”，同时有
可持久化、强制本地鉴权的 AI 任务控制面和签名 HCM。但它还不是日用消费版：真实执行器、
游戏/Office 保真兼容域、Windows Workspace、迁移 importer、消费级更新/恢复 UI、物理硬件
认证与 Mac 平台镜像都没有完成。

旧评审中以下项目已经落地：逐 action 隔离、授权补授与转换门控、存储 compaction、taskd
内核级 loopback、本地 bearer 鉴权、capability/HCM ed25519 验签、HCM CLI fail-closed、事件
读取上限与 DTO、CLI 连接 taskd、E2E 日志/marker/共享库、供应链 pin 检查。它们不应继续列为
“未实现”。

## 按产品域对账

| 产品域 | 已实现且自动验证 | 部分实现 | 未实现 / 不能声称 |
|---|---|---|---|
| 基础 OS | Fedora bootc 44、KDE/Wayland、UEFI 离线安装、revision 更新/回滚、用户数据持久 | 安全交互安装器存在；签名 runbook/模板存在但未强制 | Secure Boot/TPM 可信链、自动健康回滚、恢复环境、断电/空间不足故障注入、消费级发布签名 |
| 日用应用 | Flatpak/Discover、Firefox、LibreOffice、打印扫描、中文输入、PipeWire、常见格式 smoke | 游戏基础包与图形栈在镜像中 | Steam/Proton 受管域、逐游戏/反作弊/性能证据、ONLYOFFICE、复杂 Office 保真基准、格式安全路由 |
| Windows 兼容 | 研究、路由策略、完整桌面优先的产品决策 | 无 | KVM Windows Workspace、RDP 桌面产品流、RemoteApp/guest bridge、Wine recipe 回滚、许可证与激活门 |
| AI 控制面 | 计划/DAG/风险/能力/策略/持久化/事件/状态门、本地认证、签名准入、CLI/taskd 单一真相源 | outcome/evidence 由调用方记录；L3 确认由调用方断言 | planner/model runtime、可信 capability issuer、用户身份、确认/凭据/connector broker、attested sandbox/microVM executor、独立 verifier、撤销执行器、Task Center |
| 迁移 | **本轮交付** manifest v1 + Windows/macOS/Linux profile 只读扫描、SHA-256、跳过项与硬上限 | 用户级应用候选只是 inventory | importer、暂停恢复、云盘、身份/偏好、应用兼容映射、共存/卸载、P2V |
| 硬件 | 隐私探测、诊断、签名 HCM、artifact/evidence/expiry gate、x86-64 虚拟 pairwise matrix | 通用 x86 包/固件覆盖；只证明存在构建路径 | 任何实体 PC/Mac Supported/Certified、真实 GPU/Wi-Fi/蓝牙/摄像头/suspend/thermal、arm64 QEMU、Intel/T2/Apple-silicon 安装镜像 |
| 供应链/发布 | Renovate 配置、pin freshness、beta prerelease、checksum、E2E artifact | Renovate App 是否安装不由仓库证明；Fedora rolling base 仍未 pin | 强制 bootc 签名、密钥轮换/吊销、SBOM/源码义务完整流水线、正式消费者 release channel |
| 消费 UX | Plasma 桌面可启动 | CLI 是开发者界面 | Task Center、统一设置、更新/存储归因 UI、审计/撤销 UI、兼容门户、无障碍完成度 |

## 12 周计划复核

- 第 1–4 周：研究、ADR 模板、x86 QEMU、HCM、首镜像、安装/回滚、Plasma、硬件 probe、
  Action/Capability 契约已大体完成；**arm64 CI、目标硬件采购清单、Top 游戏/应用/格式数据库
  schema 仍缺**。
- 第 5–6 周：Flatpak 与 Office smoke 已有；本轮补上只读迁移扫描 v1；**Steam/Proton、
  Windows Workspace/RDP、Office 复杂往返、agent 文件整理 dry-run 仍缺**。
- 第 7–8 周：网络 capability 与审计数据模型已有；**断电/空间故障注入、Wine snapshot、文件
  portal、审计 UI、三台实体 PC 测试仍缺**。
- 第 9–12 周：研究覆盖若干选择，但**P2V/RDS spike、复杂文档与 10 游戏基准、提示注入集、
  本地模型隐私实验、备份/加密/身份评审和正式 Architecture Baseline Gate 均未形成可验证交付**。

## 必须建立的 17 份规格

| # | 规格 | 当前状态 |
|---:|---|---|
| 1 | threat model | 已交付 v0 |
| 2 | agent runtime | 部分散落在 task-control-plane 与研究；独立规格缺失 |
| 3 | HCM schema | 已交付并有 schema/代码漂移测试 |
| 4 | compatibility database schema | 未实现 |
| 5 | migration manifest schema | **本轮交付 v1** |
| 6 | update and recovery | 未实现 |
| 7 | Windows Workspace | 未实现 |
| 8 | format safety | 未实现 |
| 9 | telemetry and privacy | 未实现 |
| 10 | hardware certification plan | 已交付；无实体证据 |
| 11 | backup/restore/disaster recovery | 未实现 |
| 12 | identity and session | 未实现 |
| 13 | storage encryption/key recovery | 未实现 |
| 14 | credential broker/secret store | 未实现 |
| 15 | installer coexistence/uninstall | 未实现 |
| 16 | boot platform provider | 未实现 |
| 17 | Windows pain points acceptance | 研究有输入；独立验收规格缺失 |

## 自动推进顺序

1. 合并迁移 inventory v1，并在 Linux/macOS/Windows CI 验证同一 schema 与 CLI。
2. 建立 update/recovery 规格和磁盘预算模型，随后实现空间不足与中断故障注入；这是用户
   `Windows.old` 事故最直接的产品门。
3. 建立 compatibility database schema，接 Steam/Proton 最小链路、复杂 Office/格式 corpus；
   没有逐 workload 证据就不发布兼容承诺。
4. 先冻结 identity/confirmation/capability-issuer 契约，再实现受信签发服务；否则把私钥塞进
   taskd 只会制造新的自签发路径。
5. 实现 Windows Workspace 的完整 RDP 可靠路径，再把 seamless/RemoteApp 作为独立 Pilot。
6. 物理 x86 PC、Intel/T2 Mac、Apple silicon 分队列跑实验室证据。此项需要设备、固件、
   法律授权和实验室控制面，不能由 QEMU/GCP 结果代替。

## 外部阻塞与证据边界

- “支持全部硬件”不能成为可验证承诺；只能逐 cohort 晋级。当前没有任何实体机达到
  `supported` 或 `certified`。
- GCP nested KVM 和 GitHub QEMU 能验证安装、生命周期与虚拟设备，不能认证实体 GPU、无线、
  电源、固件、Mac 或游戏性能。
- bootc/HCM/release 的正式签名和吊销需要受控密钥与发布职责；仓库不能自行创造可信根。
- Windows/macOS/P2V、Office 和字体/codec 分发涉及许可证与用户持有权，必须以明确地区、SKU
  和 artifact 做法律复核。
