# 文档与产品一致性评审

## 概览与总评

**评级：B+（良好，但有 2 个 High 级必须修正的一致性缺口）**

Andromeda 的文档整体质量高于同阶段的绝大多数项目：结构清晰、边界诚实、反复强调"探测不等于支持""Ready ≠ 已授权""这不是已认证的消费 OS"，并且大量声明能与代码逐条对上（exit code、marker 序列、HCM v2 evidence 门禁、诊断类别、tri-state boot info 等经交叉验证均一致）。

但存在两处会实质误导读者的问题：

1. **安全诚实性缺陷（High）**：`task-control-plane.md` 声称把 `taskd` 绑定到非 loopback 地址是"有意的防裸奔行为"，代码证明这是错误的——远程客户端可自带 `Host: localhost` 绕过校验。此说法还与本仓库另外两处更诚实的文档直接矛盾。
2. **Support Tier 阶梯自相矛盾（High）**：`SupportTier` 这一核心产品概念在"战略文档"与"实现/开发文档 + 代码"中被定义成两套**方向相反**的阶梯，`Reference` 一词同时占据阶梯的最高位与次低位。

其余为 3 个 Medium（schema enum 顺序漂移、失效脚本引用、过时 marker 列表）与若干 Low。修掉 Tier 命名分裂与 Host-check 过度声明后，这套文档可以作为对外可信的权威材料。

## 优点（honesty, structure）

- **证据边界的诚实度罕见地高**：`daily-driver-e2e.md:135-140` 用加粗 blockquote 明确声明 GCP 的 PASS 记录**产生于证据提取器修复之前**、最终版脚本的一次性绿跑"尚未完成、仍在待办中"。该 caveat 完整、准确、无夸大。README.md:43-45 也同步转述。
- **"明确未实现"清单**贯穿全部开发文档（`task-control-plane.md:79-91`、`installable-preview.md:168-177`、README.md:79-80），从不把"计划"写成"已实现"。
- **代码侧自洽且有测试护栏**：`SupportTier` 的 `Ord`、evidence/artifact 门禁、`--require-tier` exit code、marker 顺序都有单测。
- **CLI 与文档在 exit code / tier 阶梯上一致**：`hardware-compatibility.md:73-83` 与 `crates/andromeda-cli/src/main.rs:88-97,196-203` 完全对得上（0 usable / 2 blocked / 3 below require-tier）。
- **诊断语义一致**、**虚拟硬件矩阵一致**、**docs/README.md 全部内部链接有效**、**quick-start 命令全部带 `--locked`**、**Rust 1.85 版本全仓一致**。

## 文档-代码漂移与不一致（按误导程度排序）

### 1. 【High｜安全诚实性】"非 loopback 绑定自我保护"是错误声明
- **文档**：`docs/development/task-control-plane.md:46` —"若把 `ANDROMEDA_LISTEN` 改为非 loopback 地址，所有请求都会因 Host 校验失败而被拒绝——这是有意的防裸奔行为。"
- **代码**：`crates/andromeda-taskd/src/lib.rs:51-98`（`require_loopback_host`）只检查**客户端自带的 `Host` 头**是否为 loopback，从不检查 socket 的本地/对端地址；`main.rs:13-14,29` 允许绑定任意 `SocketAddr`（含 `0.0.0.0`）。
- **问题**：Host 校验只能挡住**浏览器发起的 DNS-rebinding**。任何非浏览器远程客户端连到公网 socket 后，只需发送 `Host: localhost` 即可通过并访问**完整未鉴权 API**。因此"改为非 loopback 就全部被拒 = 自我保护"是错误的，会诱导运维认为绑定公网安全。
- **自相矛盾**：`getting-started.md:82`"当前 API 无远程认证，不得改为公网监听"、README.md:135"不应把 API 暴露到不可信网络"与之直接冲突；代码 doc-comment `lib.rs:44-50` 本身准确（明确限定为 DNS-rebinding）。出问题的只是这段散文。
- **建议**：删除"所有请求都会被拒绝/防裸奔"一句，改为"Host 校验只防御浏览器 DNS-rebinding，不能防止直接控制 Host 头的远程客户端；绑定非 loopback 会暴露未鉴权 API，禁止这样做"。

### 2. 【High｜产品一致性】Support Tier 阶梯方向相反，`Reference` 一词占据两端
- **战略文档**：`docs/product-development-plan.md:476-525`（§6.2）把阶梯定义为 `CI-0 → Tier 0 Blocked → Tier 1 Community → Tier 2 Supported → Tier 3 Certified → Tier 4 Reference`，其中 **Tier 4 `Reference` 是最高级**（OEM 共控 BOM/固件）。`os-landscape-and-andromeda-architecture.md:204-209` 同样把 `Reference` 列为 Tier 4 顶端。
- **实现 + 开发文档**：`crates/andromeda-hardware/src/model.rs:84-100` 的 `SupportTier` 声明顺序与 `Ord` 为 `Blocked < Community < Reference < Supported < Certified`，其中 **`Reference` 是次低级**。`hardware-certification-test-plan.md:220-230`、`hardware-compatibility.md:81-83`、CLI `main.rs:89-97` 全部沿用低位 `Reference`。
- **问题**：同一个 5 级枚举里，`Reference` 在战略文档是**最高**、在代码与开发文档是**次低**；两套阶梯共用其余四个名字，唯独 `Reference` 位置整体倒置。读者读完产品计划后运行 `andromeda hardware check --require-tier reference`，得到的是"高于 community、低于 supported"的判定，与其预期（最高级 OEM 线）完全相反。
- **建议**：二选一并全仓统一：(a) 若代码语义为准，则把 product-plan §6.2 / os-landscape §3.2 的"Tier 4 Reference"改名（如 `OEM Reference Design`），并把"CI-0 虚拟参考"与代码 `Reference` 合并说明；(b) 若战略语义为准，则需改 `SupportTier` 枚举与全部开发文档——代价大。推荐 (a)。

