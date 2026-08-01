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
| POST | `/v1/tasks/{id}/evaluate` | 评估、不执行；评估结果作为 `evaluated` 事件追加到任务事件史并使 revision +1 |
| POST | `/v1/tasks/{id}/transition` | 带 revision 的状态转换 |

所有 `TaskService` 调用在 `tokio::task::spawn_blocking` 中执行，阻塞的文件锁和 fsync 不会占用 async worker，`/healthz` 在锁竞争时依旧可响应。

### Host 校验（DNS rebinding 防护）

`taskd` 校验每个请求的 `Host`（HTTP/2 下回退到 `:authority`）：只接受 `localhost`、`127.0.0.1`、`[::1]`（可带端口），其余一律 403 `forbidden_host`。恶意网页即使通过 DNS rebinding 把自己的域名解析到 127.0.0.1，请求携带的仍是攻击者的 Host，会被拒绝。

注意：Host 校验**只防御浏览器发起的 DNS rebinding**，不是鉴权，也**不能保护非 loopback 绑定**。它只检查请求携带的 `Host` 头取值，不检查实际入站接口。任何非浏览器客户端（curl／脚本／攻击者）都可以自带 `Host: localhost` 通过校验——因此若把 `ANDROMEDA_LISTEN` 改为非 loopback 地址，API 会向该网络**暴露且无鉴权**。**禁止把 `taskd` 绑定到 loopback 之外。** 此外，本地任意进程/用户经 loopback 亦可无鉴权访问 API。远程鉴权在下述能力实现前不存在（参见 `getting-started.md`、`README.md` 的一致说明）。

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

## 持久化

- 每个 task revision 一个不可覆盖的格式化 JSON 文件，读取时选择最高 revision；
- 独占跨进程 lock（fs4）；
- 临时文件写入、flush、`sync_all` 后原子 rename 到新的 revision 文件；
- 目录同步（仅 Unix；Windows 上 `File::open` 无法打开目录）；
- revision 乐观并发；
- 每次受支持的状态变化和每次策略评估都追加 event；
- store 打开时在独占锁内清理崩溃残留的 `.{uuid}.tmp` 孤儿文件；
- 单个计划最多 `MAX_PLAN_ACTIONS`（10 000）个 action，依赖环检测为迭代 DFS，不受栈深限制。

TODO：revision 文件目前只增不减，长寿命任务会累积大量历史 revision 文件，需要一个保留最近 N 个 revision（或按时间）的 compaction 策略。

revision 文件不会被正常服务覆盖，但这仍不是防物理管理员篡改的 append-only ledger。正式 audit ledger 需要签名、哈希链、密钥轮换、隐私删除策略和独立导出。

## 明确未实现

- 模型调用与 planner；
- bubblewrap/SELinux/microVM executor；
- credential broker；
- 外部 connector/MCP broker；
- 签名 policy bundle；
- verifier、rollback/compensation executor；
- 用户身份、远程认证和多租户；
- Task Center 图形界面。

在这些能力实现前，`taskd` 只能作为 loopback 开发服务。
