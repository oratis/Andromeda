# Andromeda：Windows 游戏、Office 与文件格式兼容性深度研究

> 调研日期：2026-07-26
> 范围：Windows 游戏、Microsoft Office、企业文件互操作、跨 CPU 架构执行与无缝 Windows VM
> 目标：为 Andromeda 在现有 PC 与 Mac 硬件上提供“Windows 的生态兼容性，同时避免复制 Windows 的系统负担”建立可执行边界
> 说明：许可证结论仅用于产品架构初筛，不构成法律意见；发布前仍需逐组件、逐编译选项和逐国家/地区完成法律审查。

## 1. 执行摘要

### 1.1 最重要的结论

Andromeda 不应把“Windows 兼容”实现成一条技术路径，而应实现为一个**自动路由、可解释、可回退的兼容平台**：

1. Windows 游戏默认走 `Wine/Proton + DXVK/vkd3d-proton + Mesa/厂商 Vulkan 驱动 + Gamescope`，这是目前性能和兼容性最佳的开源主路径。
2. Windows 办公应用默认先使用原生 Linux 编辑器；对高保真、VBA、COM/VSTO、Power Query、Access 等工作流，自动切换到用户合法授权的 Office Web 或 Windows VM RemoteApp。
3. 跨架构执行由 FEX 或 Box64 负责 x86/x86-64 到 ARM64 的动态翻译，Wine 负责 Win32/Win64 API；这两层可以组合，但不能把组合结果宣传为与 x86 PC 等价。
4. 文件支持必须区分“预览”“读取”“基本编辑”“无损往返”“原生语义执行”五个等级，永远保留原文件，并将转换结果写成带来源记录的派生文件。
5. 反作弊、DRM、VBA/ActiveX、字体、专业打印和企业权限不是普通的格式解析问题，而是**信任、授权、服务端配合及私有生态问题**。它们决定了 Andromeda 必须保留原生 Windows 后备路径。

推荐的产品承诺不是“所有 Windows 软件都能运行”，而是：

> Andromeda 对每个应用和文件给出已验证的最佳运行方式；能原生运行就原生运行，能安全转译就转译，需要微软实现时切到 Web 或隔离 Windows 环境，无法可靠运行时明确说明原因而不损坏用户数据。

### 1.2 三项护城河能否实现

| 用户价值 | 可达到的程度 | 主要技术 | 无法由开源单独解决的部分 |
|---|---:|---|---|
| Windows 游戏生态 | 对进入 Andromeda 签名兼容矩阵、且已启用 Proton 支持的游戏可达到高水平；不能据此外推全部游戏 | Proton、Wine、DXVK、vkd3d-proton、Mesa、Gamescope、Steam Runtime | 未启用 Linux/Proton 的内核反作弊、部分 DRM、Xbox/Game Pass/UWP、专有外设/VR 驱动 |
| Microsoft Office 能力 | 普通文档高，复杂企业文档需要分层后备 | ONLYOFFICE、LibreOffice、Collabora、Office Web、Windows VM RemoteApp | 完整 VBA/ActiveX、COM/VSTO/XLL、Power Pivot/全量 Power Query、Access、精确字体排版、部分 IRM/MIP 与企业插件 |
| “什么文件都能打开” | 预览和安全提取可非常广；无损编辑不可能覆盖所有私有格式 | libmagic/Tika、FFmpeg、ImageMagick、Poppler、Ghostscript、libarchive、7-Zip、Pandoc、Calibre 等 | DRM 内容、私有工程/创意格式的全部语义、加密或损坏文件、受专利或授权约束的编解码器 |

## 2. 调研方法与成熟度口径

本研究优先使用项目官方仓库、官方文档、标准组织及厂商一手资料。项目成熟度采用以下口径：

- **生产级**：持续发布，被大规模发行版、设备或商业产品使用，有清晰兼容/安全维护流程。
- **成熟**：功能稳定且长期维护，但适用面、集成体验或商业支持弱于生产级。
- **活跃发展**：持续开发，已有实用价值，但接口、硬件范围或兼容率仍快速变化。
- **实验性**：适合原型和兼容性探索，不应作为默认用户承诺。
- **停滞/高风险**：长期缺少维护，或许可证/来源存在不适合直接复用的问题。

复用决策：

- **Adopt**：纳入默认架构，建立内部维护、回归测试和上游贡献能力。
- **Pilot**：作为可选后端或限定硬件/工作流试点。
- **Watch**：保持兼容接口并持续跟踪，暂不形成产品承诺。
- **Reject**：不直接分发或复用代码；可以研究其交互思想后自行重构。

## 3. Windows 游戏兼容

### 3.1 推荐图形与运行时栈

```text
Windows game / launcher
        │
        ├── Win32/Win64 API ───── Wine / Proton
        ├── D3D8-11 ───────────── DXVK ───────────┐
        ├── D3D12 / DXR ───────── vkd3d-proton ───┤
        ├── XAudio / media / input ─ Wine + FAudio│
        │                                          ▼
        └── per-title runtime ── Steam Runtime ─ Vulkan/OpenGL userspace driver
                                                   │
                                         Mesa RADV/ANV/NVK or NVIDIA
                                                   │
                                            Linux DRM/KMS kernel driver
                                                   │
                                              Gamescope / display
```

