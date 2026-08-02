# Hardware Compatibility Manifest 开发说明

## 探测不等于支持

硬件 probe 只能回答“系统看到了什么”。Supported/Certified 必须同时具备：

- 精确整机、主板和固件 selector；
- kernel/firmware/driver 版本组合；
- 实机启动、安装、更新、回滚、休眠、GPU、网络、音频和外设测试；
- 签名 HCM；
- 可追溯 CI evidence；
- 安全更新责任人与支持期限。

## 信任边界：真实性必须显式开启

**默认不校验真实性。** 不带 `--trusted-keys` 时，matcher 只校验清单的**内部一致性与
新鲜度**（selector 是否命中、requirement 是否满足、证据是否过期、supported 以上是否
固定了制品）。此时清单的 `tier` 是**自我声明**的，任何人写一份文件都能声明
`certified`；`sha256` 也被原样采信。这条路径的结果是**咨询性的，不是信任门控**。

### 三档校验强度

| 调用方式 | 校验内容 | 可否作信任门控 |
|---|---|---|
| `hardware check m.json` | 一致性 + 新鲜度 | **否**（会打印两条 warning） |
| `+ --artifact-root <dir>` | 追加：每个 pin 解析为 `<dir>/<name>`、重算 SHA-256 比对；不匹配或缺失即 `blocked` | 否（清单本身仍未认证） |
| `+ --trusted-keys <file>` | 追加：清单必须携带 detached ed25519 签名，且解析到 keyring 中的可信 key 并在**规范化字节**上验签，否则 `effective_tier` 被打到 `blocked` | **是** |

`--trusted-keys` 接受 `{"key_id": "<64 位 hex 验证公钥>"}` 的 JSON 文件。空文件被拒绝
（空 keyring 谁都不信，会静默阻断一切，属于易误用的配置）。

`--artifact-signing-key <id>`（可重复，需配合 `--artifact-root`）另外要求每个**制品 pin**
声明可信 key id；它认证的是制品，与 `--trusted-keys` 认证清单本身是两件事。

### 高等级门控 fail-closed

`--require-tier supported` 或 `certified` 表达的是对真实硬件的真实承诺，因此
**不带 `--trusted-keys` 时会被直接拒绝**（非零退出，不产生判定结果），而不是打印一条
没人看的警告。确实需要咨询性检查时，必须显式加 `--allow-unverified` 表示知情。

`blocked`/`community`/`reference` 不作真实硬件承诺（`reference` 只代表虚拟证据），
自我声明它们不会误导任何人，因此不受此限制。

### 签发：`hardware keygen` 与 `hardware sign`

验签只有在能签名时才有意义。两个子命令在**离线签名机**上运行，都不探测本机硬件：

```bash
andromeda hardware keygen --seed-file root.seed --key-id andromeda-hcm-root
```

从一个 **32 字节种子**（64 位 hex 或 32 原始字节）导出验证公钥，并直接打印可粘贴进
keyring 文件的 `keyring_entry`。签名是**由种子确定性导出**的，不用 RNG，因此离线签名器
可复现。

> **本工具不生成密钥材料。** 种子的生成、离线保管（HSM/离线根）与轮换是部署职责；
> 一个开发者 CLI 随手造出的密钥没人能对其负责。持有种子者即可签发任意清单。

```bash
andromeda hardware sign cohort.json --seed-file root.seed \
  --key-id andromeda-hcm-root --output cohort-signed.json
```

规范化会剥离已有 `signature`，因此对已签名清单重新签名是安全的。

完整闭环：`keygen` → 把 `keyring_entry` 写入 keyring 文件 → `sign` → `check --trusted-keys`。

### 已验证的行为

以一份 selector 命中本机、`requirements: []`、含任意 `sha256` 与 2099 年到期证据、
**且不带签名**的伪造清单实测：

- `--require-tier certified` → **拒绝执行**，提示需要 `--trusted-keys`；
- `--require-tier certified --allow-unverified` → `certified`（这正是咨询模式的含义，
  也是为什么它不能当门控）；
- `--require-tier certified --trusted-keys keys.json` → `blocked`，原因为
  `manifest is unsigned but a trusted keyring is configured; authenticity cannot be established`。

对**已签名**清单的补充实测（同一 keyring）：

