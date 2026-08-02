# Andromeda 开源组件采用矩阵

> 状态：Draft 0.1
>
> 日期：2026-07-26
>
> 本文综合四个专题的项目级研究，形成产品 BOM 的第一版决策。详细依据见同目录专题文档；许可证结论仅作工程初筛，发布前必须由法律与合规流程逐版本复核。

## 1. 决策语言

| 结论 | 含义 |
|---|---|
| Adopt | 进入默认架构；建立内部维护、回归与上游贡献能力 |
| Pilot | 限定硬件、应用或用户群试点；不形成普遍承诺 |
| Watch | 保留接口或持续跟踪；当前不进入关键路径 |
| Reference | 只吸收架构/交互思想，不绑定其实现 |
| Reject | 不采用该项目或该用法作为产品依赖 |

“Adopt”不等于直接复制代码。根据项目边界，可采用以下方式：

- 直接依赖上游发行包；
- 与上游合作并贡献补丁；
- 使用稳定协议/API；
- 复用架构但自行实现产品控制面；
- 将组件放在隔离运行时中而非特权宿主。

## 2. 推荐基础 BOM

### 2.1 内核、系统与更新

| 能力 | 首选 | 决策 | 采用方式 |
|---|---|---|---|
| 内核与驱动 | Linux LTS/稳定分支 | Adopt | 尽量接近上游；私有补丁设预算 |
| 软件生态 | Fedora/RHEL 系 | Adopt | 复用 RPM、SELinux、硬件和桌面生态 |
| OS 构建/交付 | bootc + OCI | Adopt/Pilot | 长期交付格式；原型期补齐可信启动链 |
| deployment | OSTree | Adopt | 内容寻址、多部署、回滚 |
| 近期桌面部署工具 | rpm-ostree | Adopt（过渡） | 复用成熟能力；产品 API 不与其独有接口绑定 |
| 镜像构建 | osbuild/image-builder | Adopt | 生成安装 ISO、裸盘与 VM 镜像；与 §8 一致，已用于当前 ISO 构建 |
| 启动/服务 | systemd、UKI、systemd-boot/stub | Adopt | boot counting、measured boot、资源管理 |
| 状态文件系统 | Btrfs | Pilot→Adopt | 用户/应用/agent checkpoint；不替代 OS deployment |
| 固件更新 | fwupd + LVFS | Adopt | 按设备身份、HCM 和灰度通道门控 |
| 安全启动 | shim/UKI/Secure Boot/TPM2 | Adopt | 建立自有签名、吊销与恢复流程 |

基础系统决策依据见：

- [可靠更新、隔离与 AI Agent](./reliability-update-ai-agent.md)
- [硬件、驱动与迁移](./hardware-drivers-and-migration.md)

### 2.2 桌面硬件与媒体

| 能力 | 首选 | 决策 | 采用方式 |
|---|---|---|---|
| 图形 API/开源 GPU | Mesa（RADV/ANV/NVK/Asahi） | Adopt | 与内核 DRM、Vulkan、硬件 CI 锁步 |
| NVIDIA 现实游戏路径 | NVIDIA open kernel modules + 官方用户态 | Adopt（受控渠道） | 签名模块、版本锁步、独立回滚 |
| 显示协议 | Wayland + Xwayland | Adopt | v1 不提供传统 X11 桌面会话 |
| 音视频 | PipeWire + WirePlumber | Adopt | portal 化摄像头、麦克风、屏幕共享 |
| 输入 | libinput + xkbcommon | Adopt | 机型 quirks 进入 HCM 测试 |
| 摄像头 | libcamera | Adopt 框架/Pilot 设备 | 按传感器、ISP 和调校认证 |
| 蓝牙 | BlueZ | Adopt | 睡眠重连与共存回归 |
| 指纹 | libfprint/fprintd | Pilot | 只对设备 ID 与完整安全流程承诺 |
| Thunderbolt | bolt + IOMMU | Adopt | 用户授权、DMA 防护、坞站矩阵 |
| 打印 | CUPS/OpenPrinting/PAPPL | Adopt | 优先 IPP Everywhere/driverless |
| 扫描 | SANE | Adopt 框架/Pilot 设备 | 按设备列表认证 |

### 2.3 应用与隔离

