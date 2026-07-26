# Andromeda：macOS 式可靠性、原子更新与 AI 原生 Agent 安全研究

> 调研日期：2026-07-26
> 范围：桌面 OS 的不可变/事务更新、回滚、应用与任务隔离、AI agent 运行时、安全、隐私及可复用开源项目。
> 方法：优先采用项目官方文档、上游仓库和内核文档；“成熟度”和“复用建议”是基于这些材料作出的工程判断，不等于项目方承诺。

## 1. 执行摘要

Andromeda 不应把“像 macOS 一样不容易坏”理解为换一个文件系统，或在更新前创建一次快照。可靠性来自一组互相约束的系统设计：

1. **系统、应用、驱动、用户数据分层**：系统镜像只读且有版本；应用默认沙箱化；驱动有独立兼容与签名策略；用户数据不参与 OS 回滚。
2. **更新在旁路完成**：运行中的系统不被逐文件修改。下载、校验、安装和预启动测试发生在一个新的部署中，重启只切换部署指针。
3. **“启动成功”必须是可验证状态**：不仅要求内核启动，还要验证存储、图形、网络、登录、桌面和 agent 权限服务均健康；否则自动回到上一部署。
4. **旧版本是系统对象，不是普通目录**：用户看到的是“可回退版本、占用空间、过期时间和删除影响”，而不是类似 `Windows.old` 的可误删文件夹。
5. **AI 的权力由内核和策略服务约束**：模型提出计划，确定性策略引擎授权，隔离执行器实施，验证器确认结果；模型永远不是权限判定者。
6. **AI 操作也要事务化**：每个任务产生资源清单、变更集、审计事件、验证结果和可撤销信息。不可撤销或外部副作用必须在执行前单独确认。

### 推荐基线

Andromeda v1 推荐：

- Linux LTS 内核与主流发行版硬件生态；
- **bootc/OCI 镜像作为构建与交付格式，OSTree 作为部署/回滚机制**，但在产品化前验证 bootc 尚未完成的完整可信启动链；
- 保留至少两个可启动部署和一个独立恢复环境；
- Btrfs 用于用户数据快照与快速撤销，但**不把 Btrfs 快照单独当作 OS 更新系统**；
- Flatpak + xdg-desktop-portal 作为普通 GUI 应用权限基线；
- bubblewrap/namespaces + seccomp + Landlock + SELinux 作为普通 agent 任务沙箱；
- KVM microVM 作为处理不可信代码、浏览器内容、第三方工具和高敏数据的强隔离层；
- 独立的 capability broker、tool broker、transaction manager 和 append-only audit log；
- 本地模型优先处理索引、分类、OCR、敏感信息识别和低风险自动化；云模型按数据域和任务显式授权。

不建议 v1：

- 从零设计包管理器、快照文件系统或虚拟机监控器；
- 让 agent 直接以用户完整身份运行；
- 默认开放整个主目录、密码库、SSH 凭据或无限网络；
- 把 MCP/插件声明的 `readOnly` 等元数据当作安全边界；
- 为追求“无重启”而对内核、驱动和系统服务广泛热更新；
- 用 NixOS/Guix 直接作为消费者桌面的完整产品底座，除非团队愿意承担软件生态与调试模型的迁移成本。

---

## 2. “macOS 式可靠性”应被定义为什么

### 2.1 用户可感知的可靠性指标

Andromeda 的目标不能只是“更新命令返回 0”，而应包括：

- 系统更新不会耗尽系统盘；
- 断电发生在下载、写盘或切换任一时刻，机器仍至少有一个可启动版本；
- 更新失败不损坏当前系统；
- 新版本不能进入桌面时自动回滚，无需用户理解分区、引导器或注册表；
- OS 回滚不回滚用户文档；
- 应用升级失败不破坏其他应用；
- 驱动升级失败可回退到上一内核/驱动组合；
- 清理旧版本前能证明当前部署不再引用它；
- 用户能够回答“什么变了、为什么变、占了多少空间、能否撤销”；
- AI 做出的每项修改能够定位到任务、用户意图、授权、工具调用和结果。

### 2.2 macOS 已验证的可靠性机制与边界

macOS 的可靠感不是来自单一文件系统，也不等于“永远不会坏”，而是 Apple 在受控硬件矩阵上把存储、启动验证、恢复环境和更新签名做成了一个整体：

