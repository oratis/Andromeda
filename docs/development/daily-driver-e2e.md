# Andromeda 日用消费版候选与端到端验收

本文定义 Andromeda 从“可安装 Developer Preview”迈向日用消费版时必须满足的
虚拟硬件门槛、复现方法和证据边界。通过本文的测试只代表
**Daily Driver Candidate（虚拟硬件）**，不等于任意 PC 或 Mac 已获得认证。

## 消费版基线

镜像在 Developer Preview 的 KDE Plasma、bootc 更新回滚和 AI 任务控制面之上，
增加以下日用能力：

| 领域 | 镜像能力 |
|---|---|
| 桌面 | Plasma Wayland、Xwayland、Dolphin、Konsole、Kate、Ark、Okular、Spectacle、系统监视器 |
| 应用交付 | Flatpak、Fedora Flathub remote、Plasma Discover 与桌面 portal |
| Office/文件 | LibreOffice Writer/Calc/Impress/Draw、PDF、7z、unar、缩略图和常见媒体解码 |
| 中文与字体 | Fcitx 5 中文插件、英文/中文 locale、Noto Emoji 与 Liberation 字体 |
| 音视频 | PipeWire、PulseAudio compatibility、WirePlumber、GStreamer、SOF firmware |
| 外设 | BlueZ、打印、扫描、Thunderbolt、指纹、摄像头工具、fwupd、UPower |
| 游戏基础 | 32 位 Mesa/Vulkan、GameMode、Gamescope、MangoHud、Steam udev 规则 |
| 可靠性 | systemd-oomd、zram、fstrim timer、journald 空间与保留期上限 |
| 默认暴露面 | firewalld 启用；消费版默认不启动 SSH，也不监听通配地址的 22 端口 |

这些包提供“安装和运行基础”，不代表 Steam 商店、专有 GPU 驱动、Windows
Workspace、Microsoft Office 原版或某款游戏已经通过产品认证。

## 自动验收场景

`os/scripts/test-install.sh` 在新的 64 GiB VirtIO 磁盘上执行真实 UEFI 生命周期：

1. 从 ISO 离线安装，移除 ISO 后仅从硬盘启动；
2. CI 专用账户自动进入 Plasma Wayland 会话；
3. 验证 KWin、Plasma Shell、PipeWire、WirePlumber、Flatpak、打印、防火墙、
   OOM 保护、zram、trim 和磁盘余量；
4. 用 LibreOffice 实际生成 DOCX、XLSX、PPTX 和 PDF，并验证 OOXML
   ZIP 结构与 PDF 元数据；
5. 在真实 Plasma Wayland 会话中启动隔离配置的 Firefox，确认进程保持健康后
   由 systemd 用户服务精确回收；
6. 创建用户持久文件并记录固定 SHA-256；
7. 更新到 revision 2，确认桌面能力和用户文件仍在；
8. 回滚到 revision 1，再次确认桌面能力和用户文件仍在；
9. 严格验证成功标记的顺序，最终才输出 `ANDROMEDA_E2E_OK`。

预期串口顺序为：

```text
ANDROMEDA_SELINUX_LABELS_OK
ANDROMEDA_DAILY_DRIVER_OK phase=first-boot revision=1
ANDROMEDA_FIRST_BOOT_OK revision=1
ANDROMEDA_UPDATE_STAGED_OK revision=2
ANDROMEDA_DAILY_DRIVER_OK phase=updating revision=2
ANDROMEDA_UPDATE_BOOT_OK revision=2
ANDROMEDA_ROLLBACK_STAGED_OK revision=1
ANDROMEDA_DAILY_DRIVER_OK phase=rolling-back revision=1
ANDROMEDA_ROLLBACK_BOOT_OK revision=1
ANDROMEDA_E2E_OK
```

缺少、乱序或出现 `ANDROMEDA_E2E_FAILED` 都视为失败。

## 本地 KVM 复现

在 x86-64 Linux KVM 主机安装 Podman、QEMU 和 OVMF 后运行：

```bash
sudo os/scripts/build-iso.sh "$PWD/output"
sudo os/scripts/test-install.sh "$PWD/output"
```

构建和测试会生成 ISO、校验和、安装串口、启动串口、ESP/NVRAM 信息、更新服务
日志与失败诊断。不要在真实电脑上选择 ISO 的 CI 自动安装项；它会清空第一块磁盘。

## Google Cloud 嵌套虚拟化复现

