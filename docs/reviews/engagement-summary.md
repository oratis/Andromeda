# Andromeda 系统工作总述与优化路线

> 本文档汇总一次完整的"评审 → 修复 → 合并 → 系统评审 → 按评审优化"工程闭环:做了什么、系统现在是什么状态、以及下一步的优化点。
> 基线:`main`(本文撰写时 `d27cefa`,108 commits,15 个 PR 已合并)。
> 分维度评审详见 [`docs/reviews/README.md`](README.md);E2E 流水线专项评审见 [`e2e-pipeline-review.md`](e2e-pipeline-review.md)。

## 一、系统是什么

Andromeda 是一个 **bootc 不可变 Linux 桌面 OS + AI agent 任务控制面**的早期系统,当前处于 **v0 原型**阶段。最重要的定性判断:

> **它是一个"纯决策"系统——策略引擎与硬件兼容性判定已实现且质量不低,但真正产生副作用的 executor 尚不存在。**

这一点决定了所有安全结论的解读方式:今天几乎没有可被直接利用的高危漏洞(没有组件消费判定去执行),但**信任边界与契约形状**的问题一旦被即将落地的 executor 当真,会立刻升级为严重问题。这也是本轮优化的主攻方向。

### 组成

| 组件 | 职责 | 状态 |
|---|---|---|
| `andromeda-core` | 契约层:Action/Capability/Task 状态机,零 I/O 依赖 | 成熟 |
| `andromeda-policy` | 确定性授权引擎,deny-first | 成熟 |
| `andromeda-runtime` | TaskService + 崩溃安全的 FileTaskStore | 成熟 |
| `andromeda-taskd` | loopback HTTP 控制面 | 可用(无鉴权) |
| `andromeda-hardware` | HCM 硬件兼容性探测/匹配/诊断 | 成熟 |
| `andromeda-cli` | 开发者 CLI | 可用 |
| `os/` | bootc 镜像、安装器、E2E 验证流水线 | 生产级 |
| **未实现** | executor / verifier / credential broker / planner / Task Center GUI | — |

规模:Rust ~7.5k 行(6 crate),shell ~2.1k 行,文档 24 篇。

## 二、本轮做了什么

### 阶段 1:逐 PR 深度评审(7 个 open PR)
对 7 个堆叠 PR 各出一份评审并发布到 GitHub。**7 个全部 request changes**,每个都发现了至少一个必须修复的实质问题——且模式高度一致:**PR 自己宣称的安全不变量被自己的代码击穿**,而现有测试恰好都构造在盲区之外。

### 阶段 2:修复(5 个并行 agent,16 个 commit)
按文件域分区并行修复。关键修复:

| 严重问题 | 说明 |
|---|---|
| deny 路径 `..` 穿越绕过 | `/tmp/../etc/passwd` 可绕过 `/etc` deny 规则拿到 Allow |
| `IsolationLevel` 线性排序 | `Brokered >= MicroVm` 使 L2 隔离要求被静默满足 |
| 全空 selector 提权 | 全 null 的 HCM selector 匹配任意机器,可授予 `certified` |
| USB 诊断死代码 | probe 读 `class`,而 USB sysfs 无该属性 → 缺驱动仍判 ready(false-ready) |
| ISO 默认破坏性安装 | 发布产物默认 10 秒后自动清盘,与文档承诺相反 |
| async 阻塞文件锁 | 单个全局锁可占满 tokio worker,`/healthz` 一并挂死 |
| IPv6 SSH 检查遗漏 | 只匹配 `0.0.0.0:22`,漏掉 `[::]:22`(dual-stack 同样接受 IPv4) |

### 阶段 3:合并
7 个 PR 按栈序合入 `main`。**期间 os-e2e 失败**——深挖 serial + Anaconda 日志,定位到 `bootc install to-filesystem` 在 payload 安装期失败(并非验证脚本问题),等 payload 分层回退后 os-e2e 首次转绿才合并。

### 阶段 4:系统级评审(5 个独立 agent)
在合并后的 main 上分 5 个维度独立评审,产出 [`docs/reviews/`](README.md)。评级:架构 B、安全 良/优、代码质量 A−、OS 基础设施 A−、文档 B+。多个 agent **独立印证**的 6 条主线构成后续优化的依据。

### 阶段 5:按评审优化(4 + 2 个 PR)