| 能力 | 首选 | 决策 | 采用方式 |
|---|---|---|---|
| Linux GUI 应用 | Flatpak | Adopt | 默认沙箱和版本化分发 |
| 资源授权 | xdg-desktop-portal | Adopt | 扩展为 Andromeda typed portal/capability |
| 进程沙箱 | bubblewrap + namespaces/cgroups | Adopt | 普通应用与低风险 agent 任务 |
| 系统 MAC | SELinux | Adopt | 发行版静态强制访问基线 |
| 动态文件/网络收紧 | Landlock | Adopt | 任务级只能收紧的限制 |
| syscall 过滤 | seccomp | Adopt | 与 LSM、namespace 组合 |
| 强隔离 | KVM + QEMU | Adopt | Windows Workspace 与高风险任务 |
| microVM VMM | Cloud Hypervisor/Firecracker | Pilot | 实测桌面、arm64 与 GPU 后决定 |
| OCI→VM 编排 | Kata Containers | Watch/Pilot | 复用编排思想，避免过重依赖栈 |
| 用户态内核沙箱 | gVisor | Watch | syscall 兼容性不足时不作默认 |
| 安全域 UX | Qubes OS | Reference | 吸收分域、设备域、GUI 标识 |

## 3. Windows 游戏采用矩阵

详细依据：[Windows 游戏、Office 与文件格式兼容](./windows-gaming-office-formats.md)。

### 3.1 默认采用

| 项目 | 决策 | 角色 |
|---|---|---|
| Wine | Adopt | Win32/Win64 API 兼容 |
| Proton | Adopt | Steam Windows 游戏整合 |
| DXVK | Adopt | D3D8–11 → Vulkan |
| vkd3d-proton | Adopt | D3D12/DXR → Vulkan |
| Mesa | Adopt | AMD/Intel/NVIDIA/Apple 开源 Vulkan/OpenGL |
| Gamescope | Adopt | 游戏微合成器、缩放、帧率、HDR 会话 |
| Steam Runtime 思路/上游 | Adopt | 稳定用户态 ABI，与宿主驱动白名单桥接 |
| FAudio | Adopt | XAudio/X3DAudio 兼容 |
| SDL/Linux input | Adopt | 控制器和输入 |

组件以“内容寻址运行时套件”发布，不能让用户任意混配：

```text
game profile
  → Proton/Wine version
  → DXVK/vkd3d version
  → runtime image
  → host GPU driver minimum
  → known anti-cheat/DRM status
  → tested settings and evidence
```

### 3.2 限定试点

| 项目 | 决策 | 角色/边界 |
|---|---|---|
| umu-launcher | Pilot | 非 Steam Proton 启动 |
| Bottles/Lutris/Heroic | Pilot/Reference | 前缀与商店 UX；不让多个管理器争夺同一状态 |
| FEX | Pilot 首选 | arm64 上运行 x86/x86-64 Linux/Wine |
| Box64/Box32 | Pilot/Watch | ARM/RISC-V 等后备翻译 |
| MangoHud | Pilot | 本地性能诊断，不默认上传 |
| Monado/OpenComposite | Watch/Pilot | OpenXR 与部分 VR |
| libratbag/Piper/OpenRGB/OpenRazer/Oversteer | Pilot | 按具体外设与固件认证 |
| Mod Organizer 2/BepInEx/SteamTinkerLaunch | Pilot | 只读本体 + 可回滚 mod 层；禁止进入反作弊会话 |

### 3.3 不可由开源兼容栈替代

以下路由到原生 Windows 启动项、独立 Windows PC，或明确不支持：

- PC Game Pass、Xbox App、Microsoft Store/UWP；
- 发行商未启用 Proton 的 EAC/BattlEye；
- 要求 Windows 内核、受保护启动链或禁止 VM 的反作弊；
- 部分 DRM、设备指纹和受保护媒体；
- 专有 VR runtime、低延迟 direct mode 和追踪；
- 只有 Windows 工具支持的外设完整配置与固件；
- Apple silicon 上无法满足性能/硬件条件的游戏。

VM 不应被描述为上述问题的通用答案。

## 4. Office 与文件平台采用矩阵

### 4.1 Office

| 项目/服务 | 决策 | 角色 |
|---|---|---|
| LibreOffice | Adopt | ODF、旧格式、离线办公、批处理与打印 |
| ONLYOFFICE Desktop | Pilot | OOXML 高保真候选；评估 AGPL/商业许可 |
| Collabora Online | Pilot | 自托管协作；遵守 MPL/LGPL 与 COOLWSD 许可 |
| Microsoft 365 Web | Integrate | 用户授权的官方云端兼容路径 |
| Windows Office in Workspace | Adopt as fallback | VBA/COM/Power Query/Access/复杂企业工作流 |
| FreeRDP RemoteApp | Adopt | 把授权 Windows 应用作为受控单窗口呈现 |
| WinApps | Reject direct reuse | 遗留代码许可不清；只研究交互 |
| Cassowary | Reference | 维护不足，只研究 VM 休眠/文件打开 UX |

