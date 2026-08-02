# Andromeda v0 任务控制面

## 目的

控制面把“模型建议”转换成可验证的数据，但不把模型变成权限主体。v0 的交付边界是计划、授权输入、策略评估、持久化和状态转换；真实工具执行后续接入。

## 数据流

```text
Intent
  -> versioned ActionPlan
  -> schema/risk/DAG validation
  -> Capability + deterministic Policy
  -> durable TaskRecord
  -> attested Executor (not implemented)
  -> independent Verifier (not implemented)
  -> Evidence + audit + recovery
```

## 风险与隔离

| 等级 | 定义 | 最低隔离 |
|---|---|---|
| L0 | 无工具推理 | None |
| L1 | 普通文件/任务操作 | Sandbox |
| L2 | 未知内容、解析器、可执行输入 | MicroVM |
| L3 | 网络、系统设置、外部真实副作用 | Brokered + final confirmation |

ActionKind 决定不可降低的风险下限。模型可以把动作声明得更危险，但不能声明得更安全。

## API

| 方法 | 路径 | 作用 |
|---|---|---|
| GET | `/healthz` | 服务状态与 API 版本 |
| POST | `/v1/tasks` | 校验并创建任务（重复 task_id 返回 409 `already_exists`） |
| GET | `/v1/tasks` | 列出任务，响应为 `{"tasks": [...], "warnings": [...]}`；损坏的记录文件被跳过并记入 `warnings`，不会让整个列表失败 |
| GET | `/v1/tasks/{id}` | 读取任务 |
| POST | `/v1/tasks/{id}/capabilities` | 给已存在的任务补授权：追加 capability，记 `granted` 事件并使 revision +1。每个新 capability 必须 `issued_to == plan.task_id` 且当前有效（未过期、已到 `issued_at`），否则返回 422；带 `expected_revision` 做乐观并发 |
| POST | `/v1/tasks/{id}/outcomes` | 记录单个 action 的执行结果与证据；追加 `outcome_recorded` 事件并使 revision +1。只允许在 `Running`/`Verifying` 状态记录，每个 action 至多一条（重复返回 422），action 必须属于该计划 |
| POST | `/v1/tasks/{id}/evaluate` | 评估、不执行；**逐 action** 解析隔离等级，结果作为 `evaluated` 事件追加到任务事件史并使 revision +1 |
| POST | `/v1/tasks/{id}/transition` | 带 revision 的状态转换；`Ready`/`Running`/`Succeeded` 三条边受门控（见下） |

所有 `TaskService` 调用在 `tokio::task::spawn_blocking` 中执行，阻塞的文件锁和 fsync 不会占用 async worker，`/healthz` 在锁竞争时依旧可响应。

### Host 校验（DNS rebinding 防护）

`taskd` 校验每个请求的 `Host`（HTTP/2 下回退到 `:authority`）：只接受 `localhost` 与字面回环 IP（127.0.0.0/8、`[::1]` 及其 IPv4-mapped 形式，可带端口），其余一律 403 `forbidden_host`。恶意网页即使通过 DNS rebinding 把自己的域名解析到 127.0.0.1，请求携带的仍是攻击者的 Host，会被拒绝。

注意：Host 校验**只防御浏览器发起的 DNS rebinding**，不是鉴权，也**不能保护非 loopback 绑定**。它只检查请求携带的 `Host` 头取值，不检查实际入站接口。任何非浏览器客户端（curl／脚本／攻击者）都可以自带 `Host: localhost` 通过校验。此外，本地任意进程/用户经 loopback 亦可无鉴权访问 API。远程鉴权在下述能力实现前不存在（参见 `getting-started.md`、`README.md` 的一致说明）。

### 绑定地址强制（非 Host 校验）

因为 Host 校验保护不了绑定面，`taskd` 在**启动时**校验监听地址：非回环地址直接拒绝启动并说明原因。只有显式设置 `ANDROMEDA_ALLOW_NON_LOOPBACK=1`（或 `--allow-non-loopback`）才能越过，并会打印醒目警告说明 API 无鉴权。这把"禁止绑定 loopback 之外"从文档约定变成机制。

