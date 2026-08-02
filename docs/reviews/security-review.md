# 安全评审

> 范围：merged `main` 上的 Andromeda 授权引擎、能力模型、taskd、HCM matcher 及 OS/安装/供应链侧。仅安全维度。所有 `file:line` 均已核对；HCM 伪造场景已用 `andromeda hardware check` 实机复现。

## 概览与总评

Andromeda 当前是一个**纯决策（policy-as-a-decision-function）**的 v0 原型：`PolicyEngine::evaluate` 与 `evaluate_manifest` 只产出"允许/询问/拒绝"和"支持层级"判定，仓库中**尚不存在任何 executor**——没有代码真正打开文件、发网络请求或执行外部调用。这一点是理解全部结论的前提：几乎所有"绕过"在今天都**无法转化为真实副作用**，因为没有组件消费这些判定去执行。

在此前提下，纯逻辑质量**相当高**：deny-first 排序、路径规范化（`normalized_absolute`）、`satisfies` 隔离矩阵、网络子域/端口匹配、`ExternalCall`/`MoveFile` 双端点、HCM fail-closed（空/全 null/仅空白 selector、schema 版本、过期/失败证据、未固定制品）等均有正确实现且有回归测试覆盖。任务点名要检查的三处合并加固——deny 穿越、IPv6/loopback 与 dual-stack SSH 校验、all-null selector——**均已落地且正确**。

真正需要记录的是**信任边界与文档声称**层面的问题，而非可利用的逻辑漏洞：

- **能力（Capability）不可伪造性在 taskd 边界并不成立**：能力无签名，调用方在同一请求里同时提供 plan 和 capabilities；`issued_to` 仅被要求等于调用方自选的 `task_id`。
- **HCM matcher 只验证一致性与新鲜度，不验证真实性**：伪造清单可获得 `certified`（已复现）。
- **taskd 无鉴权**；`docs/development/task-control-plane.md:46` 声称"把 `ANDROMEDA_LISTEN` 改为非 loopback 会因 Host 校验全部被拒——有意的防裸奔"，此说法**不成立**。

**总评：作为"决策核心"的实现质量为良/优，无可直接利用的高危漏洞。** 但"能力不可伪造""HCM 可信""非 loopback 绑定自我保护"这三条若被上层集成当真，会在 executor 落地后立即变成严重问题。当前风险姿态：**低（原型阶段）；但存在若干"executor 落地前必须先修"的架构前置项。**

## 已验证的加固项

授权引擎（`crates/andromeda-policy/src/lib.rs`）
- **Deny-first + 理由累积**：任一检查失败即 `Deny`，deny 规则不被匹配的能力覆盖所推翻（`lib.rs:146-220`；测试 `deny_rules_override_a_matching_capability`）。
- **路径穿越防护**：目标先经 `normalized_absolute` 词法归一，`/tmp/../etc/passwd` 命中 `/etc`；不可归一/相对路径直接拒绝（`lib.rs:257-273` + `capability.rs:134-154`；测试 `deny_rules_cannot_be_dodged_by_path_traversal`、`relative_file_targets_are_denied`）。
- **Windows deny root 大小写不敏感**：`path_starts_with` 在 `#[cfg(windows)]` 下按分量 `eq_ignore_ascii_case`（`lib.rs:408-418`）。
- **网络 deny 覆盖子域/端口/大小写且不误伤 lookalike**：`sub.evil.com`、`EVIL.com`、`evil.com.` 均命中，`notevil.com` 不命中（`lib.rs:242-250,372-396`）。但覆盖有一处已知缺口：不可解析的端口后缀（如 `evil.com:99999`）会整串被当作主机名字面量，deny 规则不命中，见[威胁模型](../andromeda-threat-model.md) §6.4。
- **`ExternalCall` 在首个 `:` 切分并拒绝 service 名含 `:` 的歧义能力**（`lib.rs:334-348`）。
- **`MoveFile` 双端点写覆盖 + 目的地 deny 检查 + 缺失 destination 即拒绝**（`lib.rs:227-239,294-305`）。
- **能力生效窗口**：`now >= issued_at && now < expires_at`（`capability.rs:98-104`）。
- **隔离矩阵精确**：`Brokered` 不满足 `MicroVm`/`Sandbox`，`MicroVm` 满足 `Sandbox`（`capability.rs:186-194`）。

HCM（`crates/andromeda-hardware/src/matcher.rs`、`model.rs`）
- **selector fail-closed**：空列表、全 null、仅空白字段均不匹配（`matcher.rs:281-324`）；已实机确认全 null 被拒。
- **schema 版本门**、**过期清单/证据 → Blocked**、**`effective_tier` 永不高于声明**（`matcher.rs:34-45,63-77,218-273`）。

