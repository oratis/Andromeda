# 代码质量与正确性评审

## 概览与总评

**评级：A-（优秀，可发布质量；仅少量健壮性与可维护性缺口）**

本次评审覆盖 `crates/*` 全部六个 crate（`andromeda-core`/`-policy`/`-runtime`/`-taskd`/`-hardware`/`-cli`，约 6300 行 Rust），聚焦 Rust 惯用法、错误处理、并发/持久化正确性、serde 正确性、确定性以及测试覆盖。**范围限定为 Rust 代码质量与正确性，不含安全策略语义与 OS shell。**

实测结果（在合并后的 `main` 上）：

- `cargo test --workspace --locked` → **全绿，109 个测试通过**（core 24、policy 25、hardware 32、runtime 12、taskd 13、cli 3；0 失败/0 忽略）。
- `cargo clippy --workspace --all-targets --locked -- -D warnings` → **零告警**（工作区已开启 `clippy::all` + `clippy::pedantic` = warn，并 `unsafe_code = forbid`）。

**总评：** 这是一棵工程素养很高的代码树——分层清晰、类型建模严谨、tri-state 语义处理到位、存储层的原子写与乐观并发设计正确、对抗性输入（深依赖链、路径穿越、子域名 deny、USB 接口层探测）均有针对性测试。**在核心算法（迭代式环检测、revision 定序、revision CAS、路径规范化、host 解析、USB 接口探测）中未发现能给出确定错误输出的高危逻辑 bug。** 真正的缺口集中在：不可信输入的 serde 加固（`deny_unknown_fields` 缺失，方向为 fail-open）、存储可扩展性（已文档化的 O(n²) 与全目录扫描）、以及两套并存的 plan 校验实现带来的漂移风险。

## 优点

- **干净的分层与契约边界。** `andromeda-core` 只承载稳定契约（`ActionPlan`/`Capability`/`TaskState`），策略与运行时依赖它而非反向；`lib.rs` 的 `pub use` re-export 收口清晰。工作区级 `unsafe_code = forbid` + pedantic clippy 是很高的基线。
- **类型建模严谨。** `TaskId`/`ActionId`/`CapabilityId` 均为 newtype（`#[serde(transparent)]` + `FromStr`/`Display`/`Default`），避免 UUID 裸传；错误类型全部用 `thiserror`，错误分类细致（如 `StoreError` 区分 `RevisionConflict` 与 `InvalidRecordRevision`）。
- **tri-state 硬件语义一致。** `BootInfo` 全部字段为 `Option<bool>`，`None`（"无法验证"）与 `Some(false)`（"验证为无"）被严格区分，并在 linux/macos/windows/other 四个后端一致贯彻（如 `tpm2_state` 对 TPM 1.2 不塌缩为 false，probe/linux.rs:83-88）。
- **确定性（时钟注入）。** 策略引擎 `EvaluationContext::at`（policy/lib.rs:104）与匹配器 `evaluate_manifest_at`（matcher.rs:21）都提供显式 `now` 注入，过期判定可确定性测试。
- **存储层的持久化与并发设计正确。** `write_atomic`（store.rs:255-270）遵循 temp（`create_new`）+ fsync + `rename` + `cfg(unix)` 目录 fsync 的标准崩溃安全序列；孤儿 `.tmp` 仅在持锁期间产生，`remove_orphan_temp_files`（217-236）在同一排他锁下清理，推理自洽；revision 采用 20 位零填充（恰好容纳 `u64::MAX`），故 `latest_path` 的字典序 `.max()`（186-197）等价于数值最大；`save` 的 revision CAS 在持锁下**重新读取**当前值再比对（156-180），两个并发 writer 中恰有一个赢、另一个得到 `RevisionConflict`。`list()` 对损坏记录不失败、以 `warnings` 上报（123-148）。
- **迭代式环检测，抗栈溢出。** core 用 Kahn 拓扑排序（action.rs:231-267），runtime 用显式栈三色 DFS（service.rs:359-397），并有 10k 深度链的测试证明不会栈溢出。
- **接口级 USB 探测 + 可注入根路径。** `collect_usb_interfaces`（sysfs.rs:73）按 `bInterfaceClass` 而非设备级 `bDeviceClass` 分类，正确处理复合设备（webcam）、过滤 hub（class 09）、跳过无父设备的孤儿接口，并有真实布局的 mock-tree 测试；`DEVICE_CAP` 截断附带 warning。
- **taskd 的运行时卫生。** 所有阻塞式存储 I/O 走 `spawn_blocking`（taskd/lib.rs:103），`/healthz` 不触存储、锁竞争时仍可响应；loopback Host 白名单针对 DNS rebinding 做了加固（51-98）；HTTP 状态码映射清晰。优雅停机对信号安装失败降级为 `pending()` 而非退出（taskd/main.rs:42）。
- **测试里有"防回归的锁"。** 枚举声明顺序断言（`risk_level_declaration_order_is_locked`、`isolation_level_declaration_order_is_locked`）防止 `Ord` 被静默改动；`satisfies_matrix_is_exact` 用全矩阵穷举；policy 层有大量负向/对抗测试。