Andromeda 自行实现 Windows Application Bridge，底层采用 KVM/QEMU、libvirt、FreeRDP、专用文件 portal 和 VM 快照。

### 4.2 格式识别、预览和转换

| 类别 | 首选项目 | 决策 |
|---|---|---|
| 类型识别/MIME | libmagic、shared-mime-info、Apache Tika | Adopt |
| PDF | Poppler、MuPDF、qpdf、Ghostscript | Adopt/Pilot，解析必须沙箱 |
| 音视频 | FFmpeg、GStreamer | Adopt，按编译选项和地区审计 codec |
| 图像 | libvips、ImageMagick、OpenImageIO | Adopt/Pilot，解析沙箱与资源配额 |
| 归档 | libarchive、7-Zip | Adopt，路径/数量/嵌套/空间限制 |
| 文本/标记 | Pandoc | Pilot，内容转换不代表版式无损 |
| 电子书 | Calibre | Pilot，明确不处理 DRM |
| OCR | Tesseract、OCRmyPDF | Pilot |
| 字体 | HarfBuzz、FreeType、fontconfig | Adopt |
| 色彩 | Little CMS、OpenColorIO | Adopt/Pilot |

任何解析器崩溃不能获得宿主完整文件系统。来自网页、邮件、压缩包和未知 USB 的文件默认在强隔离环境生成派生预览。

### 4.3 原生创作应用

| 项目 | 决策 | 边界 |
|---|---|---|
| Blender | Adopt | 不承诺 Maya/3ds Max 插件与工程无损 |
| GIMP/Krita | Adopt | 不承诺 Photoshop 全部 PSD/插件语义 |
| Inkscape | Adopt | SVG 强项，不承诺 Illustrator 完整往返 |
| Scribus | Adopt | DTP/PDF，不替代 InDesign 工程生态 |
| FreeCAD | Pilot | 开放 CAD 交换，不替代 SolidWorks/AutoCAD |
| darktable/RawTherapee | Pilot | RAW 工作流，不替代 Lightroom 云生态 |

## 5. PC 与 Mac 硬件采用矩阵

详细依据：[硬件、驱动与迁移](./hardware-drivers-and-migration.md)。

### 5.1 平台路线

| 平台 | 决策 | 可交付边界 |
|---|---|---|
| 认证 x86-64 Intel/AMD PC | Adopt/首发 | Certified/Supported |
| 未认证通用 x86-64 PC | Community | Live 探测；不承诺整机体验 |
| 不带 T2 的选定 Intel Mac | Pilot→Supported | 逐机型白名单 |
| 带 T2 的 Intel Mac | Experimental | 依赖 apple-bce；不承诺休眠/音频/SEP |
| 选定 M1/M2 Mac | Developer Preview | 复用 Asahi；硬件缺口公开 |
| M3 及更新 Apple silicon | Watch/Unsupported | M3/M4 等待官方安装器与核心硬件；尚未列入 Asahi 功能表的更新代际（如 M5）不进入安装范围 |
| 标准 UEFI+ACPI arm64 PC | Pilot | 不外推到任意手机/开发板 |

### 5.2 Mac 项目

| 项目 | 决策 | 角色 |
|---|---|---|
| Asahi Linux 上游 | Adopt/Collaborate | Apple silicon 硬件使能与 Mesa 驱动 |
| m1n1 | Adopt for Apple silicon | Apple 启动 ABI 桥 |
| U-Boot/Device Tree | Adopt | Apple silicon Linux 启动链的一部分 |
| FEX + 4 KiB game VM | Pilot | M1/M2 x86 游戏路径 |
| t2linux/apple-bce | Watch/Experimental | T2 Mac 限定镜像 |
| ndiswrapper | Reject | 旧 NDIS 路线不完整，不能作为产品依赖 |
| Boot Camp 二进制驱动复用 | Reject | Windows ABI、授权与安全均不成立 |

驱动缺口依次使用：

1. 上游 Linux 原生驱动；
2. 有公开协议的用户态实现；
3. 经认证的 USB/PCI 直通 Windows VM；
4. OEM/厂商正式合作。

## 6. 迁移采用矩阵

