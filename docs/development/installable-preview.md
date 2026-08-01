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
| 系统基础 | Fedora bootc 44 |
| 文件系统 | ext4 |
| 安装方式 | 离线 payload + 图形化 Anaconda |
| AI 控制面 | loopback-only `andromeda-taskd` |
| 目标系统安全策略 | SELinux enforcing |
| 更新 | bootc OCI deployment |
| 回滚 | bootc 前一 deployment |

Intel Mac、T2 Mac 和 Apple silicon 不在这个镜像的已验收范围。未经 Hardware
Compatibility Manifest 和真机测试的 PC 也只能视为 Community/Experimental。
当前产物具有不可变的 `pc_x86_64` 平台身份。安装预检会同时验证 CPU 架构、Boot
Provider、镜像内的 `/usr/lib/andromeda/platform.json` 与内嵌 payload OCI label；
检测到任何 Apple 硬件时默认拒绝继续。Mac 必须使用保留 macOS/Recovery 的专用
产物和安装流程，不能复用 PC 清盘路径。

## 安装前要求

- 一台可从 USB 启动的 x86-64 UEFI 测试机，或支持 OVMF 的虚拟机；
- 至少 64 GiB 空白磁盘和 8 GiB 内存；
- 已备份所有数据；
- 暂时关闭 Secure Boot。当前预览尚未完成自有密钥、签名轮换和真实固件矩阵认证。

安装器的正常默认项是 **Install Andromeda Developer Preview**，会进入图形界面让
用户明确选择磁盘并创建账户。

generic bootc ISO 的临时 SquashFS/OverlayFS 安装环境以 `selinux=0` 启动，避免
只读 live 文件系统的安全标签与运行时策略冲突。交互式和 CI Kickstart 都显式使用
`selinux --enforcing`；安装后脚本再通过 OSTree deployment API 重建目标内核参数，
并拒绝任何仍含 `selinux=0` 的 BLS 启动项。自动验收要求安装后的系统进入
`SELinux enforcing`，否则失败。

> **危险：** `Automated destructive install (CI only)` 只供 QEMU 验收。它会清空
> 第一块安装磁盘，不得在含有用户数据的机器上选择。该入口在预检阶段要求
> `systemd-detect-virt --vm` 确认虚拟机；真机即使误选也会在分区前停止。

## 构建 ISO

在 x86-64 Linux 主机安装 Podman，并允许特权容器：

```bash
sudo os/scripts/build-iso.sh
```

构建产物：

- `output/Andromeda-Developer-Preview-x86_64.iso`
- `output/Andromeda-Developer-Preview-x86_64.iso.sha256`
- `output/Andromeda-Developer-Preview-x86_64.manifest.json`
- `output/andromeda-v1-history.json`（v1 payload 的镜像层历史，CI 也会上传）
- `output/andromeda-v2.tar`（仅用于生命周期测试）

`INSTALLER_DEFAULT` 环境变量控制 ISO 的 GRUB 默认启动项，默认为 `0`
（交互式图形安装器，面向人类，安全）。只有 CI 和自动化验收才应设置
`INSTALLER_DEFAULT=1`：该值把 10 秒后自动清盘的 CI 安装项设为默认，产物
会改名为 `Andromeda-Developer-Preview-x86_64-ci.iso`（含对应的 `.sha256`
与 `.manifest.json`，manifest 中也记录 `installer_default`），绝不能把它
当作面向开发者的预览镜像分发。构建过程会校验 `INSTALLER_DEFAULT` 只能是
0 或 1，并断言 GRUB default 替换确实生效。

ISO 使用 `image-builder` 的 `bootc-generic-iso` 类型，把
`localhost/andromeda:v1` payload 嵌入安装环境，因此系统安装本身不依赖网络。
产物 manifest 把 ISO SHA-256、payload digest、架构、Boot Provider 与 HEP ID
绑定在一起，为后续签名、发布和 HCM 证据关联提供机器可读输入。

## 自动化空盘验收

在安装 QEMU、OVMF 和 KVM 的 Linux 主机运行：

```bash
sudo env INSTALLER_DEFAULT=1 os/scripts/build-iso.sh
sudo os/scripts/test-install.sh
```