### 3. 【Medium】JSON schema enum 顺序与代码声明顺序不一致（且违反代码里写明的不变量）
- `schemas/hardware-compatibility-manifest.schema.json:29-35` enum 顺序为 `blocked, community, supported, certified, reference`（`reference` 在末尾）；`model.rs:84-100` 声明顺序为 `Blocked, Community, Reference, Supported, Certified`，且 doc-comment 明确要求"Keep the declaration order, the JSON schema enum, and the ladder ... in sync"。三者本应同序，实际 schema 与代码不同序。JSON Schema `enum` 是集合、顺序不影响校验，故**无运行时 bug**，但违反了代码自我声明的不变量。**建议**：把 schema enum 重排为 `blocked, community, reference, supported, certified`。

### 4. 【Medium】失效脚本引用：`test-daily-driver.sh` 不存在
- `hardware-certification-test-plan.md:250-255` 在"合并前必须先通过"命令块里列出 `os/scripts/test-daily-driver.sh output`；仓库中**没有**该脚本（等价脚本是 `test-install.sh`）。同一清单还漏了 CI 实际会跑的 `test-containerfile-layer-budget.sh`。**建议**：改为 `test-install.sh`，并补上 `test-containerfile-layer-budget.sh`。

### 5. 【Medium】过时的 marker 序列（漏 4 个）
- `installable-preview.md:118-127` 称串口"必须按顺序出现"的成功 marker 只有 6 个；`test-install.sh:280-300` 实际强制 **10 个**（额外含 `ANDROMEDA_SELINUX_LABELS_OK` 与三个 `ANDROMEDA_DAILY_DRIVER_OK phase=...`）。`daily-driver-e2e.md:44-57` 列的 10 个才是对的。**建议**：把 installable-preview.md 更新为 10-marker 序列。

### 6. 【Low】磁盘尺寸：当前脚本 64 GiB，但"已验证"证据记录为 32 GiB
- `test-install.sh:77` 与部分文档为 64 GiB；但记录 PASS 的证据处写 32 GiB（`daily-driver-e2e.md:147`、README.md:34/39）。根因是被记录的运行早于脚本从 32→64 GiB 的调整。**建议**：在证据表加注"该运行使用 32 GiB 盘，当前脚本已改为 64 GiB"。

### 7. 【Low/Info】`docs/adr/` 目录不存在，但多处文档以现在时引用 ADR
- `CONTRIBUTING.md:31`、README.md:184、product-plan §12 引用 ADR，但仓库无 `docs/adr/` 目录、零 ADR 文件。CONTRIBUTING 用"should include"尚可接受；README.md:184 现在时"通过 ADR 推进"略高于现状。**建议**：创建 `docs/adr/0000-template.md`，或把 README 措辞改为将来时。

## 建议

**必须修（阻断"权威文档"定位）**
1. 修正 `task-control-plane.md:46` 的 Host-check 过度声明，与 getting-started.md:82 / README.md:135 对齐（见 #1）。
2. 消解 `Reference` Tier 的双语义：选定代码语义，改名/移除 product-plan §6.2、os-landscape §3.2 的"Tier 4 Reference"，并统一"虚拟参考=CI-0 还是代码 Reference"的说法（见 #2）；顺带修 schema enum 顺序（见 #3）。

**应尽快修（会让照做的人踩坑）**
3. `hardware-certification-test-plan.md:254`：`test-daily-driver.sh` → `test-install.sh`，补 `test-containerfile-layer-budget.sh`（见 #4）。
4. `installable-preview.md:118-127`：marker 序列更新到 10 个（见 #5）。

**建议修（提升精确度）**
5. 证据表标注 32 GiB→64 GiB 盘尺寸演进（见 #6）。
6. 兑现或改写 ADR 引用（见 #7）。

**流程建议**：#2 与 #3 的根因是"战略文档 tier 命名"与"代码 SupportTier 枚举 + JSON schema"缺少单一 source of truth。建议把 `SupportTier` 五个取值的**规范定义**收敛到一处，其余文档一律引用而非各自复述；并加一个 CI 断言校验 `model.rs` 声明顺序 == schema enum 顺序，防止再次漂移。

---
*说明：本次评审只读不改，关键的双向 `file:line` 均已给出，可直接据此落修。*

*Reviewed by Claude Code multi-agent review (documentation dimension).*