| 项目/能力 | 决策 | 用途 |
|---|---|---|
| Windows/macOS Source Agent | Build | 在源系统按官方 API 只读盘点与导出 |
| osquery | Pilot | 跨平台硬件/软件 inventory 的部分基础 |
| libguestfs/virt-v2v/virt-p2v | Pilot | 用户选择保留现有 Windows 环境时 P2V |
| Clonezilla/Rescuezilla | Integrate/Reference | 迁移前整盘恢复点，不作为语义迁移 |
| NTFS3 | Adopt | NTFS 读取；写入仍受休眠/BitLocker/一致性约束 |
| APFS Linux 写入 | Reject as dependency | 正式迁移不依赖实验写支持 |
| Keychain/Credential Manager 官方 API | Integrate | 用户逐项授权；不能迁移硬件绑定秘密 |

无缝迁移的定义：

- 文件、账户、设置、应用清单和工作流连续；
- 不复制 Windows/macOS 内核驱动或应用机器码并假装可运行；
- 不迁移生物特征模板、设备身份、活跃 cookie、不可导出密钥；
- 不静默改写 APFS/BitLocker/Recovery；
- 每项迁移有 manifest、哈希、来源、结果和回滚。

## 7. AI、隔离与本地模型采用矩阵

### 7.1 OS 安全控制面

| 能力 | 决策 | 实现 |
|---|---|---|
| Capability Broker | Build | OS 核心；模型不能签发权限 |
| Typed Tool Broker | Build | MCP/native tool 适配、参数校验、来源和风险 |
| Transaction Manager | Build | 文件、设置、包、deployment、外部补偿 |
| Verifier | Build | 确定性测试优先，独立验证 |
| Audit Ledger | Build | 隐私感知、append-only、可导出 |
| Credential Broker | Build | 代用短期凭据，不向模型暴露秘密 |
| MCP | Adopt as protocol | 互操作层，不是信任边界 |

### 7.2 Agent 项目

| 项目 | 决策 | 可复用价值 |
|---|---|---|
| Codex/Claude Code 范式 | Reference | 计划、工具、权限、证据、恢复 |
| OpenHands | Pilot/Reference | runtime、事件流、sandbox API |
| Aider | Reference | Git diff/test/commit 小循环 |
| Continue | Reference | 工具权限和模型可组合 UX |
| LangGraph | Pilot | durable workflow、checkpoint、HITL |
| AutoGen | Watch | 多 agent 消息/runtime |
| Semantic Kernel | Watch | 企业 plugin/process |
| Microsoft Agent Framework | Watch | 新统一方向，接口仍演进 |

这些框架均不得拥有 ambient root，也不能取代 capability broker、LSM、microVM、凭据代理和审计。

### 7.3 本地模型

| 项目 | 决策 | 用途 |
|---|---|---|
| llama.cpp | Adopt | 广硬件 LLM 推理、量化、CPU/GPU 混合 |
| ONNX Runtime | Adopt | OCR、embedding、分类、小模型、多 EP |
| Ollama | Pilot | 开发期模型管理/API；评估产品 daemon |
| ONNX Runtime GenAI | Pilot | 统一生成 API |
| MLC LLM/WebLLM | Watch | WebGPU/WASM/移动实验 |

NPU 是可选加速器，不是“AI OS”成立的前提。Intel `ivpu`/OpenVINO 与 AMD `amdxdna`/XRT 先进入 Pilot；Apple ANE 保持 Watch。

## 8. 桌面平台决策

| 项目/能力 | 决策 | 用途与边界 |
|---|---|---|
| Wayland + Xwayland | Adopt | 唯一原生桌面协议；Xwayland 承载旧应用和游戏工具 |
| KDE Plasma 6 + KWin | Adopt for v1 | 锁定发行版维护分支；通过公开插件、脚本和 D-Bus 扩展，默认零永久私有补丁 |
| GNOME/Mutter | Watch / Reference | 成熟对照组；不承担 v1 深度 shell 定制 |
| COSMIC + Smithay | Pilot | 每季度复测 Rust 桌面、无障碍、门户和多 GPU；不进入 v1 关键路径 |
| wlroots | Watch | 框架成熟，但不能替代完整桌面产品工程 |
| Weston/libweston | Pilot for CI | 嵌套、headless、DRM 与 Wayland 协议对照测试 |
| xdg-desktop-portal + Polkit + Secret Service | Adopt | 分别处理资源授权、特权策略和凭据；不能互相替代 |
| AT-SPI 2 + libei/RemoteDesktop portal | Adopt/Pilot | 语义自动化优先；合成输入仅在用户授权会话内使用 |
| PipeWire + WirePlumber | Adopt | 音视频、屏幕共享、远程桌面和 AI 媒体策略 |
| Fcitx 5 | Adopt | 中文和多语言默认输入法；GTK/Qt/Xwayland/Flatpak 独立回归 |
| osbuild + 统一 image-builder | Adopt | raw/qcow2/ISO/安装介质构建与 SBOM/provenance |
| bootc-image-builder 新依赖 | Reject | 已进入向统一 image-builder 迁移，不绑定废弃接口 |
| Anaconda 引擎 | Adopt short-term | 首个安装预览复用其存储能力；Andromeda Preflight 保持独立 |
| KDE Discover | Adopt for v1 UI | 只启用验证过的 Flatpak 后端；系统与固件更新走独立控制面 |

