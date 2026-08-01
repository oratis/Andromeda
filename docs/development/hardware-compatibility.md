# Hardware Compatibility Manifest 开发说明

## 探测不等于支持

硬件 probe 只能回答“系统看到了什么”。Supported/Certified 必须同时具备：

- 精确整机、主板和固件 selector；
- kernel/firmware/driver 版本组合；
- 实机启动、安装、更新、回滚、休眠、GPU、网络、音频和外设测试；
- 签名 HCM；
- 可追溯 CI evidence；
- 安全更新责任人与支持期限。

## v1 报告与诊断

报告包含：

- OS family 与 CPU 架构；
- manufacturer/model/board/firmware；
- CPU、逻辑核心和内存；
- UEFI、Secure Boot、TPM2、虚拟化（每项均为三态：`true`/`false`/`null`，
  `null` 表示"无法验证"，与"验证为不存在"严格区分，例如未提权的
  Windows probe 无法执行 `Confirm-SecureBootUEFI` 和 TPM 查询）；
- Linux PCI ID 与绑定驱动；PCI subsystem ID、revision 与 modalias；
- Linux USB 按**接口（function）层**采集：sysfs 中 USB 设备没有 `class`
  属性、复合设备的 `bDeviceClass` 为 `00`，真正的类别（`bInterfaceClass`）
  与驱动绑定（usbhid、uvcvideo 等）都在接口层，因此报告中的 USB 条目地址形如
  `1-1:1.0`，`class` 为两位十六进制的 `bInterfaceClass`，hub 接口不计入；
- 设备清单超过每总线 512 条时截断并追加警告；
- probe 无法验证的警告。

`andromeda hardware diagnose` 在 probe 之上分类存储、网络、GPU、音频、USB
控制器、输入、摄像头和无线设备。整机 `blocked` 按**类别存在性**判断：某个
启动关键类别（存储、网络、GPU、USB 控制器）在清单中存在、却没有任何一台
可用（已绑定驱动）设备时才阻断。只要有一条可用路径——例如有线网卡正常而
Wi-Fi 无驱动，或主 GPU 正常而副 GPU 无驱动——整机降级为 `needs_review`
并给出设备级建议，而不是全盘阻断。Nouveau/NVK 等有限路径同样为
`needs_review`。`boot_critical_missing` 字段统计的是"没有任何可用设备的
启动关键类别"数量。

为保护隐私，v0 不采集序列号、磁盘 UUID、MAC 地址、Windows machine GUID 或 Apple platform UUID。

## HCM v2 规则

- `selectors` 使用 OR：至少一个 selector 完整匹配；
- 同一个 selector 内字段使用 AND；
- 空 selector 列表永远不匹配，避免误把所有硬件升级为支持；
- 每个 selector 必须至少有一个非空判别字段（`os_family`、非空
  `architectures`、非空 `manufacturer_contains` 或非空 `model_prefix`）：
  全 null/全空的 selector 永远不匹配，matcher 与 JSON Schema 的 `anyOf`
  约束共同执行这条规则；
- `requirements` 全部满足才保留声明 tier；
- 任意 selector/requirement 失败，`effective_tier` 降为 `blocked`；
- ID 比较忽略大小写，并忽略至多一个 `0x`/`0X` 前缀（`0x0x10de` 不等于
  `10de`）；
- 需要驱动的设备只要清单中**任意一台** ID 匹配的设备绑定了驱动即满足：
  双网卡/双 GPU 中第一台未绑定驱动不会掩盖第二台已绑定的设备；
- CLI 在评估前校验 `schema_version`，未知版本直接报错拒绝；
- PC、Intel Mac、T2 与 Apple Silicon 必须声明不同 boot provider；
- `boot_provider` 会原样出现在评估输出中，但由**安装器预检**（platform
  identity 比对）而非 matcher 执行；matcher 的 evidence 中会明确注明这一点；
- Supported 及以上必须固定 kernel/driver/firmware/HEP artifact；
- capability evidence 必须通过、未过期且可追溯；
- 支持声明到期后自动降为 Blocked；
- PCI 设备可以匹配 subsystem vendor/device 和 revision，不能只按品牌承诺。

Schema 位于 [`schemas/hardware-compatibility-manifest.schema.json`](../../schemas/hardware-compatibility-manifest.schema.json)，示例位于 [`examples/hcm/developer-x86_64-pc.json`](../../examples/hcm/developer-x86_64-pc.json)。

## `hardware check` 作为预检门禁

`andromeda hardware check <manifest>` 可以直接用于脚本和 CI 门禁：

- 退出码 `0`：selector 匹配、requirements 满足，且（如提供
  `--require-tier`）有效 tier 达标；
- 退出码 `2`：`effective_tier` 为 `blocked`；
- 退出码 `3`：`effective_tier` 低于 `--require-tier <tier>` 指定的最低
  等级；
- 其他非零退出码：probe 失败、manifest 无法读取或 `schema_version`
  未知。

Tier 阶梯从低到高为 `blocked < community < reference < supported <
certified`（`reference` 只有虚拟 L0–L2 证据，因此低于要求实体认证的
`supported`）。例如：

```bash
andromeda hardware check examples/hcm/developer-x86_64-pc.json --require-tier community
```

## 下一步

1. 对 HCM 使用离线根密钥和轮换签名；
2. 对 HEP OCI digest 与签名身份进行在线/离线验证；
3. 从 Windows/macOS source agent 导入更完整但经用户同意的设备 inventory；
4. 建立 QEMU、参考 PC、Intel Mac、T2、M1/M2 分离的实验室队列；
5. 只有 CI 达到 SLO 后才允许 tier promotion。

通用镜像覆盖、虚拟硬件矩阵和实体机认证清单见
[硬件普适性工程](./hardware-enablement.md)。从虚拟验证晋级到精确机型
Supported/Certified 的节点、测试、证据和阻断协议见
[实体硬件认证测试计划](./hardware-certification-test-plan.md)。