生产部署另有内核级纵深防御：`andromeda-taskd.service` 设置 `IPAddressAllow=localhost` / `IPAddressDeny=any`。

### `Ready` 状态的语义

创建时，任务进入 `Ready` 当且仅当计划中每个 action 在"最宽松执行假设"（隔离恰好等于该 action 风险等级要求的最低隔离、外部副作用视为已确认）下会被确定性策略引擎判为 Allow。这保证了：

- 每个 `required_capabilities` 中的 id 都存在于随请求提供的 capability 里；
- 这些 capability 在创建时刻未过期（`expires_at`）；
- capability 的 scope 实际覆盖 action 的 target（文件路径前缀 + 读/写权限、网络 host、系统设置 key、外部服务 operation）；
- 没有 deny 规则命中 target。

`Ready` 不保证：

- 实际执行时的隔离等级足够（`evaluate` 会用真实 isolation 重新判定）；
- capability 在执行时刻仍未过期；
- 外部副作用已获得最终人工确认（L3 在 `evaluate` 时仍会返回 `ask`）；
- capability 本身来自可信签发方——v0 控制面不校验签发链，创建者可以自铸 capability。因此 `Ready ≠ 已授权执行`，执行前仍必须经过 `evaluate` 与（未来的）broker 审批。

### 逐 action 评估隔离

`evaluate` 按 **action** 解析隔离等级，而非用单一 task 级标量，这样才能与创建时的 `plan_fully_granted`（也是逐 action 用 `risk.minimum_isolation()`）保持同一套隔离模型。每个 action 的隔离等级按以下顺序解析：

1. 请求体顶层 `isolation`（`Option<IsolationLevel>`）若存在，覆盖**所有** action（仅为整盘测试方便）；
2. 否则查 `overrides`（`{ActionId: IsolationLevel}` 映射）中该 action 的条目；
3. 否则用该 action 声明风险对应的最低隔离。

这修复了一个契约缺陷：`IsolationLevel::satisfies` 是非线性的（`MicroVm` 与 `Brokered` 互不满足），因此一个同时含 L2（microVM 解析）与 L3（broker 外发）动作的计划，在旧的单标量 evaluate 下**永远**至少有一个动作被 Deny。逐 action 解析后，这类混合计划可以整盘 Allow。请求体还可携带可选 `subject`：一旦提供，每个 action 所需 capability 的 `issued_to` 必须与之匹配，否则该 action 被 Deny。

`evaluated` 事件记录每个 action 的 `effective_isolation`（实际使用的隔离等级）与完整决策集，供审计。

### 策略门控的状态转换

状态机边合法性之外，三条授权/证据敏感的边额外做复检，避免仅凭断言就把任务推进到"可执行"或"已成功"语义：

- **`AwaitingApproval → Ready`**：要求 `plan_fully_granted`（每个 action 的所需 capability 都齐备、有效、覆盖 target 且不命中 deny 规则），否则拒绝。这样因授权不足而挂起的任务，必须先经 `POST /v1/tasks/{id}/capabilities` 补齐授权，才能进入 `Ready`。
- **`Ready → Running`**：对每个 action 以其逐 action 最低隔离重跑策略引擎，**使用请求体显式提供的 `external_side_effect_confirmed`（默认 `false`）**。任一 action 为 `Deny` 即拒绝并列出原因；任一 action 为 `Ask`（即未确认的 L3 外部副作用）则以 `external_confirmation_required` 拒绝，并列出待确认的 action。这使 `Running` 成为强制的重新授权点与 **L3 提交点**：capability 在 `Ready` 之后过期会在此被挡下，未确认的外部副作用也无法启动。确认值记入 `state_changed` 事件，与 actor 一起留痕。
- **`Verifying → Succeeded`**：要求计划中**每个** action 都有已记录的 outcome；outcome 状态必须是 `succeeded` 或 `skipped`（`failed`/`rolled_back`/`compensated` 一律拒绝），且**每条 outcome 至少携带一条 evidence**。因此"成功"是被证明的，不是被断言的。

被门控拒绝的转换一律返回 422，`error` 码按需要的操作员动作区分：未确认 L3 外部副作用（`Ready → Running` 的 `Ask`）返回 `external_confirmation_required`；`Verifying → Succeeded` 的证据门控（缺 outcome、outcome 非成功、outcome 无 evidence）返回 `missing_evidence`；其余门控与结构性拒绝（计划未完全授权、action 被策略 Deny、非法状态转换、计划校验失败）返回 `invalid_task`。

