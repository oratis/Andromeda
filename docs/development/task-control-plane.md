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
| POST | `/v1/tasks` | 校验并创建任务 |
| GET | `/v1/tasks` | 列出任务 |
| GET | `/v1/tasks/{id}` | 读取任务 |
| POST | `/v1/tasks/{id}/evaluate` | 只评估、不执行 |
| POST | `/v1/tasks/{id}/transition` | 带 revision 的状态转换 |

## 持久化

- 每个 task revision 一个不可覆盖的格式化 JSON 文件，读取时选择最高 revision；
- 独占跨进程 lock；
- 临时文件写入、flush、`sync_all` 后原子 rename 到新的 revision 文件；
- 目录同步；
- revision 乐观并发；
- 每次受支持的状态变化追加 event。

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
