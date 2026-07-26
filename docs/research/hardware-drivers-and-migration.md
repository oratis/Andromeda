# Andromeda 硬件、驱动与 Windows/macOS 无缝迁移深度研究

> 调研日期：2026-07-26
> 范围：x86-64 PC、通用 arm64 PC、Intel Mac、带 T2 的 Intel Mac、Apple silicon Mac；启动、固件、驱动、功耗、硬件认证，以及从 Windows/macOS 迁移用户状态。
> 结论等级：本文区分“上游已有”“可产品化”“需要实机验证”和“当前不可承诺”，避免把“Linux 社区里有人启动成功”误写成 Andromeda 产品支持。

## 1. 执行摘要

### 1.1 核心判断

1. **“运行在所有硬件上”必须改写成“可探测所有硬件，并对通过认证的硬件作明确承诺”。** PC 的主板固件、ACPI 表、设备子系统、固件版本和 OEM 改型形成组合爆炸。即使 PCI/USB ID 相同，电源时序、音频拓扑、摄像头 ISP、休眠和混合显卡也可能因机型而异。一个负责任的 OS 不能只给出“支持/不支持”二元答案，而应发布机器可读的 Hardware Compatibility Manifest（HCM）和分级支持状态。
2. **主航道应是 Linux LTS 内核 + 上游驱动 + Mesa + linux-firmware。** Linux 内核同时覆盖 x86-64 和 arm64，已有 PC 所需的 PCIe、USB、NVMe、网络、输入、音频、图形和电源管理框架。原创投入应集中在认证、质量、回归隔离、固件治理和体验，而不是另写一套驱动 ABI。Linux 内核整体采用 GPL-2.0-only；大量设备仍依赖可再分发但非自由的二进制固件，内核官方也明确说明，拒绝这些固件通常会以大量硬件失效为代价（[Kernel.org FAQ](https://www.kernel.org/faq.html)）。
3. **PC 游戏要求同时经营开放和闭源 GPU 路线。** AMD/Intel 的内核 DRM 与 Mesa 路线最适合作为默认；NVIDIA 若要覆盖现实游戏硬件，不能只依赖 Nouveau/NVK，必须提供经过 Secure Boot 签名、与内核更新锁步测试的 NVIDIA 官方驱动渠道。NVK 已是 Vulkan 1.4 conformant，并覆盖 Kepler 至 Ada 以及消费级 Blackwell，但“API 一致性”不等于所有游戏、CUDA、功耗和新卡首发体验都等价于官方栈（[Mesa NVK](https://docs.mesa3d.org/drivers/nvk.html)）。
4. **Mac 必须拆成三条产品线。**
   - 不带 T2 的 Intel Mac：最接近普通 x86 PC，可对少数实测机型提供 `Supported`，但不能按“所有 Intel Mac”承诺。
   - 带 T2 的 Intel Mac：键盘、触控板、摄像头、音频等依赖 `apple-bce` 桥接；休眠、音频、混合显卡和 T2 Secure Enclave 仍有关键缺口，只适合 `Community/Experimental`。
   - Apple silicon：不是 UEFI PC，必须采用 Asahi 的 m1n1/U-Boot、Apple Device Tree 和配对固件体系。M1/M2 可做独立预览版；截至调研日，Asahi 对 M3/M4 仍无可交付安装路径，其[官方功能总览](https://asahilinux.org/docs/platform/feature-support/overview/)也尚未列出 M5，因此 M3 及更新代际都不能列入 Andromeda 可交付范围。
5. **“从 Windows/macOS 无缝切换”应通过源系统迁移代理实现，而不是赌目标系统能直接读写 NTFS/APFS。** Linux 的 NTFS3 已可完整读写 NTFS 3.1（[内核文档](https://docs.kernel.org/filesystems/ntfs3.html)）；但 Linux APFS 写支持仍属实验性质，且 FileVault、APFS volume group、云占位文件和 Keychain 都无法靠裸挂载正确迁移。因此应在原 Windows/macOS 内运行只读盘点与导出代理，经端到端加密、可续传协议传给 Andromeda。
6. **驱动支持是持续服务，不是安装镜像里的静态功能。** 发布门槛必须包括启动、安装、OTA/回滚、休眠循环、坞站热插拔、GPU、摄像头、音频、网络、功耗和固件升级的实机 CI。HCM 只有在指定固件和系统版本通过这些测试后才能签名发布。

### 1.2 建议的首发承诺

| 平台 | 首发承诺 | 原因 |
| --- | --- | --- |
| 近 4–6 年、通过实验室认证的 Intel/AMD x86-64 台式机和笔记本 | Certified / Supported | UEFI、ACPI、TPM2、主流 GPU/NVMe/Wi-Fi 已有成熟上游基础 |
| 未认证但能被通用 Linux 支持的 x86-64 PC | Community | 可尝试 Live 硬件检查，但不承诺休眠、指纹、摄像头和功耗 |
| 标准化 UEFI+ACPI 的 arm64 PC | Pilot | arm64 内核成熟，但消费级板卡仍常依赖每板 DT 和厂商固件 |
| 不带 T2 的选定 Intel Mac | Supported（机型白名单） | 基本启动和主流 Intel/AMD 设备可用，但 Apple SMC、摄像头、触控板、显卡切换等需逐机型验证 |
| 带 T2 的 Intel Mac | Experimental | 关键桥接驱动未完整上游，Suspend、音频、SEP/Touch ID 仍有缺口 |
| 选定 M1/M2 Mac | Developer Preview，后续逐机型升级 | Asahi 已实现大量功能，但 Thunderbolt/DP、SEP/Touch ID、视频编解码、ANE、上游休眠仍不完整 |
| M3/M4 Mac | Unsupported / Watch | 截至 2026-07-26，Asahi 官方功能表仍显示安装器不可用 |
| M5 及尚未列入 Asahi 功能表的更新 Mac | Unsupported / Watch | 不从 CPU 名称推断兼容；等待上游建立对应机型页、安装器和核心驱动 |

## 2. “所有硬件”的工程定义

### 2.1 不能用 CPU 架构代表整机支持

支持 `x86_64` 或 `aarch64` 只说明内核能执行该指令集。完整桌面 OS 至少还要解决：

- 固件如何描述和初始化设备；
- IOMMU、中断控制器、时钟、GPIO、热管理和供电时序；
- 内屏、外屏、GPU 显存、电源状态和混合显卡；
- NVMe、SATA、RAID/VMD、磁盘加密与掉电恢复；
- Wi-Fi/蓝牙芯片对应的二进制固件和射频区域规则；
- 声卡 codec、功放、麦克风阵列和每机型 DSP 参数；
- MIPI 摄像头传感器、ISP 和每传感器调校；
- 键盘、触控板、触屏、手写笔、指纹和设备 quirks；
- 关盖、S0ix/s2idle、S3、休眠、唤醒源和电池统计；
- USB-C PD、DP Alt Mode、Thunderbolt/USB4、坞站与 DMA 防护。

因此 Andromeda 对外应使用三个不同动词：

- **Recognized**：安装器能识别设备和风险；
- **Works**：某个组件在一次测试中可用；
- **Supported**：指定整机、固件、Andromeda 版本和功能集合持续通过回归并有修复承诺。

### 2.2 架构覆盖

#### x86-64

这是 Andromeda 的主产品线。大多数现代 PC 由 UEFI 启动，以 ACPI 描述不可自枚举设备和电源管理；PCI/USB/NVMe 设备再由总线枚举。UEFI Forum 将 ACPI 定义为 OS-directed configuration and power management 的核心标准，当前规范入口同时维护 UEFI 2.11 和 ACPI 6.6（[UEFI 规范页](https://uefi.org/specifications)）。

难点不是“有没有驱动”，而是 OEM ACPI AML、BIOS 版本和设备组合的质量。Linux NVMe 维护者也明确把 quirks 视为对量产硬件缺陷的最后手段，而不是替代厂商正确实现规范（[NVMe feature and quirk policy](https://docs.kernel.org/6.15/nvme/feature-and-quirk-policy.html)）。

#### arm64

arm64 存在两类平台：

- 服务器/标准 PC 风格：UEFI + ACPI，通常遵循 Arm BSA/BBR；
- 消费 SoC/开发板风格：Bootloader + Flattened Device Tree，每一块板都需要正确 DT 和驱动。

Linux 明确要求 arm64 的 ACPI 平台通过 UEFI 传递 ACPI 表，内核在启动时选择 ACPI 或 DT，而不是同时混用（[Linux arm64 ACPI](https://docs.kernel.org/6.12/arch/arm64/arm-acpi.html)）。这意味着“arm64 ISO”仍不能自动支持任意手机/开发板/Apple SoC；Andromeda v1 应只接纳满足 UEFI+ACPI 的标准 arm64 PC，以及有专用 board enablement 的少数设备。

### 2.3 启动、安全和恢复基线

#### 通用 PC

建议启动链：

`UEFI Secure Boot → 签名 UKI（kernel + initrd + cmdline）→ dm-verity/签名系统镜像 → TPM2 measured boot → 用户数据解密`

工程要求：

- 首发可借助被广泛信任的 shim 路线，长期建立 Andromeda 自有密钥、吊销和应急轮换；
- 所有内核模块必须来自受控仓库并签名；闭源 GPU 模块也必须进入同一更新事务；
- 记录 PCR/event log，但不能把固件偶然变化误判成用户数据永久不可解密；
- 解密策略至少有 TPM 封存、恢复密钥和用户凭据三层；
- 新内核/驱动只能在健康检查通过后“bless”。systemd 已提供 boot counting 和失败后回退前一启动项的机制（[Automatic Boot Assessment](https://systemd.io/AUTOMATIC_BOOT_ASSESSMENT/)），也定义了从 UEFI 到用户空间的 PCR 测量（[TPM2 PCR Measurements](https://systemd.io/TPM2_PCR_MEASUREMENTS/)）。

#### coreboot 的位置

coreboot 采用 GPLv2，能初始化硬件后把控制权交给 payload，也支持 VBOOT2、IOMMU、flash protection 等能力（[官方 OEM/ODM 说明](https://coreboot.org/oems_and_odms.html)）。但它是**逐主板移植的固件项目**，不是让任意零售 PC 获得统一固件的魔法层。Andromeda 可以：

- 在未来自有参考硬件/OEM 合作硬件上 Pilot coreboot；
- 用 coreboot + EDK2/UEFI payload 建立可控启动链；
- 不在现有消费 PC 上自动替换 OEM UEFI，也不把刷写 coreboot 作为安装条件。

### 2.4 为什么不能普遍复用 Windows/macOS 驱动

驱动不是一个只包含“如何读写硬件寄存器”的可移植插件，而是目标 OS 内核、设备模型和安全边界的一部分。Windows 的 WDM/KMDF/UMDF 驱动围绕 IRP、PnP/电源回调、WDF 对象、Windows 内核服务和对应版本的框架库构建；微软也明确说明 WDM 与操作系统紧密耦合，WDF 驱动加载时要链接并加载相应的 Windows framework library（[WDM 与 WDF 的差异](https://learn.microsoft.com/en-us/windows-hardware/drivers/wdf/differences-between-wdm-and-kmdf)、[WDF 构建与加载](https://learn.microsoft.com/en-us/windows-hardware/drivers/wdf/building-and-loading-a-kmdf-driver)）。macOS 驱动则依赖 IOKit/kext 或 DriverKit/System Extensions 的对象模型、entitlement、签名与公证；Apple silicon kext 还必须符合 `arm64e` 和指针认证要求（[Apple 驱动与系统扩展](https://developer.apple.com/documentation/systemextensions/implementing-drivers-system-extensions-and-kexts)）。

Linux 的驱动绑定、总线、DMA、IOMMU、中断、内存管理、网络、图形、音频和电源回调均不同。因此仅有 Windows `.sys/.inf` 或 macOS `.kext/.dext` 二进制时，Andromeda 通常既无法解析其内核调用，也无法安全地把设备接入 Linux 的 DRM、netdev、ALSA、V4L2、input 等子系统。即使 CPU 指令集相同，也还存在：

- 目标内核 ABI/API、结构布局、锁和中断语义不同；
- 固件加载、ACPI/Device Tree 属性和设备匹配方式不同；
- DMA 映射、IOMMU 隔离、缓存一致性与电源状态机不同；
- 用户空间配套库、后台服务、注册表/IORegistry 和控制面缺失；
- 驱动签名、Secure Boot、entitlement、许可证和再分发限制；
- 厂商驱动常依赖未公开协议、校准数据或与 OS 私有组件共同升级。

历史上的 `ndiswrapper` 说明“兼容层”只在非常窄的接口上可能成立，而不是通用答案。它在 Linux 内核中实现旧版 Windows NDIS 来运行部分无线网卡驱动；项目自身说明主要支持 Windows XP 时代的 NDIS 5，对 Windows 7/10 使用的 NDIS 6 实现不完整、对最终用户无用（[ndiswrapper README](https://github.com/pgiri/ndiswrapper)）。这条路线无法覆盖现代 WDF、macOS DriverKit，更绕不开闭源代码在 Linux 内核权限下运行所带来的安全、内核更新兼容、休眠和调试风险。**Andromeda 不应把 ndiswrapper 或任何通用二进制驱动包装器列为产品依赖。**

#### 可用的四级逃生路径

1. **Linux 上游原生驱动（默认且唯一可进入 Certified 的长期路线）。** 优先采用已有内核子系统和驱动；缺失时与社区、芯片厂和 OEM 共同把驱动、固件接口、Device Tree/ACPI 支持及测试送入上游。确需短期 out-of-tree 模块时只能进入 `Pilot/Experimental`，设定上游负责人、截止版本和淘汰条件。
2. **用户态实现公开或已合法获得的设备协议。** 对 USB、HID、串口、MTP/PTP、扫描、打印以及某些工业 PCI 设备，可用 `libusb`/usbfs、SANE、CUPS 或小型 UIO/VFIO 内核门面在沙箱中实现协议；Linux 官方也支持通过 usbfs 编写用户态 USB 驱动，并指出 UIO 适合不需要标准内核子系统的简单设备（[Linux USB host API](https://docs.kernel.org/driver-api/usb/usb.html)、[UIO HOWTO](https://docs.kernel.org/driver-api/uio-howto.html)）。这不是绕过协议授权或固件许可证的许可，也不适合显示、通用网卡、系统盘等深度耦合子系统。
3. **把 USB/PCI 设备直通给 Windows VM（兼容性后备，不等于宿主驱动支持）。** 当专有外设只有 Windows 驱动时，可将独立 USB interface/device 或完整 IOMMU group 交给受管理的 Windows VM，在 VM 内运行原厂驱动与应用。PCI 路线使用 KVM/VFIO；内核文档将 VFIO 定义为受 IOMMU 保护的直接设备访问框架，并明确 VM 可借此使用裸机驱动（[Linux VFIO](https://docs.kernel.org/driver-api/vfio.html)）。产品必须提示：直通后宿主不能同时使用该设备；PCI 多功能设备可能必须整组移交；热插拔、睡眠、实时音视频、许可证和 Windows 激活均需单独处理。内置键盘、系统盘、启动 GPU、无安全 IOMMU 分组的设备不得作为通用直通承诺。
4. **厂商/OEM 合作。** 对 GPU、NPU、MIPI ISP、Smart Amp、指纹、Thunderbolt retimer 等无法靠公开规范完成的部件，争取 Linux 原生驱动、公开寄存器/协议文档、可再分发固件、校准数据、签名服务、参考板和回归样机。合同应包含上游许可、CVE 响应、内核版本支持期、固件撤回/轮换和停产后的维护义务；若厂商只给不可审计且与特定内核锁死的 binary blob，该机型最高只能是 `Supported` 的受限渠道，不能成为参考平台。

安装器发现无原生驱动设备时，建议按“上游原生 → 用户态协议 → Windows VM 直通 → 厂商合作/暂不支持”的顺序解释选择，并在 HCM 中分别记录宿主支持和 VM 兼容状态，绝不把“VM 里能用”显示成“系统已支持”。

## 3. 通用 PC 驱动栈

### 3.1 GPU 与显示

| 硬件 | 推荐栈 | 现实成熟度 | Andromeda 策略 |
| --- | --- | --- | --- |
| Intel iGPU/dGPU | 内核 i915/xe + Mesa Iris/ANV | 高；新旧代际需分别回归 | 默认 Adopt，认证多屏、HDR/VRR、视频编解码、suspend |
| AMD APU/dGPU | 内核 amdgpu + Mesa RadeonSI/RADV | 高；开放栈最完整之一 | 默认 Adopt，游戏参考硬件优先 |
| NVIDIA Turing 及更新 | NVIDIA open GPU kernel modules + 官方用户态 | 游戏和 CUDA 现实必需；用户态仍受专有许可控制 | 独立受控仓库 Adopt，内核/模块/用户态锁步发布 |
| NVIDIA 开放社区栈 | Nouveau 内核 + Mesa NVK/Zink | API 一致性快速成熟；功耗、CUDA、全游戏覆盖仍不等同官方栈 | Pilot；作为无专有驱动选择和恢复后备 |
| Apple AGX | Asahi DRM + Mesa Asahi/Honeykrisp | M1/M2 可用且已取得图形 API conformant；与通用 PC 分支不同 | 仅 Apple silicon 镜像 Pilot |

必须额外验证：

- Wayland compositor、XWayland、HDR、VRR、色彩管理、fractional scaling；
- 独显直连与 MUXless PRIME offload；
- 外接显示器热插拔、USB-C/DP/HDMI、休眠后恢复；
- Vulkan/OpenGL CTS、DXVK/vkd3d-proton、视频 VA-API；
- GPU reset 后桌面是否可恢复，不能把单个游戏挂死升级成系统重启；
- NVIDIA 内核模块与 Secure Boot、内核 ABI 和 OTA 的原子性。

对 Apple GPU，Mesa 文档明确其 Asahi 驱动源于对 AGX 的逆向工程（[Mesa Asahi](https://docs.mesa3d.org/drivers/asahi.html)）；Fedora Asahi 当前宣称 M1/M2 已提供 conformant OpenGL 4.6、OpenGL ES 3.2、OpenCL 3.0 和 Vulkan 1.4（[Fedora Asahi](https://asahilinux.org/fedora/)）。这证明路线可行，但不应外推到 M3 及更新代际。

### 3.2 Wi-Fi 与蓝牙

Linux Wi-Fi 的公共框架是 cfg80211/mac80211/nl80211，各厂商驱动位于其上或采用 FullMAC；官方文档覆盖新的 802.11 驱动和用户空间接口（[Linux Wireless](https://wireless.docs.kernel.org/en/latest/)）。Intel `iwlwifi` 对 Wi-Fi 6/6E/7 已有广泛上游支持，但仍需匹配正确 `linux-firmware`；官方明确说明驱动、固件和用户空间组合需要共同验证（[iwlwifi](https://wireless.docs.kernel.org/en/latest/en/users/drivers/iwlwifi.html)）。

产品策略：

- 优先认证 Intel、Qualcomm/Atheros、MediaTek 等上游良好的器件；
- Broadcom 每个 PCI/SDIO 芯片与 board calibration 都要单独管理，Mac 尤其如此；
- HCM 必须记录固件 SHA-256、regulatory database 版本和验证过的 AP 模式；
- Wi-Fi/蓝牙共存要测试 2.4 GHz、5/6 GHz、耳机通话、手柄和睡眠唤醒；
- 默认 BlueZ，蓝牙音频走 PipeWire/WirePlumber；
- 不从用户的 Windows/macOS 安装中静默复制固件；只有在许可证允许且用户拥有相应硬件时，由本机提取器完成，并记录来源和哈希。

### 3.3 音频

底层采用 ALSA/ASoC，媒体图和兼容接口采用 PipeWire + WirePlumber。PipeWire 提供低延迟图、PulseAudio/JACK/ALSA/GStreamer 兼容，并专门支持沙箱应用的设备授权（[PipeWire](https://pipewire.org/)）。Intel/AMD DSP 机型还必须把 [Sound Open Firmware](https://www.sofproject.org/) 固件、内核驱动、拓扑文件和 [ALSA UCM 配置](https://github.com/alsa-project/alsa-ucm-conf)作为同一 HCM 组合验证，不能只记录“声卡驱动已加载”。

真正的产品缺口通常不在 PCM 播放，而在：

- 每机型 UCM profile；
- codec/amp 拓扑、插孔检测；
- 多扬声器相位、EQ、limiter 和保护算法；
- 麦克风阵列、AEC/降噪和 privacy LED；
- HDMI/DP/USB/Bluetooth 路由切换；
- 休眠后设备重建。

扬声器参数错误有物理损坏风险。任何 Smart Amp 或机型 DSP 配置都必须精确绑定 HCM 的 board ID，经过声压、温升、削波和故障注入测试；未知机型默认禁用高功率模式，而不是猜测“相近型号参数”。

### 3.4 摄像头

外置 UVC 摄像头通常成熟；现代笔记本内置 MIPI 摄像头不是简单 UVC，而是传感器 + CSI receiver + ISP + 3A 算法 + privacy 控制。libcamera 的目标正是把复杂相机管线抽象为 Linux/Android/ChromeOS 共用框架（[libcamera](https://libcamera.org/)）。

当前重要限制：libcamera 官方 FAQ 表明 Intel IPU6 的主线内核只支持 Imaging System，Processing System 驱动缺失时可用软件 ISP，但会牺牲画质和电池（[libcamera FAQ](https://libcamera.org/faq.html)）。所以：

- UVC 可作为首发硬性要求；
- MIPI/IPU6 摄像头必须逐机型认证，不能仅凭“设备被枚举”算支持；
- 摄像头测试包括分辨率/帧率、曝光/白平衡、低光、Teams/浏览器/PipeWire portal、LED 和硬件隐私开关；
- 无成熟 ISP 的机器应明确显示“兼容模式：画质/功耗受限”。

### 3.5 输入、触控与指纹

libinput 统一鼠标、键盘、触控板、触屏、手写笔、TrackPoint 和机盖/平板模式开关，并维护大量 device quirks（[libinput](https://wayland.freedesktop.org/libinput/doc/latest/what-is-libinput.html)）。它应直接 Adopt，但 Andromeda 需要在其上提供：

- 安装前输入设备检查；内置键盘/触控板失败时必须阻止无提示安装；
- 触控板手势、palm rejection、点击力度和加速度的机型测试；
- 可访问性备用输入；
- haptic touchpad 的力反馈安全与电源管理；
- 游戏手柄走 evdev/SDL，不错误塞进 libinput。

指纹采用 fprintd/libfprint，但支持按精确 USB ID/ACPI ID 计算，不能宣传为通用能力。libfprint 官方设备页也说明开发版支持项未必存在于稳定发行版（[supported devices](https://fprint.freedesktop.org/supported-devices.html)）。指纹只用于便利解锁，不代替磁盘恢复密钥；模板不能从 Windows Hello、Touch ID 或 T2/SEP 迁移，必须重新录入。

### 3.6 Thunderbolt、USB4 和坞站

Linux 内核同时支持固件 connection manager 和软件 connection manager。USB4 基于公开的 Thunderbolt 3 协议；Apple 机器通常使用软件 connection manager。PCIe tunneling 设备具有 DMA 能力，默认授权策略必须和 IOMMU 联动（[Linux USB4/Thunderbolt](https://docs.kernel.org/admin-guide/thunderbolt.html)）。

Andromeda 应：

- 采用内核 Thunderbolt/USB4 + `bolt` 用户空间授权；
- 有 IOMMU DMA protection 时才允许“记住并自动连接”PCIe 隧道设备；
- 无 IOMMU 时默认提示，禁止静默授权未知设备；
- 认证坞站的 USB、DP、以太网、音频、PD、睡眠与热插拔组合；
- 外接 NVMe 断开前强制卸载并解释数据风险；
- 通过 fwupd/LVFS 更新设备/retimer 固件，但 Apple Mac host NVM 不能假设可由 fwupd 更新；内核文档明确指出 Apple Mac 的 host NVM upgrade 不受支持。

### 3.7 存储、打印与扫描

- NVMe/SATA/AHCI 采用内核上游驱动，Intel VMD/RAID 模式需安装器预检；
- 默认文件系统应支持快照、校验和、原子系统升级；用户数据与系统镜像分离；
- NTFS3 用于迁移和兼容，但源 Windows 卷默认只读挂载，确认 Windows 已完全关机且 BitLocker 已解锁后才允许写；
- APFS 仅作为只读/实验诊断，不作为正式迁移主路径。现有 `apfsprogs` 自称 experimental、`apfsck` 主要供测试者使用（[项目说明](https://github.com/linux-apfs/apfsprogs)）；
- 打印采用 CUPS 的 driverless IPP Everywhere；OpenPrinting 明确表示符合 IPP driverless 标准的打印机可持续使用，旧设备通过 Printer Applications 过渡（[CUPS drivers](https://openprinting.github.io/cups/drivers.html)）；
- 扫描采用 SANE/eSCL/WSD，支持状态按官方设备数据库纳入 HCM，而不是泛称“扫描仪支持”（[SANE supported devices](https://www.sane-project.org/sane-supported-devices.html)）。

### 3.8 休眠、功耗和热管理

这是笔记本认证中最容易被低估的部分，也是“macOS 不容易坏”体验的核心。每台认证机必须至少测：

- s2idle/S0ix 或 S3 的进入和退出；
- 关盖、开盖、电源键、RTC、USB、蓝牙、网卡等唤醒源；
- 100/500 次循环的失败率；
- 睡眠 8 小时掉电、机身温升和异常唤醒；
- 电池容量、充电阈值、USB-C PD、低电量关机；
- CPU/GPU/NPU/独显 runtime PM；
- 风扇、热区、功率限制和 emergency shutdown；
- 休眠前后摄像头、音频、Wi-Fi、外屏和磁盘一致性。

“可 suspend 一次”不等于支持。任何会随机丢盘、黑屏、风扇失控或耗尽电池的机器不得进入 Certified。

运行时采用 UPower 统一电池/供电状态，`power-profiles-daemon` 提供面向桌面的有限功耗档位；Intel 平台可 Pilot `thermald`，服务器/工作站策略可评估 TuneD，但两者都不能覆盖 OEM 固件缺陷。认证流水线使用 FWTS 检查 ACPI/UEFI，使用 `pm-graph` 定位 suspend/resume 时间线，并把 `pciutils`、`usbutils`、`hwdata` 的规范化设备清单写入 HCM 证据。调优参数必须绑定精确机型和固件版本，通用“省电脚本”不得进入所有机器的默认配置。

### 3.9 NPU

Linux 已建立 `/dev/accel/accel*` 的 compute accelerator 子系统，并复用 DRM 的内存管理和同步基础（[内核 accel 介绍](https://docs.kernel.org/accel/introduction.html)）。但 NPU 尚不是跨厂商统一运行时：

- Intel NPU 从 Core Ultra/Meteor Lake 起有 `ivpu` 内核驱动和 Intel Level Zero/OpenVINO 用户态；Intel 官方仓库仍持续发布，但 2026 OpenVINO 对 Linux NPU 支持仍标为特定 Ubuntu 版本/preview，并要求额外驱动（[OpenVINO system requirements](https://docs.openvino.ai/2026/about-openvino/release-notes-openvino/system-requirements.html?language=en)）。
- AMD `amdxdna` 已进入内核 accel 子系统，支持 Phoenix/Hawk Point/Strix Point，用户态仍需要 XRT shim 与编译器（[内核 AMD XDNA](https://docs.kernel.org/accel/amdxdna/amdnpu.html)）。
- Apple ANE 在 M1/M2 的 Asahi 功能表中仍是 out-of-tree，M3/M4 为 TBA，更新代际尚无可验证条目；不可作为首发本地 AI 后端。

因此 Andromeda AI Runtime 应建立 CPU/GPU/NPU provider 抽象和能力探测：

1. 模型按算子、精度、内存和隐私要求选择后端；
2. NPU 不可用时可靠回落到 Vulkan/OpenCL/CPU；
3. 模型编译缓存按驱动、固件、设备 ID 和编译器版本隔离；
4. NPU 驱动崩溃不能拖垮桌面；
5. 首发把 Intel/AMD NPU 标为 Pilot，把 Apple ANE 标为 Watch，不能让“AI OS”依赖尚不稳定的专用加速器。

## 4. Mac 三条路线的具体边界

### 4.1 不带 T2 的 Intel Mac

#### 技术画像

这类 Mac 是 x86-64 + Apple EFI，Linux 基本可以通过 UEFI 启动。Apple 平台安全文档确认，无 T2 的 Intel Mac 不支持硬件 Secure Boot，UEFI 会不验证地加载 `boot.efi`（[Intel Mac boot process](https://support.apple.com/en-euro/guide/security/sec5d0fab7c6/web)）。

主要特殊点：

- Apple SMC：电池、温度、风扇、键盘背光等；
- Apple GMUX/混合显卡和部分 AMD dGPU；
- Broadcom Wi-Fi/蓝牙及机型校准固件；
- FaceTime HD 摄像头和部分需要从 macOS/Boot Camp 提取的固件；
- Apple SPI/USB 键盘、触控板、Touch Bar/T1；
- 高 DPI、内屏亮度、音频 codec/amp 拓扑；
- EFI 变量、启动项、睡眠和 Thunderbolt quirks；
- 机型普遍年限较长，OEM 固件更新和上游回归价值下降。

#### 可交付边界

**可以承诺：**

- 只对白名单机型承诺启动、安装、基本显示、NVMe/SATA、USB、以太网；
- 对实测通过的机型提供 Wi-Fi、蓝牙、声卡、摄像头、键盘/触控板和 suspend；
- 使用源 macOS 迁移代理读取 APFS/FileVault 数据，不要求 Linux 直接写 APFS；
- 保留 Apple Recovery/EFI 恢复路径。

**不能承诺：**

- “所有 2006–2020 Intel Mac”统一支持；
- Touch ID/T1 安全能力、Boot Camp 驱动原样复用；
- 每台机器达到 macOS 的风扇、续航、扬声器和 Force Touch 体验；
- 已被 Apple 停止更新的硬件仍获得等价固件安全维护。

建议首批只选 3–5 个社区资料充分、无 T2、Intel iGPU 为主的 MacBook Air/Pro 与 Mac mini，建立长期实验室样机；AMD 双显卡机型后置。

### 4.2 带 T2 的 Intel Mac

#### 技术画像

T2 不只是安全芯片。键盘、触控板、摄像头、音频等设备经 T2 的 Bridge/BCE 暴露给主 CPU，Linux 需要 `apple-bce` 创建通信通道和虚拟 USB host。t2linux 当前状态页显示：

- 内部 SSD、屏幕/iGPU、USB、键盘、Wi-Fi、摄像头可用；
- 触控板缺少 Force Touch/palm rejection；
- 蓝牙部分机型有 2.4 GHz 共存问题；
- suspend 在 macOS Sonoma 固件后被破坏；
- 音频、混合显卡和 AMD GPU 仍有稳定性问题；
- T2 Secure Enclave 和 T2 视频编码器不可用（[t2linux 状态](https://wiki.t2linux.org/state/)）。

`apple-bce` 仍需进入自定义内核/模块，t2linux 安装后指南要求专用内核、特定 IOMMU 参数，并把驱动塞进 initramfs 才能用内置键盘输入 LUKS 密码（[t2linux basic setup](https://wiki.t2linux.org/guides/postinstall/)）。Apple 官方又要求在 Recovery 的 Startup Security Utility 中显式降低安全级别并允许外部启动；默认 Full Security 不信任常见 Linux 第三方 UEFI CA（[Apple Startup Security Utility](https://support.apple.com/en-us/102522)）。

#### 可交付边界

**Experimental 可以交付：**

- 对 t2linux 已覆盖且实验室实测的少数机型提供独立镜像；
- 基本桌面、内部 SSD、键盘、触控板、摄像头、Wi-Fi；
- 明确提示用户需改变启动安全策略；
- 保留 macOS/Recovery 用于固件与恢复；
- 每个内核版本锁定 `apple-bce` 版本并提供失败回滚。

**v1 不可承诺：**

- Certified 笔记本质量；
- Touch ID、T2 Secure Enclave 密钥、Windows Hello/macOS 生物模板迁移；
- 稳定 suspend、macOS 等价音频和混合显卡切换；
- 在没有外接键鼠/网络后备的情况下保证救援安装；
- 通过复制 Boot Camp 二进制驱动解决 Linux 驱动缺口。

产品门槛：`apple-bce` 核心路径进入主线或 Andromeda 能承担长期 rebase、安全审计和 500 次 suspend CI 之前，不升级为 Supported。

### 4.3 Apple silicon Mac

#### 启动与固件现实

Apple silicon Mac **不使用 UEFI，也没有 EFI System Partition**。iBoot 从内部 NVMe 上的 APFS OS container 加载每 OS 配对的 iBoot2、Apple Device Tree 和 coprocessor firmware；第三方内核以 fuOS 进入每 OS 的 boot policy。Asahi 的平台说明指出：

- 机器允许第三方 OS，但第三方必须遵守“whatever macOS does”的 boot/firmware ABI；
- 安装自定义内核需 1TR、machine owner 身份和 Permissive Security；
- Apple 不授权第三方重新分发 system firmware，但 Mac 所有者被许可使用 Apple 提供的系统镜像；
- 每个 OS 带自己的一组配对固件，因此 coprocessor firmware ABI 没有稳定保证；
- 外部盘启动仍需把 preboot 结构放在内部存储（[Asahi Apple silicon introduction](https://asahilinux.org/docs/platform/introduction/)）。

Asahi 的 `m1n1` 把 Apple 启动 ABI 转换为适合 Linux 的环境，支持拼接 Linux kernel、DTB 和 initramfs，采用 MIT 许可证（[m1n1](https://github.com/AsahiLinux/m1n1)）。典型链路是：

`Apple SecureROM/iBoot → per-OS fuOS policy → m1n1 → U-Boot → Linux + DT`

M1/M2 的 16 KiB 原生页还影响 x86 游戏兼容；Asahi 的游戏栈用 4 KiB 页的轻量 VM + FEX + Wine + DXVK/vkd3d 解决，而不是要求主内核混用页大小（[Asahi AAA gaming](https://asahilinux.org/2024/10/aaa-gaming-on-asahi-linux/)）。

#### 当前功能边界

截至 2026-07-26：

- M1/M2：安装器可用，GPU、Wi-Fi、蓝牙、NVMe、键盘/触控板、摄像头和多数音频功能已有 `linux-asahi` 或部分主线支持；
- M1/M2：Thunderbolt、DP Alt Mode、视频解码仍 WIP；Touch ID 为 TBA；SEP WIP；ANE 为 out-of-tree；sleep/cpuidle 仍依赖 `linux-asahi` 特殊实现（[M1 功能表](https://asahilinux.org/docs/platform/feature-support/m1/)、[M2 功能表](https://asahilinux.org/docs/platform/feature-support/m2/)）；
- M3：安装器为 `no`，GPU、NVMe、USB、PCIe 等仍处于 WIP/TBA（[M3 功能表](https://asahilinux.org/docs/platform/feature-support/m3/)）；
- M4：安装器为 `no`，主要 SoC block 和整机功能仍为 TBA（[M4 功能表](https://asahilinux.org/docs/platform/feature-support/m4/)）。

#### 可交付边界

**M1/M2 Developer Preview 可以交付：**

- 从 Asahi Installer 生成安全的磁盘空间和 Apple 配对固件，不自己重写 APFS 分区逻辑；
- 采用 Asahi kernel、m1n1、U-Boot、Mesa 与音频 DSP 的完整版本组合；
- 对指定 M1/M2 机型承诺基本桌面、开发环境和源系统迁移；
- 图形栈可作为游戏/AI GPU Pilot，但明确 emulation overhead 和兼容矩阵；
- 保留 Apple system/recovery container；Andromeda 可成为默认日常 OS，但不能宣称摆脱 Apple firmware/recovery。

**v1 不可承诺：**

- M3 及更新 Apple silicon 安装；
- Touch ID/SEP、ANE、本地视频编解码、Thunderbolt/USB4/所有 USB-C 外屏；
- 抹掉所有 Apple 分区后仍正常升级固件和恢复；
- Apple 新 macOS 固件发布后 Andromeda 不受影响；
- macOS 应用二进制的直接兼容。

只有当目标机型的核心依赖主线化、Asahi 官方支持且 Andromeda 实机 CI 达标时，才从 Preview 升级为 Supported。遵循 Asahi distro guidelines：安装器不得自行改动 APFS container，必须使用官方安全腾挪空间机制，并承担自己的包、CDN、CI 和一线支持（[Asahi Distribution Guidelines](https://asahilinux.org/docs/alt/policy/)）。

## 5. 硬件支持等级

### Tier 0 — Blocked

安装器识别到已知数据损坏、无法恢复或无基本输入/存储风险，禁止安装：

- 内部盘控制器不稳定或安装器可能破坏 APFS/RAID；
- 无显示且无可验证串口/外屏路径；
- 固件/内核组合命中严重回归；
- 电池/风扇/热管理存在安全风险。

### Tier 1 — Community

- Live 环境能启动，CPU、内存、存储、一个显示输出和基本输入可用；
- 无质量、续航或长期修复承诺；
- 安装前显示缺失功能和已知问题；
- 用户可提交匿名 HCM probe 和测试结果，但社区报告不能自动升级认证。

### Tier 2 — Supported

- 安装、启动、更新和回滚通过；
- 日常必需的显示、存储、网络、输入、音频可用；
- 已知缺失不影响数据安全并在购买/安装前明确；
- 安全更新有 SLA；
- 指定机型、固件范围和外设集合进入物理 CI。

### Tier 3 — Certified

在 Tier 2 上增加：

- 内置摄像头/麦克风、蓝牙、外屏、坞站、指纹（如有）、休眠/唤醒和功耗均达到门槛；
- 100 次 nightly、500 次 release-candidate suspend/resume 无阻断故障；
- 固件更新、整机 OTA、低空间和掉电故障注入通过；
- 电池续航、待机掉电、热噪声不低于该硬件参考系统的可接受阈值；
- 厂商或 Andromeda 团队持有恢复工具、固件和替换样机。

### Tier 4 — Reference

Andromeda 与 OEM 共同控制 BOM、固件、校准、密钥、生命周期和出厂测试。只有这一层才可以接近 macOS 的“软硬件共同负责”可靠性。

支持等级是**每台整机 + 固件版本 + Andromeda release** 的属性，不是某个 CPU/GPU 的永久属性。

## 6. Hardware Compatibility Manifest（HCM）

### 6.1 目标

HCM 是签名、版本化、机器可读的硬件合同，同时服务于：

- 官网购买前兼容性查询；
- Live 安装器预检；
- 内核/initramfs/固件裁剪；
- OTA 灰度、暂停和回滚；
- 客服诊断；
- 实验室测试调度；
- AI agent 在执行驱动/固件操作前的 capability policy。

### 6.2 身份字段

```yaml
schema: andromeda.hcm/v1
machine:
  architecture: x86_64
  dmi:
    system_vendor: "Example OEM"
    product_name: "Laptop 14 Gen 3"
    product_sku: "21XX..."
    board_name: "..."
    bios_vendor: "..."
    bios_versions_tested: [">=1.18", "<1.24"]
  apple:
    model_identifier: null
    board_id: null
    soc: null
devices:
  - bus: pci
    address_class: "0300"
    vendor_device: "1002:1681"
    subsystem: "17aa:xxxx"
    driver: amdgpu
    firmware:
      package: linux-firmware
      files:
        - path: amdgpu/...
          sha256: "..."
          license_id: "..."
support:
  overall_tier: certified
  kernel:
    flavor: andromeda-lts
    min: "6.x.y"
    tested: ["..."]
  components:
    boot: pass
    installer: pass
    graphics: pass
    external_display: pass
    audio: pass
    microphone: pass
    camera: pass
    wifi: pass
    bluetooth: pass
    fingerprint: degraded
    suspend: pass
    hibernate: unsupported
    thunderbolt: pass
tests:
  suite_revision: "hwci-2026.07"
  last_full_pass: "2026-07-20"
  artifacts: ["sha256:..."]
known_issues:
  - id: AND-HW-1842
    severity: low
    summary: "..."
distribution:
  signature: "..."
  expires: "2026-10-26"
```

### 6.3 规则

- 整机主键由 DMI/Apple model + board/SKU 组成，不能只用营销名称；
- 每个 PCI 设备记录 VID:DID、subsystem VID:DID；USB 记录 VID:PID、bcdDevice；
- ACPI 记录 HID/CID/UID/DSD，DT 记录完整 `compatible`；
- 记录 BIOS/EC/SSD/Thunderbolt/PD controller/设备 firmware；
- 每个功能独立状态：`pass / degraded / unsupported / blocked / unknown`；
- HCM 有过期时间；过期后仍可启动，但不得自动扩大固件/内核灰度；
- 设备序列号、MAC、磁盘 UUID 默认只在本机匹配，不上传公共目录；
- probe 上传前做字段白名单和 k-anonymity，原始日志需用户单独授权；
- 云端 HCM 不能直接获得 root 权限；必须经本地签名验证、策略检查和事务引擎。

## 7. 硬件实验室与 CI

### 7.1 实验室结构

每个物理节点建议具备：

- 可远程断电/上电的 network PDU；
- PiKVM/HDMI capture、USB HID 注入和串口（如有）；
- 可控开盖/关盖或 Hall sensor fixture；
- USB-C PD/功率分析仪和交流功耗计；
- 可切换的 Wi-Fi AP（2.4/5/6 GHz、WPA2/3）；
- 蓝牙耳机、手柄、键鼠；
- 认证坞站、Thunderbolt/USB4 NVMe、双显示器；
- 音频 loopback、测量麦克风；扬声器测试增加温升与限幅；
- 摄像头测试卡、可控照度与 privacy LED 观测；
- SSD 备用件与硬件恢复器；
- Apple 节点额外保留一台受支持的 Mac 用于 DFU restore。

不要只收集“同 CPU”样机。笔记本必须按 OEM、主板 revision、Wi-Fi/camera panel 替换 BOM 建立变体。

### 7.2 分层测试

#### 每次内核/驱动变更

- QEMU x86-64/arm64 启动、initramfs、Secure Boot/UKI；
- 模块签名、firmware dependency、HCM schema；
- 参考 PC 冷启动、桌面登录、网络、存储、GPU smoke；
- 受影响设备的定向实机测试。

#### Nightly

- 关机/冷启动/重启；
- 100 次 suspend/resume；
- Wi-Fi 漫游、蓝牙音频、摄像头 portal；
- HDMI/DP/USB-C/坞站热插拔；
- GPU 基准与错误恢复；
- 低磁盘空间 OTA、boot counting、自动回退；
- firmware inventory 与 HCM drift。

#### Release candidate

- 全 Certified 机型完整套件；
- 500 次休眠循环和 8/24 小时待机；
- 跨两个旧稳定版本升级与回滚；
- 安装中掉电、更新中掉电、根分区校验失败；
- NVMe I/O fault injection、文件系统校验；
- Vulkan/OpenGL CTS 的规定子集与完整周期性 CTS；
- 主流 Steam/Proton 游戏、Office/打印/扫描迁移验收；
- 电池、热、风扇和噪声对照。

### 7.3 发布阻断规则

- 任一数据损坏、不可启动、风扇/温度安全问题：全量阻断；
- 指定机型黑屏/丢盘：撤回该 HCM 的更新资格，不拖累其他机型；
- firmware 与内核需锁步时，作为同一 staged transaction；
- 新设备只有连续两轮 RC 通过后可进入 Certified；
- 社区遥测只能发现线索，不能替代实验室通过；
- OTA 先 1% HCM cohort，再逐级扩大；异常率触发自动暂停。

## 8. Windows/macOS → Andromeda 无缝迁移协议

### 8.1 产品原则

“无缝”定义为：

- 用户能在安装前知道哪些数据、设置、应用和凭据可迁移；
- 源系统不被修改或删除；
- 传输可暂停、断点续传和验证；
- 目标写入是事务性的，失败可回滚；
- 应用按功能重新安装/映射，而不是复制不可运行二进制；
- 机密只在用户明确授权后导出，不能迁移的项目解释原因并给出下一步；
- 首次登录后提供可审计的迁移报告和待处理清单。

它不等于“复制整个用户目录后假设一切正常”。

### 8.2 组件

1. `andromeda-migrate-source.exe`：签名的 Windows 源代理；
2. `Andromeda Migrator.app`：公证/签名的 macOS 源代理；
3. Andromeda 安装器/首次设置中的 destination agent；
4. Portable Migration Manifest（PMM）：中立的版本化 schema；
5. 加密 chunk store：本地网络直传或外置盘离线传输；
6. app mapping catalog：源应用 → Linux 原生/Web/Wine/VM/替代品；
7. migration validator：哈希、数量、权限、可启动应用和抽样打开验证。

### 8.3 协议阶段

#### A. Pair

- 两端显示短码/QR；
- 使用一次性 PAKE 或经短码认证的 Noise/TLS 通道，双方生成临时密钥；
- 默认局域网点对点，不经云；
- 离线外置盘使用由恢复短语/硬件密钥保护的加密 bundle；
- 在屏幕上同时显示源/目标设备名称和证书指纹。

#### B. Inventory

源代理以普通用户权限扫描，确需管理员权限的项目单独解释。生成：

- 用户账户和 known folders；
- 文件、大小、哈希、时间、ACL/xattr/ADS、云占位状态；
- locale、timezone、keyboard、accessibility、wallpaper；
- 网络、打印机、浏览器 profile 元数据；
- 已安装应用、版本、许可证来源、文件关联；
- SSH/GPG、浏览器密码、系统 Keychain/DPAPI 等机密的**可迁移能力**，此时不导出明文。

Windows 的 USMT 已证明“文件 + 账户 + OS 设置 + 应用设置”的规则化迁移模型可行，并支持 include/exclude XML、离线 store 和校验（[USMT overview](https://learn.microsoft.com/en-us/windows/deployment/usmt/usmt-overview)）。Andromeda 可以借鉴分类与验证思想，但 PMM 必须是跨 OS schema，不能直接把 Windows registry 状态倒进 Linux。

#### C. Explain and select

UI 以四组呈现：

- **可原样迁移**：文档、图片、项目文件等；
- **可转换迁移**：壁纸、键位、Wi-Fi、打印机、浏览器书签；
- **可重装/映射**：应用及其支持程度；
- **必须重新登录/录入**：Passkey、Touch ID/Windows Hello、DRM、部分企业 VPN/证书。

显示总空间、预计时间、云端待下载大小、大小写文件名冲突、超长路径、权限损失和不受支持格式。用户可以逐项取消。

#### D. Snapshot and export

- 源代理要求关闭 Outlook、Office、数据库、浏览器等会持续写入的应用；
- 使用 VSS/APFS snapshot 或应用自身 export API 获得一致视图；
- 不存在可靠 snapshot 时标记为 best-effort 并在传输结束做二次增量；
- 文件按内容分块、压缩、哈希、AEAD 加密；
- manifest 单独签名；每个 chunk 可重试和去重；
- 源系统绝不删除数据。

#### E. Stage and transform

- 先写目标用户的 staging subvolume，不直接覆盖 home；
- Windows ACL、macOS ACL/xattr、NTFS ADS 保存在 sidecar metadata 中，能安全映射的才写 POSIX ACL/xattr；
- 处理 Unicode normalization、大小写冲突、Windows reserved names、symlink/alias；
- iCloud/OneDrive placeholder 优先要求源端 hydrate，或仅迁移 cloud reconnect token/路径映射，不把零字节占位符当成完成；
- 邮件、日历、联系人优先重新连接 IMAP/Exchange/CardDAV/CalDAV；本地 PST/mbox 作为专门转换任务；
- 每项转换有 deterministic version 和日志。

#### F. Reinstall and map applications

应用不直接复制：

| 源应用 | 处理 |
| --- | --- |
| 有 Linux 原生包 | 从签名仓库/Flatpak 重装，迁移经验证的配置 |
| Web/SaaS | 建立 PWA/浏览器入口，用户重新认证 |
| Windows 应用且 Wine 已认证 | 建独立 Wine prefix，安装原厂安装包，迁移白名单配置 |
| Windows-only Office/CAD/游戏 | 给出 Web、VM、远程 Windows 或兼容层选项，不伪称原生 |
| macOS-only 应用 | 建议导出开放格式或替代品；不复制 `.app` 期待运行 |
| 驱动/安全软件/系统扩展 | 不迁移；用 Andromeda 对应能力替代 |

Windows 可用 WinGet export 作为应用 inventory 的一个来源，但它只匹配可在 package source 中识别的应用，未匹配项会警告（[WinGet export](https://learn.microsoft.com/en-au/windows/package-manager/winget/export)）。macOS 开发工具可读取 Homebrew `brew bundle dump`，它能记录 formula/cask/Mac App Store/VS Code 等声明状态（[Homebrew Bundle](https://docs.brew.sh/Brew-Bundle-and-Brewfile)）。两者都只是输入，不能直接在 Andromeda 无审查执行。

#### G. Validate and commit

- 数量、总字节和内容哈希核对；
- 随机抽样打开 Office/PDF/图片/压缩包/项目文件；
- 检查 home 权限、SSH key mode、桌面/下载路径；
- 检查目标应用能启动且配置 schema 兼容；
- 用户查看报告后 commit staging snapshot；
- 保留可回滚快照和加密 migration bundle 到用户选择的期限；
- 明文临时秘密在导入后立即销毁。

### 8.4 凭据迁移红线

| 项目 | 策略 |
| --- | --- |
| 浏览器/密码管理器密码 | 只走用户触发的官方导出或 provider API；从源端加密通道直接写目标 secret store，不落明文 CSV |
| Wi-Fi 密码 | Windows/macOS 平台 API允许且用户授权时迁移；否则重新输入 |
| Passkey | 优先通过原 provider 同步；设备绑定 passkey 重新注册 |
| Windows Hello / Touch ID / 指纹模板 | 不迁移，目标重新录入 |
| DPAPI/Keychain machine-bound secrets | 不绕过系统保护；源应用显式导出或重新登录 |
| SSH/GPG 私钥 | 单项确认，保留权限和注释；硬件 token 只迁移配置，不导出私钥 |
| 企业证书/VPN/MDM | 由组织重新注册，不能把设备身份克隆到新 OS |
| BitLocker/FileVault recovery key | 可作为用户文档导入受保护 vault，但不自动变成 Andromeda 登录密钥 |
| 浏览器 cookie/session | 默认不迁移，防止复制活跃会话和绕过重新认证 |

Apple 和 Edge 都警告密码 CSV 是明文：Apple Passwords 导出后任何拿到文件的人都能看到密码，而且 Wi-Fi、部分共享密码和 Sign in with Apple 不能导出（[Apple Passwords export](https://support.apple.com/en-ca/guide/passwords/mchl35b12625/mac)）；Edge 同样要求用完后删除 CSV（[Edge export](https://support.microsoft.com/en-us/edge/export-passwords-in-microsoft-edge)）。因此 Andromeda 源代理应调用受支持导出能力后直接通过内存/pipe 加密传输，禁止让 AI agent 自行搜索用户目录里的密码库文件。

### 8.5 源文件系统策略

- **Windows**：优先在运行中的 Windows 通过 VSS/源代理读；离线 NTFS3 只作为救援路径，默认只读；
- **macOS**：优先在运行中的 macOS 通过 APFS snapshot/源代理读；不以 Linux APFS 写支持作为正式能力；
- **FileVault/BitLocker**：只在用户正常解锁的源 OS 中导出；
- **网络迁移**：源、目标都不要求共享 SMB root；使用应用层加密协议；
- **外置盘迁移**：bundle 自带 manifest、chunk hash、AEAD 和恢复短语；磁盘格式可用 exFAT，但机密性不依赖 exFAT；
- **旧机器不可启动**：提供只读救援并明确能力降级，尤其 Keychain、DPAPI、云占位和应用一致性可能无法恢复。

## 9. 驱动与固件治理

### 9.1 上游优先

驱动优先级：

1. Linux mainline；
2. 已被发行版长期维护、明确向上游推进的短期 patchset；
3. 厂商有稳定 ABI/签名/安全响应的外部模块；
4. 社区 out-of-tree driver，仅 Experimental；
5. 从 Windows/macOS 驱动逆向出来的二进制兼容层，不进入内核。

每个下游 patch 必须有：

- upstream issue/patch URL 和 owner；
- rebase 成本与删除条件；
- fuzz/static analysis；
- 覆盖机型和 CI；
- 安全响应联系人；
- 最大保留期限。

### 9.2 固件供应链

fwupd 的目标是让 Linux firmware update 自动、安全、可靠，LVFS 允许 OEM/ODM 上传并通过 UEFI capsule 或设备协议交付（[fwupd](https://github.com/fwupd/fwupd)、[LVFS introduction](https://lvfs.readthedocs.io/en/latest/intro.html)）。Andromeda 应 Adopt，但增加：

- LVFS metadata 和 payload 校验；
- HCM 精确匹配和 firmware allowlist；
- 先进入 Andromeda embargo/testing cohort；
- 电池电量、电源、恢复能力、BitLocker/FileVault 状态等 precondition；
- 升级前快照 OS 状态，但明确主板 firmware 不能靠文件系统快照恢复；
- firmware update 结果与 boot health 关联；
- 企业可配置 approved firmware；
- 不自动给 EOL 设备切换非厂商 firmware branch。

LVFS 的安全模型会核对 uploader vendor ID 与物理设备 USB/PCI/DMI 身份，限制厂商给其他厂商硬件发固件（[LVFS security](https://lvfs.readthedocs.io/en/latest/security.html)）。HCM 应保留同样的 vendor provenance，而不是让 AI 根据设备名字模糊匹配。

### 9.3 Apple/Boot Camp 固件与法律风险

Apple Boot Camp support software 是面向 Windows on Mac 的 Apple 驱动包，Apple 官方建议由 macOS Boot Camp Assistant 下载到 USB，再在 Windows 安装（[Apple Boot Camp drivers](https://support.apple.com/en-ie/102465)）。这不等于 Andromeda 获得重新分发、修改或在 Linux 中加载这些驱动的许可。

风险分层：

- **技术风险**：Windows `.sys` 使用 WDM/WDF/Windows kernel ABI，不能直接作为 Linux 驱动；固件虽可能是设备需要的 blob，但需要精确的上传协议、校准数据和版本配对；
- **版权/SLA 风险**：Asahi 的发行指南要求发行版不要再分发 Apple 专有固件，而应使用其安装/提取流程；这是一项重要的上游工程与合规边界，不是 Andromeda 的正式法律结论。Apple 软件的许可、用途与再分发权仍需逐项核对具体 [Apple SLA](https://www.apple.com/legal/sla/)；
- **DMCA/互操作性风险**：逆向工程例外因司法辖区而异，不能用工程判断代替法律意见；
- **商标/支持风险**：不能暗示 Apple 认证或支持 Andromeda；
- **安全风险**：从用户源系统提取的 blob 可能过旧、已被篡改或不适配当前 firmware。

建议政策：

1. Andromeda 镜像不打包未经明确授权的 Apple/Boot Camp 文件；
2. 若 Asahi/t2linux 已有“用户本机提取”机制，提取器只在 Apple 硬件上运行，验证文件 hash 和配对版本，不上传服务器；
3. 对 Apple CDN 下载仅使用已被上游项目审查的流程，并由法律顾问确认条款；
4. 为每个 blob 记录 `source_url / source_machine / license / sha256 / compatible_models`；
5. 驱动逆向采用 clean-room、公开接口记录和法律审查；
6. 商业发布前对 Apple EULA、Boot Camp SLA、固件再分发和当地互操作性例外出具正式法律意见。

## 10. 产品路线建议

### Phase H0：探测与数据（0–3 个月）

- 发布只读 Hardware Probe Live ISO；
- 定义 HCM v1 schema、签名和隐私规则；
- 建 20–30 台主流 PC + 3–5 台 Intel Mac + 代表性 T2/M1/M2 实验室；
- 建启动、设备枚举、GPU、网络、音频、摄像头、休眠 smoke；
- 完成 Windows/macOS source agent 的只读 inventory prototype；
- 暂不安装、不写 firmware、不写 APFS。

### Phase H1：认证 PC（3–9 个月）

- 选择 AMD APU、Intel iGPU、AMD dGPU、NVIDIA 各一条参考硬件线；
- 完成 UEFI Secure Boot、TPM2、UKI、模块签名与自动回退；
- Adopt Mesa/PipeWire/libcamera/libinput/fprintd/fwupd；
- Certified 5–10 台 PC，Supported 20–30 台；
- 迁移 MVP 覆盖文件、基础设置、浏览器书签、应用 inventory；
- 建低空间、掉电、休眠和坞站 CI。

### Phase H2：迁移与外设（9–15 个月）

- PMM v1、端到端加密、断点续传、staging/rollback；
- 应用 mapping catalog 和 Wine/VM/PWA 路由；
- 凭据经用户授权的直接导入；
- 打印/扫描、主流坞站、蓝牙音频和游戏手柄认证；
- 建 OEM/LVFS firmware testing channel；
- 发布选定非 T2 Intel Mac 的 Supported preview。

### Phase H3：Apple silicon Preview（并行，取决于上游）

- 直接参与 Asahi 上游，不长期私有 fork；
- 先选 M1/M2 各 2–3 个机型；
- 采用 Asahi installer 的安全磁盘/固件流程；
- 建 16 KiB host + 4 KiB game VM 栈；
- 明确不支持 Touch ID/ANE/TB 等功能；
- M3 及更新 Apple silicon 只有在对应 Asahi 官方机型页、installer 与核心 block 可用后立项。

### Phase H4：OEM Reference

- 与 OEM 锁定 Wi-Fi、摄像头、音频 amp、指纹、SSD 和 BIOS；
- coreboot/EDK2 仅在共同控制的硬件 Pilot；
- 建出厂 HCM、设备身份、恢复介质、固件 SLA；
- 这是 Andromeda 从“兼容 Linux 发行版”走向“macOS 级可靠性”的真正路径。

## 11. 验收指标

| 维度 | Supported | Certified |
| --- | --- | --- |
| 冷启动成功率 | ≥99.5% / 200 次 | ≥99.9% / 1000 次 |
| OTA 成功或自动回滚 | ≥99.5% | ≥99.9% |
| 休眠/恢复 | 100 次无阻断错误 | 500 次无阻断错误 |
| 8 小时待机掉电 | 机型阈值公开 | 不高于参考 OS 的约定差值 |
| GPU | 桌面/浏览器/基础 Vulkan 通过 | CTS 规定版本 + 游戏矩阵 |
| Wi-Fi/蓝牙 | 基本连接和恢复 | AP/耳机/手柄/共存矩阵 |
| 摄像头/麦克风 | 会议应用可用 | 画质、功耗、LED/隐私全测 |
| 更新后驱动回归 | 受影响机型可撤回 | HCM cohort 自动暂停 |
| 迁移数据正确性 | 100% manifest/hash | 100% + 应用级抽样验证 |
| 数据损坏 | 0 容忍 | 0 容忍 |

## 12. 关键否决项

- 不以“Linux kernel 支持某芯片”宣传整机支持；
- 不在安装器中静默改变 Secure Boot、RAID、APFS container 或 firmware；
- 不因 AI agent 判断“看起来相似”而加载其他机型的音频/EC/固件配置；
- 不把 `Windows.old` 式可见普通目录当系统回滚机制；
- 不复制 Windows/macOS 内核驱动到 Andromeda；
- 不迁移生物模板、设备身份或活跃登录 cookie；
- 不让 APFS 实验写驱动成为正式迁移依赖；
- 不在没有恢复设备和实体样机的情况下发布 firmware update；
- 不把 T2、M1/M2、M3/M4 和更新代际合并成“Mac supported”；
- 不把 M1/M2 的成功外推为所有未来 Apple silicon。

## 13. 相关驱动、固件与硬件项目目录

采用决策含义：

- **Adopt**：进入正式产品基础；
- **Pilot**：限定机型/功能，必须有 HCM 和回退；
- **Watch**：跟踪上游，不构成产品承诺；
- **Reject**：当前产品路线不采用该用法，并非否定项目本身。

| 项目 | 功能 | 许可证 | 成熟度 | 决策 | 官方链接与说明 |
| --- | --- | --- | --- | --- | --- |
| Linux kernel | x86-64/arm64、总线、驱动、功耗、安全基础 | GPL-2.0-only | 极高 | Adopt | [kernel.org](https://www.kernel.org/)、[license rules](https://docs.kernel.org/6.9/process/license-rules.html) |
| linux-firmware | GPU/Wi-Fi/蓝牙/NPU 等运行时固件 | 每文件不同；大量仅二进制可再分发 | 高，但许可与 ABI 分散 | Adopt（条件式） | [Kernel FAQ](https://www.kernel.org/faq.html)、[firmware guidelines](https://cdn.kernel.org/doc/html/latest/driver-api/firmware/firmware-usage-guidelines.html)；必须逐 blob 记录许可/哈希 |
| systemd + systemd-boot/stub | 服务、UKI、measured boot、boot counting | LGPL-2.1-or-later 为主，文件级例外 | 高 | Adopt | [systemd](https://github.com/systemd/systemd)、[boot assessment](https://systemd.io/AUTOMATIC_BOOT_ASSESSMENT/) |
| Mesa | Intel/AMD/NVIDIA/Apple 等 OpenGL/Vulkan/OpenCL 用户态 | MIT 为主 | 极高 | Adopt | [Mesa](https://docs.mesa3d.org/) |
| AMDGPU + RadeonSI/RADV | AMD 显示、图形、视频、compute | kernel GPL-2.0；Mesa MIT | 高 | Adopt | [AMDGPU kernel docs](https://docs.kernel.org/gpu/amdgpu/index.html)、[Mesa RADV](https://docs.mesa3d.org/drivers/radv.html) |
| Intel i915/xe + Iris/ANV | Intel 显示、图形、视频 | kernel GPL-2.0；Mesa MIT | 高，新 xe 需持续回归 | Adopt | [Linux GPU docs](https://docs.kernel.org/gpu/)、[Mesa ANV](https://docs.mesa3d.org/drivers/anv.html) |
| NVIDIA open GPU kernel modules + 官方用户态 | NVIDIA 游戏/CUDA 主现实栈 | kernel modules 双 GPL/MIT；用户态专有 | 高，但闭源用户态与版本锁步 | Adopt（独立受控渠道） | [NVIDIA open kernel modules](https://github.com/NVIDIA/open-gpu-kernel-modules)；仅支持其列出的 GPU |
| Nouveau + NVK/Zink | NVIDIA 全开放图形栈 | kernel GPL-2.0；Mesa MIT | 中高、快速发展 | Pilot | [Mesa NVK](https://docs.mesa3d.org/drivers/nvk.html)；API conformant 不代表所有游戏/compute 等价 |
| fwupd | 设备和 UEFI capsule 固件更新 | LGPL-2.1 | 高 | Adopt | [fwupd](https://github.com/fwupd/fwupd)、[plugin docs](https://fwupd.github.io/libfwupdplugin/) |
| LVFS | OEM/ODM 固件托管、签名 metadata、灰度 | 托管服务；payload 各自许可 | 高 | Adopt | [LVFS](https://lvfs.readthedocs.io/en/latest/)；Andromeda 增加 HCM allowlist |
| coreboot | 开源平台初始化固件 | GPL-2.0 | 高但逐主板 | Pilot（OEM 参考硬件） | [coreboot docs](https://doc.coreboot.org/)；Reject“在任意现有 PC 自动刷写” |
| PipeWire | 音视频图、低延迟、沙箱 portal | MIT | 高 | Adopt | [PipeWire](https://pipewire.org/)、[docs](https://docs.pipewire.org/) |
| WirePlumber | PipeWire session/policy manager | MIT | 高 | Adopt | [WirePlumber](https://pipewire.pages.freedesktop.org/wireplumber/) |
| Sound Open Firmware | 音频 DSP 固件、拓扑与工具 | BSD-3-Clause 为主，逐文件核查 | 中高；依赖精确平台组合 | Adopt 框架 / Pilot 机型 | [SOF Project](https://www.sofproject.org/)、[上游仓库](https://github.com/thesofproject/sof) |
| ALSA UCM configuration | 每机型声卡路由与 use-case 配置 | BSD-3-Clause | 高但覆盖依赖机型 | Adopt | [上游仓库](https://github.com/alsa-project/alsa-ucm-conf)；与内核、SOF、codec 固件锁步 |
| libcamera | MIPI/复杂摄像头管线 | LGPL-2.1-or-later；IPA/组件需逐项看 | 中高；硬件覆盖不均 | Adopt 框架 / Pilot 机型 | [libcamera](https://libcamera.org/)、[FAQ](https://libcamera.org/faq.html) |
| libinput | 键鼠、触控板、触屏、手写笔和 quirks | MIT | 高 | Adopt | [libinput](https://wayland.freedesktop.org/libinput/doc/latest/) |
| libfprint/fprintd | 指纹设备与登录集成 | libfprint LGPL-2.1-or-later；fprintd GPL-2.0-or-later | 中；精确 ID 覆盖 | Pilot | [libfprint devices](https://fprint.freedesktop.org/supported-devices.html) |
| BlueZ | Linux Bluetooth host stack | GPL-2.0-or-later / LGPL-2.1-or-later 混合 | 高 | Adopt | [上游文档](https://bluez.readthedocs.io/en/latest/) |
| bolt | Thunderbolt 设备授权数据库和策略 | LGPL-2.1-or-later | 高 | Adopt | [bolt](https://gitlab.freedesktop.org/bolt/bolt)；结合 IOMMU DMA protection |
| UPower + power-profiles-daemon | 电池/供电抽象与有限桌面功耗档位 | GPL-2.0-or-later / GPL-3.0-or-later | 高 | Adopt | [UPower](https://upower.freedesktop.org/)、[power-profiles-daemon](https://gitlab.freedesktop.org/upower/power-profiles-daemon) |
| thermald / TuneD | 热策略与工作负载调优 | GPL-2.0 / GPL-2.0-or-later | 高，但平台策略敏感 | Pilot | [thermald](https://github.com/intel/thermal_daemon)、[TuneD](https://github.com/redhat-performance/tuned)；只启用 HCM 验证配置 |
| FWTS / pm-graph | ACPI/UEFI 合规与休眠时间线诊断 | GPL-2.0-or-later / GPL-2.0 | 高（诊断） | Adopt for CI | [FWTS](https://github.com/ColinIanKing/fwts)、[pm-graph](https://github.com/intel/pm-graph) |
| pciutils / usbutils / hwdata | 总线枚举、设备 ID 与规范化 inventory | GPL 系与 XFree86 数据许可，按项目 | 高 | Adopt for HCM | [pciutils](https://git.kernel.org/pub/scm/utils/pciutils/pciutils.git/)、[usbutils](https://git.kernel.org/pub/scm/linux/kernel/git/gregkh/usbutils.git/)、[hwdata](https://github.com/vcrhonek/hwdata) |
| CUPS/OpenPrinting/PAPPL | IPP 打印、driverless 与旧打印机应用化 | CUPS Apache-2.0；组件各自许可 | 高 | Adopt | [OpenPrinting CUPS](https://openprinting.github.io/cups/)、[driver strategy](https://openprinting.github.io/cups/drivers.html) |
| SANE backends | USB/网络扫描仪 | GPL-2.0-or-later 为主，backend 逐项 | 高但设备差异大 | Adopt 框架 / Pilot 设备 | [SANE](https://www.sane-project.org/)、[device list](https://www.sane-project.org/sane-supported-devices.html) |
| libusb + Linux usbfs | USB 用户态协议与受控设备访问 | libusb LGPL-2.1-or-later；kernel GPL-2.0 | 高 | Adopt（适配的外设类） | [libusb](https://libusb.info/)、[Linux USB host API](https://docs.kernel.org/driver-api/usb/usb.html)；不是显示/GPU 等通用驱动替代 |
| KVM/QEMU + VFIO | Windows VM 与 USB/PCI 设备直通 | kernel GPL-2.0；QEMU GPL-2.0 | 高，依赖 IOMMU 与设备拓扑 | Pilot（专有外设后备） | [KVM](https://www.linux-kvm.org/)、[QEMU](https://www.qemu.org/)、[VFIO](https://docs.kernel.org/driver-api/vfio.html)；VM 可用不得标成宿主支持 |
| ndiswrapper | 在 Linux 内核包装旧 Windows NDIS 无线驱动 | GPL-2.0 | 低；主要停留在 NDIS 5，NDIS 6 不完整 | Reject 产品依赖 | [README](https://github.com/pgiri/ndiswrapper)；仅作为历史研究 |
| Intel `ivpu` + Level Zero/OpenVINO NPU | Intel Core Ultra NPU | kernel GPL-2.0；Intel userspace MIT；OpenVINO Apache-2.0 | 中，发行版/版本约束明显 | Pilot | [Intel NPU driver](https://github.com/intel/linux-npu-driver)、[OpenVINO](https://docs.openvino.ai/) |
| AMD `amdxdna` + XRT shim | Ryzen AI/XDNA NPU | kernel GPL-2.0；XRT/shim 混合 Apache-2.0/GPL，逐组件审计 | 中，主线化进行中 | Pilot | [kernel amdxdna](https://docs.kernel.org/accel/amdxdna/index.html)、[AMD repo](https://github.com/amd/xdna-driver) |
| Asahi m1n1 | Apple silicon 启动、调试和 ABI 转换 | MIT，内嵌组件各自许可 | M1/M2 高 | Adopt（Apple silicon 变体） | [m1n1](https://github.com/AsahiLinux/m1n1) |
| U-Boot | Apple silicon 上提供后续 UEFI-like 启动层及通用 boot | GPL-2.0-or-later | 高 | Adopt（Apple silicon/arm64） | [U-Boot](https://docs.u-boot.org/) |
| Asahi Linux kernel/Mesa/audio enablement | Apple silicon Linux 驱动与整机集成 | kernel GPL-2.0；Mesa MIT；组件各自 | M1/M2 中高，M3/M4 早期；更新代际未列入时不可推断 | Pilot | [Asahi docs](https://asahilinux.org/docs/)、[feature support](https://asahilinux.org/docs/platform/feature-support/overview/) |
| Asahi installer / firmware tooling | 安全调整 APFS、建立 fuOS 和提取配对固件 | 工具开源；Apple payload 受 Apple 许可 | M1/M2 高 | Adopt 流程，不自行重写 | [Asahi distro policy](https://asahilinux.org/docs/alt/policy/) |
| t2linux `apple-bce-drv` | T2 BCE、虚拟 USB、键盘/触控板/音频/摄像头通道 | 源文件 GPL-compatible；仓库缺少清晰顶层 LICENSE，需法律审计 | 中低、out-of-tree、suspend 缺口 | Watch / Experimental | [apple-bce](https://github.com/t2linux/apple-bce-drv)、[t2linux state](https://wiki.t2linux.org/state/) |
| Asahi `tiny-dfr` | Apple silicon Dynamic Function Row daemon | MIT | 中，持续活跃 | Pilot（精确机型） | [上游仓库](https://github.com/AsahiLinux/tiny-dfr)；不能替代底层显示/输入驱动 |
| t2linux `apple-ib-drv` | T2 iBridge Touch Bar/ALS 内核驱动 | GPL-2.0 | 中低，out-of-tree | Watch / Experimental | [上游仓库](https://github.com/t2linux/apple-ib-drv)；与 BCE 栈锁步测试 |
| `apfs-fuse` | APFS 只读 FUSE 实现 | GPL-2.0 | 实验/诊断 | Watch | [上游仓库](https://github.com/sgan81/apfs-fuse)；不作为正式迁移数据源 |
| `linux-apfs-rw` | APFS 内核读写实现 | GPL-2.0 | 实验；写入风险高 | Reject 正式迁移写路径 | [上游仓库](https://github.com/linux-apfs/linux-apfs-rw) |
| `apfsprogs` | APFS 测试/检查/创建工具 | GPL-2.0 | 实验；`apfsck` 面向测试者 | Watch | [上游仓库](https://github.com/linux-apfs/apfsprogs) |
| Windows Boot Camp support software | Windows on Intel Mac 的 Apple 驱动 | Apple 专有 SLA | 对 Windows 成熟，对 Linux 不可直接使用 | Reject 直接复用/再分发 | [Apple 下载说明](https://support.apple.com/en-ie/102465)；只可作为逆向和本机合法提取的研究输入 |

## 14. 最终建议

Andromeda 的硬件战略不应是“承诺越多机型越好”，而应是：

> 用 Linux 最大化可识别范围，用 HCM 缩小可承诺范围，用物理 CI 保证承诺真实，用源系统迁移代理消除切换成本，再通过 OEM Reference Hardware 逐步获得 macOS 式可靠性。

近期最有价值的投入顺序：

1. HCM + Hardware Probe；
2. 认证 AMD/Intel PC 基线；
3. NVIDIA 官方驱动的原子更新和游戏 CI；
4. 休眠、音频、摄像头、坞站实验室；
5. Windows/macOS 源迁移代理与 PMM；
6. 少量非 T2 Intel Mac；
7. 基于 Asahi 的 M1/M2 独立 Preview；
8. T2、M3 及更新 Apple silicon 保持透明的 Experimental/Watch，不用营销承诺倒逼不安全实现。

只有这样，“从现在的 PC 和 Mac 无缝切换”才不是一句安装口号，而是可验证、可回滚、可长期维护的产品能力。
