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

## HCM 清单签名与真实性（fail-closed）

matcher 过去只校验清单的**内部一致性与新鲜度**，从不校验**真实性**：伪造一份
`tier: certified`、selector 命中本机、证据“通过 / 2099 到期”的清单即可得到
`certified`（安全评审发现 #1）。本节描述的 detached 签名机制关闭该缺口。

### 机制

- 清单可携带一个可选字段 `signature: { key_id, sig }`：
  - `key_id` 指明应由哪把公钥验签；
  - `sig` 是对清单**规范化字节**的 detached ed25519 签名，64 字节、128 位小写
    十六进制。
- **规范化（canonicalization）规则**——签名与验签走同一条路径，唯一权威定义在
  `crates/andromeda-hardware/src/signing.rs` 的 `canonical_signing_bytes`：
  1. 对**类型化模型**（而非原始文件）序列化，因此源文件的空白、键顺序，以及可选
     字段写成 `null` 还是省略，都不改变被签字节；
  2. 删除 `signature` 字段（签名不能覆盖自身）；
  3. 输出紧凑 JSON，且**每个对象的键按 Unicode 标量值排序**；数组保持原有顺序；
  4. 标量沿用 `serde_json` 的编码（字符串正确转义；清单只含整数与布尔，本身确定）。
- **可信 keyring**：`TrustedKeyring` 把 `key_id` 映射到 ed25519 验签公钥（公钥同样以
  64 位十六进制表示）。keyring 是唯一信任锚——空 keyring 不信任任何东西。

### 强制语义

- **提供 keyring 时（fail-closed）**：清单必须带签名、`key_id` 命中 keyring、且签名对
  规范化字节验签通过，否则 `effective_tier` 降为 `Blocked`。未签名、未知 key、签名
  格式非法、验签失败一律 `Blocked`；此外每个 artifact pin 的 `signing_key_id` 也必须
  命中同一 keyring。
- **未提供 keyring 时（咨询性，向后兼容不变）**：不要求也不校验签名，行为与历史
  matcher 完全一致。
- **manifest 级与 artifact 级签名的关系**：manifest 级签名是**真实性门禁**——一旦验签
  通过，即同时认证了清单内声明的全部 `sha256` 与 `signing_key_id`；artifact 级
  `signing_key_id` 标识“哪把受信任密钥为该制品摘要背书”，在 keyring 存在时由 matcher
  校验其命中 keyring。至于**制品字节**是否与被认证的 `sha256` 一致，则由可选的
  `ArtifactVerifier`（如 `DirectoryArtifactVerifier`）在本地重新哈希核对。

### 库 API

- 咨询性（默认，行为不变）：`evaluate_manifest` / `evaluate_manifest_with_verifier`；
- fail-closed 验签：`evaluate_manifest_verified(report, manifest, keyring, verifier)`，
  以及显式时钟的 `evaluate_manifest_at_verified(report, manifest, now, keyring, verifier)`。

### 如何签名一份清单

代码提供**验证路径**与一个确定性签名助手 `ManifestSigningKey`；密钥的生成、分发与
**生产清单的实际签名**属于部署/运维职责，本 crate 不做固定：

1. 离线生成 ed25519 私钥（32 字节 seed），存放于 HSM 或离线根并按策略轮换——本 crate
   不规定；
2. `ManifestSigningKey::from_seed(&seed)` 载入后，对**未签名**清单调用
   `sign_manifest(&manifest, "<key_id>")`，得到 `{ key_id, sig }`；
3. 把该对象写入清单的 `signature` 字段后发布；
4. 把对应验签公钥（`verifying_key_hex()`，64 位十六进制）以 `{ "<key_id>": "<hex>" }`
   形式分发给评估方，构成其 `TrustedKeyring`。

> 本 crate 只提供“验证 + 一个可复现的签名助手 + 上述规范化与流程约定”。私钥管理、
> 公钥分发与批量生产签名是运维工程，代码不代替。

## `hardware check` 作为预检门禁

> **重要：`andromeda hardware check` 目前是咨询性的，不可作信任决策。** 现有 CLI 走
> 默认（无 keyring）路径，只校验一致性与新鲜度，**不验签清单真实性**；伪造清单仍可得到
> 非 `blocked` 结果与退出码 0。要让退出码具备真实性保证，评估方必须经库 API 传入
> `TrustedKeyring`（`evaluate_manifest_verified`）。为 `hardware check` 增加
> `--trusted-keys <path>` 开关以在 CLI 层强制验签，是自然的后续项（见“下一步”）。

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

**`SupportTier` 的唯一权威定义在代码里**：`crates/andromeda-hardware/src/model.rs`
的 `SupportTier` 枚举声明顺序即上面的阶梯，`schemas/hardware-compatibility-manifest.schema.json`
的 `tier` enum 顺序与之保持一致，并由 `matcher.rs` 的
`schema_tier_enum_matches_model_declaration_order` 测试在 CI 中锁定，防止再次漂移。
其他文档一律引用本阶梯，不得各自复述出不同顺序。

> 注意：战略/产品文档中出现的 “OEM Reference Design”（OEM 共控 BOM/固件的
> 高端产品线）是一个**产品线标签**，与这里 `SupportTier::Reference` 这个枚举
> 取值**无关**。`SupportTier::Reference` 表示“仅有虚拟 L0–L2 证据、位于阶梯次低
> 位”的支持等级，绝不是最高级。HCM 的 `tier` 字段只能取本阶梯五个值，不能声明
> 任何产品线标签。

## 下一步

1. HCM detached ed25519 验签已在**库层**落地并 fail-closed（见“HCM 清单签名与真实性”）；
   待办：为 `andromeda hardware check` 增加 `--trusted-keys <path>` 开关，把验签下沉到
   CLI 层并使退出码具备真实性保证；同时建立离线根密钥的生成、分发与轮换流程；
2. 对 HEP OCI digest 与签名身份进行在线/离线验证；
3. 从 Windows/macOS source agent 导入更完整但经用户同意的设备 inventory；
4. 建立 QEMU、参考 PC、Intel Mac、T2、M1/M2 分离的实验室队列；
5. 只有 CI 达到 SLO 后才允许 tier promotion。

通用镜像覆盖、虚拟硬件矩阵和实体机认证清单见
[硬件普适性工程](./hardware-enablement.md)。从虚拟验证晋级到精确机型
Supported/Certified 的节点、测试、证据和阻断协议见
[实体硬件认证测试计划](./hardware-certification-test-plan.md)。