仓库提供 `os/scripts/test-gcp-nested.sh` 作为 L1 Linux 主机中的统一入口：

```bash
sudo env ANDROMEDA_SOURCE_REVISION="$(git rev-parse HEAD)" \
  os/scripts/test-gcp-nested.sh "$PWD" "$PWD/output"
```

脚本先拒绝不满足以下条件的主机，再开始耗时构建：

- `/dev/kvm` 存在且 CPU 暴露 Intel `vmx`；
- 至少 4 个 vCPU、16 GiB 内存；
- 输出盘至少有 100 GiB 可用空间。

测试结束时，`output/gcp-evidence/` 保存主机环境、构建日志、测试日志、ISO
SHA-256、生命周期标记和诊断。云资源的创建、标签校验、最大运行时、证据下载与
精确清理由本地 `gcp-os-e2e` Codex skill 管理；仓库脚本本身不会创建或删除云资源。

## 证据边界

GCP 嵌套 KVM 可以证明：

- 可重复构建系统镜像和离线 ISO；
- UEFI 空盘安装与纯硬盘启动；
- VirtIO 网络、磁盘、虚拟显示和虚拟声卡上的桌面会话；
- Office/浏览器/Flatpak/音频服务烟雾测试；
- bootc 更新、回滚与用户数据持久性。

它不能证明：

- AMD/Intel/NVIDIA 真实 GPU 的性能、HDR、VRR 或多显示器；
- Wi‑Fi、蓝牙、摄像头、指纹、打印机和雷电设备的真实行为；
- 笔记本电池、待机、休眠、合盖、热管理和固件升级；
- Secure Boot/TPM 在消费级固件矩阵上的完整链路；
- Intel/T2/Apple silicon Mac；
- Steam/Proton 游戏性能、DRM 或内核级反作弊兼容性。

这些能力必须由签名 Hardware Compatibility Manifest 与真实设备实验室逐机型
认证。未获得证据的硬件只能标为 Community 或 Experimental。

## 已验证运行

本节只记录完整通过且证据已保存的运行。进行中的运行或局部成功不会列入。

| 项目 | 值 |
|---|---|
| 生命周期结果 | **PASS**；离线安装、首次启动、revision 2 更新、revision 1 回滚全部通过 |
| 时间 | 2026-07-28 18:58:31–19:25:22 UTC |
| GCP 主机 | `n2-standard-16`，16 vCPU、64 GiB RAM、Intel VT-x 嵌套 KVM，`us-central1-a` |
| L2 虚拟机 | Q35/OVMF、4 vCPU、8 GiB RAM、32 GiB VirtIO 系统盘、VirtIO GPU、Intel HDA |
| 源标识 | `daily-driver-final-b9150477f12f` |
| ISO | `Andromeda-Developer-Preview-x86_64.iso`，约 3.8 GiB |
| ISO SHA-256 | `6f8d74e5f14b7dab9c478b8fd538defbdbde717dee62bbc3c7ca5c13cc597108` |
| 原始外层状态 | `1`；OS 已输出全部成功标记，证据收集器因串口 CRLF 与 ANSI 前缀产生假阴性 |
| 规范化复验 | `0`；同一原始串口日志经修复后的提取器严格匹配 `ANDROMEDA_E2E_OK` |

规范化后的原始标记顺序为：

```text
ANDROMEDA_SELINUX_LABELS_OK
ANDROMEDA_DAILY_DRIVER_OK phase=first-boot revision=1
ANDROMEDA_FIRST_BOOT_OK revision=1
ANDROMEDA_UPDATE_STAGED_OK revision=2
ANDROMEDA_DAILY_DRIVER_OK phase=updating revision=2
ANDROMEDA_UPDATE_BOOT_OK revision=2
ANDROMEDA_ROLLBACK_STAGED_OK revision=1
ANDROMEDA_DAILY_DRIVER_OK phase=rolling-back revision=1
ANDROMEDA_ROLLBACK_BOOT_OK revision=1
ANDROMEDA_E2E_OK
```

外层假阴性的根因是串口每行以 CRLF 结尾，且一次 agetty 控制序列与更新标记位于
同一物理行；旧收集器直接使用 `grep -x`，因此没有匹配到末尾带 `\r` 的
`ANDROMEDA_E2E_OK`。收集器现改为从二进制串口流中只提取以 `ANDROMEDA_`
开头的可打印字段，并已在上述原始证据上复验通过。这个修复不改变 OS、磁盘或
生命周期结果。