[Wine](https://www.winehq.org/about) 是 Windows API 的重实现，而不是完整 Windows 虚拟机。它的低开销和可调试性使其适合默认路径，但 Windows 内核驱动、依赖内核完整性的反作弊或 DRM 不属于普通 Win32 API 兼容范围。Wine 主体为 LGPL-2.1-or-later。

[Proton](https://github.com/ValveSoftware/Proton) 把 Wine、DXVK、vkd3d-proton、音频、输入、媒体和逐游戏补丁集成为 Steam Play 兼容工具。其顶层代码是 BSD-3-Clause，但仓库是多许可证聚合体，分发时必须生成完整 SBOM 与逐组件归属，而不能把整包标为单一 BSD。

[DXVK](https://github.com/doitsujin/DXVK) 将 D3D8/9/10/11 转为 Vulkan，采用 Zlib 许可证。当前版本依赖现代 Vulkan 特性；其[驱动要求](https://github.com/doitsujin/dxvk/wiki/Driver-support)明确区分当前和旧版 DXVK，所以 Andromeda 必须按 GPU 能力选择运行时，而不能只打包“最新版”。

[vkd3d-proton](https://github.com/HansKristian-Work/vkd3d-proton) 将 D3D12 转为 Vulkan，采用 LGPL-2.1。项目明确以性能和游戏兼容为优先，并积极使用现代 Vulkan 扩展，因此旧 GPU 会比 D3D11 游戏更早退出支持。它与 DXVK 共享 DXGI；Andromeda 应把两者作为同一受测版本集合发布，禁止用户界面随意混配不兼容版本。

[Mesa](https://docs.mesa3d.org/systems.html) 提供 AMD RADV、Intel ANV、NVIDIA NVK、Apple/Asahi 等开源用户态图形驱动以及 Vulkan、OpenGL、VA-API 等 API，实现以 MIT 风格许可证为主。RADV 的官方文档说明了用户态 Vulkan 驱动与内核 `amdgpu` 驱动的职责分界；这也是 Andromeda 驱动架构应遵循的边界。[RADV](https://docs.mesa3d.org/drivers/radv.html) 已是 Steam Deck 的 Vulkan 驱动。NVK 正在快速成熟，但在部分 NVIDIA 硬件上性能仍可能低于专有驱动，因此第一阶段应同时支持开源与厂商签名驱动。

[Gamescope](https://github.com/ValveSoftware/gamescope) 是 BSD-2-Clause 微合成器，可减少拷贝、固定游戏可见分辨率、限制帧率，并提供整数缩放、FSR/NIS、HDR 等游戏会话能力。它适合成为 Andromeda 的“游戏窗口/全屏会话管理器”，但不应取代通用桌面合成器。

[Steam Runtime](https://github.com/ValveSoftware/steam-runtime) 通过容器运行时为游戏提供稳定 ABI；当前 Proton 使用 `sniper` 容器。Valve 的说明清楚区分只读运行时库与从宿主注入的图形驱动栈：图形驱动不能被冻结在旧容器里，否则新 GPU 无法工作。这应直接成为 Andromeda 的兼容运行时设计：

- 基础用户态运行时只读、内容寻址、可回滚；
- GPU、音频、输入等硬件桥接从宿主按白名单注入；
- 每游戏单独的可写层和权限；
- 运行时版本与应用兼容档案绑定，而不是全局升级。

### 3.2 DirectX、音频、媒体、输入与体验

| 能力 | 主路径 | 成熟度 | 建议 | 主要限制 |
|---|---|---:|---|---|
| D3D8/9/10/11 | DXVK | 生产级 | Adopt | 新版要求现代 Vulkan；旧 GPU 需要冻结旧运行时 |
| D3D12、DXR | vkd3d-proton | 生产级/快速演进 | Adopt | 现代扩展要求高；新游戏特性可能滞后 |
| OpenGL 旧游戏 | WineD3D/Mesa | 成熟 | Adopt 后备 | 相比 DXVK 性能可能较低 |
| XAudio2/X3DAudio | [FAudio](https://github.com/FNA-XNA/FAudio) | 成熟 | Adopt | 某些专有音频中间件、空间音频插件仍可能失败 |
| 视频过场/Media Foundation | Wine/Proton + GStreamer/FFmpeg | 活跃发展 | Pilot | 专有编解码器、DRM 视频和授权限制 |
| 控制器 | Linux input、SDL、Wine HID/XInput | 成熟 | Adopt | 厂商配置软件、宏、灯效、固件升级可能仅支持 Windows |
| 帧率/缩放/HDR | Gamescope | 生产级于 SteamOS 场景 | Adopt | 桌面、驱动、显示器三者的 HDR/VRR 组合仍需硬件矩阵 |
| 性能叠加层 | [MangoHud](https://github.com/flightlessmango/MangoHud) | 成熟 | Pilot | 不应默认收集或上传性能数据 |
| 非 Steam 启动 | [umu-launcher](https://github.com/Open-Wine-Components/umu-launcher) | 活跃发展 | Pilot | 与 Valve 官方运行时的支持边界需持续验证 |

Shader 缓存、管线缓存和逐游戏配置必须被视为兼容数据，而不是普通临时文件。删除它们不会损坏存档，但会重新引入卡顿。Andromeda 的清理界面必须区分：

- 可重建缓存；
- 游戏本体；
- DLC；
- 存档；
- Wine prefix 中的注册表和依赖；
- 云同步副本。

这可以避免重演 `Windows.old` 那类“看似可删，实际存在隐式依赖”的问题。

### 3.3 反作弊：产品承诺的硬边界

Valve 的 [Steamworks Proton 文档](https://partner.steamgames.com/doc/steamhardware/proton?l=english)明确说明：

- Proton 支持 Easy Anti-Cheat 与 BattlEye 的兼容路径；
- 但 EAC 需要发行商为该构建手动启用 Linux/Proton 支持并发布相应模块；
- BattlEye 也需要每个游戏由开发商完成配置。

因此“中间件支持 Proton”不等于“所有使用该中间件的游戏都支持 Proton”。对于需要 Windows 内核驱动、启动链测量、进程完整性、内存扫描或禁止虚拟机的反作弊，开源兼容层不能在没有发行商授权与配合的前提下可靠替代。

Andromeda 应采用以下政策：

1. 维护以游戏版本、发行商和反作弊配置为键的签名兼容清单。
2. 只使用发行商和反作弊厂商批准的用户态/Linux 模块，不绕过或伪装内核检测。
3. 不让系统 AI 读取、注入或修改受保护游戏进程；游戏会话期间收紧调试、屏幕读取与自动化能力。
4. VM 后备不宣称可解决反作弊；很多反作弊会拒绝虚拟机，即便使用 GPU 直通。
5. 不可运行时明确显示“发行商未启用 Proton”或“反作弊禁止此环境”，避免把责任模糊为“系统错误”。

### 3.4 DRM、商店与启动器

Proton 已对 Valve CEG、部分 Denuvo 场景和多种第三方启动器持续增加兼容补丁，但 DRM 通常与游戏版本、账户登录、时钟、网络、设备指纹、媒体组件和内核能力耦合。可行原则是：

- 使用用户合法购买的游戏和官方商店客户端；
- 不实现 DRM 规避；
- 每个启动器使用独立前缀，避免一个启动器更新破坏全部游戏；
- 凭据进入系统密钥环，不写入兼容脚本或普通配置文件；
- 启动器网页部分使用受测 WebView，不任意混用宿主浏览器 Cookie；
- Xbox Game Pass / Microsoft Store / UWP 应标记为原生 Windows 后备，不在早期版本承诺 Wine 支持。

### 3.5 x86 PC、Intel Mac 与 Apple Silicon

[FEX](https://github.com/FEX-Emu/FEX) 是 MIT 许可的 ARM64 Linux 用户态 x86/x86-64 翻译器，支持把 OpenGL/Vulkan 调用转发给宿主库，并明确支持与 Wine/Proton 组合。它需要 x86-64 rootfs，并且不同 CPU 的内存模型模拟会影响兼容性和性能。

[Box64](https://github.com/ptitSeb/box64) 同样采用 MIT 许可，支持 ARM64、RISC-V、LoongArch 等宿主，通过包装宿主库与动态重编译运行 x86-64 Linux 程序；其 32 位路径 Box32 仍在演进。Box64 已能运行 Wine64/Proton，但更多依赖手工配置和逐应用参数。

推荐分层：

| 宿主 | 默认游戏路径 | 现实预期 |
|---|---|---|
| x86-64 PC | Proton/Wine 原生 CPU 执行 | 最佳，作为首发基线 |
| Intel Mac 安装 Andromeda | Proton/Wine 原生 CPU 执行 | CPU 兼容好，GPU/无线/电源驱动决定体验 |
| Apple Silicon Mac 安装 Andromeda | FEX + Wine/Proton + Mesa/Asahi | Pilot；双重兼容层，逐游戏验证 |
| ARM64 PC | FEX 或 Box64 + Wine/Proton | Pilot；依赖 ARM GPU Vulkan 驱动和内存模型 |
| 无法转译的应用 | Windows VM 或保留原系统启动项 | 可靠性优先，不隐藏切换 |

“无缝切换”应指用户账户、文件、应用入口、设置和数据迁移无缝，而不是承诺所有二进制在所有 CPU/GPU 上无性能损耗。特别是在 Apple Silicon 上，x86 指令翻译、Windows API 转译和 D3D→Vulkan 可能同时存在，必须在兼容页面展示实际路径。

### 3.6 游戏栈复用结论

**Adopt：** Wine/Proton、DXVK、vkd3d-proton、Mesa、Gamescope、Steam Runtime 思路及 per-title profile。
**Pilot：** umu、Bottles/Lutris/Heroic 的管理思想，FEX/Box64，MangoHud。
**不直接复用：** 任何绕过反作弊/DRM 的补丁集、来源不明的专有 DLL、未经审计的社区 codec 包。
**组织要求：** Andromeda 必须建立图形驱动、Wine、Proton、游戏兼容与发行商合作的常设团队；仅“打包开源项目”无法维持兼容性。

### 3.7 VR、游戏外设与 Mod 工具

#### VR 不是普通显示器兼容问题

VR 需要同时满足 HMD/控制器驱动、位置追踪、低延迟合成、显示器 direct mode、畸变校正、运行时 API 和游戏 API。建议采用：

1. OpenXR 作为 Andromeda 原生 XR API；
2. [Monado](https://monado.freedesktop.org/about-runtimes.html) 作为开源 OpenXR 运行时 Pilot；
3. Valve 官方 SteamVR Linux 运行时作为 SteamVR 设备/游戏路径；
4. [OpenComposite](https://gitlab.com/znixian/OpenOVR) 研究 OpenVR → OpenXR 的兼容路径。

Monado 官方明确说明，它包含设备驱动、合成器、OpenXR API 和常驻服务，并在 Intel、AMD、NVIDIA GPU 上处理 direct mode；但同一文档也明确说它尚未对所有设备提供完整终端用户体验，也不是其他 VR SDK 的即插即用替代品。其 [SteamVR 插件](https://monado.freedesktop.org/steamvr.html)只能让 SteamVR 使用 Monado 的硬件驱动，不能直接让 OpenVR 游戏运行在 Monado/OpenXR 上。

因此以下场景必须按设备、固件、GPU、追踪方式和游戏逐项认证：厂商专有运行时、inside-out 摄像头追踪、无线串流、眼动/手势追踪、空间音频、基站管理和固件升级。缺少 Linux 运行时或专有功能时，保留原生 Windows 双系统；VR 的低延迟与直连显示链也使普通 RemoteApp/VM 后备通常不合格。

#### 外设支持应区分“能输入”和“可配置全部功能”

标准 USB HID、Xbox/PlayStation 类控制器和常见键鼠可以走 Linux input/USB 栈；但 DPI、板载配置、宏、RGB、力反馈、方向盘角度、耳机混音、固件升级通常使用厂商私有协议。

- [libratbag](https://github.com/libratbag/libratbag) 以 MIT 许可提供游戏鼠标配置守护进程，[Piper](https://github.com/libratbag/piper) 是其 GPL-2.0 图形界面；官方设备库覆盖多个品牌，但新协议仍需逐设备逆向和测试。
- [OpenRGB](https://gitlab.com/CalcProgrammer1/OpenRGB) 可统一部分 RGB 设备；[OpenRazer](https://openrazer.github.io/) 覆盖部分 Razer 设备；[Oversteer](https://github.com/berarma/oversteer) 提供部分方向盘配置。
- 若设备支持板载内存，Andromeda 可在合法的 Windows 厂商工具中完成一次配置后使用板载状态；不能因此宣称宏、遥测、固件和全部灯效已原生支持。
- 固件更新、USB 恢复模式、专业方向盘/飞行摇杆、厂商音频 DSP 或 VR 外设无法在 Linux 安全完成时，应路由到原生 Windows；普通 VM 只有在 USB 直通经设备认证后才可使用。

#### Mod 应是可回退的内容层，不应污染游戏本体

[BepInEx](https://github.com/BepInEx/BepInEx)、[Mod Organizer 2](https://github.com/ModOrganizer2/modorganizer) 和 [SteamTinkerLaunch](https://github.com/sonic2kk/steamtinkerlaunch) 分别展示了插件框架、虚拟文件覆盖与 Linux 启动编排的能力，但它们不是通用兼容保证。Andromeda 的 Mod Manager 应：

1. 以只读游戏本体 + 可排序 Mod 覆盖层运行，每个 Profile 记录来源、哈希、依赖、加载顺序和可复现锁文件；
2. “纯净联机”和“Mod 单机”使用不同前缀、存档和启动入口，切换前校验游戏文件与进程注入状态；
3. 绝不在反作弊多人游戏中自动加载 DLL、调试器、注入器或内存工具；社区工具自身也警告此类组合可能触发封禁；
4. AI 可以解释冲突、生成变更计划和提供回退，但未经来源验证、权限确认和恶意扫描，不得自动下载或执行 Mod；
5. 保存原始存档与迁移副本；Mod 卸载不等于存档一定可恢复，界面必须提前显示风险。

## 4. Microsoft Office 与企业办公兼容

### 4.1 为什么“实现 OOXML”仍不等于 Microsoft Office

[ECMA-376](https://ecma-international.org/publications-and-standards/standards/ecma-376/) 和 ISO/IEC 29500 定义了 OOXML 的词汇、包装和生产/消费要求；[ODF 1.3](https://www.oasis-open.org/standard/open-document-format-for-office-applications-opendocument-version-1-3/) 则是开放、平台独立的办公文档标准。然而 Microsoft 还发布了大量 [Office 文件格式实现与扩展文档](https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-offdi/24ed256c-eb5b-494e-b4f6-fb696ad2b4dc)。例如 `.xlsx` 的 Microsoft 扩展可以包含外部数据连接、公式、图表和实现专有结构。

兼容难度至少有六层：

1. ZIP/OPC 包和 XML 是否可解析；
2. 标准元素是否被正确映射；
3. Microsoft 扩展、旧二进制格式和嵌入对象是否保留；
4. 字体度量、换行、分页、图表和动画是否一致；
5. VBA、ActiveX、OLE、COM/VSTO/XLL、外部数据连接是否执行；
6. OneDrive、SharePoint、Teams、IRM/MIP、签名和企业身份是否保持原服务语义。

任何开源套件都可以在前几层表现很好，但不能据此推导第 5、6 层等价。

### 4.2 三层 Office 策略

#### 第一层：原生开源编辑器

[LibreOffice](https://www.libreoffice.org/) 采用 MPL-2.0，贡献通常双许可 MPL-2.0/LGPL-3.0+。它是功能最完整、格式覆盖最广、可脚本化程度最高的原生开源办公套件，适合作为 ODF、旧格式、批处理、打印和离线办公底座。官方文档明确说明 [VBA 支持不完整](https://help.libreoffice.org/latest/en-US/text/sbasic/shared/vbasupport.html)，虽然覆盖常见 Excel 对象模型，但复杂宏需要改写。

[ONLYOFFICE Desktop Editors](https://helpcenter.onlyoffice.com/desktop/getting-started.aspx) 以 OOXML 为核心，采用 AGPL-3.0，桌面版支持 Windows、Linux 和 macOS。它适合作为 DOCX/XLSX/PPTX 的默认高保真编辑器候选。AGPL-3.0 的具体义务取决于是否修改、是否形成派生作品、链接/进程边界以及是否通过网络向用户提供交互；未修改的独立进程分发和修改后深度集成不能混为一谈。商标、组件版本与商业条款也需分别核对，正式方案交由法律审查。

[Collabora Online](https://github.com/CollaboraOnline/online) 基于 LibreOfficeKit，核心在线代码采用 MPL-2.0，适合自托管协同编辑和主权数据场景。它不自动获得 Microsoft 365 的服务端语义，仍需要存储、身份、版本历史、审计和 WOPI 类集成。

推荐不是二选一：

- ONLYOFFICE：默认处理 OOXML 的交互编辑；
- LibreOffice：ODF、旧格式、批量转换、复杂功能和打印后备；
- Collabora/ONLYOFFICE Docs：组织选择的自托管协作后端；
- 统一的 Andromeda 文档壳层负责文件历史、AI、权限、预览和路由。

#### 第二层：Microsoft Office Web

用户有合法 Microsoft 365 账户时，Andromeda 应把 Office Web 做成一级应用入口。微软官方说明共同创作依赖 OneDrive/SharePoint 等共享存储和 DOCX/XLSX/PPTX 等现代格式；[共同创作文档](https://support.microsoft.com/en-us/sharepoint/get-started-with-sharepoint/document-collaboration-and-co-authoring)也说明 IRM/RMS 保护文档存在限制。

Office Web 的价值是由 Microsoft 自己解释格式、协作状态和权限，但它不能替代全部桌面功能，且依赖云服务、网络、账户、组织策略及地区可用性。Andromeda 不应抓取或逆向 Web UI，而应通过浏览器/PWA、文件协议和用户授权的官方 API 集成。

#### 第三层：无缝 Windows VM

对于无法由前两层安全完成的工作流，可在 Linux/KVM 上运行合法授权的 Windows 与 Office。**完整 RDP 桌面是近期可靠基线，RemoteApp 单窗口只能作为待验证的 Pilot**：FreeRDP 实现客户端协议，并不证明普通 Windows Home/Pro VM 具备受支持的 RemoteApp host 能力，也不解决 Windows 虚拟化权利、RDS CAL、Office Shared Computer Activation 和企业激活。

- [QEMU](https://www.qemu.org/docs/master/about/license.html) 为 GPL-2.0；KVM 属于 Linux 内核 GPL-2.0。
- [libvirt](https://libvirt.org/) 提供成熟的 VM 生命周期、存储、网络和权限管理。
- [FreeRDP](https://github.com/FreeRDP/FreeRDP) 为 Apache-2.0，并实现 [RemoteApp](https://github.com/FreeRDP/FreeRDP/wiki/RemoteApp)。
- [WinApps](https://github.com/winapps-org/winapps) 展示了 Office/Adobe 窗口、文件关联与 Linux 桌面的整合方式，但其许可证文件承认原始项目部分没有开源许可，而重写部分主要是 AGPL-3.0。**不建议直接 vendor 其代码。**
- [Cassowary](https://github.com/casualsnek/cassowary) 为 GPL-2.0，证明了自动挂起 VM、双向文件打开等体验，但维护活跃度不足，只适合研究交互。

正式产品必须比较并由法律/商业流程确认四条路径：

1. Windows Pro 完整 RDP 桌面；
2. [Windows Server RDS RemoteApp](https://learn.microsoft.com/en-us/windows-server/remote/remote-desktop-services/overview) + [RDS CAL](https://learn.microsoft.com/en-us/windows-server/remote/remote-desktop-services/rds-client-access-license)；
3. Azure Virtual Desktop/Windows 365；
4. 自有 guest agent 窗口桥。

企业 Office 还需核对 [Microsoft 365 Apps Shared Computer Activation](https://learn.microsoft.com/en-us/microsoft-365-apps/licensing-activation/overview-shared-computer-activation) 和 [Windows 11 虚拟桌面许可](https://www.microsoft.com/licensing/docs/documents/download/Windows%2011%20licensing%20for%20Virtual%20Desktops.pdf)。在这些问题核清前，不把“单窗口 Office”列为确定能力。

建议 Andromeda 自行实现一个以 libvirt + FreeRDP 为基础的“Windows Application Bridge”原型，采用独立代码和协议：

- 应用菜单只展示已发布的 RemoteApp；
- 文件通过受控的临时共享区进入 Windows，不把整个主目录暴露给 VM；
- Windows 与 Office 更新在 VM 快照上先验证再切换；
- Office 进程退出后按策略休眠 VM；
- 宿主剪贴板、打印机、摄像头和 USB 按次授权；
- Windows 许可证、Office 许可证和组织合规状态明确展示；
- 用户可一键打开完整 Windows 桌面排错。

### 4.3 Office 无法被开源方案完整替代的能力

| 能力 | 开源现状 | 推荐后备 |
|---|---|---|
| VBA 宏及完整 Office 对象模型 | LibreOffice 官方确认只提供有限兼容；宏行为与安全模型不同 | Windows/macOS Office；迁移可选 Office Scripts |
| ActiveX、OLE 嵌入、Windows 自动化 | 与 Windows COM、注册表、进程和控件深度耦合 | Windows VM |
| COM/VSTO/XLL 插件 | 微软明确称 COM/VSTO 是 [Windows-only](https://learn.microsoft.com/en-us/office/dev/add-ins/develop/make-office-add-in-compatible-with-existing-com-add-in) | Windows VM；长期推动厂商迁移 Office Web Add-in |
| Power Query 全连接器与 Power Pivot/Data Model | Excel Windows 功能最全；微软只称 Mac 有“部分”支持，Web 也有限制 | Excel for Windows VM |
| Access、Publisher、部分 Visio/Project | Access/Publisher 为 PC-only，开源套件没有二进制/插件级等价 | Windows VM 或 Web 版本（若产品提供） |
| 复杂 Excel 财务模型 | 函数、迭代、浮点边界、外链、XLL、数据连接共同影响结果 | Excel for Windows 验证 |
| Word 像素级分页与 PowerPoint 动画 | 字体度量、渲染器和私有行为导致差异 | Office Web/桌面；导出 PDF 冻结最终版 |
| Outlook MAPI、企业插件、归档 | 与 Exchange、MAPI、COM、组织策略耦合 | Outlook Web 或 Windows VM |
| OneDrive/SharePoint/Teams 原生共同创作 | 自托管套件可协作，但服务协议、权限和版本语义不同 | 官方 Microsoft 365 Web/API |
| IRM/RMS/MIP、敏感度标签、数字签名 | 开源实现不具备完整端到端信任与组织策略 | 官方 Office/服务；只读提示而非静默移除 |

微软的 [Office Scripts 与 VBA 对比](https://learn.microsoft.com/en-us/office/dev/scripts/resources/vba-differences)还说明：VBA 面向桌面，Office Scripts 面向跨平台云端；COM 插件仅 Windows 支持。这意味着长期迁移方向可以是 JavaScript/Web Add-in/Office Scripts，但 Andromeda 不能擅自自动改写关键企业宏后直接执行。

### 4.4 字体、排版与打印

#### 字体不是可随系统复制的“普通资源”

Microsoft 的 [字体再分发 FAQ](https://learn.microsoft.com/en-my/typography/fonts/font-faq)明确说明，通常不得把 Windows 字体文件复制到其他应用、设备或 Web 服务器；文档嵌入只在字体内嵌标志允许时成立。因此：

- Andromeda 不应默认打包 Calibri、Aptos、Arial、Times New Roman 等 Microsoft 字体；
- 仅允许用户导入其另行取得、且许可证明确允许在 Andromeda 目标设备安装的字体；“拥有 Windows 许可”本身不产生复制字体到另一系统或设备的权利；
- 文档内嵌字体只能在字体 embedding flag、文档格式和字体许可证共同允许的范围内用于该文档，不得自动提取为系统字体；
- 可预装开源度量兼容字体，但必须提示“替代字体可能改变分页”；
- 打开文档时显示缺失字体、替换关系及受影响页数；
- 最终交付前提供“用 Microsoft Office 验证”和 PDF 冻结选项。

基础栈建议采用：

- [HarfBuzz](https://harfbuzz.github.io/what-is-harfbuzz.html)：跨平台 OpenType/复杂文字 shaping；
- [FreeType](https://freetype.org/freetype2/docs/index.html)：字体栅格化，FTL/GPL-2.0 双许可；
- [Fontconfig](https://www.freedesktop.org/wiki/Software/fontconfig/)：字体发现与替换；
- [Little CMS](https://www.littlecms.com/)：ICC 色彩管理。

#### 打印

[CUPS](https://openprinting.github.io/cups/) 采用 Apache-2.0（带 GPL-2.0 链接例外），通过 IPP Everywhere、AirPrint、Printer Applications 和旧 PPD 支持网络/USB 打印。它足以覆盖主流家用和办公打印，但以下能力仍可能依赖 Windows/macOS 厂商驱动：

- 专业装订、打孔、计费和部门代码；
- 大幅面、标签、票据、证卡和专用色彩流程；
- 扫描/传真/墨量/固件升级一体化工具；
- 私有打印认证与企业安全插件。

Andromeda 应优先使用无驱动 IPP，旧 PPD 放入隔离 Printer Application；对 Windows-only 功能，通过 Windows VM 打印桥后备，不允许打印驱动进入宿主内核。

### 4.5 Office 兼容质量门槛

“文件打开成功”不能作为发布指标。建议建立至少五类语料库：

1. Word：长文档、目录、脚注、修订、域、邮件合并、浮动对象、中日韩排版；
2. Excel：公式、数组、日期系统、图表、透视表、外链、Power Query、VBA、XLL；
3. PowerPoint：主题、母版、字体、图表、视频、动画、演讲者备注；
4. 安全：宏签名、IRM/MIP、密码、外部链接、恶意嵌入对象；
5. 打印：纸张、边距、双面、颜色、装订、PDF/A/PDF/X。

每次编辑器或字体更新执行：

- 首次渲染与 Microsoft Office 基准 PDF 的图像差分；
- 保存—重开—再保存的往返差分；
- 公式计算结果与错误类型差分；
- XML/OPC 结构差分并区分无意义顺序变化与语义丢失；
- 宏、外链、嵌入对象和签名的保留/禁用审计；
- 打印输出的页数、换行、颜色与纸张属性比对。

## 5. “支持所有文件”的格式平台

### 5.1 兼容等级

每个格式必须公开标记以下等级，而不是只显示“支持”：

| 等级 | 含义 | 示例 |
|---|---|---|
| L0 识别 | 判断真实 MIME/容器、扩展名伪装和风险 | `file`/libmagic、shared-mime-info |
| L1 安全预览 | 在无脚本、无外链环境生成缩略图/文本 | Poppler、FFmpeg、ImageMagick 沙箱 |
| L2 读取/提取 | 可靠读取主要内容和元数据 | Tika、ExifTool、libarchive |
| L3 基本编辑 | 可编辑常见内容，但可能丢失高级语义 | LibreOffice、GIMP、Inkscape |
| L4 无损往返 | 编辑后未知字段和未修改对象仍被保留 | 需严格语料测试，按格式/版本认证 |
| L5 原生语义 | 宏、插件、参数化模型、DRM、权限全部工作 | 常需原厂应用或 Windows VM |

### 5.2 建议架构：File Intelligence Service

```text
File open request
      │
      ▼
type detection ── signature / MIME / container / encryption / risk
      │
      ▼
policy router ─── preview | native editor | compatibility layer | VM | web
      │
      ├── immutable original
      ├── sandboxed parser
      ├── derived conversion + provenance
      ├── thumbnail / text / metadata index
      └── round-trip and loss warning
```

核心原则：

- 扩展名不是可信类型；使用 libmagic、容器结构和内容探测交叉判断。
- 原始文件默认不可变；编辑器保存到新版本，再由事务切换“当前版本”。
- PDF、图片、媒体、压缩包、Office 文档等解析器运行在无网络、低权限、限时限内存的独立进程。
- 解压前检查路径穿越、符号链接、压缩炸弹、嵌套深度和磁盘预算。
- 转换前显示会丢失的能力；AI 不得隐瞒宏、图层、参数化约束、批注或签名丢失。
- 任何转换都记录源文件哈希、转换器版本、参数、时间和损失报告。
- 加密和 DRM 文件不尝试破解；路由到合法客户端或提示缺少授权。

### 5.3 关键格式工具

#### PDF、PostScript 与扫描件

- [Poppler](https://poppler.freedesktop.org/)：基于 Xpdf 的成熟 PDF 渲染库，GPL-2.0-or-later；适合桌面预览、文本提取和缩略图。
- [Ghostscript/GhostPDL](https://ghostscript.com/about/)：PostScript、PDF、PCL、XPS 等页面描述语言解释器，AGPL-3.0 或商业许可；功能强但高风险输入面大，必须沙箱。
- [MuPDF](https://mupdf.com/)：高性能 PDF/XPS/EPUB 渲染，AGPL 或商业许可；适合作为第二渲染器和移动/低资源候选。
- [qpdf](https://github.com/qpdf/qpdf)：Apache-2.0，做 PDF 结构检查、线性化、加密和修复，不负责完整页面渲染。
- [OCRmyPDF](https://ocrmypdf.readthedocs.io/) + [Tesseract](https://tesseract-ocr.github.io/)：为扫描 PDF 添加可搜索文本，分别 MPL-2.0 与 Apache-2.0。

[ISO 32000-2](https://pdfa.org/sponsored-standards/) 已公开定义 PDF 2.0，但 PDF 的 JavaScript、嵌入文件、表单、签名、色彩、字体和损坏恢复仍会导致实现差异。预览器默认禁用 JavaScript、外部启动和自动访问网络。

#### 图片与创意资产

- [ImageMagick](https://imagemagick.org/formats/) 支持一百多种主要图片格式，使用宽松的 ImageMagick License；适合后台转换，不适合作为不受限的宿主服务。
- [libvips](https://www.libvips.org/) 为 LGPL-2.1+，流式/按需图像处理适合大图缩略图服务。
- [ExifTool](https://exiftool.org/) 适合广泛元数据读写；写入前需保留原始文件和 sidecar。
- [LibRaw](https://www.libraw.org/) 可处理大量相机 RAW；相机厂商私有元数据仍可能不完整。
- [Little CMS](https://www.littlecms.com/) 负责 ICC 色彩转换。

#### 音视频

[FFmpeg](https://ffmpeg.org/general.html) 覆盖极广的容器、编解码器、滤镜和设备格式。默认核心为 LGPL-2.1+，启用某些组件后整体变为 GPL-2.0+；官方[法律说明](https://ffmpeg.org/legal.html)还要求关注专利。开放源代码不等于获得 H.264/H.265/AAC 等所有地区和使用场景的专利许可。

Andromeda 应：

- 构建“基础开放编解码器包”和“地区/商业许可包”；
- 默认优先 AV1、Opus、VP9、FLAC、WebM/Matroska 等开放生态；
- 使用 GStreamer 管理实时管线和桌面应用集成，FFmpeg 负责广泛解码/转换；
- DRM 流媒体交给受支持浏览器 CDM，不把解密能力放入通用转换服务；
- 硬件解码通过 VA-API/Vulkan Video/厂商接口按能力协商。

#### 压缩、磁盘镜像与安装包

- [libarchive](https://www.libarchive.org/) 采用 BSD 许可，流式读取 tar/cpio/zip/ISO/CAB/RAR 等并写入常见开放格式；适合系统默认归档 API。
- [7-Zip](https://www.7-zip.org/) 大部分为 LGPL，部分 BSD，RAR 解码涉及 unRAR 限制；适合用户工具和特殊格式后备，不应把整个代码库误标为纯 LGPL。
- 压缩包预览和解包必须使用路径归一化、目标配额、文件数量上限和嵌套上限。
- MSI/NSIS/DMG/VHD/VMDK 等“能读取”不等于适合挂载或执行；默认只读分析，写入和启动需额外授权。

#### 文本、标记、电子书

- [Pandoc](https://pandoc.org/) 是 GPL 的通用文档转换器，覆盖 Markdown、DOCX、ODT、RTF、EPUB、HTML、LaTeX、PPTX 等；适合内容级转换，不承诺复杂页面布局无损。
- [Calibre](https://calibre-ebook.com/about) 是 GPL-3.0 电子书管理、阅读和转换平台；官方明确说明[不支持 DRM](https://manual.calibre-ebook.com/en/drm.html)，PDF 也属于最难可靠转换的输入。
- [EPUB 3.3](https://www.w3.org/TR/epub-33/) 是 W3C Recommendation，以 ZIP 打包 HTML/CSS/SVG 等资源；阅读器仍需隔离脚本和远程内容。

### 5.4 开源创作工具的定位与边界

| 项目 | 强项 | 成熟度/许可证 | Andromeda 建议 | 不能替代的部分 |
|---|---|---|---|---|
| [FreeCAD](https://www.freecad.org/) | 参数化 CAD、STEP/IGES/STL/IFC 等开放交换 | 活跃成熟，LGPL-2.1+ | Pilot 原生 CAD | SolidWorks/AutoCAD 插件和特征树、完整 DWG；官方手册也说明 DWG/3D DXF 受专有格式限制 |
| [Blender](https://www.blender.org/) | 建模、动画、渲染、视频、USD/FBX/OBJ 等 | 生产级，GPL | Adopt | Maya/3ds Max 专有插件、完全无损工程往返 |
| [GIMP](https://developer.gimp.org/core/standards/images/) | 栅格编辑、广泛图片导入导出、PSD/PSB | 成熟，GPL-3.0+ | Adopt | Photoshop 智能对象、调整层、插件、某些 PSD/CMYK；GIMP 官方也称 PSD 公开规范不完整 |
| [Krita](https://krita.org/) | 数字绘画、动画、PSD 互操作 | 生产级，GPL | Adopt | Photoshop 全部插件/自动化与像素级 PSD 往返 |
| [Inkscape](https://inkscape.org/about/) | SVG 原生矢量编辑，AI/EPS/PDF/PS 导入导出 | 成熟，GPL-2.0+ | Adopt | Illustrator 插件、效果和原生 AI 完整往返 |
| [Scribus](https://www.scribus.net/) | DTP、PDF/X、印前 | 成熟，GPL | Adopt 打印补充 | InDesign 工程、插件与排版引擎完全兼容 |
| [darktable](https://www.darktable.org/) / [RawTherapee](https://rawtherapee.com/) | RAW 工作流、非破坏编辑 | 成熟，GPL | Pilot | Lightroom 云目录、厂商色彩与插件生态 |

这些项目应作为优质原生应用提供，而不是被宣传为“打开同类专有格式就完全等价”。Andromeda 的差异化能力是格式路由、版本保留、差异预览和原厂后备。

## 6. Windows VM 与无缝应用桥

### 6.1 VM 的适用范围

Windows VM 适合：

- Microsoft Office 完整桌面能力；
- 企业插件、Access、Visio、Project、Outlook；
- 私有工程/创意格式的原厂应用；
- 低延迟要求不高、但兼容要求高的普通 Windows 工具。

Windows VM 不应默认用于：

- 需要最低延迟的游戏；
- 禁止 VM 的反作弊；
- 只有一块 GPU 且无法安全共享/直通的游戏笔记本；
- 期望极低功耗和即时唤醒的移动场景。

QEMU 官方说明硬件虚拟化可让客户代码接近原生速度，但虚拟 GPU 的 3D/Vulkan 加速仍在演进；Windows 高性能游戏通常需要 GPU 直通，而直通会带来 IOMMU、复位、双 GPU、显示切换、安全隔离和笔记本布线问题。[QEMU 安全文档](https://www.qemu.org/docs/master/system/security.html)还把客户机、远程显示、磁盘镜像和直通设备都视为不可信输入。

### 6.2 推荐实现

1. QEMU/KVM + libvirt 管理 VM；
2. qcow2 基础镜像只读，用户状态位于快照链；
3. virtio 磁盘/网络/输入，TPM 2.0 和 Secure Boot；
4. FreeRDP RemoteApp 显示 Office 等单窗口；
5. SPICE/完整 RDP 作为排错和安装界面；
6. 文件桥使用专用共享目录和复制写入，避免双系统同时写同一 NTFS 分区；
7. GPU 直通仅作为高级、硬件认证功能；
8. 每次 Windows 更新先在临时快照启动、验证 Office 和桥接代理，成功后原子切换；
9. VM 磁盘占用、旧快照、回退能力和依赖应用在存储界面逐项展示。

双启动场景需要特别防止 NTFS 损坏。内核 [NTFS3](https://cdn.kernel.org/doc/html/latest/filesystems/ntfs3.html) 已提供完整常规读写和日志重放；[NTFS-3G 文档](https://github.com/tuxera/ntfs-3g/wiki/Manual)提醒 Windows 休眠和 Fast Startup 会使分区处于不安全状态。Andromeda 检测到休眠 NTFS 时必须只读挂载并解释原因，不允许“强制修复”作为默认按钮。

### 6.3 必须保留 Windows Workspace 或双系统的判定

这里必须区分两种后备：

- **Windows Workspace**：隔离的 Windows VM，通过 RemoteApp 或完整桌面呈现，适合 Office、企业工具和普通桌面应用；
- **原生 Windows 双系统**：重启后让 Windows 直接控制 GPU、内核启动链和硬件驱动，适合 VM 无法满足的场景。

| 场景 | Windows Workspace | 原生 Windows 双系统 | 原因 |
|---|---:|---:|---|
| 完整 Office、VBA/COM、Access、企业插件 | 首选 | 可选 | 原厂 Windows 用户态环境即可，RemoteApp 可隐藏桌面切换 |
| 普通闭源 Windows 文件工具 | 首选 | 可选 | VM 可隔离格式解析和许可证环境 |
| Xbox App、PC Game Pass、Microsoft Store/UWP 游戏 | 不承诺 | **必留** | 安装、授权、更新、Gaming Services 与 Windows 系统组件深度绑定 |
| 内核反作弊、禁止 VM、要求受测启动链的游戏 | 不可作为后备 | **必留** | VM/GPU 直通仍可能被明确拒绝 |
| Linux/Proton 未覆盖的 GPU、DirectX 或专有媒体路径 | 仅低性能应用可试 | **游戏必留** | 原生驱动、延迟和功能集决定可用性 |
| 厂商专有 VR 运行时、追踪、无线/眼动/手势功能 | 通常不合格 | **必留** | 低延迟 direct mode、摄像头/USB 和专有服务难以远程化 |
| 固件升级、USB 恢复、专业外设驱动 | 认证后才可 | **建议必留** | USB 直通不等于厂商支持，失败可能损坏设备 |
| GPU/USB 加密狗/低延迟认证的专业创作软件 | 逐应用认证 | **常需保留** | 许可证、直通和厂商认证可能排除 VM |

首个稳定版本不得覆盖或删除现有 Windows/macOS 恢复能力。安装器应默认缩小而非抹除原系统分区，检测 BitLocker/FileVault、Windows Fast Startup 与固件启动项，提供可读的磁盘规划和一键回退。共享数据采用明确的复制/同步协议；休眠中的 NTFS 只读挂载。

在 Mac 上还应保留 macOS，用于固件更新、硬件诊断、macOS 独占购买和暂未支持的 Apple 硬件功能。Apple Silicon 不能原生启动标准 x86 Windows；可用的 Windows Workspace 是受许可、驱动和 ARM 应用兼容限制的 Windows on ARM VM。对只能在 x86 Windows 原生运行的游戏，可靠后备可能是另一台 Windows PC、串流主机或保留独立 PC，而不是本机“双系统”。

所谓“无缝切换”应落实为同一个应用入口自动选择原生、Web、Wine 或 Windows Workspace，并保持用户身份、文件位置和窗口体验一致；需要双系统时必须明确提示保存状态并执行冷切换，不能把重启包装成“无缝执行”。

## 7. 统一兼容代理：Andromeda 的原创价值

上游项目解决的是单个技术层，Andromeda 应原创的是统一的 `Compatibility Broker`：

### 7.1 核心组件

- **Application Resolver**：识别 Steam AppID、Windows PE 架构、商店、反作弊、依赖与已验证运行时。
- **File Resolver**：识别真实格式、版本、字体、宏、外链、加密、签名与潜在损失。
- **Environment Manager**：管理不可变 Proton/Wine/Office VM 版本、per-app 可写层、快照和回滚。
- **Capability Gateway**：文件、网络、摄像头、麦克风、USB、打印、剪贴板、辅助功能均按能力授权。
- **Bridge Layer**：窗口、通知、文件选择器、URI、打印、密钥环、剪贴板和拖放的协议桥。
- **Compatibility Knowledge Base**：把应用版本、硬件、驱动、运行时、结果和已知问题组成可签名规则。
- **Loss & Trust UI**：在切换编辑器、转换格式、启用宏或删除运行时前展示具体影响。
- **AI Tool API**：AI 只能调用有类型、可预览、可审计、可撤销的兼容操作。

### 7.2 安全模型

Wine 应用不是天然沙箱。默认每个应用应处于独立 mount/user/pid/network namespace，结合 Landlock/seccomp/cgroup 和桌面 portal：

- 游戏只看到自己的库、存档和用户明确选择的文件；
- Office 文档中的宏默认禁用，启用时进入专用 Windows VM 或高风险沙箱；
- 解析器不拥有用户主目录写权限；
- Windows VM 不直接访问宿主密钥环；
- AI 不以“帮你修复”为由获得全盘递归删除权；
- 所有自动修复先生成计划、快照与可读影响清单。

兼容性和安全有时冲突。某些游戏、启动器或反作弊要求广泛进程/设备访问，Andromeda 应提供一个明确的“受信任游戏会话”，临时暂停会冲突的 AI 自动化和调试能力，而不是静默降低全系统安全。

## 8. 兼容工作流开发计划

### 阶段 A：基础设施与 x86-64 PC 基线（0—6 个月）

- 建立 Compatibility Broker、内容寻址运行时、per-app 数据层和回滚。
- 集成 Mesa/厂商 Vulkan 驱动、Wine/Proton、DXVK、vkd3d-proton、Gamescope。
- 建立 libmagic/shared-mime-info、Poppler、FFmpeg、ImageMagick、libarchive 的解析沙箱。
- 集成 ONLYOFFICE 与 LibreOffice，不修改其核心代码，先通过文件关联和自动化 API 协作。
- 建立 100 款游戏、1,000 份 Office/格式文件的首个回归集。

退出条件：删除一个运行时不会删除存档；更新失败可回滚；每个游戏/文件显示实际执行路径；恶意压缩包和 PDF 不突破解析沙箱。

### 阶段 B：Office 与无缝 Windows 后备（6—12 个月）

- 实现 libvirt/FreeRDP Windows Application Bridge。
- 支持 Office Web 一级入口和用户授权的 OneDrive/SharePoint 文件打开。
- 完成字体缺失检测、合法导入、替换预览与 PDF 基准比对。
- 完成宏、外链、签名、IRM/MIP 状态检测；不能处理时不允许无提示另存。
- 与 CUPS/IPP Everywhere 集成，建立主流打印机认证集。

退出条件：常见 OOXML 达到明确的 L3/L4 指标；复杂 Office 工作流能在一次操作内切到 Windows VM；VM 更新与磁盘快照对用户完全可解释。

### 阶段 C：ARM64 与 Mac 硬件试点（12—24 个月）

- FEX 作为首选、Box64 作为实验后端；固定翻译器 + Wine + 图形驱动的整套版本。
- 在 Apple Silicon/ARM PC 上建立游戏和 Office VM 限定白名单。
- 同步用户文件、兼容前缀配置和应用入口，而不是复制不兼容的机器码缓存。
- 逐机型验证外接显示器、控制器、音频、睡眠唤醒和 GPU。

退出条件：支持名单内的 ARM64 设备可以无数据迁移地进入 Andromeda；每个 x86 应用显示翻译成本和已知限制；用户始终可回到原系统或 Windows VM。

### 阶段 D：生态合作与规模化（18 个月以后）

- 与 EAC、BattlEye、游戏发行商建立正式认证与问题升级通道。
- 与 Microsoft 365、文档管理系统、打印机和专业软件厂商建立官方集成。
- 发布兼容测试 SDK、文件语料规范和硬件认证计划。
- 对上游 Wine、Mesa、FreeRDP、LibreOffice 等持续贡献，而不是维护不可上游的巨大补丁集。

## 9. 成功指标与不可误导的表达

### 9.1 指标

- 游戏：按用户实际游戏时长加权的“可启动、可完成登录、可联网、帧时间稳定”比例，而非简单标题数量。
- Office：按格式和功能分层的打开率、往返无损率、视觉差异、公式一致率、宏明确处理率。
- 文件：L0/L1/L2 覆盖率、解析崩溃率、转换损失提示召回率、恶意样本隔离率。
- 更新：兼容运行时回滚成功率、更新后回归率、缓存/存档误删事件数。
- ARM：按设备和应用白名单统计，不把 x86-64 数据混入。

### 9.2 禁止使用的宣传语

- “100% 兼容 Windows 游戏”
- “完整支持所有反作弊”
- “完全替代 Microsoft Office”
- “所有文件格式无损编辑”
- “所有 PC 和 Mac 都可以无差别运行”

可以使用：

- “对已验证应用提供自动选择的最佳兼容路径”
- “默认保护原文件并在转换前说明损失”
- “支持名单透明，失败原因可解释，更新可回退”

## 10. 明确无法由开源方案替代的能力

### 10.1 游戏

1. 未由发行商启用 Proton/Linux 的 EAC、BattlEye 及其他内核反作弊。
2. 要求 Windows 内核驱动、受保护启动链或禁止 VM 的反作弊。
3. 某些 Denuvo/自研 DRM、一次性激活、设备指纹和受保护视频。
4. Xbox App、PC Game Pass、Microsoft Store/UWP 的完整购买、安装与授权链。
5. 厂商专有 VR、方向盘、灯效、宏、固件和遥测工具。
6. 新 DirectX/Agility SDK 特性在 Vulkan 与驱动尚无等价能力时的零日兼容。
7. 所有 x86 游戏在 Apple Silicon/ARM64 上与 x86-64 PC 同等的性能和稳定性。

### 10.2 Office

1. 完整 VBA、ActiveX、OLE、COM/VSTO/XLL 和基于 Windows 注册表/进程的自动化。
2. Excel Windows 的全部 Power Query 连接器、Power Pivot/Data Model 及企业数据插件。
3. Access、Publisher 和部分 Visio、Project、Outlook 企业工作流。
4. Microsoft 字体、Word 分页、PowerPoint 动画与 Excel 图表的逐像素一致。
5. OneDrive/SharePoint/Teams 的全部实时协作、权限、审计、版本和组织策略语义。
6. IRM/RMS/MIP、敏感度标签、受保护视图和数字签名的完整信任链。
7. 依赖原厂插件、模板、宏或第三方闭源连接器的行业解决方案。

### 10.3 文件和专业创作

1. DRM 电子书、流媒体和受硬件安全模块保护的内容。
2. Photoshop/Illustrator/InDesign、SolidWorks/AutoCAD 等私有工程文件的全部可编辑语义。
3. 只有 Windows/macOS 私有驱动支持的专业打印、扫描和色彩设备。
4. 受专利、商标或再分发许可限制的字体、编解码器和 SDK。
5. 损坏、加密、混淆或恶意文件的“保证打开”。

## 11. 相关开源项目目录与采用建议

活跃度为截至 2026-07-26 的架构判断；“生产级”不代表 Andromeda 可以不做自身回归测试。

### 11.1 游戏、图形与 Windows API

| 项目 | 类别 | 许可证 | 活跃度/成熟度 | 决策 | 官方链接 |
|---|---|---|---|---|---|
| Wine | Win32/Win64 API 兼容 | LGPL-2.1+ | 生产级、持续发布 | Adopt | [WineHQ](https://www.winehq.org/) |
| Proton | Steam Windows 游戏整合 | 多许可证；顶层 BSD-3 | 生产级、Valve 维护 | Adopt | [GitHub](https://github.com/ValveSoftware/Proton) |
| DXVK | D3D8-11 → Vulkan | Zlib | 生产级、活跃 | Adopt | [GitHub](https://github.com/doitsujin/DXVK) |
| vkd3d-proton | D3D12 → Vulkan | LGPL-2.1 | 生产级、活跃 | Adopt | [GitHub](https://github.com/HansKristian-Work/vkd3d-proton) |
| Mesa | Vulkan/OpenGL/视频驱动 | MIT 为主 | 生产级 | Adopt | [文档](https://docs.mesa3d.org/) |
| Gamescope | 游戏微合成器 | BSD-2 | SteamOS 生产级 | Adopt | [GitHub](https://github.com/ValveSoftware/gamescope) |
| Steam Runtime | 游戏 ABI/容器运行时 | 多许可证；Valve 脚本 MIT | 生产级 | Adopt 架构/协作上游 | [GitHub](https://github.com/ValveSoftware/steam-runtime) |
| FAudio | XAudio 重实现 | Zlib | 成熟 | Adopt | [GitHub](https://github.com/FNA-XNA/FAudio) |
| umu-launcher | 非 Steam Proton 启动 | GPL-3.0 | 活跃发展 | Pilot | [GitHub](https://github.com/Open-Wine-Components/umu-launcher) |
| Bottles | Wine 前缀/依赖管理 | GPL-3.0 | 活跃成熟 | Pilot，复用交互思想 | [GitHub](https://github.com/bottlesdevs/Bottles) |
| Lutris | 多商店游戏管理 | GPL-3.0 | 活跃成熟 | Pilot | [GitHub](https://github.com/lutris/lutris) |
| Heroic | Epic/GOG/Amazon 启动器 | GPL-3.0 | 活跃成熟 | Pilot | [GitHub](https://github.com/Heroic-Games-Launcher/HeroicGamesLauncher) |
| MangoHud | 游戏性能叠加层 | MIT | 成熟 | Pilot | [GitHub](https://github.com/flightlessmango/MangoHud) |
| vkBasalt | Vulkan 后处理层 | Zlib | 活跃/小型 | Watch | [GitHub](https://github.com/DadSchoorse/vkBasalt) |
| Monado | OpenXR 运行时 | Boost-1.0 | 活跃发展 | Watch/Pilot VR | [官网](https://monado.freedesktop.org/) |
| OpenComposite | OpenVR → OpenXR | GPL-3.0 | 活跃发展 | Watch | [GitLab](https://gitlab.com/znixian/OpenOVR) |
| libratbag | 游戏鼠标配置守护进程 | MIT | 活跃、设备特定 | Pilot | [GitHub](https://github.com/libratbag/libratbag) |
| Piper | libratbag 图形界面 | GPL-2.0 | 活跃成熟 | Pilot | [GitHub](https://github.com/libratbag/piper) |
| OpenRGB | RGB 设备控制 | GPL-2.0+ | 活跃、硬件矩阵广但不完整 | Pilot | [GitLab](https://gitlab.com/CalcProgrammer1/OpenRGB) |
| OpenRazer | Razer 设备 Linux 驱动/守护进程 | GPL-2.0 | 活跃、设备特定 | Pilot | [官网](https://openrazer.github.io/) |
| Oversteer | 方向盘配置 | GPL-3.0 | 活跃、小型生态 | Pilot | [GitHub](https://github.com/berarma/oversteer) |
| SteamTinkerLaunch | Steam/Wine 启动与 Mod 编排 | GPL-3.0 | 活跃、社区工具 | Pilot 交互；不进入反作弊会话 | [GitHub](https://github.com/sonic2kk/steamtinkerlaunch) |
| Mod Organizer 2 | 游戏 Mod 管理/虚拟覆盖 | GPL-3.0 | Windows 成熟；Wine 需逐版本测试 | Pilot | [GitHub](https://github.com/ModOrganizer2/modorganizer) |
| BepInEx | Unity/XNA 插件与 Mod 框架 | LGPL-2.1 | 成熟但依赖游戏引擎/版本 | Pilot | [GitHub](https://github.com/BepInEx/BepInEx) |
| Proton-GE | 社区 Proton 构建 | 多许可证 | 活跃但非官方 | Watch，不默认分发 | [GitHub](https://github.com/GloriousEggroll/proton-ge-custom) |

### 11.2 跨架构、虚拟化与无缝 Windows

| 项目 | 类别 | 许可证 | 活跃度/成熟度 | 决策 | 官方链接 |
|---|---|---|---|---|---|
| FEX | x86/x86-64 → ARM64 Linux | MIT | 活跃发展、游戏可用 | Pilot 首选 ARM | [GitHub](https://github.com/FEX-Emu/FEX) |
| Box64 | x86-64 → ARM/RISC-V/LoongArch | MIT | 活跃发展 | Pilot/后备 | [GitHub](https://github.com/ptitSeb/box64) |
| Box86/Box32 | 32 位 x86 翻译 | MIT | Box86 成熟、Box32 演进 | Watch | [Box86](https://github.com/ptitSeb/box86) |
| QEMU | 机器仿真与虚拟化 | GPL-2.0 | 生产级 | Adopt | [官网](https://www.qemu.org/) |
| KVM | Linux 硬件虚拟化 | GPL-2.0 | 生产级 | Adopt | [内核文档](https://www.kernel.org/doc/html/latest/virt/kvm/index.html) |
| libvirt | VM 生命周期 API | LGPL-2.1+ | 生产级 | Adopt | [官网](https://libvirt.org/) |
| FreeRDP | RDP/RemoteApp | Apache-2.0 | 生产级/活跃 | Adopt | [官网](https://www.freerdp.com/) |
| SPICE | VM 显示与设备重定向 | LGPL-2.1+ | 成熟、创新放缓 | Adopt 后备 | [官网](https://www.spice-space.org/) |
| Looking Glass | GPU 直通低延迟显示 | GPL-2.0 | 活跃、专业用户 | Pilot | [官网](https://looking-glass.io/) |
| crosvm | 安全型 VMM | BSD-3 | ChromeOS 生产使用 | Watch/Pilot 沙箱 VM | [官网](https://crosvm.dev/) |
| rust-vmm | Rust VMM 组件 | Apache-2.0/BSD-3 | 活跃生态 | Watch | [GitHub](https://github.com/rust-vmm) |
| WinApps | Windows RemoteApp 桌面整合 | 混合：新代码多为 AGPL-3，遗留部分无许可 | 活跃但法律风险 | Reject 直接复用 | [GitHub](https://github.com/winapps-org/winapps) |
| Cassowary | Windows VM 无缝应用 | GPL-2.0 | 维护不足 | Reject 代码；研究交互 | [GitHub](https://github.com/casualsnek/cassowary) |

### 11.3 Office、协作、Windows 文件与企业互操作

| 项目 | 类别 | 许可证 | 活跃度/成熟度 | 决策 | 官方链接 |
|---|---|---|---|---|---|
| LibreOffice | 桌面办公/格式转换 | MPL-2.0（贡献常双许可 LGPL-3+） | 生产级 | Adopt | [官网](https://www.libreoffice.org/) |
| Collabora Online | 自托管在线办公 | MPL-2.0 | 生产级/商业支持 | Pilot/Adopt 企业版 | [GitHub](https://github.com/CollaboraOnline/online) |
| ONLYOFFICE Desktop Editors | OOXML 优先桌面办公 | AGPL-3.0/商业 | 生产级 | Adopt，先不 fork | [GitHub](https://github.com/ONLYOFFICE/DesktopEditors) |
| ONLYOFFICE Docs | 在线协同编辑 | AGPL-3.0/商业 | 生产级 | Pilot | [API](https://api.onlyoffice.com/) |
| Samba | SMB3、AD、文件/打印 | GPL-3.0 | 生产级 | Adopt | [官网](https://www.samba.org/) |
| Open XML SDK | OOXML 操作库 | MIT | 成熟、微软维护 | Adopt 工具链 | [GitHub](https://github.com/dotnet/Open-XML-SDK) |
| Apache POI | Java Office 格式 API | Apache-2.0 | 成熟 | Pilot 服务端 | [官网](https://poi.apache.org/) |
| docx4j | Java OOXML | Apache-2.0 | 成熟 | Watch/Pilot | [GitHub](https://github.com/plutext/docx4j) |
| Gnumeric | 轻量电子表格 | GPL-2.0+ | 成熟但生态较小 | Watch | [官网](http://www.gnumeric.org/) |
| AbiWord | 轻量文字处理 | GPL-2.0+ | 维护有限 | Reject 默认 | [官网](https://www.abisource.com/) |
| Apache OpenOffice | 办公套件 | Apache-2.0 | 维护存在但落后 LibreOffice | Reject 默认 | [官网](https://www.openoffice.org/) |
| NTFS3 | 内核 NTFS 读写 | GPL-2.0 | 内核生产可用 | Adopt | [内核文档](https://www.kernel.org/doc/html/latest/filesystems/ntfs3.html) |
| NTFS-3G | FUSE NTFS 读写/工具 | GPL-2.0+ | 成熟 | Adopt 救援/后备 | [GitHub](https://github.com/tuxera/ntfs-3g) |

### 11.4 PDF、媒体、图片、归档、文本与识别

| 项目 | 类别 | 许可证 | 活跃度/成熟度 | 决策 | 官方链接 |
|---|---|---|---|---|---|
| Poppler | PDF 渲染/提取 | GPL-2.0+ | 生产级 | Adopt | [官网](https://poppler.freedesktop.org/) |
| Ghostscript/GhostPDL | PDF/PS/PCL/XPS | AGPL-3.0/商业 | 生产级 | Adopt，隔离服务 | [官网](https://ghostscript.com/) |
| MuPDF | PDF/XPS/EPUB 渲染 | AGPL/商业 | 生产级 | Pilot 第二引擎 | [官网](https://mupdf.com/) |
| qpdf | PDF 结构/修复/加密 | Apache-2.0 | 成熟 | Adopt | [GitHub](https://github.com/qpdf/qpdf) |
| PDFium | Chromium PDF 引擎 | BSD-3 | 生产级 | Watch/浏览器内使用 | [仓库](https://pdfium.googlesource.com/pdfium/) |
| OCRmyPDF | 扫描 PDF OCR 管线 | MPL-2.0 | 成熟 | Adopt | [官网](https://ocrmypdf.readthedocs.io/) |
| Tesseract | OCR 引擎 | Apache-2.0 | 成熟 | Adopt | [GitHub](https://github.com/tesseract-ocr/tesseract) |
| FFmpeg | 音视频编解码/转换 | LGPL-2.1+；可变为 GPL | 生产级 | Adopt，严格构建审计 | [官网](https://ffmpeg.org/) |
| GStreamer | 多媒体管线 | LGPL-2.1+ | 生产级 | Adopt | [官网](https://gstreamer.freedesktop.org/) |
| libplacebo | GPU 视频渲染 | LGPL-2.1+ | 活跃成熟 | Pilot | [官网](https://libplacebo.org/) |
| libheif | HEIF/AVIF | LGPL-3.0 | 活跃成熟 | Pilot/许可审查 | [GitHub](https://github.com/strukturag/libheif) |
| dav1d | AV1 解码 | BSD-2 | 生产级 | Adopt | [VideoLAN](https://code.videolan.org/videolan/dav1d) |
| ImageMagick | 广泛图片转换 | ImageMagick License | 生产级 | Adopt，强沙箱 | [官网](https://imagemagick.org/) |
| libvips | 大图/流式处理 | LGPL-2.1+ | 生产级 | Adopt | [官网](https://www.libvips.org/) |
| LibRaw | 相机 RAW | LGPL-2.1/CDDL | 成熟 | Adopt | [官网](https://www.libraw.org/) |
| ExifTool | 元数据 | Perl Artistic/GPL | 生产级 | Adopt | [官网](https://exiftool.org/) |
| Little CMS | ICC 色彩管理 | MIT | 生产级 | Adopt | [官网](https://www.littlecms.com/) |
| libarchive | 归档/压缩 API | BSD-2 | 生产级 | Adopt | [官网](https://www.libarchive.org/) |
| 7-Zip | 归档与镜像格式 | LGPL/BSD/unRAR 限制 | 生产级 | Adopt 工具/后备 | [官网](https://www.7-zip.org/) |
| Pandoc | 通用文档转换 | GPL | 生产级 | Adopt | [官网](https://pandoc.org/) |
| Calibre | 电子书阅读/转换/管理 | GPL-3.0 | 生产级 | Adopt | [官网](https://calibre-ebook.com/) |
| Apache Tika | 文档类型/文本/元数据提取 | Apache-2.0 | 生产级 | Adopt 服务端/索引 | [官网](https://tika.apache.org/) |
| libmagic/file | 内容类型探测 | BSD-2-Clause | 生产级 | Adopt | [仓库](https://github.com/file/file) |
| shared-mime-info | 桌面 MIME 数据库 | GPL-2.0 | 生产级 | Adopt | [GitLab](https://gitlab.freedesktop.org/xdg/shared-mime-info) |

### 11.5 字体、打印与创作

| 项目 | 类别 | 许可证 | 活跃度/成熟度 | 决策 | 官方链接 |
|---|---|---|---|---|---|
| HarfBuzz | 文字 shaping | MIT | 生产级 | Adopt | [官网](https://harfbuzz.github.io/) |
| FreeType | 字体渲染 | FTL/GPL-2.0 | 生产级 | Adopt | [官网](https://freetype.org/) |
| Fontconfig | 字体发现/替换 | MIT | 生产级 | Adopt | [官网](https://www.freedesktop.org/wiki/Software/fontconfig/) |
| CUPS | 打印系统/IPP | Apache-2.0 + 例外 | 生产级 | Adopt | [官网](https://openprinting.github.io/cups/) |
| cups-filters | 打印过滤/无驱动桥 | Apache-2.0 等，按组件核查 | 生产级 | Adopt | [上游仓库](https://github.com/OpenPrinting/cups-filters) |
| PAPPL | Printer Application 框架 | Apache-2.0 | 活跃成熟 | Adopt | [GitHub](https://github.com/michaelrsweet/pappl) |
| FreeCAD | 参数化 CAD | LGPL-2.1+ | 活跃成熟 | Pilot | [官网](https://www.freecad.org/) |
| Blender | 3D/DCC | GPL | 生产级 | Adopt | [官网](https://www.blender.org/) |
| assimp | 3D 格式导入库 | BSD-3 | 成熟 | Pilot | [GitHub](https://github.com/assimp/assimp) |
| OpenSCAD | 程序化 CAD | GPL-2.0 | 成熟 | Pilot | [官网](https://openscad.org/) |
| GIMP | 栅格图像编辑 | GPL-3.0+ | 生产级 | Adopt | [官网](https://www.gimp.org/) |
| Krita | 数字绘画 | GPL-3.0 | 生产级 | Adopt | [官网](https://krita.org/) |
| Inkscape | SVG 矢量编辑 | GPL-2.0+ | 生产级 | Adopt | [官网](https://inkscape.org/) |
| Scribus | 桌面出版/PDF/X | GPL-2.0+ | 成熟 | Adopt | [官网](https://www.scribus.net/) |
| darktable | RAW/摄影工作流 | GPL-3.0 | 生产级 | Pilot | [官网](https://www.darktable.org/) |
| RawTherapee | RAW 处理 | GPL-3.0 | 成熟 | Pilot | [官网](https://rawtherapee.com/) |

## 12. 主要一手资料索引

### 游戏与图形

- [Wine：About](https://www.winehq.org/about)
- [Valve Proton 官方仓库](https://github.com/ValveSoftware/Proton)
- [Steam Hardware and Proton：反作弊支持](https://partner.steamgames.com/doc/steamhardware/proton?l=english)
- [DXVK 官方仓库](https://github.com/doitsujin/DXVK)
- [DXVK 驱动要求](https://github.com/doitsujin/dxvk/wiki/Driver-support)
- [vkd3d-proton 官方仓库与驱动要求](https://github.com/HansKristian-Work/vkd3d-proton)
- [Mesa 平台与驱动](https://docs.mesa3d.org/systems.html)
- [Mesa RADV 架构](https://docs.mesa3d.org/drivers/radv.html)
- [Gamescope 官方仓库](https://github.com/ValveSoftware/gamescope)
- [Steam Runtime 官方仓库](https://github.com/ValveSoftware/steam-runtime)
- [Monado：OpenXR 运行时能力与边界](https://monado.freedesktop.org/about-runtimes.html)
- [Monado SteamVR 插件边界](https://monado.freedesktop.org/steamvr.html)
- [SteamVR for Linux](https://github.com/ValveSoftware/SteamVR-for-Linux)
- [libratbag 游戏外设架构与设备库](https://github.com/libratbag/libratbag)
- [SteamTinkerLaunch](https://github.com/sonic2kk/steamtinkerlaunch)
- [Mod Organizer 2](https://github.com/ModOrganizer2/modorganizer)
- [BepInEx](https://github.com/BepInEx/BepInEx)
- [Khronos Vulkan 规范](https://registry.khronos.org/vulkan/specs/latest/html/vkspec.html)
- [FEX 官方仓库](https://github.com/FEX-Emu/FEX)
- [Box64 官方仓库](https://github.com/ptitSeb/box64)

### Office、标准与企业互操作

- [ECMA-376 Office Open XML](https://ecma-international.org/publications-and-standards/standards/ecma-376/)
- [OASIS OpenDocument 1.3](https://www.oasis-open.org/standard/open-document-format-for-office-applications-opendocument-version-1-3/)
- [Microsoft Office 文件格式文档总览](https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-offdi/24ed256c-eb5b-494e-b4f6-fb696ad2b4dc)
- [LibreOffice 许可证](https://www.libreoffice.org/licenses/)
- [LibreOffice VBA 支持边界](https://help.libreoffice.org/latest/en-US/text/sbasic/shared/vbasupport.html)
- [ONLYOFFICE Desktop Editors](https://helpcenter.onlyoffice.com/desktop/getting-started.aspx)
- [Microsoft 365 系统要求](https://support.microsoft.com/en-us/office/system-requirements/system-requirements-for-microsoft-365-for-business-education-and-government-use)
- [Microsoft Office Scripts 与 VBA 对比](https://learn.microsoft.com/en-us/office/dev/scripts/resources/vba-differences)
- [Microsoft Power Query 平台/版本数据源差异](https://support.microsoft.com/en-US/Excel/power-query-data-sources-in-excel-versions)
- [Microsoft 365 共同创作](https://support.microsoft.com/en-us/sharepoint/get-started-with-sharepoint/document-collaboration-and-co-authoring)
- [Microsoft 字体再分发 FAQ](https://learn.microsoft.com/en-my/typography/fonts/font-faq)
- [Samba：What is Samba](https://www.samba.org/samba/what_is_samba.html)

### 虚拟化与文件

- [QEMU 许可证](https://www.qemu.org/docs/master/about/license.html)
- [QEMU 安全模型](https://www.qemu.org/docs/master/system/security.html)
- [QEMU virtio-gpu](https://www.qemu.org/docs/master/system/devices/virtio/virtio-gpu.html)
- [FreeRDP RemoteApp](https://github.com/FreeRDP/FreeRDP/wiki/RemoteApp)
- [Linux NTFS3](https://cdn.kernel.org/doc/html/latest/filesystems/ntfs3.html)
- [NTFS-3G Windows 休眠与 Fast Startup 注意事项](https://github.com/tuxera/ntfs-3g/wiki/Manual)
- [ISO 32000-2 / PDF 2.0](https://pdfa.org/sponsored-standards/)
- [W3C EPUB 3.3](https://www.w3.org/TR/epub-33/)
- [FFmpeg 法律与许可证](https://ffmpeg.org/legal.html)
- [ImageMagick 格式清单](https://imagemagick.org/formats/)
- [Ghostscript 简介与许可证](https://ghostscript.com/about/)
- [libarchive 官方说明](https://www.libarchive.org/)
- [7-Zip 格式与许可证](https://www.7-zip.org/)
- [Pandoc 格式清单](https://pandoc.org/)
- [Calibre DRM 边界](https://manual.calibre-ebook.com/en/drm.html)