#### L3 确认的 v0 边界

`external_side_effect_confirmed` 由**调用方自报**，不是受信 broker 的证明。当前它保证的是
"确认这一步确实发生过、并被归属到某个 actor 并留痕"，**不保证**"确认来自真实的人"。
真正的确认代理（把参数摘要绑定到用户确认，防止批准后调包）属于 host broker 的职责，
见下方"明确未实现"。

## 持久化

- 每个 task revision 一个格式化 JSON 文件；每次写入后做 **compaction**，只保留最新 revision 文件（详见下）；
- 每个 task 一个 `{task_id}.latest` 指针文件，原子写入，记录当前最新 revision 号；读取时先读指针（O(1)），指针缺失/不可解析/指向的文件已不存在时回退到目录扫描（兼容旧 store 与崩溃窗口）；
- 独占跨进程 lock（fs4）；
- 临时文件写入、flush、`sync_all` 后原子 rename 到新的 revision 文件；
- 目录同步（仅 Unix；Windows 上 `File::open` 无法打开目录）；
- revision 乐观并发；
- 每次受支持的状态变化、每次授权补授（`granted`）、每次结果记录（`outcome_recorded`）和每次策略评估都追加 event；
- 执行结果与证据保存在 `TaskRecord.outcomes`（每个 action 至多一条，append-only：已记录的 outcome 不可被覆盖）；
- store 打开时在独占锁内清理崩溃残留的 `.{uuid}.tmp` 孤儿文件；
- 单个计划最多 `MAX_PLAN_ACTIONS`（10 000）个 action，结构校验（去重/悬挂依赖/环检测）由 core `ActionPlan::validate`（迭代 Kahn 拓扑排序）单一实现负责，runtime 复用它，不再各写一套。

### Compaction 策略

写入顺序为：先原子写 revision 文件（durable），再原子推进 `latest` 指针，最后在同一独占锁内删除该 task 其余 revision 文件（`compact(keep=当前 revision)`）。因此稳态下每个 task 只留一个 revision 文件与一个指针文件，消除了"revision 只增不减"的 O(R²) 膨胀与随文件总数线性增长的全目录扫描。

写序保证崩溃安全：revision 文件先于指针落盘，compaction 只在指针指向幸存者之后运行——崩溃至多留下一个多余 revision 文件（下次成功写入时被 compaction 或原子 rename 覆盖回收），绝不会出现指针悬空指向缺失文件。

无锁读（`get`/`list`）容忍 compaction 竞态：若选中的 revision 文件在解析前被并发 compaction 删除（读到 `NotFound`），`get` 会重新解析最新 revision 并重试（`GET_MAX_ATTEMPTS`），`list` 则静默跳过已被更高 revision 取代的失效路径。`get` 与 `list` 都走同一套"指针优先、扫描兜底"的解析，二者不会给出不一致的最新 revision。

注意：compaction 会主动删除被取代的 revision 快照，因此磁盘上的 revision 文件**不是** append-only 历史（完整审计线索保存在最新记录内嵌的 `events` 里，而非历史 revision 文件里）。这本就不是防物理管理员篡改的 append-only ledger；正式 audit ledger 需要签名、哈希链、密钥轮换、隐私删除策略和独立导出。

## 明确未实现

- 模型调用与 planner；
- bubblewrap/SELinux/microVM executor；
- credential broker；
- 外部 connector/MCP broker；
- 签名 policy bundle；
- **独立 verifier**：`Verifying → Succeeded` 现在强制要求带证据的 outcome，但证据由执行方
  自行记录，尚无独立断言器复核（"验证不能由执行模型自证"仍是未兑现的设计目标）；
- **确认代理**：L3 确认目前由调用方自报（见上方 v0 边界），尚无把参数摘要绑定到用户
  确认的 host broker；
- rollback/compensation executor；
- 用户身份、远程认证和多租户；
- Task Center 图形界面。

在这些能力实现前，`taskd` 只能作为 loopback 开发服务。
