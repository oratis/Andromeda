# Andromeda 硬件普适性工程

> 状态：Hardware Enablement Phase 1
> 更新：2026-07-30

## 目标与不可伪造的边界

“支持全部硬件”在工程上不能表示每个历史设备、封闭协议和未来硬件都已经可用。
Andromeda 将目标定义为：

1. 对 Fedora/Linux 已有上游驱动的主流 PC，尽量做到开箱即用；
2. 对每台机器自动识别未绑定驱动、缺失固件和有限支持路径；
3. 只有精确硬件、驱动、固件和功能测试证据完整时才标记 Supported/Certified；
4. NVIDIA 专有栈、T2 Mac、Apple Silicon 等进入隔离的 Hardware Enablement
   Image，而不是污染通用镜像；
5. 新硬件通过 HCM、自动矩阵和实机实验室持续进入，不依赖一次性“万能 ISO”。

探测到设备不等于支持；安装了一个包也不等于设备的 GPU、Wi‑Fi、睡眠或摄像头
功能已经通过验收。

## 已落地的通用 x86-64 覆盖

### 内核、固件与诊断

- Fedora bootc 内核、`kernel-modules-extra` 和完整 `linux-firmware` 路径；
- 显式固定 AMD/Intel/NVIDIA GPU、Intel/Qualcomm/MediaTek/Realtek/Broadcom/NXP
  无线及 Intel/Cirrus 音频固件子包；
- 显式固定 AMD CPU microcode，并为旧 Intel `iwlegacy`、部分 b43 开放固件和
  Qualcomm WWAN 保留可验证路径；
- `pciutils`、`usbutils`、`tpm2-tools`、`fwupd`；
- 首次启动生成隐私友好的 `hardware.json` 与 `hardware-diagnosis.json`；
- 不采集序列号、MAC 地址、磁盘 UUID 或 Apple platform UUID。