## 正确性问题（按影响排序）

### 1. [中] 不可信输入的 serde 未启用 `deny_unknown_fields`，字段拼写错误被静默丢弃 → fail-open

全工作区无任何 `deny_unknown_fields`（已核对）。同时 serde 对 `Option<T>` 字段缺省即 `None`（无需 `#[serde(default)]`）。二者叠加，对经 HTTP body 直接反序列化的不可信结构（`ActionPlan`/`ActionSpec` core/action.rs、`Capability`/`CapabilityResource` core/capability.rs:84-96、`PolicySet` policy/lib.rs:47-54、`HcmManifest` hardware/model.rs）存在 fail-open 风险。

**失败场景：** 向 `POST /v1/tasks` 提交的 capability JSON 把 `expires_at` 误写成 `expiresAt`：`expiresAt` 作为未知字段被静默忽略，`expires_at` 缺失 → `None` → `Capability::is_active_at`（capability.rs:102-104）对该 grant **永远返回 true**（永不过期）。方向为放宽权限。

**严重度：中。** 建议对上述不可信输入结构加 `#[serde(deny_unknown_fields)]`（HcmManifest 的 schema 版本已在 CLI 侧 gate，但 capability/plan 的 HTTP 路径无此保护）。

### 2. [低→中] 写操作在持有全局排他锁期间做整目录扫描 + 全量反序列化；与未来 compaction 存在 TOCTOU

`save`（store.rs:156-180）在 `lock_exclusive()` 后立即调用 `self.get()` → `latest_path` → `record_paths()` 的整目录 `read_dir` + 解析最新 revision 文件，再 `write_atomic`（含 fsync）。**临界区很大，所有 writer 串行在一次整目录枚举 + JSON 解析 + fsync 之上**，写吞吐随文件总数线性下降。

**当前无正确性 bug**，因为读路径 `get`/`list` 无锁、且依赖"revision 文件只增不减"这一不变量保证 `latest_path` 返回的文件必定存在。**但一旦落地已文档化的 compaction（见问题 3）删除旧 revision**，`latest_path`→`read_record`（store.rs:186-197 → 250-253）之间会出现 TOCTOU：并发的 compaction 删掉刚被选中的文件，读会命中 `NotFound`。

**严重度：当前低 / compaction 后中。** 建议：compaction 落地时让无锁读对 `NotFound` 重试，或引入 `latest` 指针文件以同时消除全目录扫描与该窗口。

### 3. [低] 每个 revision 文件内嵌完整事件历史 → 存储与 `list` 皆 O(n²)（已文档化）