脚本必须以 root 运行（`modprobe`、`qemu-nbd`、`mount`），并要求存在
`Andromeda-Developer-Preview-x86_64-ci.iso`——无人值守生命周期需要 CI 自动
安装项作为 GRUB 默认项。OVMF 固件默认路径是 Debian/Ubuntu 布局
（`/usr/share/OVMF/OVMF_CODE_4M.fd` 与 `OVMF_VARS_4M.fd`）；Fedora 等其他
发行版请用环境变量覆盖，例如：

```bash
sudo env \
  OVMF_CODE=/usr/share/edk2/ovmf/OVMF_CODE.fd \
  OVMF_VARS_TEMPLATE=/usr/share/edk2/ovmf/OVMF_VARS.fd \
  os/scripts/test-install.sh
```

脚本执行以下真实状态转换：

1. 创建新的 64 GiB qcow2 磁盘；
2. 以 OVMF UEFI 启动 ISO；
3. 通过 CI 专用 Kickstart 清空并分区该磁盘；
4. 从 ISO 内嵌 OCI payload 安装系统，注册 Andromeda UEFI NVRAM 启动项并写入
   标准 fallback 路径；
5. 关机、移除 ISO，仅从安装后的磁盘启动；
6. 验证 UEFI、SELinux enforcing、SDDM、硬件报告和 `taskd /healthz`；
7. 下载 revision 2 归档，按宿主经 fw_cfg 注入的期望 SHA-256 校验通过后才
   导入，执行 `bootc switch` 并重启；
8. 确认 revision 2 已启动，执行 `bootc rollback` 并重启；
9. 确认 revision 1 恢复，输出 `ANDROMEDA_E2E_OK`。

CI 分区使用 UEFI ESP、独立 `/boot` 和带 `andromeda-root` 标签的根分区。
独立 `/boot` 遵循 bootc/bootupd 的推荐磁盘布局；验收脚本按 GPT 类型和文件系统
标签发现分区，不依赖易漂移的 `p1`、`p2` 顺序。

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

## 已验证构建

截至 2026-07-28，Developer Preview 0 已获得首个完整通过的安装与生命周期证据：

| 项目 | 已验证值 |
|---|---|
| 源分支提交 | `469869e268220e6be56d9adc19c9fbdd4a58c10c` |
| GitHub Actions | [Installable OS #30341131852](https://github.com/oratis/Andromeda/actions/runs/30341131852) |
| 运行结论 | `success` |
| ISO SHA-256 | `c04d8f6de780f978e261e1867283894abf2a7996b6105525660c52343ae45073` |
| ISO artifact | `Andromeda-Developer-Preview-x86_64-e47629d3573764e3c57a495d0a9c05054662c769`（ID `8682102044`） |
| ISO artifact ZIP digest | `sha256:5318570359dd7bb21347b8a8c0545447fb0cd6211e7b5ab987d2b090d8185fe8` |
| 串口证据 artifact | `Andromeda-serial-evidence-e47629d3573764e3c57a495d0a9c05054662c769`（ID `8682089312`） |
| 串口 artifact ZIP digest | `sha256:99ddabef2e7eb1d2481323068b7c1046fa7f5f643684722b78c0e262be1dde8d` |

产物名中的 `e47629d…` 是 pull request 测试使用的临时 merge commit；上表另列出了
实际源分支提交。GitHub Actions artifact 有保留期限，长期可复核记录以运行日志、
本节中的 digest 和仓库测试脚本为准。

该运行提供了以下实际证据：

- 在全新 32 GiB qcow2 上建立 600 MiB ESP、2 GiB `/boot` 和
  29.4 GiB `andromeda-root`，安装器退出状态为 0；
- ESP 同时包含 Fedora shim/GRUB 和 `EFI/BOOT/BOOTX64.EFI` fallback，OVMF NVRAM
  包含 `Andromeda` 启动项；
- 三次硬盘启动都保留同一 `root=UUID` 与 `boot=UUID`，使用
  `selinux=1 enforcing=1`，没有继承安装环境的 `selinux=0`；
- 首次启动、revision 2 和回滚后的 revision 1 均通过 UEFI、SELinux enforcing、
  硬件报告、`andromeda-taskd /healthz` 与 SDDM 检查；
- 串口依次出现本节上一段列出的六个成功标记，最终为 `ANDROMEDA_E2E_OK`，且没有
  `ANDROMEDA_E2E_FAILED`；
- `qemu-img check` 报告磁盘镜像无错误。

这个证据只证明表格中定义的 QEMU/KVM x86-64 + OVMF 边界，不把任何真实 PC 或
Mac 自动提升为 Supported/Certified。

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