| PR | 内容 |
|---|---|
| [#14](https://github.com/oratis/Andromeda/pull/14) | **逐 action 隔离契约**、授权闭环(grant 端点 + 策略门控转换)、存储 compaction、合并双 plan 校验、`deny_unknown_fields`、macOS deny-root |
| [#12](https://github.com/oratis/Andromeda/pull/12) | HCM 制品 sha256 校验(fail-closed)、SupportTier 阶梯统一 + 漂移守卫 |
| [#13](https://github.com/oratis/Andromeda/pull/13) | **bootc stderr 可观测性**、taskd 内核级 loopback、供应链固定、action SHA 钉死 |
| [#11](https://github.com/oratis/Andromeda/pull/11) | SupportTier 文档改名、marker 6→10、ADR 模板 |
| [#19](https://github.com/oratis/Andromeda/pull/19) *(open)* | HCM **ed25519 分离签名验证**,fail-closed |
| [#18](https://github.com/oratis/Andromeda/pull/18) *(open)* | 层大小守卫改为构建后跑真实 history、bootc 签名 runbook(未激活) |

## 三、本轮的关键教训(工程性的,值得留档)

### 1. 声称与实现脱节是本仓库最普遍的缺陷模式
7 个原始 PR 中有 5 个存在"文档/注释宣称的安全属性,代码并未实现或被自己绕过"。典型:Host 校验被描述成"防裸奔保护"(实际只防浏览器 rebinding)、`single_use` 注释写"由 runtime 强制"(runtime 无任何实现)、HCM 宣称 fail-closed(全空 selector 可绕过)。
**对策**:每条安全承诺都要配一条**对抗性回归测试**;注释用现在时描述未实现的东西,等同于文档撒谎。

### 2. 给滚动标签按 digest 固定 = 定时炸弹
PR #13 出于供应链安全把 `quay.io/fedora/fedora-bootc:44` 按 `@sha256` 固定。该标签是频繁重建的滚动标签,Fedora 数天内即 GC 掉旧 digest → 之后**每一次构建都失败**(`manifest unknown`),且是潜伏回归(合并时是绿的)。
**对策**(已落地 `d27cefa`):滚动标签**不能**在没有刷新自动化的前提下 pin digest。要么接 Renovate/Dependabot 自动刷新后再 pin,要么显式跟踪标签并注明理由。`rust:1.85-bookworm` 保持 digest 固定(Docker Hub 保留历史 digest)。

### 3. 测试不采集失败证据 = 没有测试
os-e2e 失败时无法定位根因,因为 `test-install.sh` 的失败路径在收割 ESP 诊断**之前**就退出,且从未一等采集 bootc 子进程 stderr——上传的证据只有 anaconda 包装后的 "exited with status 1"。
**对策**(PR #13 已修):失败路径必须与成功路径拿到**同样的**磁盘侧证据;对关键子进程做一等日志采集。

### 4. "契约形状"的修改窗口极短
`evaluate` 的单一 task 级隔离与非线性 `satisfies` 矩阵冲突,使混合 L2+L3 计划**永远**无法全 Allow。这类问题在 executor 接入前改是几行代码,接入后是破坏性迁移。
**对策**:v0 阶段应主动清算所有"接了就难改"的 wire 契约。

## 四、系统当前状态

- **`main` 全绿**:Rust CI ✅、组合 os-e2e ✅(合并后已验证);本地 `fmt`/`clippy -D warnings`/12 套件测试 ✅
- **评审中标记为"executor 落地前必须修"的三条主线已全部合入**:逐 action 隔离契约、授权闭环、taskd 内核级 loopback
- 2 个优化 follow-up PR 待合(#18/#19),另有并发会话的 #16/#17

## 五、优化点(下一步)

### P0 — executor 落地前必须完成
1. **本地鉴权**。taskd 目前对本机任意进程/用户无鉴权。内核级 `IPAddressDeny` 已挡住远程,但本地仍敞开。建议 UNIX socket + `SO_PEERCRED`,或按套接字对端地址判定而非 Host 头。
2. **能力签发与评估分离**。当前调用方在同一请求里同时提供 plan 与 capabilities,`issued_to` 只需等于调用方自选的 `task_id`——能力可自铸。executor 落地前必须改为受信任宿主签发(签名或独立 grant store)。
3. **isolation 由执行环境证明而非调用方自报**。`evaluate` 的 isolation/confirmation 目前是自报参数;应改为 attestation。
4. **`single_use` 真正实现**,或从契约中移除。

### P1 — 尽快
5. **HCM 签名接入 CLI**。[#19](https://github.com/oratis/Andromeda/pull/19) 的库 API 已就绪,需要 `hardware check --trusted-keys <path>` 才能让退出码具备真实的真实性保证;在此之前 `hardware check` 只应作咨询用。
6. **供应链刷新自动化**。接 Renovate/Dependabot,然后把 fedora-bootc 重新按 digest 固定;同时覆盖 image-builder 容器、OVMF、dnf 包集。
7. **bootc 签名策略激活**。[#18](https://github.com/oratis/Andromeda/pull/18) 已提供 runbook + 未激活模板;需要真实签名密钥与发布流程,之后把 `:edge` 换成按 digest + 签名验证。
8. **层大小阈值按真实基线收紧**。守卫此前一直静默跳过;#18 修好后需从 CI 的 `OBSERVED` 行读出真实最大层,把 3 GiB 兜底收紧到 `OBSERVED + margin`。

### P2 — 架构与可维护性
9. **taskd 引入 DTO 层 + 事件读取加界**。当前直接把内部 struct 当 wire 格式,`GET /v1/tasks` 返回完整事件史(无分页/投影/上限)。
10. **CLI 与 taskd 的关系收敛**。CLI 直开 FileTaskStore(默认 `.andromeda/state`),taskd 用 DynamicUser 私有目录——真机上 `andromeda task list` 永远看不到 taskd 的任务。要么让 CLI 成为 HTTP 客户端,要么在文档与命令层显式区分两种模式。
11. **`Draft` 状态不可达**,`ActionSpec` 字符串化契约(魔法参数键、冒号分割排除 IPv6)建议在下次 schema 升版时改为带类型载荷的枚举。
12. **属性测试与并发测试**。`normalized_absolute`、`split_host_port`、`is_loopback_host` 是 property test 的理想目标;存储层缺真正的多进程竞争测试。

### P3 — 纵深与流程
13. macOS 大小写不敏感 deny-root 已修,但执行层仍须以 `realpath`/`openat` 语义复核以堵 symlink/TOCTOU。
14. 把 `SupportTier` 的规范定义收敛到单一 source of truth(已加 schema/enum 漂移守卫,建议再扩展到文档)。
15. E2E 流水线自身的优化见 [`e2e-pipeline-review.md`](e2e-pipeline-review.md)。

---

*本文档由 Claude Code 多 agent 评审与实现闭环产出。所有结论基于对 merged `main` 的实际代码阅读、`cargo test`/`clippy` 实跑与 CI 运行记录,关键项经独立复核或实机复现。*