`write_atomic` 每次落盘整个 `TaskRecord`（含 `events` 全量），`task_path`（store.rs:182-184）每 revision 一个独立文件且**只增不减**。含 R 次修订的任务磁盘占用 ~O(R²)；`list()`（123-148）读取全部 revision 文件、`get()` 每次做整目录枚举来定位 latest，单次操作即 O(总文件数）。已在 `docs/development/task-control-plane.md:75` 记录 compaction TODO。**严重度：低（已知且已文档化）**，但在 compaction 落地前应限制单机长寿命任务规模。

### 4. [低] `evaluate_device` 用迭代器副作用实现"匹配但无驱动"判定，依赖惰性短路，脆弱

matcher.rs:190-196 用 `.inspect(|_| any_id_match = true)` 配合 `.any(...)` 判断"存在 ID 匹配但无绑定驱动"。逻辑正确（`.any` 短路后 `any_id_match` 语义仍成立），但依赖 `.any` 的惰性消费顺序；若日后重构为 `collect`/并行迭代会静默破坏 `any_id_match`。**严重度：低（可读性/健壮性）**。建议改为显式两步（先 `filter().count()`，再判 driver）。

### 5. [信息] `record.revision += 1` 非饱和加

service.rs:221、261 用 `+= 1`，而 store `save` 用 `saturating_add`（store.rs:166），风格不一致；`u64` 溢出仅在实践不可达的 `u64::MAX` 时于 debug 触发 panic。**严重度：trivial**，统一为 `saturating_add` 即可。

## 测试覆盖缺口

现有 109 个单元测试对**单线程**正确性覆盖很好，但存在以下有意义的缺口：

- **无集成测试目录、无属性/模糊测试。** 全仓无 `crates/*/tests/`，无 `proptest`/`quickcheck`。`normalized_absolute`、`split_host_port`、`is_loopback_host`、两套环检测是 property test 的理想目标（尤其可对比 core Kahn 与 runtime DFS 的**等价性**）。
- **存储并发只做了单线程模拟。** 现有测试用 stale `expected_revision` 模拟冲突，但**没有真正的多线程/多进程竞争测试**证明"两 writer 恰一胜出"以及 fs4 锁能跨进程串行化。
- **无崩溃注入/持久化断言。** 仅通过 `open_removes_orphan_temp_files` 间接覆盖；缺少"`list()`/`get()` 期间存在 `.tmp` 应被忽略""半写 temp 永不成为可见记录"这类断言。
- **两套 plan 校验无一致性测试。** `ActionPlan::validate`（core）与 `validate_plan`（runtime）可静默漂移，却没有断言二者对同一 plan 给出一致结论的测试。
- **服务层时间不可注入，时间边界无法确定性测试。** `TaskService::create`/`evaluate`/`plan_fully_granted` 内部直接 `Utc::now()`（service.rs:150、205、289）。
- **macOS/Windows 探测几乎无测试。** 二者 shell 调用 `sysctl`/`powershell`，命令输出不可注入；只有 Linux sysfs 具备可注入根路径的测试夹具。
- **sysfs 测试的临时目录管理未用 RAII。** 建议 hardware 也引入 `tempfile::TempDir`（RAII）统一。

## 可维护性与可扩展性建议

1. **消除两套并存的 plan 校验（最高优先）。** `ActionPlan::validate`（core/action.rs:212，Kahn）在**生产路径中是死代码**——真正持久化的路径走 runtime `validate_plan`（service.rs:298）+ `has_cycle`（service.rs:359，DFS），且并存两套语义重叠的错误枚举 `ValidationError`（service.rs:85-104）与 `PlanValidationError`（core/action.rs:170-181）。建议 runtime 委托给 `ActionPlan::validate` 并做错误映射，或删除 core 的方法，二选一收敛。
2. **合并重复的 hex-ID 工具。** `strip_hex_prefix`/`id_matches` 在 matcher.rs:347-356 与 diagnosis.rs:212-221 各有一份；且 diagnosis 的 `parse_hex`（206）用会剥离**重复**前缀的 `trim_start_matches("0x")`，与只剥一次的 `strip_hex_prefix` 语义不一致。抽到单一共享 helper。
3. **统一 `Option` 字段的 `#[serde(default)]` 约定。** `DeviceInfo`（model.rs:52-62）只对部分 `Option` 字段标 `#[serde(default)]`（对缺失 Option 本就默认 None，属冗余），读起来像存在语义差异。
4. **为 `TaskService` 引入可注入时钟。** 参照 policy/matcher 已有的 `_at`/`now` 模式，消除隐式 `Utc::now()`，使服务层可确定性测试。
5. **`ApiError` 可用 thiserror 收敛。** taskd/lib.rs:179-232 手写 `Display` 与两个 `From`，与全仓 thiserror 风格统一后更省样板。
6. **落地存储 compaction 并加索引。** 实现文档化的保留策略，并考虑 `latest` 指针/索引文件以消除每次 `get`/写操作的整目录扫描（问题 2、3）；compaction 落地时同步让无锁读对 `NotFound` 容错重试以闭合 TOCTOU 窗口。

---

*评审依据：合并后 `main` 源码通读 + `cargo test --workspace --locked`（109 通过）+ `cargo clippy --workspace --all-targets --locked -- -D warnings`（零告警）。核心算法未发现可复现的高危逻辑 bug。*

*Reviewed by Claude Code multi-agent review (code-quality dimension).*