AI Task Center 采用“深度集成、进程与接口解耦”：`andromeda-taskd`、Capability Broker、事务服务和验证器不进入 KWin 进程；Plasma 只提供 KRunner、Plasmoid、Kirigami、KWin Script/Effect 等轻量 adapter。自研 compositor 只有在至少两个产品版本后，出现三个以上无法通过标准协议、KWin 公共 API 或上游合作解决的核心需求，并且图形团队与硬件实验室具备多年维护能力时才重新立项。

## 9. 明确拒绝的组合

| 组合 | 原因 |
|---|---|
| 从零内核 + 首版广泛硬件支持 | 目标直接冲突 |
| 手工包管理修改当前 root | 无法提供已测试整体和可靠回滚 |
| Btrfs/ZFS 快照单独承担 OS 更新 | 不覆盖 boot、驱动、schema 与外部副作用 |
| agent 默认继承登录用户权限 | 提示注入爆炸半径不可接受 |
| Docker 等同于强安全 VM | 共享宿主内核 |
| MCP 自报“只读”即自动授权 | 不可信元数据不能成为安全边界 |
| Windows/macOS 驱动直接加载到 Linux | ABI、对象模型、签名与许可不兼容 |
| Wine/VM 宣称解决全部反作弊 | 技术和发行商政策均不成立 |
| 开源编辑器宣称完全替代 Office | 宏、插件、字体、企业服务语义不成立 |
| 非 Apple 硬件产品化 macOS | 技术、授权和支持风险 |
| APFS 实验写入作为正式迁移路径 | 数据完整性风险 |
| 用户直接删除旧 OS deployment | 重演 `Windows.old` 事故 |

## 10. 许可证与供应链门

每次进入镜像的构建必须产生：

- 精确源码与二进制版本；
- commit/tag 与内容 hash；
- SPDX SBOM；
- 许可证、notice、源码提供义务；
- patent/codec/字体/固件限制；
- 是否允许修改、再分发、商业使用与网络服务；
- 构建 provenance 与签名；
- CVE/安全公告来源；
- 上游维护和应急回滚责任人。

特别审查：

- Proton、Mesa、FFmpeg、GStreamer、7-Zip 等多组件/多许可证组合；
- linux-firmware 的逐文件许可；
- NVIDIA 用户态、Microsoft/Apple 字体、Boot Camp、codec、DRM CDM；
- ONLYOFFICE/Collabora 的 AGPL/MPL/LGPL 与商用条款；
- WinApps 遗留代码许可；
- 模型权重、tokenizer、训练数据声明；
- MCP/plugin 的依赖和自动更新。

## 11. 需要原型裁决的项目

1. bootc + OSTree 与固定 A/B 在低容量 SSD 上的峰值空间；
2. Btrfs state checkpoint 与数据库/application quiesce；
3. Plasma/KWin 基线的多 GPU、HDR/VRR、输入法、无障碍、屏幕共享、Xwayland 和 adapter contract test；COSMIC/Smithay 作为季度对照 Pilot；
4. Firecracker、Cloud Hypervisor 与普通 KVM VM 的桌面成本；
5. Windows RemoteApp 在 Office、剪贴板、打印和睡眠下的稳定性；
6. Wine/Proton runtime 版本组合与自动回滚；
7. LibreOffice 与 ONLYOFFICE 的真实 OOXML 视觉/结构基准；
8. M1/M2 的 FEX + 4 KiB VM + Proton 游戏体验；
9. NVIDIA 驱动、Secure Boot 与 image deployment 的锁步更新；
10. Windows/macOS Source Agent 的权限、断点续传和数据校验；
11. portal permission store 对任务期、一次性和数据流权限的表达；
12. 本地模型在最低硬件上的延迟、功耗和安全分类准确率。

所有 Pilot 项目都必须有：

- 明确目标硬件/应用/版本；
- 可自动运行的验收测试；
- 用户可见的限制；
- 失败回退路径；
- 退出或升级为 Adopt 的日期与条件。