| 场景 | 结果 |
|---|---|
| 用可信种子签名，但制品 pin 声明了 keyring 之外的 key id | `blocked` —— `artifact 'vmlinuz' names signing key 'totally-real-key', which is not in the trusted keyring`（纵深防御：清单真实性与制品真实性分别成立才放行） |
| 内部一致、正确签名、制品哈希与 key id 均匹配 | **`certified`，`missing: []`，退出码 0**（正向路径确实可通过——一个永远 `blocked` 的门控没有价值） |
| 在签名后篡改任一字段（如改 `name`） | `blocked` —— `manifest signature failed ed25519 verification ... Verification equation was not satisfied` |

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
- capability evidence 必须未过期、可追溯，且其判定不阻断所声明的 tier（见下）；
- 支持声明到期后自动降为 Blocked；
- PCI 设备可以匹配 subsystem vendor/device 和 revision，不能只按品牌承诺。

Schema 位于 [`schemas/hardware-compatibility-manifest.schema.json`](../../schemas/hardware-compatibility-manifest.schema.json)，示例位于 [`examples/hcm/developer-x86_64-pc.json`](../../examples/hcm/developer-x86_64-pc.json)。

## 证据判定词汇

`evidence.result` 取四值。此前只有 `passed`/`failed`，导致
[认证测试计划](./hardware-certification-test-plan.md) §7 要求的"降级"与"未知"
**无法表达**——而这两种状态恰恰是整套支持策略赖以成立的部分：把降级压成
`passed` 是夸大，压成 `failed` 又会阻断一台真正可用的机器。

| 取值 | 含义 | 阻断哪些 tier |
|---|---|---|
| `passed` | 能力验证通过 | 无 |
| `degraded` | 能力可用但有**必须公开披露**的限制（例如 codec 只能解码不能编码、suspend 仅在交流供电下可靠） | 仅阻断 `certified` |
| `failed` | 能力验证失败 | **全部** |
| `unknown` | 未产出判定：未运行、结论不确定或证据不可获取 | **全部** |

两条设计约束值得记录：

1. **`unknown` 在每个 tier 都 fail-closed。** "没测"是最容易被误当成"测过没问题"的
   状态，认证计划也明确 `unknown` 不能晋级，因此它不被当作"缺省无此项"忽略。
2. **认证计划的 `blocked` 映射到 `failed`，`failed` 没有被改名。** 规范化会重新
   序列化类型化模型，改名或调整既有取值的编码会让**所有已签名清单的签名失效**。
   同理，新增取值不影响 `passed`/`failed` 的编码——仓库有两条测试锁定这一点：
   一条断言四个取值的 wire 编码，一条对只用旧取值的清单做真实签名→验签往返。

`degraded` 通过时，评估输出的 `evidence` 中会显式带上"限制必须公开披露"，
避免降级被静默接受。

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

> **真实性语义见上文[“信任边界：真实性必须显式开启”](#信任边界真实性必须显式开启)
> 及其三档校验强度表。** 概括：不带 `--trusted-keys` 的调用是咨询性的，退出码不具备
> 真实性保证；带 `--trusted-keys` 时验签 fail-closed，退出码可作信任门控；
> `--require-tier supported|certified` 不带 keyring 会被直接拒绝执行，除非显式
> `--allow-unverified`。

`andromeda hardware check <manifest>` 可以直接用于脚本和 CI 门禁：

- 退出码 `0`：selector 匹配、requirements 满足，且（如提供
  `--require-tier`）有效 tier 达标；
- 退出码 `1`：**拒绝执行**——`--require-tier supported|certified` 既未带
  `--trusted-keys` 也未带 `--allow-unverified`，不产生判定结果；probe 失败、
  manifest 无法读取或 `schema_version` 未知等输入错误同样以 `1` 退出；
- 退出码 `2`：`effective_tier` 为 `blocked`；
- 退出码 `3`：`effective_tier` 低于 `--require-tier <tier>` 指定的最低
  等级。

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

1. HCM detached ed25519 验签已在库层与 CLI 层（`hardware check --trusted-keys`，配套
   `hardware keygen` / `hardware sign`）落地并 fail-closed；待办：建立离线根密钥的
   生成、分发与轮换流程，并补充签名/密钥撤销机制；
2. 对 HEP OCI digest 与签名身份进行在线/离线验证；
3. 从 Windows/macOS source agent 导入更完整但经用户同意的设备 inventory；
4. 建立 QEMU、参考 PC、Intel Mac、T2、M1/M2 分离的实验室队列；
5. 只有 CI 达到 SLO 后才允许 tier promotion。

通用镜像覆盖、虚拟硬件矩阵和实体机认证清单见
[硬件普适性工程](./hardware-enablement.md)。从虚拟验证晋级到精确机型
Supported/Certified 的节点、测试、证据和阻断协议见
[实体硬件认证测试计划](./hardware-certification-test-plan.md)。