[Fedora 44 kernel-modules-extra](https://packages.fedoraproject.org/pkgs/kernel/kernel-modules-extra/fedora-44.html)
提供不在最小内核模块集合中的长尾设备驱动；[Fedora linux-firmware](https://packages.fedoraproject.org/pkgs/linux-firmware/)
按厂商拆分固件。镜像显式列出子包，避免依赖构建机器 modalias 或弱依赖决定最终
设备覆盖。

### GPU、显示与视频

- Intel/AMD：Mesa DRI、Vulkan、VA-API、32 位游戏运行库；
- NVIDIA：通用镜像只提供 Nouveau/NVK 与 Fedora 可分发固件；
- 双 GPU：`switcheroo-control`；
- Wayland、Xwayland、Gamescope、MangoHud、GameMode；
- `vulkaninfo`、`vainfo` 可用于 HCM 功能证据。

[Fedora Mesa Vulkan 包](https://packages.fedoraproject.org/pkgs/mesa/mesa-vulkan-drivers/fedora-44.html)
同时提供 Intel、Radeon、Nouveau/NVK 与 VirtIO 等 ICD。Nouveau/NVK 被诊断为
`needs_review`，不能自动晋级成 NVIDIA 游戏认证。NVIDIA 专有模块必须使用独立
镜像、固定内核 ABI、签名模块和 Secure Boot 验收。

### 网络与无线

- NetworkManager Ethernet、Wi‑Fi、Bluetooth、PPP、WWAN；
- `wpa_supplicant` 是唯一默认 Wi‑Fi 后端，不同时启用 iwd 管理相同接口；
- `wireless-regdb`、`iw`、ModemManager、USB modeswitch；
- Atheros、Broadcom brcmfmac、Intel iwlwifi、MediaTek、Realtek、NXP 固件。

[NetworkManager 官方 Fedora 子包](https://packages.fedoraproject.org/pkgs/NetworkManager/)
将 Wi‑Fi、Bluetooth 和 WWAN 分开交付，因此仅安装 NetworkManager 核心包不能
视为无线网络覆盖。

### 音频、摄像头和移动设备

- PipeWire、WirePlumber、ALSA UCM、SOF/传统 ALSA 固件，以及 32 位游戏音频库；
- BlueZ 与 Plasma Bluetooth；
- libcamera、PipeWire libcamera 插件、GStreamer/V4L2 PipeWire bridge 和
  V4L2 工具；
- libinput、libwacom、IIO 方向传感器、指纹守护程序；
- Thunderbolt `bolt`、Plasma Thunderbolt、UPower、thermald。

摄像头“枚举成功”仍不代表浏览器 WebRTC、隐私 LED、麦克风同步和视频质量已经
认证；Intel MIPI/IPU、Mac FaceTimeHD 等继续按机型处理。

### 存储、文件系统与外设

- VirtIO、NVMe、SATA/AHCI 的内核路径和诊断工具；
- LVM2、mdraid、Btrfs、XFS、ext4、F2FS、exFAT、NTFS、FAT；
- UDisks2/LVM2 桌面挂载与 KDE Partition Manager；
- CUPS、IPP USB、mDNS、网络发现、SANE/AirScan 与 Plasma 打印管理。

ZFS、厂商 RAID 内核模块和需要不可再分发插件的打印机不进入通用镜像。

## 启动时硬件诊断

用户或支持工程师可以运行：

```bash
andromeda hardware probe
andromeda hardware diagnose
```

诊断器对支持相关设备给出：

- `ready`：驱动已经绑定；
- `limited`：能启动，但需要精确机型功能验证；
- `missing_driver`：没有绑定内核驱动；
- 整机 `ready`、`needs_review` 或 `blocked`；
- `boot_critical_missing`，覆盖存储、网络、GPU 和 USB 控制器。

关键设备无驱动时，CI 和 HCM 不允许把机器提升为 Supported。Broadcom 无驱动和
NVIDIA Nouveau/NVK 会生成专门建议，而不是让 AI 自动安装来源不明的 DKMS 包。

## HCM v2

HCM v2 新增：

- 精确 PCI subsystem vendor/device、revision 与 modalias 事实；
- `pc_uefi_shim`、Intel Mac、T2、Apple Silicon Asahi 等启动供应者；
- kernel、driver、firmware、Hardware Enablement Image 的版本、来源、SHA-256
  和签名 key ID；
- capability evidence 的结果、证据地址、采集时间和到期时间；
- 整份支持声明到期时间。

Supported/Certified/Reference 必须具备固定 artifact、通过且未过期的证据以及
支持到期时间；否则运行时强制降为 Blocked。Community 可以用于收集未知硬件
报告，但不是质量承诺。

## 自动虚拟硬件矩阵

完整基线仍执行：

```text
Q35 + OVMF + VirtIO disk/network + VirtIO GPU + Intel HDA
ISO 安装 → 纯硬盘首启 → Plasma → 更新 → 回滚
```

基线通过后，`test-hardware-matrix.sh` 使用独立 qcow2 overlay 和独立 OVMF
变量盘验证：

| Profile | 芯片组 | 存储 | 网卡 | USB/音频 | CPU topology |
|---|---|---|---|---|---|
| `modern-nvme` | Q35 | NVMe | e1000e | XHCI/HDA | 1×2×2 |
| `q35-sata` | Q35 | AHCI/SATA | e1000e | XHCI/HDA | 2×2×1 |
| `legacy-i440fx` | i440fx + UEFI | IDE | e1000 | UHCI/AC97 | 1×2×1 |

每个 profile 必须从 guest 输出与 profile 对应的硬件诊断 marker、零关键缺失、
桌面健康和唯一 `ANDROMEDA_E2E_OK`，并在停止后通过 `qemu-img check`。

这组矩阵证明的是模拟控制器与驱动路径，不证明真实 Wi‑Fi、蓝牙、HDR、VRR、
摄像头、麦克风音质、雷电、睡眠、电池或固件更新。

## 不能放进同一个通用镜像的硬件

### NVIDIA 专有驱动

规划三个互斥 cohort：

1. `nvidia-nvk`：Nouveau/NVK，Community；
2. `nvidia-open`：适用硬件的 NVIDIA open kernel module + 专有用户态；
3. `nvidia-legacy-proprietary`：老 GPU 的限期 Experimental 路径。

专有路径必须固定 kernel、模块、用户态、i686 图形库、GSP firmware、initramfs、
签名证书和 OCI digest。不能运行 NVIDIA `.run` 安装器，也不能让 Nouveau 与
专有模块同时成为活动路径。模块签名要求见
[Linux 内核官方文档](https://docs.kernel.org/admin-guide/module-signing.html)。

### Intel Mac 与 T2 Mac

- 非 T2 Intel Mac：只做精确型号 Pilot；
- T2 Mac：独立 Experimental 内核、T2Linux 组件和从本机 macOS 提取的固件；
- 安装器必须保留 macOS/Recovery，绝不能复用 CI 的清盘 Kickstart；
- Touch ID、Secure Enclave、混合显卡、睡眠、摄像头必须逐型号验证。

### Apple Silicon

Apple Silicon 需要独立 arm64 产品线：

```text
Apple boot policy → APFS stub → m1n1 → U-Boot
→ Asahi kernel/device tree → 机器配对固件 → Asahi Mesa
```

它不能启动当前 x86-64 PC ISO。M1/M2 可以进入独立 Asahi Preview；更新型号必须
以 Asahi 对应机型支持页和安装器为准。[Fedora Asahi Remix](https://docs.fedoraproject.org/en-US/fedora-asahi-remix/)
还使用 muvm/FEX 处理部分 x86/x86-64 应用，这也不能替代逐游戏实体机验证。

## 实机认证队列

虚拟矩阵之后，每个精确机型必须完成：

1. 安装、纯盘启动、更新、失败回滚和恢复介质；
2. GPU Vulkan/视频解码/多屏/HDR/VRR；
3. Wi‑Fi 扫描、关联、吞吐、监管域和 suspend/resume；
4. 蓝牙键鼠、耳机、麦克风与恢复连接；
5. 扬声器、耳机孔、麦克风阵列和音量键；
6. 摄像头、隐私开关和浏览器 WebRTC；
7. 触控板、触摸屏、手写笔、方向传感器和指纹；
8. USB-C、Dock、Thunderbolt、外接存储和打印扫描；
9. S0ix/S3、休眠、电池、热管理和风扇；
10. fwupd/LVFS、TPM、Secure Boot 和模块签名。

所有证据都必须绑定镜像 digest、内核、驱动、固件哈希和有效期。证据过期或更新
改变其中任一关键组件时，认证自动回到 Needs Review。
