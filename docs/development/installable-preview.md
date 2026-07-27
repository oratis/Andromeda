# Andromeda Developer Preview 安装与验收

本文说明第一个可安装垂直切片的边界、构建方法和验收证据。它面向开发者，
不是“任意 PC/Mac 已获得支持”的声明。

## 支持边界

| 项目 | Developer Preview 0 |
|---|---|
| CPU | x86-64 |
| 固件 | UEFI |
| 已自动验收 | QEMU/KVM + OVMF + VirtIO 空白磁盘 |
| 桌面 | KDE Plasma + SDDM |
| 系统基础 | Fedora bootc 43 |
| 文件系统 | ext4 |
| 安装方式 | 离线 payload + 图形化 Anaconda |
| AI 控制面 | loopback-only `andromeda-taskd` |
| 目标系统安全策略 | SELinux enforcing |
| 更新 | bootc OCI deployment |
| 回滚 | bootc 前一 deployment |

Intel Mac、T2 Mac 和 Apple silicon 不在这个镜像的已验收范围。未经 Hardware
Compatibility Manifest 和真机测试的 PC 也只能视为 Community/Experimental。

## 安装前要求

- 一台可从 USB 启动的 x86-64 UEFI 测试机，或支持 OVMF 的虚拟机；
- 至少 32 GiB 空白磁盘和 8 GiB 内存；
- 已备份所有数据；
- 暂时关闭 Secure Boot。当前预览尚未完成自有密钥、签名轮换和真实固件矩阵认证。

安装器的正常默认项是 **Install Andromeda Developer Preview**，会进入图形界面让
用户明确选择磁盘并创建账户。

generic bootc ISO 的临时 SquashFS/OverlayFS 安装环境按 image-builder 上游合约以
`selinux=0` 启动；这个参数不写入安装后系统。自动验收要求目标系统重新进入
`SELinux enforcing`，否则失败。

> **危险：** `Automated destructive install (CI only)` 只供 QEMU 验收。它会清空
> 第一块安装磁盘，不得在含有用户数据的机器上选择。

## 构建 ISO

在 x86-64 Linux 主机安装 Podman，并允许特权容器：

```bash
sudo os/scripts/build-iso.sh
```

构建产物：

- `output/Andromeda-Developer-Preview-x86_64.iso`
- `output/Andromeda-Developer-Preview-x86_64.iso.sha256`
- `output/andromeda-v2.tar`（仅用于生命周期测试）

ISO 使用 `image-builder` 的 `bootc-generic-iso` 类型，把
`localhost/andromeda:v1` payload 嵌入安装环境，因此系统安装本身不依赖网络。

## 自动化空盘验收

在安装 QEMU、OVMF 和 KVM 的 Linux 主机运行：

```bash
sudo os/scripts/test-install.sh
```

脚本执行以下真实状态转换：

1. 创建新的 32 GiB qcow2 磁盘；
2. 以 OVMF UEFI 启动 ISO；
3. 通过 CI 专用 Kickstart 清空并分区该磁盘；
4. 从 ISO 内嵌 OCI payload 安装系统，同时写入 UEFI vendor 与标准 fallback 路径；
5. 关机、移除 ISO，仅从安装后的磁盘启动；
6. 验证 UEFI、SELinux enforcing、SDDM、硬件报告和 `taskd /healthz`；
7. 导入 revision 2，执行 `bootc switch` 并重启；
8. 确认 revision 2 已启动，执行 `bootc rollback` 并重启；
9. 确认 revision 1 恢复，输出 `ANDROMEDA_E2E_OK`。

串口必须按顺序出现：

```text
ANDROMEDA_FIRST_BOOT_OK revision=1
ANDROMEDA_UPDATE_STAGED_OK revision=2
ANDROMEDA_UPDATE_BOOT_OK revision=2
ANDROMEDA_ROLLBACK_STAGED_OK revision=1
ANDROMEDA_ROLLBACK_BOOT_OK revision=1
ANDROMEDA_E2E_OK
```

GitHub Actions 运行同一脚本，并保存 ISO、SHA-256、安装串口和首次启动串口作为
可审计证据。

## 当前限制

- 只有 QEMU/KVM x86-64 达到自动安装门槛；还没有任何消费级 PC/Mac 获得
  Supported 或 Certified 等级。
- 默认远程更新引用预留为 `ghcr.io/oratis/andromeda:edge`，在建立签名发布流水线
  前不应依赖它；CI 使用本地 OCI archive 验证 deployment 和回滚机制。
- 没有双系统分区器、Windows/macOS 数据迁移器、恢复 UI 或磁盘加密安装选项。
- 游戏、Microsoft Office/Windows Workspace 和 AI 图形化 Task Center 仍属于后续
  产品里程碑。
- Secure Boot、TPM 测量启动、签名 HCM 和真实硬件 suspend/resume 尚未认证。

## 上游合约

- [image-builder generic bootc ISO](https://osbuild.org/docs/developer-guide/projects/image-builder/advanced/bootc/isos/)
- [image-builder 容器安装方式](https://osbuild.org/docs/developer-guide/projects/image-builder/installation/)
- [bootc install](https://bootc-dev.github.io/bootc/bootc-install.html)
- [bootc switch](https://bootc-dev.github.io/bootc/bootc-switch.html)
- [bootc rollback](https://bootc-dev.github.io/bootc/bootc-rollback.html)
- [Anaconda bootc Kickstart](https://pykickstart.readthedocs.io/en/latest/commands.html#bootc)