- [APFS](https://support.apple.com/guide/security/role-of-apple-file-system-seca6147599e/web) 提供写时复制、克隆、快照、空间共享和原子安全保存，降低元数据更新与空间预分配造成的故障面；
- [Signed System Volume](https://support.apple.com/guide/security/signed-system-volume-security-secd698747c9/web) 把只读系统卷的内容通过 Merkle 树封印；Apple silicon 启动链会验证该 seal，更新失败时可使用先前 APFS 系统快照恢复；
- [安全软件更新](https://support.apple.com/guide/security/secure-software-updates-secf683e0b36/web)把签名、设备身份、启动策略和 RecoveryOS 串成可信链；
- [Background Security Improvements](https://support.apple.com/guide/security/background-security-improvements-sec87fc038c2/web)把部分安全修复与完整 OS 大版本解耦，降低更新延迟与一次性变更量；
- 系统卷与 Data 卷分离，但 FileVault 仍负责用户数据卷的静态加密；“系统可验证”不能替代“用户数据可恢复”。

Andromeda 应复制这些**产品性质**——只读且可验证的系统对象、独立用户数据、原子快照、受保护恢复环境和签名更新——而不能假设 Linux 在开放硬件上天然拥有 Apple 的闭环优势。Apple 控制 SoC、固件、驱动和系统发布节奏；Andromeda 必须通过 HCM、每硬件类别内核通道和可回退驱动组合补偿这一差距。

### 2.3 `Windows.old` 事故对应的系统缺陷

微软把 `Windows.old` 用于保存升级前的 Windows 文件，并提供了[通过系统设置删除旧版本](https://support.microsoft.com/en-us/windows/deployment/install-upgrade/delete-your-previous-version-of-windows)和[从该目录找回升级前文件](https://support.microsoft.com/en-us/windows/deployment/install-upgrade/retrieve-files-from-the-windows-old-folder-after-a-windows-upgrade)的官方流程。删除旧版本会失去对应的回退能力，并可能删除尚未迁出的旧文件。用户报告“删除后多个软件也消失”应记录为真实事故输入，但仅凭现象不能断定软件丢失的根因。

从产品语义看，`Windows.old` 同时承载旧 OS、回退材料和迁移残留，却仍以普通目录暴露。它造成四种问题：

- **空间不可预测**：升级没有给出峰值空间和保留策略；
- **所有权不清楚**：用户无法判断目录是否仍被应用或迁移流程引用；
- **删除语义不清楚**：删除“文件”实际上可能删除回退能力或未迁移数据；
- **恢复不是事务**：当前系统、旧系统、应用和用户数据之间的依赖没有作为一个可检查对象展示。

Andromeda 应禁止用户或普通 agent 直接遍历、修改和删除部署存储。清理接口必须由部署管理器提供，并在删除前检查：当前部署、下一次启动目标、回退部署、恢复环境、驱动组合、挂起更新和数据迁移是否仍引用目标对象。

---

## 3. 原子 OS、更新与回滚项目调研

### 3.1 OSTree、rpm-ostree、Fedora Atomic Desktops 与 bootc

#### OSTree

[OSTree](https://ostreedev.github.io/ostree/introduction/) 是面向 Linux OS 的完整文件树版本与部署系统，可理解为“用于 OS 二进制的 Git”。它使用内容寻址对象、并行部署、只读 `/usr`，为每个部署保留独立 `/etc`，而 `/var` 在部署之间共享。[原子升级设计](https://ostreedev.github.io/ostree/atomic-upgrades/)保证断电后要么得到旧系统，要么得到新系统，而不是二者混合。

优点：

- 内容寻址、增量下载和跨部署去重；
- 可保留多个部署，不限于固定 A/B；
- 当前运行树不被修改；
- `/etc` 三方合并；
- GPG 签名与成熟的 Fedora/RHEL CoreOS 生产使用经验。

边界：

- `/var` 不由 OSTree 管理，数据库和应用状态迁移仍需产品自己设计；
- `/etc` 合并并不等于配置 schema 的事务迁移；
- bootloader、固件、磁盘布局与恢复环境仍需要额外工程。

#### rpm-ostree 与 Fedora Atomic Desktops

[rpm-ostree](https://coreos.github.io/rpm-ostree/) 把 RPM 组装成 OSTree 部署，并支持客户端 package layering 和 override。它的默认操作是离线、事务式构造新根，不修改当前启动根；[管理员手册](https://coreos.github.io/rpm-ostree/administrator-handbook/)明确说明 layered package 可跨升级保留，也可用于第三方内核模块或驱动用户态服务。

[Fedora Atomic Desktops](https://fedoraproject.org/atomic-desktops/) 已覆盖 GNOME Silverblue、KDE Kinoite、Sway、Budgie、COSMIC，多年实际桌面使用证明“只读基座 + Flatpak 应用 + 容器开发环境”可行。

风险：

- 桌面软件遇到 host integration、内核模块、输入法、VPN、专业硬件时仍可能需要 layering；
- layering 增加组合数量，削弱“发布镜像整体测试”的价值；
- `apply-live` 对 `/etc` 的修改不是完整事务，[其架构文档](https://coreos.github.io/rpm-ostree/apply-live/)也指出配置可能从部分 live apply 中泄漏；
- rpm-ostree 官方说明开发重点已转向 bootc、dnf 及其生态，因此新项目应避免对 rpm-ostree 的独有接口做过深绑定。

#### bootc

[bootable container/bootc 的目标](https://containers.github.io/bootable/)是使用 OCI 镜像、容器构建和签名工具交付完整 Linux OS，同时做到原子更新、回滚、保留 `/etc` 与 `/var`、工厂重置和硬件到应用的可信链。它非常适合 Andromeda 的 GitOps、供应链签名和多硬件镜像流水线。

但官方的[待完成事项](https://containers.github.io/bootable/what-needs-work.html)仍列出重要缺口：完整硬件到应用的可信链尚未完整实现，通用基础镜像、从普通 Linux 转入 bootc 后的回退、RPM 以外组件等仍有限制。

**判断**：bootc 是 Andromeda 最合适的长期镜像交付方向；OSTree 是近期更成熟的部署/回滚基础。应采用接口隔离，避免把产品控制面写死到单一实现。

### 3.2 Ubuntu Core / Snap

[Ubuntu Core](https://documentation.ubuntu.com/core/explanation/core-elements/snaps-in-ubuntu-core/index.html) 把 kernel、gadget、base、snapd 和 app 都建模为 snap；系统与应用只读，权限通过 interface 显式连接，更新强调自动、可恢复与适应不稳定网络。Snap 隔离结合 AppArmor、seccomp、mount namespace 和 device cgroup；[安全文档](https://documentation.ubuntu.com/core/explanation/security-and-sandboxing)还描述了 revision 级数据路径与 `snap revert`。

优势：

- kernel/gadget/base/app 的设备产品模型完整；
- 商业维护、商店、签名 assertion、分批发布和 fleet 管理成熟；
- 应用版本与其数据 revision 有明确联系。

限制：

- Ubuntu Core 主要定位嵌入式和设备产品，不是完整通用 PC 桌面；
- Snap Store 与 assertion 生态受 Canonical 控制，Andromeda 若要求独立治理需自建商店和签名体系；
- kernel/gadget 与设备型号绑定，通用 PC/Mac 的开放硬件组合会显著增加验证矩阵；
- Snap 的 UX、包体积、主题集成与第三方接受度需要单独验证。

**复用建议**：学习其 model assertion、validation set、revision 数据与更新门控；不建议直接把 Ubuntu Core 当 v1 通用桌面基座。

### 3.3 openSUSE MicroOS / transactional-update / Snapper

MicroOS 的 `transactional-update` 在只读根的 Btrfs 快照中运行包更新，成功后重启进入新快照；失败可回到工作快照。SUSE 的[事务更新文档](https://documentation.suse.com/sle-micro/6.1/pdf/transactional-updates_en.pdf)和 [Snapper 基础](https://documentation.suse.com/sles/16.0/pdf/SLES-snapper-basics_en.pdf)覆盖快照比较、启动与系统回滚。

优势：

- 沿用 RPM/Zypper 包生态，改造成本低；
- Btrfs 快照空间效率高，系统恢复直观；
- openSUSE 已有长期集成经验。

限制：

- 文件系统快照与软件供应链对象不是同一概念；
- 根快照布局必须仔细分离 `/var`、日志、数据库和用户数据；
- 快照会隐式占用空间，若没有配额、保留和可视化，可能重演“旧版本占满系统盘”；
- snapshot 并非独立备份，同一磁盘损坏会同时影响原始数据与快照。

### 3.4 NixOS 与 GNU Guix

[NixOS](https://nixos.org/manual/nixos/stable/) 用声明式配置构造系统 generation，可在启动器选择历史配置，也可执行 `nixos-rebuild switch --rollback`；Nix store 内容寻址、构建隔离和事务数据库有很强的可复现性。

GNU [Guix](https://guix.gnu.org/manual/en/guix.html) 同样使用函数式包管理、generation、回滚和可复现构建，并强调用户可控与自由软件。

优势：

- 系统状态接近“声明 + 构建结果”，适合审计、重现和机器批量生成；
- generation 天然支持回滚；
- side-by-side 依赖解决传统包冲突。

限制：

- 普通桌面用户、商业应用、专有驱动和 ISV 支持不足；
- 构建表达式、store 路径和调试方式与传统 Linux 差异大；
- “声明式系统”仍不能自动解决用户数据 schema、GPU 固件、引导器和外部服务的回滚；
- Guix 对非自由固件的政策与“尽可能支持所有 PC/Mac 硬件”存在直接冲突。

**复用建议**：采用 generation、可复现构建、closure/SBOM 和配置声明思想；不把 Nix/Guix 本身作为 v1 唯一包生态。

### 3.5 A/B 更新：RAUC、Mender、SWUpdate、systemd-sysupdate

#### RAUC

[RAUC](https://rauc.readthedocs.io/en/latest/) 是成熟的嵌入式更新客户端，支持中断安全、slot、启动成功/失败标记、X.509 签名、HTTP streaming、加密 bundle、PKCS#11 和灵活冗余布局。[A/B 示例](https://rauc.readthedocs.io/en/latest/examples.html)展示更新非活动 slot、启动后 mark-good、失败后由 GRUB 回退。

适合：固定 SKU、嵌入式设备、恢复环境。
不适合直接照搬：通用桌面的双完整 rootfs 空间成本、动态驱动组合、应用与 OS 独立更新。

#### Mender

[Mender](https://github.com/mendersoftware/mender) 的核心是 A/B rootfs、失败自动回滚、分批发布和 fleet 管理，也支持应用、文件与容器更新。它比 RAUC 更接近完整商业 OTA 服务。

风险：社区版与商业服务边界需逐项核查；面向设备 fleet 的控制面不等于个人电脑更新 UX。

#### SWUpdate

[SWUpdate](https://sbabic.github.io/swupdate/) 支持 single copy、double copy、rescue、签名、加密、多种 bootloader、handler、hawkBit 和 Yocto，灵活性极高。

风险：策略组合多，产品方承担更多集成、测试和安全配置责任；GPLv2 许可对分发与修改有合规要求。

#### systemd-sysupdate

[`systemd-sysupdate`](https://www.freedesktop.org/software/systemd/man/latest/systemd-sysupdate.html) 使用声明式 transfer 配置更新文件、目录或分区，支持 A/B/C 等多个并行版本。它适合作为较低层的下载、版本选择和原子替换组件，不是完整桌面发布、驱动兼容、健康检查或用户数据迁移方案。

### 3.6 ChromeOS：经过验证的消费级范式

ChromiumOS 的[文件系统和自动更新设计](https://www.chromium.org/chromium-os/chromiumos-design-docs/filesystem-autoupdate/)把根文件系统设为只读，系统与用户状态分区，增量更新写入另一份系统并只需一次重启。[启动设计](https://www.chromium.org/chromium-os/chromiumos-design-docs/boot-design/)使用 GPT 的 priority、tries、successful 标志：新内核在若干次启动内未被标记为成功，固件自动选择旧系统。[Verified Boot](https://new.chromium.org/chromium-os/chromiumos-design-docs/verified-boot/)则从只读固件开始逐级验证后续代码。

这是 Andromeda 最值得借鉴的消费级结果：

- 静默旁路更新；
- 启动次数预算；
- 由启动器而不是新系统自己保证回退；
- 根只读和 stateful 分离；
- 验证失败进入恢复；
- developer/unlocked 模式是显式状态。

但 ChromeOS 能做到高度稳定，也因为其硬件认证、功能范围和应用模型比通用 Windows PC 窄。Andromeda 若承诺“所有硬件”，必须接受更大的驱动验证成本。

### 3.7 Btrfs 与 OpenZFS：数据层工具，不是完整更新产品

[Btrfs subvolume 文档](https://btrfs.readthedocs.io/en/latest/Subvolumes.html)明确指出 snapshot 是带初始内容的 subvolume，创建快且使用 COW；但 snapshot 不是备份，嵌套 subvolume 默认不递归，根回滚必须把 `/var`、日志、数据库等持久状态分离。

[OpenZFS](https://openzfs.github.io/openzfs-docs/Basic%20Concepts/Datasets/Snapshots%20and%20Clones.html)提供原子快照、clone、rollback、hold、send/receive 和清晰空间统计。它的数据完整性和复制能力很强，但存在：

- CDDL 与 Linux 内核 GPL 的许可/分发集成约束；
- 内核外模块增加 secure boot、ABI 和驱动更新复杂度；
- 内存、运维和恢复模型对普通 PC 产品偏重。

**判断**：Andromeda v1 更适合 Btrfs 作为用户数据/任务撤销层，OSTree/bootc 作为 OS 部署层。ZFS 可作为服务器、工作站或高级存储选项，不应成为所有硬件的默认根。

### 3.8 更新方案比较

| 项目/路线 | 桌面成熟度 | 原子与回滚 | 硬件/驱动适配 | 主要许可 | Andromeda 建议 |
|---|---:|---|---|---|---|
| OSTree | 高 | 多部署、内容寻址、成熟 | 与发行版内核/RPM 生态结合好 | LGPL-2.0+ | v1 部署核心 |
| rpm-ostree | 高 | 事务部署、layering | 第三方 RPM/内核模块较实用 | GPL-2.0-or-later / LGPL-2.0-or-later / Apache-2.0 OR MIT，按文件 | 近期可复用，接口抽象 |
| bootc | 中高、快速发展 | OCI 镜像、原子、回滚 | 继承 Linux 镜像能力 | MIT OR Apache-2.0 | 长期交付格式，先补可信链 |
| Ubuntu Core/Snap | 设备高、通用桌面中低 | revision、revert、门控 | kernel/gadget 偏 SKU | GPL-3.0/LGPL 等混合 | 借鉴模型，不直接采用 |
| MicroOS/Snapper | 中高 | Btrfs 根快照 | 传统 RPM 生态友好 | GPL 系 | 可作为备选原型 |
| NixOS | 中 | generation、声明式 | 专有/长尾集成成本高 | MIT/LGPL 等混合 | 借鉴 closure 与构建 |
| Guix | 低到中 | generation、声明式 | 非自由固件目标冲突 | GPL-3.0+ | 研究参考 |
| RAUC | 嵌入式高 | A/B、mark-good | 固定板卡最佳 | LGPL-2.1 | 恢复/SKU 方案参考 |
| Mender | 嵌入式高 | A/B、fleet rollout | 固定设备最佳 | Apache-2.0 为主，服务混合 | fleet 版参考 |
| SWUpdate | 嵌入式高 | 多策略、rescue | Yocto/板卡强 | GPL-2.0 | 特殊设备可选 |
| systemd-sysupdate | 中 | A/B/C 资源更新 | 取决于上层集成 | LGPL-2.1+ | 可复用低层组件 |
| ChromeOS 模式 | 产品级高 | A/B、tries、verified boot | 依赖认证硬件 | ChromiumOS 多开源许可 | 最重要的产品范式 |

---

## 4. Andromeda 的更新与驱动架构建议

### 4.1 存储与部署布局

建议逻辑布局：

```text
EFI / boot metadata
├── 可验证引导器与 UKI
├── deployment A 元数据
├── deployment B 元数据
└── recovery 元数据

system store（普通用户不可直接写）
├── 内容寻址的 OS/驱动对象
├── 当前部署
├── 待启动部署
└── 最近已知良好部署

state
├── /etc 的受控机器配置
├── /var 的服务状态（按 schema 管理）
├── 应用数据
├── 用户 home
└── agent transaction / audit
```

原则：

- OS 对象和回退部署不是普通可浏览目录；
- home、应用数据、日志、模型缓存与 OS deployment 分配独立配额；
- 系统保留空间不可被普通应用耗尽；
- 更新前计算“下载后 + 解包后 + 回退保留 + 临时工作区”的峰值空间；
- 空间不足时先清理可再生缓存和过期、无引用对象；不得默认删除唯一回退版本或用户数据；
- 每个可删除对象都显示引用者、删除后果和可恢复性。

### 4.2 更新状态机

```text
发现更新
  → 获取签名 manifest / SBOM / 硬件适配声明
  → 空间与电源预检
  → 下载到内容寻址 store
  → 签名、哈希、版本/反回滚验证
  → 组装新 deployment
  → 离线检查与 VM/硬件实验室结果门控
  → 安装 boot entry，保留旧 deployment
  → 重启进入试用状态（tries=N）
  → 内核/根盘/TPM/图形/输入/网络/音频/登录/桌面健康检查
       ├── 通过：mark-good，后台清理过期对象
       └── 失败：固件/bootloader 自动回到 last-known-good
```

数据迁移必须遵循：

- schema 迁移有版本、预检、日志和恢复策略；
- 尽量使用向前/向后兼容的 expand-contract；
- OS 回滚若不能安全降级数据，先以旧 OS + 新 schema 兼容层启动，而不是静默破坏数据；
- 迁移工具运行在独立沙箱，只有目标数据集的短期 capability。

### 4.3 驱动是可靠性的一等公民

“可从现有 PC 和 Mac 无缝切换”不能承诺字面意义的所有硬件。建议公开三层支持：

- **认证硬件**：CI 中真实机器覆盖完整安装、睡眠、图形、网络、音频、摄像头、蓝牙、更新和回滚；
- **社区支持硬件**：使用上游 Linux 驱动，已知限制可查；
- **实验硬件**：可安装但不承诺关键功能。

驱动交付策略：

1. 优先内核上游驱动和 `linux-firmware`，减少 DKMS。
2. 对 NVIDIA、特殊 Wi-Fi、专业音频等建立签名的 Hardware Enablement Pack（HEP）。
3. deployment manifest 固定 `kernel + initramfs + firmware + out-of-tree module + userspace daemon` 的已测试组合。
4. HEP 不直接修改当前系统，而是生成新部署。
5. Secure Boot 下使用 Andromeda 签名链；用户自行模块需要显式进入 developer mode 或导入自有 Machine Owner Key。
6. 更新预检读取 PCI/USB/ACPI/DMI/设备树清单，拒绝安装缺少启动盘、GPU 或网络关键驱动的镜像。
7. 健康判定包括 DRM/KMS、显示输出、输入设备、根存储、网络、音频、睡眠恢复；不能只以 systemd 到达 `graphical.target` 为成功。
8. Apple Silicon Mac 复用 Asahi Linux 上游成果；Intel Mac 走标准 x86_64/UEFI 路径，但 T2、Touch Bar、摄像头、睡眠等需逐型号验证。
9. 不覆盖 Apple 固件/恢复分区；安装器默认保留 macOS Recovery 和可逆双启动路径。

关键结论：**驱动兼容不是在不可变 OS 之外的例外，它必须进入镜像构建、签名、部署、健康判定和回滚事务。**

---

## 5. 桌面应用与 AI 任务的隔离技术

### 5.1 Flatpak、bubblewrap 与 xdg-desktop-portal

[Flatpak](https://docs.flatpak.org/en/latest/basic-concepts.html)默认让应用只看到其 sandbox；用户文件、网络、图形 socket、D-Bus 和设备需要显式授权。[bubblewrap](https://github.com/containers/bubblewrap)提供底层 unprivileged namespaces 与精确 mount view。

[xdg-desktop-portal](https://flatpak.github.io/xdg-desktop-portal/docs/)是更重要的产品接口：沙箱应用通过文件选择、URI、打印、摄像头、屏幕共享、USB 等 portal 请求宿主资源。Document portal 只把用户选中的文件暴露给应用；USB portal 可按应用和具体设备授权并传递已打开的 file descriptor。

可直接转化为 Andromeda 的设计：

- “给 AI 一个文件”应传 capability/fd，不应把整个 home 路径加入白名单；
- 动态同意优于安装时一次性列出几十项权限；
- portal 负责参数验证、UI、permission store 和审计；
- capability 可一次性、限时、限对象和可撤销；
- 应用/agent 获得的是资源句柄，不是原始永久凭据。

局限：

- Flatpak 主要隔离 GUI 应用，不是执行任意不可信代码的最强边界；
- 过宽的 `--filesystem=home`、D-Bus 或 Wayland/X11 权限会削弱隔离；
- portal 本身成为高价值 broker，必须精简、模糊测试并防 confused deputy。

### 5.2 Linux 原生约束层

- **namespaces/cgroups**：隔离 mount、PID、IPC、network、user，并限制 CPU、内存、I/O、进程数。
- **seccomp**：Linux [seccomp-BPF](https://www.kernel.org/doc/html/latest/userspace-api/seccomp_filter.html)按 syscall 与参数过滤，减少可达内核攻击面；它不是完整授权系统。
- **Landlock**：[Landlock](https://cdn.kernel.org/doc/html/latest/security/landlock.html)允许非特权进程给自身和子进程叠加文件、网络端口及部分 IPC 限制，只能收紧、不能放宽，适合按任务生成的文件 capability。
- **SELinux/AppArmor**：系统级强制访问控制。SELinux 更适合统一 label 与跨服务策略；AppArmor 的路径策略更容易上手。Andromeda 更适合以 SELinux 作为发行版基线、Landlock 作为任务级动态收紧层。
- **systemd sandbox**：`ProtectSystem`、`PrivateDevices`、`RestrictAddressFamilies`、`SystemCallFilter` 等适合约束长期服务，但不应让模型直接生成并信任 unit 策略。

这些机制必须组合使用；任何一个单独机制都不是完整 agent sandbox。

### 5.3 Qubes OS

[Qubes OS](https://doc.qubes-os.org/en/latest/developer/system/architecture.html)使用 Xen 将安全域分到不同 VM，并提供跨域 GUI、网络与存储代理，是“security by compartmentalization”的最成熟桌面示范。

值得借鉴：

- 工作、个人、银行、未知内容分域；
- 网络栈、USB 与应用域可拆分；
- 跨域复制/打开是显式动作；
- GUI 能标识窗口所属安全域。

不宜直接作为大众 Andromeda 的默认 UX：

- 资源开销、设备直通、GPU 加速、游戏与睡眠复杂；
- 用户要理解域，日常摩擦较高；
- Xen 硬件矩阵与 Andromeda 的通用 PC/Mac 目标不完全一致。

建议提供“敏感空间/隔离工作区”产品能力，底层可用 KVM microVM，不必复制 Qubes 的整套用户模型。

### 5.4 microVM、Kata 与 gVisor

| 项目 | 隔离模型 | 成熟度 | 许可证 | 复用建议与风险 |
|---|---|---:|---|---|
| [Firecracker](https://github.com/firecracker-microvm/firecracker/blob/main/docs/design.md) | KVM microVM、极少设备模拟、jailer | 云生产高，桌面集成中低 | Apache-2.0 | 适合 x86_64 Linux host 的不可信任务；设备/GUI/Apple 平台适配不是其目标 |
| [Cloud Hypervisor](https://github.com/cloud-hypervisor/cloud-hypervisor) | Rust VMM、KVM/MSHV、virtio | 中高 | Apache-2.0/BSD-3-Clause | 比 Firecracker 更通用，支持 x86_64/AArch64；仍以云负载为目标 |
| [Kata Containers](https://github.com/kata-containers/kata-containers/blob/main/docs/design/virtualization.md) | OCI/Pod 语义映射到轻量 VM | 云生产高 | Apache-2.0 | 可复用 runtime 编排；桌面单机依赖栈偏重 |
| [gVisor](https://gvisor.dev/docs/) | Go 用户态 application kernel 拦截 syscall | 云生产高 | Apache-2.0 | 启动轻、无完整 VM 固定成本；syscall、`/proc`、`/sys` 不完整会造成兼容问题 |

建议三级执行：

- **L0：只读推理**——无工具，或只访问模型已获得的文本；
- **L1：普通任务 sandbox**——bubblewrap + user/mount/PID/network namespace + cgroup + seccomp + Landlock + SELinux；
- **L2：强隔离 microVM**——未知仓库、构建脚本、浏览器下载、第三方 MCP、解压解析不可信文件、高敏工作区；
- **L3：外部真实副作用**——通过宿主 tool broker 单次调用，VM 内不持有永久凭据。

---

## 6. Codex、Claude Code 与开源 agent 项目的范式

### 6.1 Codex 与 Claude Code 可复用的核心模式

OpenAI 的 [Codex 安全部署说明](https://openai.com/index/running-codex-safely/)描述了两个独立控制旋钮：OS 强制 sandbox 决定能触及什么，approval policy 决定何时必须停下询问；网络、身份、凭据、规则、组织策略与 agent 原生审计共同构成控制面。重要经验是：**批准不是沙箱的替代，沙箱也不是用户意图授权的替代。**

Claude Code 的[CLI](https://docs.anthropic.com/en/docs/claude-code/cli-usage)同样提供 allowed/disallowed tools、plan mode、permission mode、最大 turn 与危险的跳过权限选项。其[安全文档](https://docs.anthropic.com/en/docs/claude-code/security)和[身份与访问管理文档](https://docs.anthropic.com/en/docs/claude-code/iam)描述了只读默认、写入与 Bash 请求权限、项目级规则、企业策略和 MCP 信任边界。

Andromeda 应提炼而不是复制其 CLI：

- 计划和执行分离；
- 只读检查通常自动允许；
- 写入、网络、外部账户和不可逆动作分级；
- 权限以工作区/资源/工具操作为单位；
- durable policy 有清晰优先级，组织策略不能被用户或模型绕过；
- 任务有步数、时间、费用、网络、存储和副作用预算；
- 工具结果、exit code、stdout/stderr、diff、测试均作为证据；
- 用户随时中断，任务可恢复但不会偷偷扩大权限。

### 6.2 MCP：互操作层，不是信任层

[MCP](https://modelcontextprotocol.io/)标准化 tools、resources 和 prompts，可减少每个模型与每个服务之间的 N×M 集成。其[安全最佳实践](https://modelcontextprotocol.io/docs/tutorials/security/security_best_practices)强调请求认证、不能把 session ID 当认证、随机且绑定用户的 session、OAuth 安全等。

Andromeda 必须在 MCP 之外增加：

- 安装来源、发布者、签名、版本、哈希和 SBOM；
- 每个 tool 的确定性风险等级；
- 参数级策略，而不只看工具名；
- 用户/agent/工具的独立身份；
- 下游访问使用短期、限定 audience/scope 的 token；
- tool 输出标记为不可信数据，不能提升为指令；
- 网络目的地 allowlist 和 DNS/IP 重绑定防护；
- 对 localhost、Unix socket、metadata endpoint 也做认证与策略，不能假设 loopback 可信；
- 更新后权限 diff 与重新审批；
- 一键撤销、隔离和回滚插件版本。

MCP 的 tool annotation 或第三方自报的“只读/非破坏性”只可用于 UI 提示；执行策略必须由 Andromeda 自己验证。

### 6.3 开源 coding agent

#### OpenHands

[OpenHands Runtime](https://docs.openhands.dev/openhands/usage/architecture/runtime)通过 client-server 和 Docker sandbox 执行任意代码，具备较完整的 agent、runtime、浏览器和评测体系。核心开源部分以 MIT 为主，但仓库中 Enterprise 部分是 source-available，集成前需按目录核查许可证。

复用：sandbox API、事件流、agent/runtime 分离、可替换模型。
风险：Docker 共享宿主内核；将宿主目录 mount 入 runtime 会扩大泄漏面；项目全栈较大，直接嵌入 OS 会形成高耦合。

#### Aider

[Aider](https://github.com/aider-ai/aider)以 Git 为事务边界，自动 commit、lint、test，Apache-2.0。它证明“生成 → diff → 测试 → Git 可撤销”的小而清晰循环非常有效。

复用：变更表示、自动提交、lint/test 反馈循环、architect/edit 分工。
风险：Git 只覆盖版本库文件，不覆盖数据库、系统设置、外部账户或未追踪文件。

#### Continue

[Continue](https://docs.continue.dev/index)是 Apache-2.0 的 IDE/CLI coding agent；[tool permissions](https://docs.continue.dev/cli/tool-permissions)使用 allow/ask/exclude，读工具默认允许，写和 Bash 默认询问，headless 中无法询问的工具被排除。

复用：模型/规则/工具可组合配置、IDE UX、权限分级。
风险：是开发工具而非 OS 安全运行时，权限配置不能取代内核隔离。

### 6.4 通用 agent 编排框架

| 项目 | 作用 | 成熟度/许可 | 可复用价值 | 不能误认为 |
|---|---|---|---|---|
| [LangGraph](https://docs.langchain.com/oss/python/langchain/human-in-the-loop) | 有状态图、checkpoint、interrupt/HITL | 高；MIT | durable workflow、暂停审批、恢复、重试 | 内核沙箱、IAM、事务文件系统 |
| [AutoGen](https://microsoft.github.io/autogen/stable/user-guide/core-user-guide/core-concepts/architecture.html) | agent 通信、生命周期、多 agent runtime | 中高；代码许可需按当前仓库核查 | 多 agent 消息与 runtime 抽象 | 安全边界自动成立 |
| [Semantic Kernel](https://learn.microsoft.com/en-us/semantic-kernel/frameworks/agent/) | 多模型、plugin、process、企业集成 | 高；MIT | .NET/Java/Python、OpenAPI/MCP 插件 | OS capability broker |
| [Microsoft Agent Framework](https://learn.microsoft.com/en-us/agent-framework/overview/) | AutoGen/SK 的后续统一方向 | 新、快速演进；开源 | 观察长期 workflow/state API | v1 稳定依赖 |

这类框架解决的是“模型怎样编排工具和状态”；Andromeda 必须提供的是“工具是否可以执行、在哪里执行、以谁的身份执行、如何审计与撤销”。二者应通过稳定 Agent Runtime API 解耦。

---

## 7. AI agent 威胁模型与确定性防线

### 7.1 主要威胁

1. **直接提示注入**：用户或上层应用诱导 agent 绕过政策。
2. **间接提示注入**：网页、邮件、文档、代码注释、issue、图片 OCR、tool output 中藏有指令。
3. **confused deputy**：低权限来源借 agent 的高权限完成未授权动作。
4. **数据外泄**：敏感文件被拼入 URL、搜索词、日志、遥测、模型请求或外部消息。
5. **工具供应链**：恶意 MCP、skill、plugin、更新包、模型权重、prompt 模板或依赖。
6. **权限聚合**：单个工具权限看似合理，串联后可以读取秘密并向外发送。
7. **持久化污染**：恶意内容写入长期 memory、规则、启动项、agent 配置或模型缓存。
8. **撤销假象**：文件可回滚，但邮件、转账、发布、密钥泄露不可撤销。
9. **审计泄密**：完整 prompt/tool 日志反而保存密码、token 和私人文档。
10. **资源滥用**：无限循环、磁盘填满、GPU/电池耗尽、云 API 费用失控。

[OpenAI 对提示注入的说明](https://openai.com/safety/prompt-injections/)将其类比为面向 AI 的社会工程，并明确建议限制数据访问、具体化任务、在重要动作前确认。后续[工程分析](https://openai.com/index/designing-agents-to-resist-prompt-injection/)指出不能只依赖输入分类器，应以 source-sink 分析限制“受攻击来源”到“危险能力”的路径。

### 7.2 权限不能由自然语言决定

Andromeda 的授权对象应是结构化 capability：

```json
{
  "subject": "task:7f2...",
  "on_behalf_of": "user:alice",
  "resource": "file-object:sha256-or-handle",
  "actions": ["read"],
  "tool": "document-parser@digest",
  "network": "none",
  "expires_at": "2026-07-26T12:10:00Z",
  "max_uses": 1,
  "purpose": "summarize selected document",
  "parent_intent": "signed-user-intent-id"
}
```

要求：

- deny by default；
- capability 不可由模型伪造，由 broker 签发；
- 限 subject、resource、action、audience、时间、次数和任务；
- 子 agent 只能获得父 capability 的衰减版本；
- token 不进入 prompt，执行器在调用时注入；
- 对高风险 action 绑定用户确认后的参数摘要，防止确认后换参；
- 每次使用写入不可抵赖审计事件；
- 用户撤销、任务结束、策略变化时立即失效。

微软的[agent 最小权限指导](https://learn.microsoft.com/en-us/security/zero-trust/sfi/least-privilege-for-ai-agents)也建议 agent 独立身份、JIT entitlement、短期 token、tool allowlist、on-behalf-of 记录和可测试的撤销路径。

### 7.3 风险分级与审批

| 等级 | 例子 | 默认处理 |
|---|---|---|
| R0 观察 | 读取已选中文件、列目录、查询本地索引 | 在明确任务 scope 内自动 |
| R1 可逆本地修改 | 写工作副本、生成草稿、改项目文件 | 自动创建 checkpoint/diff，执行后验证 |
| R2 系统/敏感修改 | 安装应用、改设置、读取密码库、扩展目录权限 | 参数化审批，短期 capability |
| R3 外部副作用 | 发邮件、发布、提交 PR、删除云数据、购买 | 执行前展示最终目标/内容/费用；必须确认 |
| R4 高危或不可合理恢复 | 固件写入、关闭安全启动、批量永久删除、密钥导出 | 默认禁止或进入专用管理流程 |

“一直允许”必须被拆成具体 tool + 参数范围 + workspace + 时间；不能提供一个等价于永久 root 的按钮。

### 7.4 Prompt injection 的系统防线

- 所有外部内容携带 provenance 和 trust label；
- 用户/管理员政策、任务意图、工具返回分别放在不同结构化通道；
- 模型看到“此内容不可信”只是辅助，真正约束在 tool broker；
- source-sink taint：来自网页/邮件的不可信数据不能未经批准流入网络发送、凭据读取、shell 或持久 memory；
- 浏览与执行分域：研究 agent 默认无写权限，执行 agent 只接收已结构化计划；
- 网络默认关闭；按域名、协议、端口和请求类型开放；
- 下载文件先进入 quarantine，解析器在 microVM；
- 密钥使用 signing/decryption broker，尽量不把明文秘密暴露给 agent；
- 工具参数使用严格 schema，拒绝额外字段、路径穿越、shell 拼接和模糊 URL；
- 模型输出不能直接成为 shell；必须转为 typed action；
- 高风险组合策略：`read(secret) + network(send)` 即使分别获准，也需新增 data-flow 审批；
- 持久 memory 只接受明确来源和用户批准的事实，不存外部页面中的命令性文本。

### 7.5 工具与模型供应链

每个 model/tool/MCP/skill/plugin 作为软件供应链组件管理：

- 发布者身份、签名、版本、内容 digest；
- SBOM、依赖锁定、已知漏洞与安全响应渠道；
- reproducible build 或至少 provenance/SLSA 证明；
- manifest 声明最低权限，但由独立静态/动态分析验证；
- 首次安装和权限扩大需审批；
- 自动更新只在权限不扩大且兼容验证通过时静默进行；
- canary、kill switch、版本 pin、快速回滚；
- 第三方工具在 microVM，无宿主永久 token；
- 模型权重同样校验 hash、license、来源和反序列化风险。

---

## 8. AI 操作的事务、验证与撤销

### 8.1 统一事务模型

OS 更新和 agent 修改应共享抽象：

```text
Intent
  → Plan
  → Capability Set
  → Pre-state / Checkpoint
  → Typed Actions
  → Evidence
  → Verification
  → Commit or Compensate
```

事务记录至少包括：

- 用户原始意图与澄清；
- 计划版本和风险等级；
- 使用的模型、prompt policy digest、工具版本；
- capability 签发、使用和撤销；
- 每个动作的结构化参数摘要；
- 文件 diff、设置 diff、包/部署 diff；
- 测试、健康检查、退出码；
- commit、rollback 或 compensation 结果。

### 8.2 撤销层次

- **文件**：Btrfs reflink/snapshot、版本历史或应用级 undo；
- **项目代码**：Git commit/branch/worktree；
- **系统设置**：声明式设置数据库与 inverse operation；
- **应用安装**：版本化包与数据 schema；
- **OS/驱动**：deployment rollback；
- **外部 API**：如果服务提供撤销 API，记录 compensation；否则执行前确认；
- **秘密泄露**：无法真正撤销，立即吊销/轮换，并作为安全事件。

必须明确区分：

- rollback：恢复精确旧状态；
- compensate：执行一个相反业务动作，未必恢复原状态；
- retry：再次执行，要求 idempotency key；
- resume：从 durable checkpoint 延续；
- abort：停止后续动作，不代表已完成动作被撤销。

### 8.3 验证不能由执行模型自证

建议：

- deterministic test 优先；
- 重要变更使用独立 verifier，必要时使用不同模型；
- 文件修改校验 scope、语法、测试、敏感信息和意外大 diff；
- 系统设置校验实际读取值与服务健康；
- 外部动作校验 API receipt、对象 ID 和最终状态；
- verifier 无权扩大原 capability；
- 验证失败自动回滚可逆本地变更，外部动作进入人工恢复队列。

---

## 9. 本地模型与隐私

### 9.1 可复用运行时

- [llama.cpp](https://github.com/ggml-org/llama.cpp)：MIT，C/C++、量化、Metal/CUDA/HIP/Vulkan/SYCL、CPU+GPU 混合，适合广泛 PC/Mac 本地推理。
- [Ollama](https://docs.ollama.com/)：MIT 为主，提供模型管理和本地 API，开发体验好；产品集成需固定后端版本并审计模型来源。
- [ONNX Runtime](https://onnxruntime.ai/docs/execution-providers/)：MIT，通过 Execution Provider 抽象 CPU、DirectML、CUDA、TensorRT、OpenVINO、CoreML 等硬件，适合分类、OCR、embedding 和端侧小模型。
- [MLC LLM](https://llm.mlc.ai/)：Apache-2.0，编译到多平台和 WebGPU/WASM，适合跨设备实验。

### 9.2 隐私分区

数据策略至少分：

- **Local only**：密码、密钥、健康/财务、未授权私人文档；禁止离机；
- **Local preferred**：索引、embedding、分类、摘要初稿；本地能力不足时询问；
- **Cloud allowed with redaction**：删除身份与秘密后可发送；
- **Explicit cloud**：用户明确选择具体模型/服务/数据范围；
- **Enterprise governed**：依据组织 DLP、保留、地域和审计策略。

本地模型并不自动等于安全：

- 模型文件和 tokenizer 仍是供应链输入；
- 本地服务监听 TCP/localhost 仍可能被浏览器或恶意应用攻击；
- prompt、embedding、KV cache、swap、崩溃转储和日志可能泄密；
- GPU 显存清理、进程隔离和多用户访问要纳入威胁模型；
- 本地模型能力不足时，更应减少可用工具，而不是给更大权限“补偿”质量。

---

## 10. Andromeda AI Runtime 参考架构

```text
User / App
   │ signed intent + selected resources
   ▼
Intent & Policy Service ───── Organization / Device Policy
   │
   ├── Planner (local or cloud model; no ambient authority)
   │
   ▼
Risk Engine + Capability Broker
   │ short-lived attenuated capabilities
   ▼
Tool Broker ───── MCP adapter / native typed tools / portals
   │
   ├── L1 process sandbox
   ├── L2 microVM sandbox
   └── Host privileged service (very narrow API)
   │
   ▼
Transaction Manager ───── Btrfs/Git/settings/deployment checkpoints
   │
   ▼
Verifier ───── tests / policy / independent model / health checks
   │
   ├── commit
   └── rollback / compensate / incident

All components → append-only audit + privacy-aware redaction
```

### 必须由 OS 提供、不能留给 agent 框架的能力

- 用户/agent/tool/device 身份；
- capability 签发和撤销；
- 文件、设备、网络、账户 portal；
- LSM、namespace、seccomp、microVM 执行；
- credential broker；
- OS/应用/设置/文件 checkpoint；
- 审计、资源预算、kill switch；
- 更新、驱动和恢复；
- 跨模型一致的风险 UI。

### 可替换的上层组件

- planner/model；
- LangGraph、AutoGen、Semantic Kernel 等编排器；
- MCP client/server；
- coding agent；
- verifier model；
- 本地推理后端。

这种边界使 Andromeda 不会被某个模型厂商或 agent 框架锁定，同时保证更换模型不会改变设备的安全语义。

---

## 11. 分阶段实施建议

### Phase 0：威胁模型与验证基线（0—3 个月）

- 定义 deployment、state、app、driver、task 五类对象和所有权；
- 选定首批认证硬件：至少 Intel/AMD x86_64 台式机与笔记本、NVIDIA/AMD/Intel GPU、Intel Mac、两代 Apple Silicon；
- 建立真实硬件 lab 和断电/磁盘满/坏更新测试；
- 定义 capability schema、risk taxonomy、audit event schema；
- 对 `Windows.old` 类空间与回退事故编写验收测试。

退出标准：在 VM 中 1000 次随机断电更新测试均保留可启动 deployment；审计可重建每个 agent 动作。

### Phase 1：可靠系统原型（3—8 个月）

- bootc/OCI 构建 + OSTree 部署；
- UKI/Secure Boot、签名 manifest、SBOM；
- A/B 或多 deployment、tries、mark-good、recovery；
- 系统保留空间、对象引用跟踪、清理 UI；
- HEP v0：内核/固件/模块组合固定；
- Btrfs state 布局和独立 home。

退出标准：更新中任意断电可恢复；图形/存储驱动失败自动回滚；用户数据不随 OS 回滚。

### Phase 2：应用与 Agent 安全原型（6—12 个月）

- Flatpak/portal 权限 UI；
- L1 sandbox：bubblewrap + cgroup + seccomp + Landlock + SELinux；
- Tool Broker、credential broker、MCP adapter；
- task capability、预算、审计和 kill switch；
- Git/Btrfs/settings 三类事务；
- 本地 llama.cpp/ONNX Runtime 处理敏感低风险任务。

退出标准：恶意网页/文档不能从默认研究任务读取秘密并外传；task 结束后所有临时 capability 失效。

### Phase 3：强隔离与真实工作流（10—18 个月）

- KVM microVM runtime，比较 Firecracker、Cloud Hypervisor、Kata；
- 浏览/解析/构建不可信内容进入 L2；
- Codex/Claude Code 式 plan/edit/test/verify UX；
- Office 文档、邮件、浏览器、设置和开发任务的 typed tools；
- 外部副作用的参数绑定审批和 compensation；
- 多模型路由与企业本地策略。

退出标准：通过 prompt injection、tool poisoning、credential exfiltration、localhost 攻击和供应链红队测试。

### Phase 4：硬件扩展与产品化（持续）

- 发布认证硬件清单和自动化兼容报告；
- Apple Silicon/Intel Mac 安装与可逆双启动；
- canary rollout、远程 kill switch、崩溃与回滚遥测（用户可控）；
- 第三方 tool/app 签名、权限审核和安全响应；
- 开发者模式与普通模式的清晰边界。

---

## 12. 关键决策与待验证问题

### 可以现在决定

1. Linux 上游驱动生态作为底座。
2. OS deployment 与用户数据严格分离。
3. 更新必须旁路、签名、可自动回滚。
4. 旧 deployment 只能通过引用感知的系统 API 清理。
5. Flatpak portal 是普通应用权限接口的起点。
6. agent 没有 ambient authority，模型不参与最终授权。
7. capability 是短期、可衰减、可撤销、参数绑定的。
8. 不可信内容与危险工具之间做 source-sink 隔离。
9. 所有可逆动作先 checkpoint；不可逆动作执行前确认。
10. agent 框架可替换，OS 安全控制面不可下放给框架。

### 必须用原型回答

1. bootc 当前可信启动链缺口需要多少自研，何时可替代 rpm-ostree 接口？
2. 通用桌面采用“多 OSTree deployment”还是固定 rootfs A/B，在低容量 SSD 上的峰值空间差异多大？
3. Btrfs state snapshot 与数据库一致性怎样通过应用 quiesce/portal 协调？
4. 哪些 GPU/游戏/反作弊工作负载不能接受默认应用沙箱？
5. NVIDIA 专有模块、Secure Boot 与 HEP 的签名/回滚 UX 如何产品化？
6. Firecracker、Cloud Hypervisor、Kata、gVisor 在桌面交互、GPU、网络和 Apple Silicon 上的成本分别如何？
7. portal permission store 怎样表达一次、任务期、永久、设备级和数据流组合授权？
8. 审计日志如何兼顾可追责与不保存敏感 prompt/文档？
9. 本地模型在目标最低硬件上的延迟、功耗和安全任务准确率是否足够？
10. OS 数据 schema 回滚的兼容窗口和开发者契约如何制定？

---

## 13. 最终建议

Andromeda 的差异化不应是“Linux 加一个聊天框”，而应是两套可组合的可靠事务：

- **系统事务**保证 OS、驱动、应用和数据在更新后仍处于已知、可启动、可恢复的状态；
- **意图事务**保证 AI 的计划、权限、动作、验证和撤销始终对应用户真正授权的目标。

前者吸收 ChromeOS、OSTree/bootc、RAUC 和 Btrfs 的经验；后者吸收 Codex、Claude Code、Flatpak portal、Qubes、capability security 和 durable workflow 的经验。两者通过 deployment、capability、checkpoint、evidence 和 audit 形成统一控制面。

产品上的一句话可以是：

> Andromeda 保留 PC 的硬件、游戏和文件兼容能力，以镜像化原子更新获得接近 Mac 的低系统熵，并让 AI 只能在可解释、最小权限、可验证和可撤销的事务中行动。

这条路线比“所有操作都交给 AI”保守，但它更接近一个能长期托付个人数据、工作账户和真实设备的操作系统。

---

## 14. 基础系统选型记录

### 14.1 推荐方案

**推荐：Fedora/RHEL 系 Linux 用户态生态 + Linux LTS/稳定内核策略 + bootc OCI 构建交付 + OSTree deployment + systemd-boot/UKI + Btrfs state。**

这里的“Fedora/RHEL 系”主要指 RPM、SELinux、Mesa、PipeWire、systemd、libvirt、Flatpak、firmware 与硬件 enablement 的工程生态，不要求 Andromeda 在产品层面表现为 Fedora，也不要求永远绑定 Fedora 发布节奏。

具体分工：

| 层 | 推荐实现 | 原因 |
|---|---|---|
| 内核与硬件 | Linux LTS/稳定分支 + upstream-first 驱动 + linux-firmware + 签名 HEP | PC 驱动覆盖最现实；便于吸收 Intel/AMD/NVIDIA 和 Asahi 上游工作 |
| 构建/发布 | bootc-compatible OCI image、签名、SBOM、provenance | 可复用容器供应链、registry、分层和 CI；长期方向清晰 |
| 本机部署 | OSTree deployment，近期可借 rpm-ostree 集成 | 内容寻址、多版本、增量、断电安全和成熟回滚 |
| 启动 | UEFI Secure Boot + UKI + tries/mark-good + 独立 recovery | 把内核、initrd、命令行纳入签名对象；坏版本不依赖自身完成回滚 |
| 持久状态 | Btrfs 分 subvolume、配额、快照与 send；应用感知迁移 | 高效本地 checkpoint，适合 agent 文件撤销；与 OS deployment 解耦 |
| GUI 应用 | Flatpak + xdg-desktop-portal | 最成熟的 Linux 桌面动态授权与应用交付组合 |
| 强制策略 | SELinux + seccomp + Landlock + namespace/cgroup | 系统标签策略与任务级动态收紧互补 |
| 强隔离 | KVM + Cloud Hypervisor/Firecracker 原型对比 | 复用 Linux 虚拟化，不自研 VMM；处理不可信代码和内容 |

### 14.2 为什么不选择其他方案作为唯一基座

| 未选为唯一基座的方案 | 原因 | 仍然吸收什么 |
|---|---|---|
| 传统 Debian/Ubuntu 可变根 + apt | 包脚本直接修改运行根，状态空间大；很难给出 deployment 级原子性和精确回退 | DEB 软件生态、硬件适配经验、LTS 维护方法 |
| Ubuntu Core 全量采用 | kernel/gadget/model 更适合固定设备 SKU；Snap Store/Brand Store 治理和桌面生态绑定较深 | validation set、revision 数据、interface 权限、分批更新 |
| openSUSE MicroOS 作为主基座 | 事务更新成熟，但主要依赖 Btrfs snapshot，软件供应链对象、启动可信链和 deployment 内容寻址不如 OSTree/bootc 一体化 | Snapper UX、包管理兼容、快照清理与恢复 |
| NixOS | 声明式和 generation 极强，但专有驱动、商业软件、ISV 支持、路径语义和调试模型增加消费者桌面门槛 | closure、可复现构建、声明式配置、generation |
| GNU Guix | 自由软件政策与“现有 PC/Mac 尽量可用”的非自由固件现实冲突更大 | 可审计构建、用户 profile、generation |
| 固定分区 RAUC/Mender A/B | 固定设备很稳，但通用 PC 双完整 rootfs 空间成本高；桌面应用、驱动 layering 和多历史版本不够灵活 | boot tries、mark-good、bundle 签名、fleet rollout |
| ChromeOS/ChromiumOS 直接 fork | 产品稳定性范式优秀，但浏览器中心应用模型、Google 特定构建/认证与硬件范围不符合游戏、Office、专业文件目标 | verified boot、A/B、stateful 分区、silent update、Powerwash/recovery |
| OpenZFS 默认根 | 数据完整性强，但 CDDL/GPL 分发组合、内核外模块、Secure Boot 与通用桌面维护成本偏高 | snapshot/clone/hold、空间统计、send/receive |
| Qubes OS/Xen | 安全域隔离极强，但 GPU、游戏、设备直通、续航、睡眠和普通用户 UX 成本过高 | 安全域、可信 GUI、USB/网络拆分、跨域 portal |
| 从零微内核/新内核 | 驱动、游戏、Office、反作弊与硬件兼容会被推迟多年，违背“现在的 PC/Mac 无缝切换”目标 | capability、内存安全、形式化验证思想可逐层引入 |

### 14.3 需要保留的实现替换点

基础方案不是永久锁定，必须设计以下稳定接口：

- `ImageSource`：bootc registry、离线介质或企业镜像源；
- `DeploymentManager`：OSTree、未来 systemd-sysupdate 或其他部署后端；
- `SnapshotProvider`：Btrfs、ZFS、高级存储或云盘快照；
- `SandboxProvider`：process sandbox、gVisor、KVM microVM；
- `ModelProvider`：llama.cpp、ONNX Runtime、MLC、云 API；
- `WorkflowEngine`：内建状态机、LangGraph、Microsoft Agent Framework；
- `ToolTransport`：native portal、MCP、OpenAPI；
- `PolicyEngine`：设备、本地用户、企业和应用商店策略。

替换点的安全语义由 Andromeda 定义，后端不能自行扩大 capability。

---

## 15. 项目目录与技术雷达

等级含义：

- **Adopt**：可作为 v1 基础依赖或产品标准；
- **Pilot**：应立即做受控原型，以实测决定集成深度；
- **Watch**：持续跟踪，暂不进入关键路径；
- **Reject**：拒绝作为 Andromeda 的该层核心方案；不代表项目本身质量差。

许可证列为项目主要代码许可的概括；一个仓库、发行版或产品可能包含多种许可，正式分发前必须由合规工具按文件和依赖重新扫描。

### 15.1 OS、更新、存储与可信启动

| 项目 | 主要许可证 | 成熟度 | 雷达 | Andromeda 用途/判断 | 官方链接 |
|---|---|---:|---|---|---|
| OSTree | LGPL-2.0+ | 生产高 | Adopt | 内容寻址 deployment、原子切换、回滚 | [官方文档](https://ostreedev.github.io/ostree/) |
| rpm-ostree | GPL-2.0-or-later / LGPL-2.0-or-later / Apache-2.0 OR MIT，按文件 | 生产高、重点迁移中 | Adopt（近期） | RPM 与 OSTree 集成、驱动/host layering；产品 API 要隔离 | [上游许可](https://github.com/coreos/rpm-ostree/tree/main/LICENSES) |
| Fedora Atomic Desktops | 多开源许可 | 桌面高 | Adopt（参考发行基线） | 验证不可变桌面、Flatpak 与容器工作流 | [Fedora](https://fedoraproject.org/atomic-desktops/) |
| bootc / Bootable Containers | MIT OR Apache-2.0 | 中高、快速发展 | Pilot | 长期 OCI OS 构建与交付；补可信链和桌面产品化 | [上游许可](https://github.com/bootc-dev/bootc#license) |
| Ubuntu Core / snapd | GPL-3.0/LGPL 等混合 | 设备生产高 | Watch | 学习 model assertion、validation、interface；不作通用桌面基座 | [Ubuntu Core](https://documentation.ubuntu.com/core/) |
| openSUSE MicroOS / transactional-update | GPL 等混合 | 生产高 | Pilot（比较组） | 与 OSTree 路线做断电、空间、驱动与恢复对比 | [SUSE 文档](https://documentation.suse.com/sle-micro/6.1/pdf/transactional-updates_en.pdf) |
| Snapper | GPL-2.0 | 生产高 | Adopt（设计参考） | Btrfs 快照管理、比较、清理和启动回滚 UX | [Snapper 文档](https://documentation.suse.com/sles/16.0/pdf/SLES-snapper-basics_en.pdf) |
| Nix / NixOS | LGPL-2.1/MIT 等混合 | 生产高、桌面小众 | Watch | closure、generation、可复现构建；不作唯一用户态生态 | [NixOS 手册](https://nixos.org/manual/nixos/stable/) |
| GNU Guix System | GPL-3.0+ | 稳定、小众 | Reject（主基座） | 固件政策与所有硬件目标冲突；保留声明式研究价值 | [Guix 手册](https://guix.gnu.org/manual/en/guix.html) |
| RAUC | LGPL-2.1 | 嵌入式生产高 | Pilot（恢复/SKU） | A/B、mark-good、签名 bundle、恢复环境 | [官方文档](https://rauc.readthedocs.io/en/latest/) |
| Mender | Apache-2.0 为主，服务分层 | 嵌入式生产高 | Watch | fleet、分批更新、A/B 自动回退 | [官方仓库](https://github.com/mendersoftware/mender) |
| SWUpdate | GPL-2.0 | 嵌入式生产高 | Watch | 特殊板卡/Yocto 的高度可定制更新 | [官方文档](https://sbabic.github.io/swupdate/) |
| systemd-sysupdate | LGPL-2.1+ | 中高 | Pilot | transfer 与 A/B/C 底层更新原语 | [上游 man page](https://www.freedesktop.org/software/systemd/man/latest/systemd-sysupdate.html) |
| ChromiumOS update_engine / Verified Boot | BSD 类及多开源许可 | 消费产品高 | Adopt（设计） | tries、successful、A/B、逐级验证、恢复 | [设计文档](https://www.chromium.org/chromium-os/chromiumos-design-docs/filesystem-autoupdate/) |
| Btrfs | GPL-2.0（内核） | 生产高 | Adopt | state、home、agent checkpoint、配额与 send | [官方文档](https://btrfs.readthedocs.io/) |
| OpenZFS | CDDL-1.0 | 生产高 | Watch | 高级工作站/服务器存储选项；不作默认根 | [官方文档](https://openzfs.github.io/openzfs-docs/) |

### 15.2 桌面权限、内核隔离与强沙箱

| 项目/机制 | 主要许可证 | 成熟度 | 雷达 | Andromeda 用途/判断 | 官方链接 |
|---|---|---:|---|---|---|
| Flatpak | LGPL-2.1+ | 桌面生产高 | Adopt | 普通 GUI 应用打包与静态沙箱 | [官方文档](https://docs.flatpak.org/) |
| bubblewrap | LGPL-2.0+ | 生产高 | Adopt | L1 mount/user/PID 等 namespace 组装 | [官方仓库](https://github.com/containers/bubblewrap) |
| xdg-desktop-portal | LGPL-2.1+ | 桌面生产高 | Adopt | 文件、屏幕、相机、USB 等动态 capability UX | [官方文档](https://flatpak.github.io/xdg-desktop-portal/docs/) |
| SELinux | GPL-2.0 等 | 生产高 | Adopt | 系统服务、数据标签和强制访问控制基线 | [Fedora 文档](https://docs.fedoraproject.org/en-US/quick-docs/selinux-getting-started/) |
| AppArmor | GPL-2.0 等 | 生产高 | Watch | 路径策略备选和兼容参考 | [Ubuntu 文档](https://documentation.ubuntu.com/security/security-features/privilege-restriction/apparmor/) |
| seccomp/libseccomp | GPL-2.0 内核 / LGPL-2.1 库 | 生产高 | Adopt | syscall 面收缩；不能单独承担资源授权 | [内核文档](https://www.kernel.org/doc/html/latest/userspace-api/seccomp_filter.html) |
| Landlock | GPL-2.0（内核） | 中高、能力持续扩展 | Adopt | 非特权 task 级文件/网络/IPC 收紧 | [内核文档](https://cdn.kernel.org/doc/html/latest/security/landlock.html) |
| systemd service sandbox | LGPL-2.1+ | 生产高 | Adopt | 约束长期 broker、updater、model service | [systemd.exec](https://www.freedesktop.org/software/systemd/man/latest/systemd.exec.html) |
| Qubes OS | GPL 等混合 | 安全桌面生产中高 | Adopt（设计） | 安全域、可信 GUI、USB/网络拆分；不直接作为基座 | [架构](https://doc.qubes-os.org/en/latest/developer/system/architecture.html) |
| Firecracker | Apache-2.0 | 云生产高 | Pilot | 不可信任务 microVM；验证桌面/ARM/设备限制 | [官方仓库](https://github.com/firecracker-microvm/firecracker) |
| Cloud Hypervisor | Apache-2.0/BSD-3-Clause | 云生产中高 | Pilot | x86_64/AArch64、较通用 virtio microVM 候选 | [官方仓库](https://github.com/cloud-hypervisor/cloud-hypervisor) |
| Kata Containers | Apache-2.0 | 云生产高 | Watch | OCI 到 VM runtime 可复用，但桌面依赖栈偏重 | [官方仓库](https://github.com/kata-containers/kata-containers) |
| gVisor | Apache-2.0 | 云生产高 | Pilot | 比 VM 轻的第二内核边界；需测 syscall 兼容 | [官方文档](https://gvisor.dev/docs/) |

### 15.3 AI agent、协议与编排

| 项目 | 主要许可证/形态 | 成熟度 | 雷达 | Andromeda 用途/判断 | 官方链接 |
|---|---|---:|---|---|---|
| Codex 权限/沙箱范式 | 闭源产品/公开文档 | 产品高 | Adopt（范式） | sandbox 与 approval 分离、网络/身份策略、证据与审计 | [OpenAI 安全部署说明](https://openai.com/index/running-codex-safely/) |
| Claude Code 权限范式 | 商业产品 | 产品高 | Adopt（范式） | plan/allow/deny、企业策略、hook、MCP 信任 | [安全](https://docs.anthropic.com/en/docs/claude-code/security)、[IAM](https://docs.anthropic.com/en/docs/claude-code/iam) |
| Model Context Protocol | 开放规范；SDK 许可各异 | 快速成为标准 | Adopt（传输） | tool/resource 互操作；明确不作为信任边界 | [MCP 文档](https://modelcontextprotocol.io/) |
| OpenHands core | MIT 为主；Enterprise source-available | 中高 | Pilot | agent/runtime 分离、Docker sandbox、事件流 | [官方文档](https://docs.openhands.dev/) |
| Aider | Apache-2.0 | 高 | Adopt（工作流参考） | Git diff/commit、lint/test、architect/editor | [官方仓库](https://github.com/aider-ai/aider) |
| Continue | Apache-2.0 | 中高 | Pilot | allow/ask/exclude、IDE/CLI、模型与工具配置 | [官方文档](https://docs.continue.dev/) |
| LangGraph | MIT | 中高 | Pilot | durable graph、checkpoint、interrupt、HITL | [官方文档](https://docs.langchain.com/oss/python/langgraph/overview) |
| AutoGen | 代码许可与文档许可分离，需按文件核查 | 中高、方向整合中 | Watch | agent runtime/消息模型；等待与 Agent Framework 收敛 | [官方仓库](https://github.com/microsoft/autogen) |
| Semantic Kernel | MIT | 生产中高、方向整合中 | Watch | 企业 plugin、process、.NET/Java/Python 参考 | [官方文档](https://learn.microsoft.com/en-us/semantic-kernel/) |
| Microsoft Agent Framework | MIT/开源生态，较新 | 新、快速演进 | Watch | 观察 AutoGen/SK 统一后的 workflow/state API | [官方文档](https://learn.microsoft.com/en-us/agent-framework/overview/) |

### 15.4 本地模型运行时

| 项目 | 主要许可证 | 成熟度 | 雷达 | Andromeda 用途/判断 | 官方链接 |
|---|---|---:|---|---|---|
| llama.cpp | MIT | 生产中高 | Adopt | 广硬件 LLM 推理、GGUF、量化、CPU/GPU 混合 | [官方仓库](https://github.com/ggml-org/llama.cpp) |
| Ollama | MIT | 产品中高 | Pilot | 开发期模型管理/API；评估产品级 daemon、安全与更新 | [官方文档](https://docs.ollama.com/) |
| ONNX Runtime | MIT | 生产高 | Adopt | OCR、embedding、分类、小模型与多 NPU/GPU EP | [官方文档](https://onnxruntime.ai/docs/) |
| ONNX Runtime GenAI | MIT | 中、仍演进 | Pilot | 统一生成 API 与硬件 EP | [GenAI 文档](https://onnxruntime.ai/docs/genai/) |
| MLC LLM / WebLLM | Apache-2.0 | 中高 | Watch | WebGPU、WASM、移动端与浏览器本地推理 | [官方文档](https://llm.mlc.ai/) |

### 15.5 明确拒绝的安全反模式

| 反模式 | 雷达 | 原因 |
|---|---|---|
| agent 默认继承登录用户全部文件、网络和凭据 | Reject | 任意一次提示注入即可获得最大爆炸半径 |
| 将 Docker 容器等同于强隔离 VM | Reject | 共享宿主内核，mount 与 daemon 配置错误可突破边界 |
| 依据模型或 MCP 自报的“安全/只读”自动授权 | Reject | 非确定性或不可信元数据不能成为安全边界 |
| 让模型直接拼接 shell、SQL、URL 或系统路径 | Reject | 注入、参数混淆、路径穿越和数据外泄难以可靠约束 |
| 所有操作只靠“执行前弹窗” | Reject | 造成审批疲劳，用户无法理解组合权限与隐性数据流 |
| 用单一 Btrfs/ZFS snapshot 代替应用一致性与 OS deployment | Reject | snapshot 不处理 schema、外部副作用、引导器和驱动组合 |
| 以普通目录暴露旧 OS/恢复对象 | Reject | 重演 `Windows.old` 的误删、依赖不明和磁盘占满 |
| 为“所有硬件”允许任意未签名内核模块静默加载 | Reject | 破坏 Secure Boot、更新可复现性和回滚保证 |