taskd（`crates/andromeda-taskd/src/lib.rs`、`main.rs`）
- **DNS-rebinding 防护（针对浏览器）**：只接受 `localhost/127.0.0.1/[::1]`（`lib.rs:72-98`）。
- **阻塞操作放入 `spawn_blocking`**，请求体 2MB 上限，plan `MAX_PLAN_ACTIONS=10_000` 上限，单任务 capability 总量 `MAX_TASK_CAPABILITIES=10_000` 上限（按授予后总量计，创建与补授两条路径都强制）。
- **plan 校验**：迭代式环检测不爆栈；**存储**：临时文件 + fsync + 目录 fsync + 原子 rename + 跨进程排他锁 + 乐观修订号 + 孤儿清理。

OS / 安装 / 供应链
- **SSH 关闭且 socket 也关闭**（`Containerfile:306`）；verify 脚本拒绝 IPv4/IPv6/dual-stack 通配 :22（`andromeda-daily-driver-verify:170-183`，显式覆盖 `[::]:22`）。
- **taskd systemd 加固**：`DynamicUser`、`ProtectSystem=strict`、`RestrictAddressFamilies`、`UMask=0077` 等。
- **安装 SELinux 处理正确**：安装器 `selinux=0`，向**目标**强制写 `selinux=1 enforcing=1` 并断言（`install-uefi-fallback.sh:107-125`）。
- **破坏性安装多重门控**：非默认启动项 + `ci` 模式要求 VM + `andromeda.ci=1` 路径。
- **更新载荷完整性**：期望 SHA-256 经可信 fw_cfg 通道下发并强校验（`andromeda-ci-verify:39-54`）。

## 发现

### 1. HCM 清单无真实性/签名校验，可伪造任意支持层级（中等）
- **攻击场景（已复现）**：构造 `tier: "certified"`、selector 命中本机、`requirements: []`、含**任意** `sha256` 制品与"passed / 2099 到期"证据的清单。运行 `andromeda hardware check forged.json --require-tier certified` 得到 `effective_tier: certified`、退出码 0。matcher 只校验**内部一致性与新鲜度**，从不核对制品哈希、证据 URI 或签名；`ArtifactPin.signing_key_id`（`model.rs:163`）被解析但**从未被使用**。
- **严重级别**：中等。今日安装器 preflight 用镜像 label 而非 HCM `check`，故 HCM 判定目前仅"咨询性"；但 `hardware check` 的退出码显然为脚本门控设计，一旦用于放行安装/驱动启用即升为严重。
- **修复建议**：为 HCM 引入 detached 签名 / cosign，matcher 评估前强制验签并用 `signing_key_id` 绑定可信公钥；对 `sha256` 与本地实际制品比对；未验签视作 `Blocked`。实现前文档需声明 `hardware check` 结果不可作信任决策。

### 2. taskd 无鉴权；Host 头对非 loopback 绑定不构成保护（文档过度声称）（中等）
- **攻击场景**：Host 中间件只检查 `Host` 头**取值**，不检查入站接口/套接字。若运维把 `ANDROMEDA_LISTEN` 设为 `0.0.0.0:7777`，同网段攻击者用任意非浏览器客户端发 `Host: 127.0.0.1` 即通过校验并获得**全部 API**（无鉴权、无 CSRF token）。`task-control-plane.md:46` 的"改为非 loopback 会全部被拒——防裸奔"**错误**。此外本地任意进程/用户均可经 loopback 无鉴权访问 API。
- **严重级别**：中等（今日无 executor，泄露面限于 task 元数据/plan；真实暴露还需运维主动改绑定）。executor 落地后为严重。
- **修复建议**：(a) 修正文档；(b) systemd 层加 `IPAddressAllow=localhost` + `IPAddressDeny=any` 做内核级 loopback 强制；(c) executor 落地前引入本地鉴权（`SO_PEERCRED`/UNIX socket）或按套接字对端地址判定回环。

### 3. 能力自证、isolation/confirmation 自报（中等，架构信任边界）
- **攻击场景**：`CreateTaskRequest` 由同一调用方同时提供 `plan` 与 `capabilities`（`service.rs:26-32`）。能力**无签名**，唯一主体绑定是"`issued_to` 必须等于 `plan.task_id`"（`service.rs:344-350`），而 `task_id` 也由调用方自选——可自签发任意能力。`/evaluate` 的 `isolation` 与 `external_side_effect_confirmed` 同样是调用方自报（`service.rs:198-238`，`subject=None`）。结果：taskd 边界唯一无法自绕的强制项仅剩默认 deny 路径根与动作 risk floor。
- **严重级别**：中等（当前无 executor 消费判定，不可利用）。
- **修复建议**：executor/broker 落地前把能力签发与策略评估分离——能力应由受信任宿主签发（签名或独立 grant store），taskd 不接受调用方自带裸能力；isolation/confirmation 改为由受信任执行环境证明（attestation）。将 `with_subject` 实际接入数据面。

