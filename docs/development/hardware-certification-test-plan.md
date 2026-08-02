# Andromeda 实体硬件认证测试计划

> 状态：测试协议 v0.1。本文定义怎样把虚拟机验证升级为精确机型的
> `Supported` 或 `Certified` 证据，不代表当前已有实体机通过认证。

## 1. 目标与边界

“支持全部硬件”不是一个可以一次性证明的布尔值。Andromeda 将它实现为：

1. 通用 PC 镜像尽量包含可再分发的上游驱动与固件；
2. 每台机器先运行隐私受控的 probe 和诊断；
3. 精确主板、设备 ID、固件、内核和 HEP 组合进入独立 cohort；
4. 虚拟机证明安装与系统生命周期，实体机证明真实设备行为；
5. 只有未过期、可追溯的实体证据才能提升支持 Tier；
6. 未知或变化过的组合安全降级，不从相似 CPU 或产品名称推断兼容。

当前 `pc_x86_64` ISO 的 QEMU 结果只能作为 `Reference` 证据。非 T2 Intel
Mac、T2 Mac 与 Apple silicon 使用不同 Boot Provider 和镜像路线，不能用
PC ISO 的结果代替。

## 2. 认证对象

一个认证对象不是营销型号，而是以下字段的稳定组合：

- OEM、产品型号、主板名称和主板 revision；
- BIOS/UEFI、EC、Thunderbolt/USB4、PD controller 与 SSD firmware；
- CPU family、GPU PCI ID/subsystem ID/revision；
- 存储、网卡、Wi-Fi、蓝牙、音频、摄像头和输入设备 ID；
- 显示面板或外接显示路径；
- Andromeda OCI digest、ISO digest、kernel、driver、firmware 与 HEP digest；
- Boot Provider；
- 测试协议版本和测试设备版本。

同一笔记本型号发生 Wi-Fi、摄像头、面板或 SSD BOM 替换时，必须产生新 cohort
或显式扩展 selector，不能自动继承旧证据。

## 3. 证据等级

| 等级 | 环境 | 能证明什么 | 不能证明什么 |
| --- | --- | --- | --- |
| L0 Source | Linux/macOS/Windows CI | schema、单测、跨平台 probe | OS 能安装或设备可用 |
| L1 Artifact | 原生架构构建节点 | ISO/OCI digest、签名、SBOM、模块与 firmware 依赖 | 真实启动固件和外设 |
| L2 Virtual | QEMU + OVMF | 空盘安装、首次启动、更新、回滚和模拟硬件矩阵 | 真实 GPU、睡眠、无线、电池与 Apple 启动链 |
| L3 Physical | 精确实体 cohort | 真机启动、设备功能、功耗、恢复和长稳 | 未测试的 BOM 或相似型号 |
| L4 Promotion | 签名证据审核 | 将精确 HCM 晋级并投入灰度 | 永久支持；证据仍会过期 |

任何 artifact pin、系统镜像、kernel、HEP、关键 firmware 或测试协议变化，都使
相关 L3 证据进入 `needs_review`。新的 L0–L2 结果不能自动恢复
`Supported`/`Certified`。

## 4. 实验室节点

每个节点至少具备：

- 独立测试盘；不得把研发镜像自动安装到含用户数据的磁盘；
- PiKVM 或等价 HDMI capture、USB HID 注入和远程电源控制；
- 可保存的 UEFI 设置、固件版本和启动画面；
- 串口、netconsole 或 pstore 中至少一种崩溃证据通道；
- 可切换的 WPA2/WPA3 2.4/5/6 GHz AP；
- 蓝牙耳机、手柄、键鼠和可编程 USB hub；
- HDMI、DisplayPort、USB-C/USB4/Thunderbolt 坞站与双显示器；
- 音频 loopback、摄像头测试卡和 privacy LED 观察；
- 交流功耗计；笔记本增加 USB-C PD 分析仪；
- 可控断电和备用恢复介质。

Apple silicon/T2 节点还必须保留可用的 macOS Recovery；Apple silicon 实验室
必须有另一台受支持的 Mac 和数据线执行 DFU restore。未经验证的脚本不得修改
APFS container、Apple boot policy 或 Recovery。

## 5. 首批实体机队列

首批队列优先覆盖差异，而不是追求相同 CPU 的数量：

