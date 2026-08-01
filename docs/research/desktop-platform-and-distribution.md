# Andromeda 桌面平台与发行工程开源项目深度研究

> 调研日期：2026-07-26
>
> 范围：Linux 桌面显示栈、桌面环境、通用桌面服务、远程桌面、安装器、镜像构建、启动与更新、应用商店，以及 AI Task Center 的系统级集成边界。
>
> 目标：为 Andromeda v1 给出可执行的 Adopt / Pilot / Watch / Reject 决策，而不是简单罗列项目。
> 许可证说明：本文记录的是上游主许可证或主要组件许可证，用于技术选型初筛，不构成法律意见；正式发布前仍需基于锁定的源码提交生成逐文件 SPDX/SBOM 并由法律人员复核。

## 1. 执行摘要

### 1.1 结论

1. **Andromeda v1 应采用 Wayland + Xwayland，并完整复用 KDE Plasma 6 / KWin，而不是自研 compositor。**
   KWin/Plasma 已经把多 GPU、NVIDIA、分数缩放、HDR/色彩管理、VRR、绘图板、屏幕共享、输入法、无障碍和 Xwayland 放进同一个持续维护的产品面。Plasma 同时提供 QML/Kirigami、Plasmoid、KRunner、KWin Script、D-Bus 和系统设置扩展点，足以先把 Andromeda 的差异化放在 Task Center、权限、更新与迁移上。KDE 官方的 Plasma 6.3/6.4 发布说明已经显示其在分数缩放、ICC/HDR、平板和无障碍上的持续成熟（[Plasma 6.3](https://kde.org/announcements/plasma/6/6.3.0/)、[Plasma 6.4](https://kde.org/announcements/plasma/6/6.4.0/)）。
2. **GNOME 是成熟备选，但不适合作为 Andromeda v1 的深度定制底座。**
   Mutter/GNOME Shell 的可靠性、无障碍和远程桌面能力很强；GNOME 50 已默认改进 VRR、分数缩放、NVIDIA 路径、HDR 屏幕共享和硬件加速远程桌面（[GNOME 50](https://release.gnome.org/50/)）。问题是 Andromeda 若要改变 Shell 的核心工作流，容易走向 GNOME Shell 扩展兼容追逐或长期 fork。
3. **COSMIC/Smithay 是最值得长期 Pilot 的 Rust 路线，但不应替代 v1 主桌面。**
   COSMIC Epoch 1 于 2025-12-11 发布，证明 Rust + Smithay + iced/libcosmic 可以组成完整桌面（[System76 更新](https://blog.system76.com/blog/post/cosmic-epoch-1-updates)）。但它只有一个大版本的生产历史，`libcosmic` 的无障碍特性仍明确标为 experimental，桌面和门户仍在快速补齐。它适合做并行实验和上游合作，不适合让 v1 同时承担新 OS、新 AI 权限模型和新 compositor 三重风险。
   *以上 Plasma 6.3/6.4、GNOME 50 与 COSMIC Epoch 1 的版本与能力描述均为调研日（2026-07-26）上游状态快照；进入 Adopt/Pilot 前须按当日上游状态复核。*
4. **AI Task Center 必须“深度集成，但进程和接口解耦”。**
   深度集成不等于把模型塞进 KWin，也不等于让 agent 读取 `/dev/input`、直接调用任意 D-Bus 或长期持有管理员权限。推荐由独立 `andromeda-taskd`、确定性的 capability broker、系统事务服务和 Task Center UI 组成；Plasma/KWin 只提供表面入口、窗口语境与呈现。窗口自动化优先语义工具，其次 AT-SPI，再次是经 RemoteDesktop portal 授权的 libei 输入，不使用全局隐形键鼠注入。
5. **系统发行采用 bootc/OCI + OSTree 后端能力，镜像构建采用统一 `image-builder`/osbuild。**
   bootc 的 CLI/API 已宣布稳定，提供基于 OCI 镜像的事务更新与回滚（[bootc 简介](https://bootc.dev/bootc/)、[更新与回滚](https://bootc.dev/bootc/upgrades.html)）。截至调研日，独立 `bootc-image-builder` 已合并进统一 `image-builder` 并进入弃用迁移，不应成为新的长期依赖（[迁移说明](https://osbuild.org/docs/bootc/deprecation-notice/)）。
6. **首个可安装开发者预览复用 Anaconda 的存储和安装能力，但隔离 Andromeda 自己的引导体验；同时 Pilot container-based `bootc-installer`。**
   Calamares 更容易换皮，但在 Fedora/bootc、Kickstart、复杂存储与 SELinux 交付链上的集成成本更高。长期也不应把产品 UI 锁死在 Anaconda，因为 Image Builder 已公开指向 container-based installer ISO 的迁移方向。
7. **应用、系统和固件更新必须是三个不同事务域。**
   - Flatpak：第三方 GUI 应用；
   - bootc：基础 OS、内核、关键驱动和桌面；
   - fwupd/LVFS：设备固件。
   PackageKit 可以作为传统包管理兼容层研究，但不能成为 Andromeda 基础系统更新引擎。
8. **“在现有 PC 和 Mac 上无缝切换”主要由内核/固件/认证决定，桌面栈不能补救缺失的内核驱动。**
   Mesa、libinput、PipeWire 等会把已存在的内核能力变成一致桌面体验；dracut 的通用 initramfs、可恢复启动、硬件预检和 HCM 认证才决定系统能否安全安装和迁移。Apple silicon 仍需独立的 Asahi 启动与硬件分支，不能套用 PC 的 UEFI/UKI 假设。

### 1.2 v1 推荐组合

| 层 | v1 决策 | 说明 |
| --- | --- | --- |
| 显示协议 | Wayland + Xwayland | 不提供 Xorg 桌面会话；保留大量旧应用和工具兼容 |
| 桌面环境 | KDE Plasma 6 + KWin | 锁定一个发行版维护分支；尽量零私有 KWin 补丁 |
| 图形/输入 | Mesa + libinput + xkbcommon | 闭源 NVIDIA 栈作为同一 OS 事务中的受控例外 |
| 音视频 | PipeWire + WirePlumber | 屏幕共享、远程桌面和 AI 媒体工具也走同一策略层 |
| 安全桥 | xdg-desktop-portal + Polkit + Secret Service | portal 管资源授予，Polkit 管特权授权，密钥服务管应用凭据；三者不可相互替代 |
| 应用 | Flatpak + AppStream | 不让第三方 GUI 修改基础系统 |
| 网络/外设 | NetworkManager、BlueZ、CUPS、SANE、fwupd | 通过稳定 D-Bus/IPP/API 接入 Task Center |
| 无障碍 | AT-SPI 2 | v1 发布门；同时作为受授权 UI 语义观察通道，而非隐形万能自动化接口 |
| 输入法 | Fcitx 5 默认，IBus 兼容测试 | 面向中文和多语言输入；每次桌面升级做 GTK/Qt/Xwayland/Flatpak 矩阵 |
| OS 构建 | osbuild + 统一 image-builder | 产出 raw/qcow2/ISO/云镜像；记录可重现清单与 SBOM |
| OS 更新 | bootc + OSTree 当前后端 | Andromeda 自己提供稳定 Update API，不把产品 API 绑死在后端命令输出 |
| 启动 | UKI + Secure Boot + boot counting；dracut | 适用于 UEFI PC/选定 Intel Mac；Apple silicon 走独立启动链 |
| 安装 | Anaconda 引擎（短中期）+ Andromeda Preflight | 同时 Pilot bootc-installer，避免长期深 fork Anaconda UI |
| 商店 | Discover 作为 v1 临时 UI，限定 Flatpak；自有 Update Center | 不通过 Discover/PackageKit 原地修改基础 OS |
| AI | 独立 Task Center + capability broker + Plasma adapters | 模型无直接系统权限；所有动作计划化、事务化、可验证、可撤销 |

## 2. 决策标准

### 2.1 状态定义

- **Adopt**：进入 v1 默认产品路径，并承担持续回归、打包和安全响应。
- **Pilot**：进入受控原型或开发者功能；有明确验证门，不对普通用户作完整承诺。
- **Watch**：持续跟踪或作为备选，不进入 v1 默认镜像。
- **Reject**：拒绝的是当前用途或集成方式，不代表项目本身质量差。

### 2.2 成熟度定义

- **高**：多年被主流发行版或产品默认使用，API/运维和故障路径已较充分暴露。
- **中高**：已有稳定用户和发行版采用，但仍存在快速演进或某些桌面边界缺口。
- **中**：可用并有真实用户，Andromeda 所需路径仍需大量集成或硬件验证。
- **早期**：适合验证概念，不宜作为 v1 的单点基础。

### 2.3 不把“能编译”当成产品成熟

每个 Adopt 项目都必须同时满足：

1. 能从 Andromeda 锁定的源码和依赖可重现构建；
2. 在 Intel、AMD、NVIDIA 和选定 Mac 硬件上通过相应测试；
3. 有明确崩溃隔离、日志、升级和回滚路径；
4. 不要求模型或 UI 绕过系统安全边界；
5. 许可证、商标、固件再分发和源码提供义务可履行；
6. 私有补丁有预算、有上游 issue/MR、有删除日期。

## 3. 显示协议、compositor 框架与桌面环境

### 3.1 Wayland 与 Xwayland

Wayland 是 compositor 与应用之间的协议及架构，不是一个可直接安装的统一显示服务器。每个桌面都要由 compositor 负责窗口、输入、输出和最终合成（[Wayland 官方说明](https://wayland.freedesktop.org/)、[架构](https://wayland.freedesktop.org/architecture.html)）。

Xwayland 是运行在 Wayland compositor 内的完整 X11 server。rootless 模式下，每个 X11 窗口可与原生 Wayland 窗口混排，但 compositor 还必须充当 X Window Manager；同一个 Xwayland 实例中的 X11 客户端通常不能互相隔离（[Xwayland 架构](https://wayland.freedesktop.org/docs/book/Xwayland.html)）。

| 项目 | 作用 | 许可证 | 成熟度 | 决策 | Andromeda 用法 |
| --- | --- | --- | --- | --- | --- |
| [Wayland / wayland-protocols](https://wayland.freedesktop.org/) | 显示协议与 IPC 库 | MIT/X11 | 高 | **Adopt** | v1 唯一原生桌面协议；只依赖 stable 协议，实验协议逐项开关 |
| [Xwayland](https://wayland.freedesktop.org/docs/book/Xwayland.html) | X11 应用兼容 server | MIT/X.Org 族，逐文件不同 | 高 | **Adopt** | 旧游戏启动器、Office 辅助工具、专业软件兼容；视为较弱隔离域 |

产品要求：

- Xwayland 崩溃只能影响 X11 应用，不得拖垮桌面会话；
- Task Center 必须标记任务目标是 Wayland 原生还是 Xwayland；
- 处理敏感数据的 agent 不因“同属 Xwayland”假设窗口间隔离；
- 不把全局 X11 键盘抓取能力重新带回 Wayland 原生应用。

### 3.2 compositor 框架

| 项目 | 技术定位 | 许可证 | 成熟度 | 决策 | 结论 |
| --- | --- | --- | --- | --- | --- |
| [wlroots](https://wlroots.pages.freedesktop.org/wlroots/) | C 语言、可组合的 Wayland compositor 基础模块 | MIT | 高（框架） | **Watch** | 功能覆盖和硬件后端强，但仍要自己实现完整桌面策略、shell、a11y、门户、设置和 QA |
| [Smithay](https://smithay.github.io/pages/about.html) | Rust compositor 框架，封装 DRM、GBM、libinput、Wayland 和 Xwayland | MIT | 中高（框架） | **Pilot** | 最适合未来 Rust 原生 compositor 研究；COSMIC 已证明路线可行 |
| [Weston/libweston](https://wayland.pages.freedesktop.org/weston/) | Wayland 参考 compositor、嵌入式/车载/kiosk 基础 | MIT | 高 | **Pilot**（测试） | 作为协议正确性、嵌套、headless、DRM CI 和故障对照；不作为通用消费桌面 |

#### wlroots 仓库状态必须正确记录

旧的 [GitHub `swaywm/wlroots`](https://github.com/swaywm/wlroots) 已在 2021-11-01 归档，只读，并明确指向 `gitlab.freedesktop.org/wlroots/wlroots`。当前上游是 freedesktop GitLab，API 文档位于 [wlroots.pages.freedesktop.org](https://wlroots.pages.freedesktop.org/wlroots/)。不得从旧 GitHub release 判断当前版本、活跃度或缺陷状态。

#### 为什么框架不等于桌面

选择 wlroots 或 Smithay 后，Andromeda 仍要自己负责：

- 窗口规则、焦点、防抢焦点、虚拟桌面、tiling 和多显示器语义；
- HDR、色彩管理、VRR、显式同步、GPU reset、多 GPU copy/offload；
- Xwayland XWM、IME text-input 协议、屏幕键盘；
- lock screen、login/session、通知、剪贴板、截图、屏幕共享和门户；
- 放大镜、屏幕阅读器、键盘导航、高对比度和 AT-SPI 集成；
- 远程桌面、headless session、休眠恢复；
- 每一个 NVIDIA/Intel/AMD/OEM 异常路径的回归。

wlroots 自称可省掉约六万行常见 compositor 代码，但这不包含一个可与 Windows/macOS 竞争的完整桌面产品。Smithay 文档同样明确，它主要提供低层系统/Wayland 抽象，不替项目决定窗口管理和绘制策略（[Smithay API](https://smithay.github.io/smithay/smithay/index.html)）。

### 3.3 成熟桌面候选

| 项目 | 架构与扩展方式 | 许可证 | 成熟度 | 决策 | 主要理由 |
| --- | --- | --- | --- | --- | --- |
| [KDE Plasma / KWin](https://develop.kde.org/docs/plasma/) | Qt/QML/Kirigami；Plasmoid、KRunner、KWin Script/Effect、D-Bus | GPL-2.0-or-later 为主，逐组件/文件不同 | 高 | **Adopt** | 定制面广、游戏/显示能力强、可先插件化 Task Center，不必改 compositor |
| [GNOME Shell / Mutter](https://gnome.pages.gitlab.gnome.org/gnome-shell/shell/) | GTK/libadwaita + GNOME Shell JS；Mutter 与 Shell 紧耦合 | Shell、Mutter 均 GPL-2.0-or-later | 高 | **Watch** | 一致性、a11y、远程桌面优秀；核心工作流深改容易进入 extension/fork 维护 |
| [COSMIC / cosmic-comp / libcosmic](https://github.com/pop-os/cosmic-epoch) | Rust；Smithay compositor；iced/libcosmic UI | cosmic-comp/shell 多为 GPL-3.0；libcosmic MPL-2.0 | 中 | **Pilot** | 架构方向与 Andromeda 接近，但生产历史和 a11y/门户成熟度不足 |

#### 为什么 v1 选择 Plasma/KWin

1. **Andromeda 需要的不是极简 shell，而是大量兼容入口。** Plasma 对窗口规则、显示设置、手写板、系统托盘、传统应用和桌面插件更宽容。
2. **AI 表面可以插件化。** KRunner 适合自然语言入口，Plasmoid 适合常驻任务状态，Kirigami 适合 Task Center 主应用，KWin scripting 可做有限窗口编排。KDE 官方提供 [KWin scripting API](https://develop.kde.org/docs/plasma/kwin/api/) 和 [脚本教程](https://develop.kde.org/docs/plasma/kwin/)。
3. **不需要先接管渲染关键路径。** Task Center 可以显示在 panel、overview、通知和独立窗口中，而执行、授权和审计仍在独立服务。
4. **对游戏和复杂显示更符合首发用户。** Plasma 已公开交付 HDR 校准、色彩深度、P010、分数缩放和绘图板改进。

选择 Plasma 不代表照搬所有 KDE 默认：

- v1 只暴露经产品验证的设置，专家选项放入“开发者模式”；
- 统一 Andromeda 视觉、快捷键和工作区预设，但不私改 KWin 内部 ABI；
- 删除重复的更新入口和包管理入口；
- 保持标准 Plasma session 可作为恢复后备；
- 所有品牌修改通过主题、配置、插件和独立应用实现。

#### GNOME 的适用位置

GNOME/Mutter 应保留为持续对照平台：

- 对比桌面“低系统熵”和无障碍体验；
- 参考 gnome-remote-desktop 的 RDP/headless session；
- 验证 Task Center 的核心服务是否真正桌面无关；
- 确保 Andromeda 工具不依赖 KDE 私有 UI 才能执行。

不建议 v1 采用 GNOME Shell 的原因不是功能不足，而是产品差异化将主要改变 Activities、任务流、窗口语境和系统权限呈现，这正落在 Shell 核心。若大量依赖 GNOME Shell extension，GNOME 每个大版本都可能形成兼容工作；若直接 fork，则同时承担 Mutter/Shell 紧耦合维护。

#### COSMIC 的适用位置

建议建立一个季度更新的 COSMIC Pilot：

- 在相同 Andromeda 基础镜像上运行 COSMIC session；
- 验证 `cosmic-comp` 的多 GPU、NVIDIA、HDR、屏幕共享、IME、a11y 和远程桌面；
- 用 libcosmic 实现一个只读 Task Center 客户端，验证 UI 层可替换；
- 对适合共用的 portal、AT-SPI、Smithay 或 Rust D-Bus 组件直接向上游贡献；
- 不 fork 整个 COSMIC epoch。

### 3.4 何时才允许自研 compositor

只有同时满足以下条件才从 Pilot 升为正式项目：

1. Task Center 的核心空间/窗口模型经过至少两个版本验证，并有至少三个无法通过标准 Wayland、KWin API、Plasma 插件或上游改进解决的关键需求；
2. 有不少于 6–8 名长期专职显示/compositor 工程师，另有独立 QA、无障碍和安全负责人；
3. 已有至少 100 个认证 PC/Mac 型号的自动化显示、输入、休眠、坞站和 GPU 回归；
4. 已能持续跑 Wayland/Xwayland、Mesa/Vulkan、PipeWire portal、IME 和 AT-SPI 互操作测试；
5. 有明确的协议上游策略，不依靠 Andromeda 私有 Wayland 协议锁住应用；
6. 可在 compositor 崩溃、GPU reset、Xwayland 崩溃和远程会话断开时恢复用户任务；
7. 自研带来的产品价值足以覆盖至少 3–5 年的维护成本。

届时优先评估 Smithay；若团队和上游需求更适合 C，再评估 wlroots。Weston 保持参考/嵌入式用途。**不要把长期维护的 KWin fork 当成自研 compositor 的捷径。**

## 4. 桌面基础设施项目矩阵

### 4.1 图形、输入与媒体

| 项目 | 作用 | 许可证 | 成熟度 | 决策 | 关键要求 |
| --- | --- | --- | --- | --- | --- |
| [Mesa](https://docs.mesa3d.org/) | OpenGL/Vulkan/EGL、开源 GPU 用户态驱动 | 核心 MIT，多组件逐文件不同 | 高 | **Adopt** | 与内核 DRM、固件、libdrm 锁步；游戏和桌面共用认证矩阵 |
| [libinput](https://wayland.freedesktop.org/libinput/doc/latest/) | 鼠标、键盘、触控板、触屏、绘图板等输入归一化 | MIT | 高 | **Adopt** | 维护机型 quirks；手柄仍走 evdev/SDL，不塞入 libinput |
| [libxkbcommon](https://xkbcommon.org/doc/current/) | XKB keymap、Compose、dead key | MIT/X11 族，逐文件不同 | 高 | **Adopt** | 与 xkeyboard-config、IME、远程键盘布局共同测试 |
| [PipeWire](https://docs.pipewire.org/) | 低延迟音视频图、屏幕流、兼容 PulseAudio/JACK | MIT 为主，少数文件例外 | 高 | **Adopt** | 所有 capture 都经 portal/策略；不让 agent 直连任意 node |
| [WirePlumber](https://pipewire.pages.freedesktop.org/wireplumber/) | PipeWire session/policy manager | MIT | 高 | **Adopt** | 路由策略、蓝牙 profile、隐私设备状态进入统一策略和审计 |

Mesa 核心采用 MIT，但官方明确提醒不同组件/文件可能有不同许可证（[Mesa 许可证](https://docs.mesa3d.org/license.html)）。构建系统必须从实际提交生成 SBOM，不能用表格里的“MIT”替代合规扫描。

PipeWire 提供机制，WirePlumber 决定设备、node 和 link 的策略；PipeWire 官方也明确把“哪些组件何时可以互联”的决定放在 session manager（[PipeWire Session Manager](https://docs.pipewire.org/page_session_manager.html)）。因此麦克风、摄像头、屏幕捕获和 AI 音频分析必须经过 portal 与 WirePlumber 策略，不能把 PipeWire socket 视为自然授权。

### 4.2 沙箱、权限和凭据

| 项目/规范 | 作用 | 许可证 | 成熟度 | 决策 | Andromeda 边界 |
| --- | --- | --- | --- | --- | --- |
| [xdg-desktop-portal](https://flatpak.github.io/xdg-desktop-portal/docs/) | 沙箱资源授予：文件、截图、屏幕、远程输入、打印等 | LGPL-2.1-or-later | 高 | **Adopt** | 应用和低权限 agent 的首选资源桥；使用 KDE backend |
| [Flatpak](https://docs.flatpak.org/en/latest/) | GUI 应用分发与 sandbox | LGPL-2.1 | 高 | **Adopt** | v1 第三方原生 GUI 默认格式；权限 diff 进入 Store/Task Center |
| [Polkit](https://polkit.pages.freedesktop.org/polkit/) | 系统服务的动作授权与认证 agent | LGPL-2.0-or-later | 高 | **Adopt** | 只对类型化 action 授权；不允许 `run arbitrary shell as root` |
| [Secret Service API](https://specifications.freedesktop.org/secret-service-spec/latest/) | 桌面应用凭据互操作 D-Bus API | 规范；实现许可证不同 | 中高 | **Adopt**（兼容） | 应用凭据兼容；AI 长期令牌仍经 Andromeda credential broker 和硬件绑定 |

Portal 是一个前端/后端分离的安全 API：前端负责校验、permission/document store，桌面 backend 提供与会话一致的 UI（[设计说明](https://flatpak.github.io/xdg-desktop-portal/)）。Andromeda 应先复用 `xdg-desktop-portal-kde`，只在出现 Andromeda 特有、无法上游的一类权限交互时新增小型 backend，不能复制整套 portal。

Portal、Polkit 和 AI capability 的关系：

```text
用户目标
  → AI capability（本任务可以做什么、对哪些对象、多久）
    → portal（向低权限进程安全交付文件/屏幕/输入等资源）
      → Polkit（系统服务执行明确的高权限 action）
        → 事务服务提交、验证、审计和回滚
```

Portal 的“记住选择”和 restore token 不能自动等同于 AI 永久授权。RemoteDesktop portal 已定义会话级和持久授权模式、可撤销 restore token 以及用户选择界面（[RemoteDesktop portal](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.RemoteDesktop.html)）；Andromeda 还要在上层标记“由哪个 agent、为哪个任务、为何使用”。

### 4.3 网络、蓝牙、打印和扫描

| 项目 | 作用 | 许可证 | 成熟度 | 决策 | 关键要求 |
| --- | --- | --- | --- | --- | --- |
| [NetworkManager](https://networkmanager.dev/docs/api/latest/) | 有线、Wi‑Fi、VPN、热点及网络状态 | daemon GPL-2.0-or-later；libnm LGPL-2.1-or-later | 高 | **Adopt** | Task Center 只调用稳定 D-Bus/libnm；连接修改使用 checkpoint/回滚 |
| [BlueZ](https://bluez.readthedocs.io/en/latest/) | Linux 蓝牙协议栈与 D-Bus API | GPL-2.0 系工具/daemon；LGPL-2.1 系库，逐文件 SPDX | 高 | **Adopt** | 音频交给 PipeWire/WirePlumber；配对和敏感 profile 必须可见授权 |
| [CUPS](https://openprinting.github.io/cups/) | IPP 打印系统 | Apache-2.0 + GPLv2-only linking exception | 高 | **Adopt** | 优先 IPP Everywhere/Printer Applications；旧 PPD/driver 明确为兼容路径 |
| [SANE](https://gitlab.com/sane-project/backends) | 扫描 API、backend 与网络协议 | frontend/backend 多为 GPL，带 SANE linking exception；API/协议 public domain | 中高 | **Adopt** | 按精确设备认证；不承诺所有扫描仪，旧 backend 放进受限服务 |

NetworkManager 提供稳定 system-bus D-Bus API，可查询和修改连接；其 API 还包含 checkpoint 对象（[D-Bus 总览](https://www.networkmanager.dev/docs/api/latest/spec.html)）。AI 改 VPN、DNS、热点或路由时，应先创建 checkpoint，验证联网和 DNS 后提交，失败自动恢复。

CUPS 已公开说明传统 printer driver、backend 和 PPD 将在未来 feature release 中不再支持；Andromeda 应把无驱动 IPP 打印作为默认，将旧打印机支持作为可移除兼容组件（[CUPS 说明](https://openprinting.github.io/cups/doc/man-cups.html)）。

### 4.4 无障碍与输入法

| 项目 | 作用 | 许可证 | 成熟度 | 决策 | 关键要求 |
| --- | --- | --- | --- | --- | --- |
| [AT-SPI 2 / at-spi2-core](https://gnome.pages.gitlab.gnome.org/at-spi2-core/libatspi/) | 桌面无障碍对象树与事件 API | LGPL-2.1-or-later | 高 | **Adopt** | 发布门；Task Center 自身必须完整可访问 |
| [IBus](https://github.com/ibus/ibus) | Linux/Unix 输入法框架 | LGPL-2.1，Unicode 数据另有许可 | 高 | **Adopt**（兼容） | 作为 GNOME/部分应用兼容矩阵，不与 Fcitx 同时争抢 active IM |
| [Fcitx 5](https://www.fcitx-im.org/wiki/Fcitx_5/en) | 模块化、多语言输入法框架 | 核心 LGPL-2.1-or-later，addons 逐项不同 | 高 | **Adopt**（默认） | 中文首发默认；验证 Qt/GTK、Wayland text-input、XIM、Flatpak |

AT-SPI 提供 accessible object、role、state、Action、Text、Selection、Table 等语义接口（[API](https://gnome.pages.gitlab.gnome.org/at-spi2-core/libatspi/)）。它对 AI 很有价值，但首要目的仍是无障碍：

- agent 读取 AT-SPI 必须绑定当前任务和应用范围；
- 密码框、私密通知、未授权窗口内容不得因 AT-SPI 被广泛收集；
- 能通过应用原生工具/API 完成时，不使用 UI 自动化；
- AT-SPI 数据不能替代操作后的业务验证。

Fcitx 5 的 Wayland 集成依赖 compositor 对 text-input 和虚拟键盘路径的支持；官方文档也指出 KWin Wayland 要把 Fcitx 作为特殊 virtual keyboard client 启动（[Fcitx 5 设置](https://fcitx-im.org/wiki/Setup_Fcitx_5)）。安装镜像必须为中日韩输入做独立验收，而不是只验证英文键盘。

## 5. AI Task Center：深度集成而不 fork 失控

### 5.1 目标架构

```text
Plasma Panel / KRunner / Global Shortcut / Notifications
                         │
                         ▼
             Andromeda Task Center (Qt/Kirigami)
                         │  typed D-Bus/Unix API
                         ▼
        andromeda-taskd（每用户任务状态与事件日志）
          │              │                 │
          │              │                 └─ verifier / undo ledger
          │              └─ planner + model adapters（无系统权限）
          └─ capability broker（确定性策略）
                         │
             ┌───────────┴────────────┐
             ▼                        ▼
    用户态 semantic tools      andromeda-system-broker
  portals/AT-SPI/libflatpak    Polkit + 类型化系统事务
  NM/BlueZ/CUPS/app APIs       bootc/fwupd/storage/service
```

### 5.2 Plasma 集成面

v1 只使用以下公开扩展点：

1. **Task Center 主应用**：Qt 6 + Kirigami/QML，展示计划、授权、执行、验证、日志、资源和撤销。
2. **Plasmoid**：只显示任务数、风险状态、进行中步骤和停止入口；不在 panel 进程执行 agent。
3. **KRunner runner**：解析“打开 Task Center”“继续任务”“停止任务”和自然语言输入，将请求交给 `taskd`。
4. **通知**：用于提交点、完成、失败和需要人类输入；通知按钮只调用稳定 task action。
5. **KWin Script/Effect adapter**：有限提供窗口定位、workspace 移动、任务窗口高亮、overview 关联；无 shell、文件、网络或 root 权限。
6. **System Settings 模块**：管理模型、数据边界、权限历史、默认确认级别和自动化策略。

### 5.3 绝不能放进 compositor 的能力

- 模型推理和 prompt；
- 网络访问、OAuth token、Secret Service 解密；
- 任意 shell/代码执行；
- 包安装、OS 更新、磁盘操作；
- 长期任务数据库；
- 审计日志的唯一副本；
- 自动化策略最终判定。

理由很直接：KWin 位于显示、输入和会话的关键路径。模型崩溃、网络卡顿、数据库损坏或任务死循环不应造成黑屏、输入冻结或无法解锁。

### 5.4 窗口语境适配器

Wayland 有意不提供任意客户端枚举和控制所有窗口的通用能力。Andromeda 若要知道“用户正在处理哪个项目/窗口”，应采用分级信息：

1. 应用主动通过 Andromeda Tool API 报告项目、文档和可用动作；
2. desktop file/app-id、当前活动窗口和 workspace 等最小元数据由 KWin adapter 提供；
3. 文本/控件语义通过任务范围内的 AT-SPI；
4. 像素观察通过 ScreenCast portal；
5. 键鼠执行通过 RemoteDesktop portal + libei。

KWin adapter 必须满足：

- 单独进程或可热卸载脚本；
- 版本化接口，例如 `org.andromeda.DesktopContext1`；
- 默认不返回窗口标题全文和像素；
- 每个字段有来源、时间戳和应用 ID；
- Plasma/KWin 更新时运行 contract test；
- 适配器失效只降级上下文能力，不阻断桌面。

### 5.5 libei：AI 输入的正确底层

[libei](https://libinput.pages.freedesktop.org/libei/) 为 Wayland 提供受 compositor 控制的模拟输入协议。`libei` 是客户端库、`libeis` 是 compositor/server 库，`liboeffis` 可以通过 XDG RemoteDesktop portal 获取受授权连接（[库关系](https://libinput.pages.freedesktop.org/libei/libraries/)）。

决策：**Pilot，随 Plasma/KWin 和 portal backend 的实现成熟度逐步 Adopt。**

规则：

- 不向 agent 开放 `/dev/uinput` 或 `/dev/input`；
- 每次输入序列都关联 task ID、目标应用、窗口/屏幕区域和过期时间；
- 屏幕锁定、用户切换、权限撤销、目标窗口变化时立即 pause；
- 保留明显的“AI 正在控制”系统指示器和硬停止快捷键；
- 优先发送 text/semantic action，最后才发送像素坐标点击；
- 输入成功不代表任务成功，必须读取应用状态或输出文件验证。

### 5.6 fork 预算

v1 稳定分支采用以下硬规则：

- KWin、Plasma、Wayland、Mesa、PipeWire、portal **默认零永久私有补丁**；
- 安全或硬件发布阻塞可临时 backport，但必须同时有上游 issue/MR、责任人和不超过 90 天的复审日期；
- 自定义 UI 通过主题、插件、独立应用和 D-Bus adapter；
- 不复制上游内部 QML 文件后长期改名维护；
- 私有插件使用公开 API；若必须使用 private API，只能进入 Pilot；
- 连续两个上游版本都需要同一私有补丁时，必须选择：上游化、删除需求或正式立项维护 fork，不能默默续期。

## 6. 远程桌面与远程应用

### 6.1 三种需求不能混为一个项目

| 需求 | 推荐技术 | v1 状态 |
| --- | --- | --- |
| 把一个 Wayland 应用经 SSH 转发到本地 | waypipe | Pilot，开发者功能 |
| 分享/控制当前登录桌面 | PipeWire + ScreenCast/RemoteDesktop portal + WebRTC | Pilot → Adopt |
| 无人值守登录/headless 完整会话 | 独立受管 remote session，评估 RDP/WebRTC | Pilot，普通用户默认关闭 |

### 6.2 项目矩阵

| 项目 | 作用 | 许可证 | 成熟度 | 决策 | 说明 |
| --- | --- | --- | --- | --- | --- |
| PipeWire + XDG portals | 屏幕/窗口流和输入授权 | MIT + LGPL | 高 | **Adopt** | 本地 capture/control 的安全基础 |
| [WebRTC](https://webrtc.org/) | 实时音视频、data channel、拥塞控制、NAT traversal | 主实现 BSD-3-Clause，标准/API 本身开放 | 高（传输） | **Pilot** | 作为 Andromeda 跨平台远程协议；仍需 signaling、TURN、身份和会话产品层 |
| [waypipe](https://gitlab.freedesktop.org/mstoeckl/waypipe) | 类似 `ssh -X` 的 Wayland 应用转发 | 当前 Rust `waypipe` GPL-3.0-or-later；`waypipe-c` MIT；依赖另计 | 中高 | **Pilot** | 适合开发/运维单应用，不是完整远程桌面 |
| [wayvnc](https://github.com/any1/wayvnc) | wlroots compositor 的 VNC server | ISC | 中 | **Watch** | 仅 wlroots 系，GNOME/KDE/Weston 不支持；VNC 不作为产品默认 |

WebRTC 官方说明其支持浏览器和主要原生平台的视频、语音和数据传输（[WebRTC](https://webrtc.org/)），但 WebRTC 不是完整远程桌面产品。Andromeda 仍需实现：

- 设备配对、端到端身份和密钥轮换；
- signaling 与 TURN，明确中继可见的元数据；
- portal 授权、会话指示器和即时撤销；
- PipeWire zero-copy/硬件编码及软件编码降级；
- 多显示器、HDR/SDR 映射、分数缩放和动态分辨率；
- 键盘布局、IME、剪贴板、文件传输和音频回声；
- 网络切换、断线恢复和带宽上限；
- 锁屏、登录屏和 headless session 的独立安全模型。

### 6.3 推荐远程流水线

```text
User approval / device trust
       │
       ▼
RemoteDesktop + ScreenCast portal
       │  PipeWire node + libei/EIS fd
       ▼
andromeda-remote-session（低权限、每会话沙箱）
       │
       ├─ video/audio encode → WebRTC SRTP
       ├─ data channel → clipboard/file portal messages
       └─ remote input → capability check → libei
```

无人值守远程登录不得复用“用户曾经同意过一次屏幕共享”的 token。它需要设备级信任、磁盘解锁边界、登录认证、速率限制、审计和恢复模式的单独设计。

## 7. 安装、镜像构建、initramfs 与启动

### 7.1 安装器

| 项目 | 作用 | 许可证 | 成熟度 | 决策 | Andromeda 用法 |
| --- | --- | --- | --- | --- | --- |
| [Anaconda](https://github.com/rhinstaller/anaconda) | Fedora/RHEL 系安装器、存储/加密/Kickstart/硬件配置 | GPL-2.0 | 高 | **Adopt（过渡）** | 首个可安装预览复用存储和部署引擎；Andromeda Preflight 独立 |
| [Calamares](https://calamares.euroquis.nl/docs/documentation/) | 跨发行版、模块化图形安装框架 | GPL-3.0-or-later 为主，模块逐项不同 | 中高 | **Watch** | 适合快速 Live ISO/非 Fedora 原型；不作为 v1 主安装链 |

Anaconda 的优势：

- Fedora/RHEL、SELinux、Kickstart、复杂存储和企业安装经验；
- 能与现有 Image Builder/bootc 路线组合；
- 失败路径和硬件差异暴露更充分。

风险：

- 重、发行版耦合、产品 UI 深改成本高；
- 上游正在向 Web UI 和 image mode/container installer 演进；
- 如果把迁移、AI、账户和完整品牌流程直接写进 Anaconda，未来替换会很痛苦。

因此安装流程拆成：

1. `andromeda-preflight`：Live 环境硬件检查、磁盘空间、BitLocker/FileVault/APFS/NTFS 风险、网络、输入、显示、备份和 HCM；
2. Anaconda/bootc 安装引擎：执行经过确认的分区和镜像部署；
3. `andromeda-firstboot`：账户、密钥、迁移、模型和恢复设置；
4. Task Center：继续长时间数据迁移、应用恢复和驱动验证。

Calamares 保留为备选，但不能为了换皮容易而牺牲复杂存储、断电恢复和 bootc 集成。

### 7.2 镜像构建

| 项目 | 作用 | 许可证 | 成熟度 | 决策 | 说明 |
| --- | --- | --- | --- | --- | --- |
| [osbuild](https://osbuild.org/docs/developer-guide/projects/osbuild/) | pipeline-based OS artifact builder | Apache-2.0 | 高 | **Adopt** | 构建可重现 stage/manifest；作为低层执行器 |
| [image-builder](https://osbuild.org/docs/developer-guide/projects/image-builder/installation/) | 统一 CLI/服务，生成 raw/qcow2/ISO/云镜像并支持 bootc 输入 | Apache-2.0 | 中高且活跃 | **Adopt** | v1 主要镜像入口 |
| [bootc-image-builder](https://osbuild.org/docs/bootc/) | 旧的 bootc 容器转磁盘镜像工具 | Apache-2.0 | 已进入迁移 | **Reject（新依赖）** | 仓库已并入 image-builder；只保留兼容旧流水线 |
| [mkosi](https://mkosi.systemd.io/) | 多发行版镜像、initrd、sysext 构建 | LGPL-2.1-or-later 为主 | 中高 | **Pilot** | 开发/测试/recovery/initrd 快速原型；不同时维护第二套正式产品清单 |

osbuild 官方把 stage 设计为“只弃用、不破坏”，相同 manifest 应持续产生相同结果（[OSBuild](https://osbuild.org/docs/developer-guide/projects/osbuild/)）。统一 image-builder 已支持 package mode 与 bootc 输入，且每周轮转发布多个组件（[发布流程](https://osbuild.org/docs/developer-guide/general/releasing/)）。

推荐流水线：

```text
Source + lockfiles + RPM repos + firmware manifest
        │
        ▼
Signed bootc OCI image
        │  SBOM / provenance / vulnerability gate
        ▼
Unified image-builder + osbuild
        │
        ├─ raw/qcow2（VM 与硬件 CI）
        ├─ installer/recovery ISO
        └─ OEM disk image
              │
              ▼
UKI signing + release metadata + HCM compatibility set
```

`bootc-image-builder` 的文档在 2026 年已明确标注仓库合并和弃用；新代码必须调用统一 `image-builder`。旧命令可作为兼容 wrapper，但不能让 Andromeda API 或 CI 日志格式绑定它。

### 7.3 dracut

[dracut-ng](https://dracut-ng.github.io/dracut/) 生成 initramfs，在真实 rootfs 可用前加载存储、网络、RAID、LVM、加密和必要驱动；许可证 GPL-2.0。官方区分：

- host-only：较小，只包含当前机器所需内容；
- default/generic：较大，适合通用内核和切换硬件。

决策：**Adopt**。

Andromeda 要求：

- 安装 ISO、恢复镜像和可移动系统盘使用 generic initramfs；
- 普通已安装系统可以优化为 host-only，但始终保留一个 generic recovery UKI；
- 用户准备把系统盘移到另一台 PC/Mac 时，Task Center 先生成并验证 generic 启动项；
- NVIDIA、存储、加密、键盘和网络早期驱动与 OS deployment 同一事务；
- initramfs 构建产物可重现并签名，不在目标机器上静默拼装未知脚本。

### 7.4 systemd-repart

[systemd-repart](https://systemd.io/BUILDING_IMAGES/) 可按声明操作 GPT 分区、首次启动扩容、创建/加密/填充分区，并强调中断不应留下半完成分区。

决策：**Pilot → Adopt（限定场景）**。

适合：

- OEM/预构建镜像首次启动扩展；
- 已知 GPT 布局的 recovery、factory reset 和数据分区初始化；
- TPM2 绑定加密分区；
- 自动创建明确缺失的分区。

不适合：

- 替代交互式双系统安装器；
- 未确认地缩小 Windows BitLocker、macOS APFS 或用户数据分区；
- 在未知 OEM 分区表上“智能猜测”；
- 作为通用 MBR 修复工具。

### 7.5 UKI 与启动

systemd 将 Unified Kernel Image 定义为一个 UEFI PE 文件，组合 `systemd-stub`、Linux kernel 和 initrd；推荐启动模型还可以使用 `systemd-boot` 的 boot counting 与自动回滚（[Boot Components & Root FS Discovery](https://systemd.io/ROOTFS_DISCOVERY/)）。

决策：**Adopt（UEFI PC/选定 Intel Mac）**。

要求：

- kernel、initrd、cmdline 和必要元数据一起签名；
- 更新保留 current、staged、rollback，且旧项只有在新系统 mark-good 后才可回收；
- Secure Boot、TPM PCR 策略与恢复密钥共同设计，固件变化不能把用户永久锁死；
- UKI 的版本、目标架构、driver set 和对应 bootc digest 进入机器可读状态 API；
- Apple silicon 不采用“原生 UEFI PC”假设，使用 Asahi/m1n1/U-Boot 对应启动链和独立镜像。

## 8. 系统更新：避免制造另一个 `Windows.old`

### 8.1 bootc 与 OSTree

bootc 提供 OCI/Docker 镜像驱动的事务 OS 更新；当前后端使用 OSTree，运行系统默认不被就地修改，新 deployment 在下次启动使用。`bootc rollback` 调整启动项回到前一个 deployment（[bootc 更新](https://bootc.dev/bootc/upgrades.html)）。

OSTree 提供内容寻址对象、多个 bootable roots、增量复制、事务升级和回滚（[libostree](https://ostreedev.github.io/ostree/)）。rpm-ostree 仍广泛用于 Fedora Silverblue/Kinoite/CoreOS，但其官方文档已经说明新开发重点转向 bootc、DNF 和相关生态（[rpm-ostree](https://coreos.github.io/rpm-ostree/)）。

决策：

- bootc：**Adopt**；
- OSTree：**Adopt（当前后端能力）**；
- rpm-ostree 客户端 layering：**Watch/限制使用**，不作为普通用户修改基础 OS 的入口。

### 8.2 Andromeda Update API

产品 UI 不直接解析 `bootc status` 文本。建立稳定、版本化的 `org.andromeda.Update1`：

```text
Check
  → Plan{from_digest,to_digest,download_bytes,staged_bytes,
         reclaimable_bytes,required_free_bytes,reboot,firmware_conflicts}
  → Download
  → Verify{signature,provenance,SBOM,HCM}
  → Stage
  → RebootToStaged
  → HealthCheck
  → MarkGood | AutomaticRollback
  → RetainRollback
  → GarbageCollect(after policy)
```

UI 必须明确显示：

- 当前、候选和回滚版本；
- 下载量、解包/暂存峰值、最终净占用和最低安全余量；
- 哪些内核、GPU、固件和应用会受影响；
- 是否失去对旧版本的回退能力；
- `/etc` 三方合并和 schema migration 风险；
- 上次成功启动、health check 和失败原因；
- 旧 deployment 的系统管理位置、保留期和删除条件。

旧系统不能以普通用户目录形式出现，也不能允许文件管理器直接删除。垃圾回收由 Update 服务执行，并在任何当前应用/驱动/回滚依赖存在时拒绝。

注意：bootc 官方 rollback 文档说明，回滚 deployment 会回到该 deployment 的 `/etc` 状态（[bootc rollback](https://bootc.dev/bootc/man/bootc-rollback.8.html)）。因此 Andromeda 必须为 `/etc`、数据库 schema 和用户态配置定义明确的跨版本语义，不能把“能切回旧 root”误当成完整业务回滚。

### 8.3 健康判定

新 deployment 只有满足以下条件才 mark-good：

- UKI、dm-verity/composefs（如启用）和签名验证成功；
- root、用户数据和密钥服务可用；
- compositor 启动并完成一次渲染；
- 内置键盘/触控板或至少一个可用输入设备；
- 当前认证 GPU 驱动加载且无持续 reset；
- NetworkManager、PipeWire、portal、Polkit、Task Center 基础服务健康；
- 更新 schema migration 成功；
- 在限定时间没有进入 crash loop。

系统不能要求云端模型在线才 mark-good；AI 功能离线不影响基础 OS 可启动性。

## 9. 应用商店与更新 UI

### 9.1 项目矩阵

| 项目 | 作用 | 许可证 | 成熟度 | 决策 | Andromeda 用法 |
| --- | --- | --- | --- | --- | --- |
| [AppStream](https://www.freedesktop.org/software/appstream/docs/) | 应用元数据、截图、组件/更新描述 | LGPL-2.1-or-later 为主 | 高 | **Adopt** | Store 索引和详情的标准元数据 |
| libflatpak / Flatpak | 安装、更新、卸载和权限 | LGPL-2.1 | 高 | **Adopt** | GUI 应用事务后端 |
| [KDE Discover](https://apps.kde.org/discover/) | 多后端软件中心 | GPL-2.0-or-later | 高 | **Adopt（v1 UI）** | 仅启用经验证的 Flatpak 后端；品牌/导航通过公开扩展和配置 |
| [GNOME Software](https://apps.gnome.org/Software/) | GNOME 软件与离线更新 UI | GPL-2.0 | 高 | **Watch** | UX/离线更新对照，不与 Plasma 混用 |
| [COSMIC Store](https://github.com/pop-os/cosmic-store) | Rust/iced 软件商店 | GPL-3.0 | 中 | **Watch/Pilot** | 研究快速 UI 和 libflatpak/PackageKit 接入，不作为 v1 主商店 |
| [PackageKit](https://packagekit.freedesktop.org/gtk-doc/specification.html) | 跨发行版包管理 D-Bus 抽象 | GPL-2.0-or-later 为主，库逐项 | 中高 | **Reject（基础 OS 更新）** | 不让其修改 bootc 基础；仅为传统兼容实验保留 |
| [fwupd/LVFS](https://fwupd.org/) | 设备固件发现、下载和更新 | fwupd LGPL-2.1 | 高 | **Adopt** | 独立 Firmware 事务，统一在 Update Center 呈现 |

Discover 能管理发行版仓库、Flatpak、Snap 和部分 AppImage 来源（[官方页面](https://apps.kde.org/discover/)），但 Andromeda 不应默认把所有 backend 都打开。来源越多，签名、权限、更新、卸载和回滚语义越不一致。

### 9.2 v1 Store 边界

- 默认只展示经过 Andromeda policy 审核的 Flatpak remote；
- 清楚显示发布者、来源、许可证、数据访问、后台能力和历史权限变更；
- 安装前展示权限 diff，而不是只展示笼统“可能访问文件”；
- 每次安装/更新都产生 Task Center job；
- 下载可暂停/继续，失败不留下半安装导出；
- 应用更新可单独回退到仓库中仍保留的 commit；
- 系统应用随 bootc 镜像交付，不伪装为可单独删除的 Flatpak；
- Windows 游戏、Office workspace、Web/PWA 和开发容器使用各自 catalog/recipe，不硬塞进 Flatpak 语义。

### 9.3 一个 UI，四种事务

用户可以从一个 Update Center 看见所有更新，但必须按事务域分组：

| 域 | 后端 | 是否重启 | 回滚对象 | 失败影响 |
| --- | --- | --- | --- | --- |
| System | bootc | 通常需要 | 整个 OS deployment | 自动启动回滚 |
| Apps | Flatpak | 通常不需要 | 单个 app/runtime commit | 单应用 |
| Firmware | fwupd | 设备相关，可能重启 | 厂商/设备能力决定 | 可能影响硬件，必须更高风险提示 |
| Dev/Compatibility | OCI、Wine/Proton recipe、VM | 分域决定 | 容器/前缀/VM snapshot | 不污染基础 OS |

不要把“全部更新”实现为一个无边界的大事务。UI 可以编排顺序，但每个后端保持自己的验证和回滚。

### 9.4 是否长期 fork Discover

不建议。阶段策略：

1. v1 使用 Discover 的 Flatpak 浏览、搜索、详情和基础事务；
2. 通过 D-Bus 把任务状态投影到 Task Center；
3. 系统与固件更新使用独立 Andromeda Update Center；
4. 当 Store 的权限 diff、AI 推荐解释、兼容 recipe 和多域工作流成为核心产品后，再使用 AppStream/libflatpak 公共库构建 Andromeda Store；
5. 新 Store 与 Discover 可并存一个版本，完成数据和功能验收后替换，而不是维护永久 Discover fork。

## 10. PC/Mac 硬件切换对桌面与发行工程的要求

本节只补充桌面/发行层，底层驱动详细结论见 [hardware-drivers-and-migration.md](./hardware-drivers-and-migration.md)。

### 10.1 同一镜像不等于同一支持承诺

- x86-64 通用 PC 可以共享一个基础 bootc image，但 HCM 决定哪些机型是 Certified；
- Intel Mac 可复用 x86-64 用户态和大量驱动，但启动、SMC、摄像头、音频、触控和混合显卡逐机型验证；
- Apple silicon 需要 arm64 + Asahi 设备树、固件和启动链，是独立产品镜像；
- 闭源 NVIDIA kernel module、用户态库和 Secure Boot 签名必须与目标 kernel 同一事务；
- Mesa/Wayland 可跨厂商复用接口，但不能让缺失的内核驱动“凭空出现”。

### 10.2 安装前必须完成的 Live 检查

1. 显示：内屏、外屏、刷新率、缩放、HDR/VRR 能力和 GPU reset；
2. 输入：内置键盘、触控板、触屏、鼠标和无障碍备用输入；
3. 存储：控制器、NVMe/SATA、VMD/RAID、BitLocker/FileVault/APFS/NTFS、剩余空间；
4. 网络：Wi‑Fi 固件、以太网、蓝牙、区域码；
5. 音频/摄像头：播放、录音、privacy LED、PipeWire portal；
6. 电源：电池、充电、关盖、一次 suspend/resume；
7. 启动：UEFI/Secure Boot/TPM，或对应 Mac 启动链；
8. 恢复：确认 recovery UKI/分区可启动；
9. 兼容：Windows/macOS 数据迁移路径和保留/回退选择；
10. HCM：生成“已验证、降级、未知、阻止安装”报告。

未知的内置输入、系统盘或启动 GPU 不得无提示继续安装。

### 10.3 驱动与镜像的交付关系

- 通用上游驱动随 kernel/firmware/Mesa 进入基础 image；
- 机型配置、UCM、libinput quirks 和摄像头 pipeline 与 HCM 版本绑定；
- 可选闭源驱动是签名的 system extension 或受控 image variant，但必须与 kernel ABI 锁步；
- 不允许用户通过 Store/PackageKit 随意替换核心 GPU/kernel module；
- generic recovery initramfs 永远包含足以进入恢复 UI 的显示、输入、存储和网络集合；
- 驱动更新失败时回滚整个 deployment，而不是让“新内核 + 旧用户态”混合启动。

## 11. 分阶段实施计划

### Phase 0：4 周，冻结接口与基线

- 锁定 Fedora/bootc 基础、Plasma/KWin 维护分支和 Qt 版本；
- 定义 `Task1`、`Capability1`、`DesktopContext1`、`Update1` D-Bus/JSON schema；
- 建立 osbuild/image-builder manifest、SBOM、签名和 provenance；
- 启动 Intel/AMD/NVIDIA 三类 VM/裸机冒烟；
- 禁止任何 KWin 私有补丁进入主分支。

退出门：

- Plasma 会话、Xwayland、Flatpak、PipeWire portal、Fcitx 5、AT-SPI 正常；
- Task Center 可以只读显示任务和系统更新状态；
- 同一 source digest 能可重现生成 raw/qcow2。

### Phase 1：8 周，可启动桌面与 AI 表面

- Task Center 主应用、Plasmoid、KRunner runner、通知；
- taskd 和模型 adapter 进程隔离；
- portal 文件/截图/屏幕会话；
- NetworkManager checkpoint 工具和 Flatpak 安装工具；
- Discover 仅 Flatpak 配置；
- generic recovery UKI。

退出门：

- 模型服务崩溃不影响桌面；
- 用户能在 2 秒内停止所有 AI 控制；
- 所有动作具备 task ID、capability、日志和验证结果；
- 屏幕锁定立即撤销 capture/input。

### Phase 2：8–12 周，安装与事务更新

- Andromeda Preflight + Anaconda/bootc 安装链；
- image-builder 输出 installer/recovery ISO；
- bootc staged update、boot counting、mark-good、自动回滚；
- 精确磁盘预算、旧 deployment 保留和垃圾回收；
- fwupd 独立事务；
- 断电/磁盘满/网络断开/签名失败故障注入。

退出门：

- 更新任何阶段断电均能启动 current 或 rollback；
- 空间不足在下载前给出可解释失败；
- 删除旧 deployment 不影响当前应用或用户数据；
- 30 次连续更新/回滚循环无不可恢复状态。

### Phase 3：12 周，硬件和远程

- 选定 PC/Intel Mac 的 HCM 认证；
- Apple silicon 独立 Pilot；
- PipeWire + portal + WebRTC 远程桌面；
- libei 输入、键盘布局、IME、剪贴板和多显示器；
- suspend/resume、dock、HDR/VRR、NVIDIA 回归。

退出门：

- 远程会话全程有可见指示器且可即时撤销；
- 断线不留下键/按钮按下状态；
- 本地和远程输入法、缩放、剪贴板通过矩阵；
- 所有 Certified 机型可从 recovery 启动并回滚。

### Phase 4：并行长期研究

- COSMIC/Smithay session 每季度重测；
- 评估自有 Store 替换 Discover；
- 向 KDE/Wayland/portal/libei 上游提交缺失接口；
- 收集是否需要自研 compositor 的真实产品证据；
- 不在主产品排期中预设“必须重写 compositor”。

## 12. 主要风险与缓解

| 风险 | 结果 | 缓解 |
| --- | --- | --- |
| Plasma/KWin 私有 API 泄漏 | 每次升级大量修复 | adapter、contract test、零永久私有补丁、上游优先 |
| AI 获得过宽桌面控制 | 隐私泄漏或破坏系统 | typed capability、portal/libei、明显指示器、硬停止、短时令牌 |
| PackageKit 与 bootc 双重修改 | 系统漂移、无法回滚 | 基础 OS 只允许 Update service/bootc；Store 不开放系统包 backend |
| bootc 后端/API继续演进 | 产品 UI 被命令实现绑死 | `Update1` 稳定抽象、后端 conformance test |
| `/etc` 与数据 schema 回滚不一致 | 旧系统无法使用新数据 | 版本化 schema、双向兼容窗口、迁移快照、回滚测试 |
| 通用 initramfs 太大 | 启动慢、ESP 占用 | installed host-only + 永久 generic recovery UKI |
| 只验证 Linux 桌面，不验证游戏 | 选择偏离用户核心价值 | Plasma/Mesa/KWin 与 Proton、HDR、VRR、反作弊测试共用发布门 |
| COSMIC 方向更快但 v1 已选 KDE | 后续迁移成本 | Task/Capability/Update API 桌面无关；季度 COSMIC Pilot |
| 许可证简化过度 | 发布合规风险 | 锁定 commit 后逐文件 SPDX、SBOM、source offer、NOTICE 和商标复核 |

## 13. 最终决策清单

### Adopt

- Wayland、Xwayland；
- KDE Plasma 6、KWin；
- Mesa、libinput、libxkbcommon；
- PipeWire、WirePlumber；
- xdg-desktop-portal-kde、Flatpak；
- Polkit、Secret Service 兼容接口；
- NetworkManager、BlueZ、CUPS、SANE；
- AT-SPI 2；
- Fcitx 5 默认、IBus 兼容；
- osbuild、统一 image-builder；
- bootc、OSTree 当前后端；
- dracut；
- UKI（UEFI 平台）；
- AppStream、Discover 的 Flatpak UI；
- fwupd/LVFS。

### Pilot

- COSMIC/cosmic-comp/libcosmic、Smithay；
- Weston 作为参考、headless 和 DRM CI；
- libei 输入；
- PipeWire/portal/WebRTC 远程桌面；
- waypipe 开发者功能；
- systemd-repart 的首次启动/恢复用途；
- mkosi 的 recovery/initrd/快速实验；
- container-based bootc-installer；
- Apple silicon 独立桌面镜像。

### Watch

- GNOME Shell/Mutter 作为桌面、a11y 和远程对照；
- wlroots 作为未来自研 compositor 的 C 路线；
- Calamares；
- COSMIC Store、GNOME Software；
- rpm-ostree client layering；
- wayvnc 和其他 compositor 专属远程方案。

### Reject

- v1 自研 compositor；
- 长期 KWin、GNOME Shell 或 COSMIC 整体 fork；
- 从旧的已归档 GitHub wlroots 镜像取版本和补丁；
- 新建对 `bootc-image-builder` 的长期依赖；
- PackageKit 修改 bootc 基础 OS；
- 让 agent 直接访问 `/dev/input`、`/dev/uinput`、PipeWire 全图或任意 root shell；
- 把旧 deployment 暴露成可手工删除的普通目录；
- 用 systemd-repart 猜测性缩小 Windows/macOS 用户分区；
- 假设 PC 的 UKI/UEFI 启动方案可直接套用 Apple silicon。

## 14. 官方资料索引

### 显示与桌面

- [Wayland](https://wayland.freedesktop.org/)
- [Wayland architecture](https://wayland.freedesktop.org/architecture.html)
- [Xwayland](https://wayland.freedesktop.org/docs/book/Xwayland.html)
- [wlroots 当前文档](https://wlroots.pages.freedesktop.org/wlroots/)
- [wlroots 旧归档 GitHub 镜像](https://github.com/swaywm/wlroots)
- [Smithay](https://smithay.github.io/pages/about.html)
- [Weston/libweston](https://wayland.pages.freedesktop.org/weston/)
- [KDE Plasma 开发文档](https://develop.kde.org/docs/plasma/)
- [KWin scripting API](https://develop.kde.org/docs/plasma/kwin/api/)
- [GNOME Shell API](https://gnome.pages.gitlab.gnome.org/gnome-shell/shell/)
- [Mutter API](https://gnome.pages.gitlab.gnome.org/mutter/meta/)
- [COSMIC Epoch](https://github.com/pop-os/cosmic-epoch)
- [cosmic-comp](https://github.com/pop-os/cosmic-comp)
- [libcosmic](https://github.com/pop-os/libcosmic)

### 桌面基础设施

- [Mesa](https://docs.mesa3d.org/)
- [libinput](https://wayland.freedesktop.org/libinput/doc/latest/)
- [libxkbcommon](https://xkbcommon.org/doc/current/)
- [PipeWire](https://docs.pipewire.org/)
- [WirePlumber](https://pipewire.pages.freedesktop.org/wireplumber/)
- [XDG Desktop Portal](https://flatpak.github.io/xdg-desktop-portal/docs/)
- [Flatpak](https://docs.flatpak.org/en/latest/)
- [Polkit](https://polkit.pages.freedesktop.org/polkit/)
- [Secret Service API](https://specifications.freedesktop.org/secret-service-spec/latest/)
- [NetworkManager](https://networkmanager.dev/docs/api/latest/)
- [BlueZ](https://bluez.readthedocs.io/en/latest/)
- [OpenPrinting CUPS](https://openprinting.github.io/cups/)
- [SANE backends 上游仓库](https://gitlab.com/sane-project/backends)
- [AT-SPI 2](https://gnome.pages.gitlab.gnome.org/at-spi2-core/libatspi/)
- [IBus](https://github.com/ibus/ibus)
- [Fcitx 5](https://www.fcitx-im.org/wiki/Fcitx_5/en)
- [libei](https://libinput.pages.freedesktop.org/libei/)

### 远程、安装、构建与更新

- [WebRTC](https://webrtc.org/)
- [waypipe](https://gitlab.freedesktop.org/mstoeckl/waypipe)
- [Anaconda installer](https://github.com/rhinstaller/anaconda)
- [Calamares](https://calamares.euroquis.nl/docs/documentation/)
- [OSBuild](https://osbuild.org/docs/developer-guide/projects/osbuild/)
- [Image Builder](https://osbuild.org/docs/developer-guide/projects/image-builder/installation/)
- [bootc-image-builder 迁移公告](https://osbuild.org/docs/bootc/deprecation-notice/)
- [bootc](https://bootc.dev/bootc/)
- [libostree](https://ostreedev.github.io/ostree/)
- [rpm-ostree](https://coreos.github.io/rpm-ostree/)
- [dracut-ng](https://dracut-ng.github.io/dracut/)
- [systemd 安全构建镜像/repart](https://systemd.io/BUILDING_IMAGES/)
- [systemd UKI/启动组件](https://systemd.io/ROOTFS_DISCOVERY/)
- [mkosi](https://mkosi.systemd.io/)
- [KDE Discover](https://apps.kde.org/discover/)
- [GNOME Software](https://apps.gnome.org/Software/)
- [fwupd/LVFS](https://fwupd.org/)