### 4. 供应链：目标镜像可变 tag 且未验签；构建基础镜像未按 digest 固定（中等）
- **攻击场景**：安装均把 `--target-imgref` 指向 `ghcr.io/oratis/andromeda:edge`（`interactive-defaults.ks:1`、`andromeda-ci.ks:18`）——可变 tag、无 bootc 签名策略。若仓库被攻陷/tag 被重指，后续 `bootc switch`/更新会启动被篡改镜像。构建侧基础镜像亦按 tag 而非 digest 固定（`Containerfile:4,11`）。
- **严重级别**：中等（CI 侧 tar 更新已有 fw_cfg SHA-256 保护，此项主要针对 registry 拉取路径）。
- **修复建议**：对 `--target-imgref` 启用签名策略（sigstore/`containers-policy.json`）并按 digest 固定；基础镜像用 `@sha256:` 摘要固定。

### 5. macOS 上 `/System` deny root 因大小写敏感可绕过（中等偏轻）
- **攻击场景**：非 Windows 平台 `path_starts_with` 用大小写**敏感**的 `Path::starts_with`（`lib.rs:403-406`），`normalized_absolute` 只做词法归一、不折叠大小写。macOS 默认 APFS **大小写不敏感**：plan 写 `/system/Library/x` 或 `/SYSTEM/...` 归一后 `starts_with("/System")` 为 false → 不命中 deny root，而内核仍解析到真正 `/System`。Windows 侧已正确处理，macOS 未处理。
- **严重级别**：中等偏轻（今日无 executor，且 macOS SSV 让 `/System` 本就只读）。
- **修复建议**：在大小写不敏感文件系统上对 deny-root 与能力根做大小写不敏感分量比较；并落实文档已承诺的"executor 必须以 `realpath`/`openat` 语义复核"（`capability.rs:123-132` 已注明词法归一不解析 symlink，存在 TOCTOU/symlink 逃逸风险，需执行层兜底）。

### 6. `single_use` 声称"由 runtime 强制"，实为未实现（轻微）
- `capability.rs:91-95` 以现在时声称 single-use "is enforced by the runtime layer"，但 runtime 无任何消费/失效逻辑。无 executor 故不可利用，但声称具误导性。**修复**：改为"预留、尚未实现"，或在 runtime 落地执行状态时真正实现一次性失效。

### 7. subject 绑定特性未接入数据面（轻微）
- `EvaluationContext::with_subject`/`subject` 存在且有测试，但 taskd `EvaluationRequest` 无 subject 字段、`service.evaluate` 用 `subject=None`，该主体核验在守护进程路径上是**死代码**。**修复**：引入认证主体后把请求方 subject 传入评估上下文并强制 `issued_to` 匹配。

## 剩余攻击面与建议

- **executor 落地前的前置项（阻塞级）**：先解决发现 2/3——本地鉴权 + 能力签发与评估分离 + isolation 证明；否则"决策正确"不等于"执行安全"。
- **HCM/策略签名**（发现 1/4）：用 `signing_key_id` 建立可信公钥集合，matcher/安装器验签后再采信；发布镜像按 digest + 签名策略固定。
- **systemd 纵深加固**：`andromeda-taskd.service` 建议补 `IPAddressAllow=localhost`/`IPAddressDeny=any`、`SystemCallFilter=@system-service`、`LockPersonality=yes`、`MemoryDenyWriteExecute=yes`、`RestrictSUIDSGID=yes`、`ProtectProc=invisible`。
- **文件系统语义**：deny-root 与能力根在大小写不敏感 FS 上做不敏感比较（发现 5）；执行层强制 `realpath`/`openat` 复核以堵 symlink/TOCTOU。
- **文档一致性**：修正 `task-control-plane.md:46` 的"非 loopback 绑定自保护"表述；修正 `capability.rs` 对 single_use 的现在时声称。
- **默认策略**：`denied_network_hosts` 默认空——网络访问靠能力 allow-list（精确 host）把关，可接受；若未来允许更宽网络能力，应同步充实 deny 列表。

*（评审只读，未修改任何仓库文件；复现所用 `forged.json` 仅位于会话 scratchpad。）*

*Reviewed by Claude Code multi-agent review (security dimension).*