| 节点 | 代表路径 | 初始目标 |
| --- | --- | --- |
| Intel 核显商务本 | Intel CPU/iGPU、Intel Wi-Fi、Thunderbolt | PC Supported 候选 |
| AMD APU 笔记本 | AMD CPU/GPU、Realtek/MediaTek 无线 | PC Supported 候选 |
| NVIDIA 游戏本 | 混合显卡、MUX、高刷、专有驱动 | 独立 HEP Pilot |
| AMD 独显台式机 | AMD GPU、多个 NVMe、USB 音频 | PC Supported 候选 |
| 旧 UEFI/SATA PC | SATA/AHCI、旧 USB 和旧无线 | Community/Pilot |
| 非 T2 Intel Mac | Apple EFI、Intel iGPU、外置目标盘 | 逐机型 Pilot |
| T2 Intel Mac | T2Linux kernel/HEP、本机固件导入 | Experimental |
| M1 与 M2 Mac 各一台 | Asahi 启动链与配对固件 | 独立 Preview |

M3 及更新 Apple silicon 在上游安装器、设备支持与恢复路径就绪前只进入
`Watch`，不得从 M1/M2 结果外推。

## 6. 安全安装协议

### 6.1 预检

1. 记录系统时间、测试员、节点 ID 和 capture 流；
2. 保存固件设置并确认恢复路径；
3. 对目标盘读取型号、容量和序列号的本地哈希；
4. 运行 `andromeda hardware probe` 与 `andromeda hardware diagnose`；
5. 验证 ISO SHA-256、payload OCI digest、Platform Variant、Boot Provider 和
   HEP ID；
6. 确认目标盘是实验室独立盘或已完成镜像备份；
7. 安装器必须拒绝错误架构、错误 Platform Variant、Apple/PC 交叉刷写和未知
   Boot Provider。

### 6.2 安装

- 从冷关机状态启动 USB；
- 验证固件启动菜单和 fallback EFI 路径；
- 只选择已登记目标盘；
- 在分区、payload 导入、bootloader 写入三个阶段分别做一次受控断电；
- 每次断电后验证源盘、目标分区表、Recovery 和再次安装能力；
- 完整安装后验证 ESP、根文件系统、启动项和一次性安装介质标记；
- 移除 USB 后完成首次启动。

Apple 路径的额外限制：

- 非 T2 Intel Mac 第一阶段只允许空白外置盘；
- T2 必须使用独立 Experimental 镜像和本机 firmware importer；
- Apple silicon 必须调用经过审核的 Asahi 安装/空间分配流程，不能复用 PC
  Anaconda 分区逻辑。

## 7. 必测套件

每项输出 `pass`、`degraded`、`unsupported`、`blocked` 或 `unknown`，并附原始
证据 URI。`unknown` 不能晋级。

写入 HCM 时按下表映射到 `evidence.result`（详见
[HCM 开发说明](./hardware-compatibility.md#证据判定词汇)）：

| 套件判定 | HCM `evidence.result` | 说明 |
|---|---|---|
| `pass` | `passed` | |
| `degraded` | `degraded` | 仅阻断 `certified`；限制必须公开披露 |
| `blocked` | `failed` | HCM 保留 `failed` 而非改名为 `blocked`——改名会使**所有已签名清单失效** |
| `unknown` | `unknown` | 在每个 tier 都阻断 |
| `unsupported` | —— | 表示"本机不具备该能力"，不是测试失败。**不写入 `evidence[]`**，应记入 cohort 的已知缺口清单，否则会与"测过但不通过"混淆 |

### 7.1 启动与生命周期

- 冷启动 10 次、热重启 10 次、关机 10 次；
- 断开安装介质后从正确磁盘启动；
- 首次启动、用户创建、磁盘加密恢复密钥确认；
- vN → vN+1 更新和手动回滚；
- 启动计数触发的自动回滚；
- 更新下载时磁盘不足、payload 导入时断电、部署完成后断电；
- 回滚后用户数据和系统设置保持；
- `qemu-img check` 仅用于虚拟磁盘；实体盘使用只读健康与文件系统检查。

### 7.2 睡眠、电源与热

- AC 和电池各执行 100 次 suspend/resume；
- Release Candidate 执行 500 次循环和 24 小时待机；
- 合盖/开盖、外接屏睡眠、低电量唤醒；
- Wi-Fi、蓝牙、摄像头和音频在唤醒后恢复；
- 空闲、视频播放和 GPU 负载功耗基线；
- 风扇、温度和 thermal throttling；
- 任何过热、风扇失控或扬声器安全问题均为全量阻断。

### 7.3 存储与外设

- NVMe/SATA 读写、TRIM、SMART/NVMe health；
- USB 2/3、Type-C、UAS、SD 卡和可移动盘热插拔；
- HDMI/DP/USB-C 显示热插拔、旋转、缩放和多屏；
- USB4/Thunderbolt 授权、休眠后恢复和 DMA policy；
- 键盘、触控板、鼠标、功能键、触摸屏和游戏手柄；
- 打印、扫描、手机 MTP/PTP 和常见文件系统只读/读写策略。

### 7.4 网络

- 有线 DHCP、静态地址、IPv4/IPv6；
- 2.4/5/6 GHz、WPA2/WPA3 和 AP roaming；
- suspend/resume 后自动重连；
- captive portal、VPN、断网恢复和 DNS 变化；
- 蓝牙配对、A2DP/HFP 切换、手柄和 BLE；
- WWAN 节点验证 SIM/eSIM、掉线恢复和飞行模式。

### 7.5 图形、游戏与媒体

- Wayland 登录、软件渲染回退和 GPU driver 确认；
- 内外屏分辨率、缩放、高刷、VRR 和 HDR 能力声明；
- Vulkan/OpenGL smoke、规定 CTS 子集和 GPU reset 恢复；
- 混合显卡 offload、MUX 和外接显示归属；
- Steam/Proton 代表游戏启动、手柄、音频与 shader cache；
- PipeWire 输出/输入、HDMI/USB/蓝牙设备切换；
- 摄像头 PipeWire portal、privacy LED 和麦克风静音；
- 硬件编解码能力按 codec 分项记录，不能写成笼统的“视频支持”。

## 8. 故障注入与恢复

至少执行：

- 安装、更新、bootloader 写入和首次启动阶段断电；
- 根分区校验失败、无可用空间和只读文件系统；
- 失效的新部署与连续三次启动失败；
- Wi-Fi/AP 消失、USB/坞站突然断开和 GPU reset；
- 可恢复的 NVMe I/O 错误注入；
- 固件升级只在厂商支持的恢复路径和备用设备就绪时测试。

验收结果必须说明恢复是自动完成、用户可完成，还是需要实验室/厂商工具。
“重新安装能用”不能替代更新回滚或数据保留证明。

## 9. 证据包

每次运行产生一个内容寻址、只追加的证据包：

```text
evidence/<run-id>/
├── run.json
├── platform-probe.json
├── diagnosis.json
├── artifact-manifest.json
├── firmware-inventory.json
├── results.json
├── serial-or-pstore/
├── screenshots/
├── power/
└── sha256sums.txt
```

`run.json` 至少包含：

- cohort/HCM ID、匿名节点 ID 和测试协议版本；
- ISO、OCI、HEP、kernel、driver 与 firmware digest/version；
- Boot Provider、安装目标类型和固件设置摘要；
- 每个 test ID 的开始/结束时间、结果和 evidence path；
- runner/controller 版本；
- 证据包 SHA-256 和实验室签名身份。

设备序列号、MAC、磁盘 UUID 和 Apple platform UUID 默认不进入公共证据。公共
HCM 只保存满足精确匹配所需的非唯一字段和证据 URI。

## 10. Tier 晋级与阻断

正式支持 Tier 与代码中的 `SupportTier` 枚举一一对应，阶梯从低到高为
`Blocked < Community < Reference < Supported < Certified`（`Reference`
只有虚拟 L0–L2 证据，低于要求实体认证的 `Supported`）。该阶梯的**唯一权威
来源**是 `crates/andromeda-hardware/src/model.rs` 的 `SupportTier` 枚举声明
顺序；JSON schema 的 `tier` enum 与之同序，并由 CI 测试
（`matcher.rs::schema_tier_enum_matches_model_declaration_order`）锁定。

> 战略/产品文档里的 “OEM Reference Design”（OEM 共控 BOM/固件的高端产品线）
> 是**产品线标签**，与本表 `Reference` 这个 `SupportTier` 取值无关：代码语义为准，
> `Reference` 是阶梯的**次低**位、只代表虚拟参考证据，不是最高级。HCM 的 `tier`
> 字段只能取上述五个值之一。

| Tier | 最低要求 |
| --- | --- |
| Blocked | 错误平台、启动关键驱动缺失、证据失败/过期或 artifact 未固定 |
| Community | 上游路径存在；公开缺口；不承诺实验室 SLO |
| Reference | L0–L2 全绿；用于虚拟参考，不代表真机 |
| Supported | 两轮连续 RC 全套通过；artifact pin、当前证据、责任人和期限完整 |
| Certified | Supported 加长稳、外设/功耗目标、发布灰度和支持 SLO |

本文其余章节出现的 Pilot、Experimental、Preview 与 Watch 是**实验室
cohort 阶段标签**，不是 `SupportTier` 的取值，HCM 中不能声明它们。规划中
的 Pilot 阶段（一台精确 cohort 完成 L3 基础套件、有已知问题和恢复路径）
在成为正式 Tier 之前，对应机型在 HCM 中最多声明 `Community`。

发布阻断规则：

- 数据损坏、不可恢复启动、过热或设备安全风险：停止全量发布；
- 单一 cohort 黑屏、丢盘或无法回滚：撤回该 HCM 更新资格；
- 任一必需证据失败或过期：有效 Tier 降为 `Blocked`；
- 新 cohort 连续两轮 RC 通过后才能进入 `Supported`；
- OTA 按 HCM cohort 从实验室、1%、10%、50% 到 100% 分批；
- 社区遥测只能创建调查线索，不能替代实体实验室证据。

## 11. 与现有自动化衔接

合并前必须先通过：

```bash
cargo test --workspace --locked
os/scripts/test-installer-platform-guard.sh
os/scripts/test-containerfile-layer-budget.sh
os/scripts/test-hardware-matrix.sh output
os/scripts/test-install.sh output
```

当前虚拟矩阵覆盖 `modern-nvme`、`q35-sata` 与 `legacy-i440fx`。实体调度器应
复用相同的生命周期成功/失败标记，但不能把虚拟矩阵结果写入实体 HCM。

实体结果写回 HCM 前必须：

1. 校验 evidence 包签名和所有 digest；
2. 校验 HCM selector 精确匹配；
3. 校验 artifact pins 与被测镜像一致；
4. 校验全部必需 capability evidence 为 `passed` 且未过期；
5. 由第二位审核者批准 Tier 变化；
6. 保存撤销入口，以便发现回归时立即阻断该 cohort。

## 12. HCM 清单签名与真实性门禁

第 3 节的证据级别只回答“清单声称的事情是否被验证过”，不回答“这份清单是不是被
篡改或伪造的”。真实性由**清单级 detached ed25519 签名**保证（安全评审发现 #1）：

- `Supported`/`Certified` 清单**必须**由可信密钥签名，且评估方**必须提供
  `TrustedKeyring`**（`evaluate_manifest_verified` / `evaluate_manifest_at_verified`）
  后再采信；未签名、未知 `key_id`、签名格式非法或验签失败一律 fail-closed 到
  `Blocked`。artifact pin 的 `signing_key_id` 也必须命中同一 keyring。
- 规范化规则、`signature` 字段格式、库 API 与签名流程见
  [HCM 开发说明](./hardware-compatibility.md) 的“HCM 清单签名与真实性”一节，
  唯一权威实现在 `crates/andromeda-hardware/src/signing.rs`。
- **`andromeda hardware check` 默认咨询性、不可作信任决策**：不带 keyring 的调用只
  校验一致性与新鲜度。把清单真实性纳入 CI/发布门禁时必须传入 keyring：
  `andromeda hardware check <manifest> --require-tier supported --trusted-keys keys.json`。
  不带 `--trusted-keys` 时，该 `--require-tier` 组合会被直接拒绝执行（退出码 1，
  除非显式 `--allow-unverified` 承认仅作咨询）。
- 密钥生成、分发、轮换与**生产清单的实际签名**是部署/运维职责；代码提供验证路径、
  可复现的签名助手 `ManifestSigningKey` 与上述流程约定，不代替运维。

这一门禁与第 11 节的写回校验叠加：evidence 包签名保证“测试证据未被篡改”，清单签名
保证“采信的这份 HCM 本身来自可信发布方”。
