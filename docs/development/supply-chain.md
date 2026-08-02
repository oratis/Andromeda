# 供应链分级与输入稳定性

本文定义 Andromeda 构建输入的**分级策略**、每个输入的**当前状态**、
`fedora-bootc` 的**重新 pin 运行手册**，以及 pin 新鲜度检查的用法。

对应 `docs/reviews/e2e-pipeline-review.md`「评估 6 供应链 / 输入稳定性」与 P1 #8，
以及 `docs/reviews/security-review.md` 发现 #4。

## 为什么需要分级：一次真实故障

`quay.io/fedora/fedora-bootc:44` 曾经按 `@sha256` 固定。Fedora 在数天内就会把
被取代的 fedora-bootc 旧 digest 从 quay.io 上回收，于是这个 pin 变成
`manifest unknown`，**每一次构建同时失败**（Actions run 30702410393）。
`d27cefa` 把它解绑，这是正确的止血，但**不是终局**。

教训不是「不要 pin」，而是：

> **先落地刷新自动化，再 pin。**
> 一个没有刷新机制的 digest pin，是一颗定时炸弹而不是一道防线。

同样重要的是：**不同输入的风险类别不同，不该用同一种手段**。把 300 个 dnf 包
逐个固定 NEVRA 与把一个基础镜像固定 digest，成本和收益完全不在一个量级。

## 分级定义

| Tier | 手段 | 适用条件 | 失败模式 |
|---|---|---|---|
| **A** | digest pin **+ 自动刷新** | 少量、单点、可被 registry 回收的引用 | pin 过期 → `manifest unknown` → 全量构建中断 |
| **B** | 快照（而非逐项 pin） | 数量大、逐项固定不可维护的包集合 | 同一 commit 今天与明天构建出不同产物 |
| **C** | 记录（而非 pin） | 宿主工具链，不由本仓库控制 | 行为静默漂移，事后无法归因 |
| **D** | 签名 | 会被真实用户跟随的发布物 | 仓库/tag 被攻陷即分发被篡改镜像 |

**Tier A 的顺序是硬约束**：① 接入刷新自动化 → ② 验证它确实能开 PR →
③ 才 pin → ④ 新鲜度检查兜底。跳过 ①②直接 ③，就是上面那次故障。

## 输入清单与当前状态

| 输入 | 位置 | Tier | 当前状态 | 备注 |
|---|---|---|---|---|
| `docker.io/library/rust` | `os/Containerfile:15` | A | ✅ digest pin | Docker Hub 永久保留历史 digest，pin 不会失效。但**内容已 501 天**，见下方新鲜度检查输出 |
| `quay.io/fedora/fedora-bootc:44` | `os/Containerfile` payload stage | A | ⚠️ **tag 跟踪，待重新 pin** | 阻塞项：Renovate App 未安装。见下方运行手册 |
| `ghcr.io/osbuild/image-builder-cli` | `os/scripts/build-iso.sh:7` | A | ✅ digest pin | **风险已排除**，见「ghcr.io 保留策略调查」 |
| dnf 包（23 事务，~300 包） | `os/Containerfile` payload stage | B | ❌ 完全不固定 | **可复现性最大黑洞**。见「Tier B：dnf 快照评估」 |
| GitHub Actions | `.github/workflows/*.yml` | A | ✅ 三方 SHA pin + 版本注释；first-party major tag | Renovate 装上后接管刷新 |
| apt 包（ovmf/qemu/podman） | `os-e2e.yml` | C | ❌ 未记录 | OVMF/QEMU 漂移会静默改变固件行为，而 UEFI 引导正是被测对象 |
| `ghcr.io/oratis/andromeda:edge` | 两份 kickstart | D | ❌ 可变、未签名 | security-review #4；在有真实用户跟随之前必须配签名策略 |

## Tier A：自动化（`.github/renovate.json`）

`.github/renovate.json` 配置了：

- `docker:pinDigests` + `matchManagers: ["dockerfile"]` 的 `pinDigests: true`
  —— 基础镜像保持 digest 固定，上游重新发布时开 PR。
- `helpers:pinGitHubActionDigests` —— Actions 保持 SHA 固定，Renovate
  更新 SHA 的同时重写尾部版本注释（与 `ci.yml` 既有 pin 策略一致）。
- `enabledManagers` 限定为 `dockerfile` + `github-actions` —— 刻意不接管
  Cargo，避免把供应链 PR 和 Rust 依赖 PR 混在一起。
- 常规更新走每周窗口；**digest 刷新不排队**（`schedule: ["at any time"]` +
  `prPriority: 10`）。因为 quay 是按天回收的，「周一才提交周二发现的刷新」
  这段延迟本身就是故障窗口。
- 安全更新（`vulnerabilityAlerts`）完全绕过窗口与冷却期。

### ⚠️ 这份配置在 Renovate App 安装之前完全不起作用

**Renovate 是一个 GitHub App，不是 workflow。** 仓库里放一个 `renovate.json`
不会让任何事情发生 —— 没有 PR、没有 dashboard、没有 digest 刷新。必须由有仓库
管理权限的人在 <https://github.com/apps/renovate> 上把 App 装到
`oratis/Andromeda`（或在组织层面启用）。

**在此之前，不要把 `fedora-bootc` 重新 pin 回 digest。** 没有刷新机制的 pin
会在数天内重演 run 30702410393。

安装完成后的验收信号（三者齐备才算生效）：

1. 仓库出现一个 Renovate 开的 `Dependency Dashboard` issue；
2. Renovate 自己提交一个 "Pin dependencies" PR，把
   `quay.io/fedora/fedora-bootc:44` 改写成 `:44@sha256:...`；
3. `os/scripts/check-pin-freshness.sh` 对该引用报 `resolvable: yes`。

## 重新 pin 运行手册（fedora-bootc）

**前置条件（缺一不可）**

- [ ] Renovate App 已安装在 `oratis/Andromeda`；
- [ ] Renovate 已经实际开过至少一个 PR（证明它能写这个仓库，而不只是被安装）；
- [ ] `os/scripts/check-pin-freshness.sh` 当前无 `UNRESOLVABLE` 发现。

**首选路径：让 Renovate 自己 pin。**
`pinDigests: true` 会让 Renovate 首次运行时就开一个 "Pin dependencies" PR。
合并它即可 —— 这条路径的价值在于：**开 PR 这个动作本身就证明了刷新机制是活的**，
而手工 pin 得不到这个证明。

**回退路径：手工 pin（仅当 Renovate 因故不能改写该文件时）**

1. 解析当前 digest —— 检查脚本会直接打印可粘贴的引用：

   ```console
   $ os/scripts/check-pin-freshness.sh
   quay.io/fedora/fedora-bootc:44
     declared in os/Containerfile, TAG-TRACKED (no digest pin)
     ...
     to re-pin, the reference becomes:
       quay.io/fedora/fedora-bootc:44@sha256:413aa29c...
   ```

2. 把 `os/Containerfile` 的 payload stage 改为该引用（**保留 `:44` tag**，
   digest 只是追加；tag 提供可读性，digest 提供不可变性）。
3. 删除该行上方标注 “TIER A / RE-PIN PENDING” 的注释块。
4. 验证：`os/scripts/check-pin-freshness.sh --strict` 必须通过。
5. 合并后**盯住第一个 Renovate digest 刷新 PR**。如果两周内一个都没出现，
   说明自动化没生效 —— 此时应当立刻解绑回 tag，而不是等它烂掉。

**回滚**：任何时候 pin 变成 `manifest unknown`，把 `@sha256:...` 删掉即可
恢复构建（这正是 `d27cefa` 做的事）。这是一个单行、零风险的操作。

## pin 新鲜度检查

`os/scripts/check-pin-freshness.sh` 是 Tier A 的兜底：它在 pin 烂掉**之前**报警，
而不是等构建炸了才发现。

对扫到的每个镜像引用回答两个问题：

1. **RESOLVABLE?** registry 是否还提供这个 digest —— 这就是 fedora-bootc 那次故障；
2. **HOW OLD?** pin 的内容有多旧 —— 既是回收风险的代理指标，也是「漏了多少上游修复」的信号。

未固定的引用也会报告，并打印它当前解析到的 digest，让上面的运行手册变成复制粘贴。

```console
$ os/scripts/check-pin-freshness.sh          # 咨询模式，永远退出 0
$ os/scripts/check-pin-freshness.sh --strict # 有发现即退出 1
$ os/scripts/check-pin-freshness.sh --max-age-days 30
```

- 后端：优先 `skopeo`；缺失时回退到 `curl` + `jq` 走 Docker Registry v2 HTTP API
  （ubuntu-latest 预装两者，因此不需要 apt 步骤，也能在 macOS 开发机上跑）。
- 默认年龄预算 90 天，可用 `ANDROMEDA_PIN_MAX_AGE_DAYS` 覆盖。
- 默认扫描 `os/Containerfile` 与 `os/scripts/build-iso.sh`（**只读**）。

### 为什么 CI 里是非阻塞的

`.github/workflows/ci.yml` 的 `supply-chain` job 用 `continue-on-error: true`。

这一步报告的是**仓库之外的世界**：上游 registry 现在是否还提供某个 digest。
这个答案可以在没有任何人改动本仓库的情况下变化。如果把它设成合并门禁，
一个完全无关的 PR 会因为它作者既没造成、也无法在该 PR 里修复的问题而变红 ——
**这恰恰就是 run 30702410393 的体验**。这个信号属于仪表盘，不属于合并门禁。

需要门禁语义的调用方（例如 release 前检查）显式加 `--strict`。

### 当前输出（2026-08-02）

```
docker.io/library/rust@sha256:e51d0265...
  resolvable: yes
  !! STALE: built 2025-03-18T20:40:17Z (501 days ago, budget 90).

quay.io/fedora/fedora-bootc:44
  TAG-TRACKED (no digest pin)
  current digest: sha256:413aa29c...
  tag content built 2026-08-01T11:06:26Z (0 days ago)

ghcr.io/osbuild/image-builder-cli@sha256:67f1c248...
  resolvable: yes
  age: 7 days (built 2026-07-25T22:01:46Z), within the 90 day budget
```

两点值得注意：

- rust 的 501 天**是真阳性**，不是噪声。pin 本身没坏（Docker Hub 不回收），
  但它意味着 17 个月未合入的 bookworm 安全更新。Renovate 装上后会自动开这个 PR。
- fedora-bootc 的 tag 内容是**1 天前**构建的 —— 直观印证了它是高频重建的滚动 tag，
  也就是「digest 数天内被回收」的直接原因。

## ghcr.io 保留策略调查（评审标记的「下一颗地雷」）

**结论：`ghcr.io/osbuild/image-builder-cli` 的 digest pin 不会像 fedora-bootc 那样烂掉。可以保持现状。**

评审把它标为同一风险类别（`build-iso.sh:7` 已 digest pin，但 ghcr 的 GC 策略未验证）。
实测与文档两条证据都指向「安全」：

1. **GitHub 不做自动回收。** GitHub Packages 文档只描述用户主动删除，
   全文没有任何自动过期、保留期或 GC 机制；并且删除后 30 天内还可恢复。
   市面上那一堆 `ghcr-cleanup-action` / `container-retention-policy` 第三方
   Action 恰恰反证了这一点 —— 如果 GitHub 自己会清理，就不需要它们。
   （来源：<https://docs.github.com/en/packages/learn-github-packages/deleting-and-restoring-a-package>）
2. **实测该 digest 仍可解析。** 直接查 ghcr registry API，
   `sha256:67f1c248...` 返回 HTTP 200，而 `:latest` 已经指向
   `sha256:469a4269...` —— 即被取代的历史 digest 依然保留。
3. **该 digest 至今仍然「有 tag」。** 遍历该仓库全部 334 个 tag 逐一解析，
   `sha256:67f1c248...` 仍被 `sha-f8b00323e919f442980b5d316f3220dc2c3c9e3a`
   指向。osbuild 给每次构建都打 `sha-<commit>` tag（334 个 tag 里除 `latest`
   外全是这个形式），因此历史版本根本不进入「untagged」状态 ——
   即使将来 GitHub 引入 untagged 回收，也够不到它们。这是独立于第 1 条的
   第二层保护。

**与 quay/Fedora 的差别在于策略而非技术**：Fedora 主动修剪 fedora-bootc 的旧
digest 以控制存储；GitHub 不主动修剪。因此同样是 digest pin，前者需要刷新自动化，
后者不需要。

**残余风险（低）**：这是上游项目的一个策略决定，不是合同。若 osbuild 将来启用
清理 Action，pin 就会失效。`check-pin-freshness.sh` 已经覆盖 `build-iso.sh`，
会在那之前报 `UNRESOLVABLE`。

> `os/scripts/build-iso.sh` 由**另一个 PR**（`opt/e2e-p1-cache-split`）拥有，
> 本次未改动它。上述结论是「保持现状即可」，因此也无需改动。

## Tier B：dnf 快照评估

**结论：Fedora 目前没有为受支持版本提供可用的、持久的带日期快照源。不实施。**

评审建议让 payload 指向「带日期的 Fedora snapshot 镜像源」。实测四个候选端点：

| 候选 | 探测结果 | 可用性 |
|---|---|---|
| `kojipkgs.fedoraproject.org/compose/updates/Fedora-44-updates-<YYYYMMDD>.N/` | repodata HTTP 200 | ❌ **索引里只有 10 个日期**（F44：0724–0802；F43、F42 同样约 10 天窗口） |
| `dl.fedoraproject.org/pub/fedora/linux/releases/44/Everything/x86_64/os/` | HTTP 200 | ⚠️ 稳定但**冻结在 GA**，零安全更新 |
| `dl.fedoraproject.org/pub/fedora/linux/updates/44/...` | HTTP 200 | ❌ 滚动，无日期维度（这正是当前漂移的来源） |
| `dl.fedoraproject.org/pub/archive/.../updates/40/...` | HTTP 200 | ❌ 仅 EOL 版本；F44 在生命周期内不会进 archive |

**决定性事实**：`kojipkgs` 的 dated compose 只保留约 10 天
（2026-08-02 实测 F44 仅 `20260724.0`–`20260802.0`）。
把 Containerfile 指向其中某一个日期，**它会在约 10 天后 404** ——
比当初把构建打断的那个 quay digest 烂得还快。这会把一个可复现性问题
换成一个可用性问题，净损失。

> 注：10 天是**实测观察**，不是 Fedora 公布的策略；没有找到文档化的保留期承诺。
> 这本身也是不该依赖它的理由。

### 那么假如实施，Containerfile 会长什么样

记录下来以便将来 Fedora 真的提供持久快照时直接落地：

```dockerfile
ARG FEDORA_SNAPSHOT=20260802
RUN printf '%s\n' \
        '[fedora-snapshot]' \
        'name=Fedora $releasever snapshot' \
        "baseurl=<持久快照根>/${FEDORA_SNAPSHOT}/Everything/\$basearch/os/" \
        'gpgcheck=1' \
        > /etc/yum.repos.d/fedora-snapshot.repo \
    && rm -f /etc/yum.repos.d/fedora.repo /etc/yum.repos.d/fedora-updates.repo
```

代价（即使快照源存在也依然成立）：

- **快照不推进 = 不进安全更新**。必须配套一个「每周推进 `FEDORA_SNAPSHOT`」的
  定时 PR，否则把「不可复现」换成了「长期不打补丁」，后者更糟。
- 与基础镜像语义打架：`fedora-bootc:44` 本身自带仓库定义并按自己的节奏重建，
  外挂快照源会出现「基础镜像已经是 N+1，包却来自 N」的混合状态。
- 需要处理 GPG key 与 `$releasever`/`$basearch` 展开，`printf` 里的 `\$` 转义
  是易错点。

### 推荐替代方案（都比外部快照源可靠）

1. **内容寻址的 payload 层缓存（P1 #6，已规划）** —— 对 `Containerfile` +
   `os/files/**` + `crates/**` + `Cargo.lock` 算内容哈希，推拉
   `ghcr.io/oratis/andromeda-payload-cache:<hash>`。命中缓存 = 复用完全相同的
   包集合，**在不依赖任何外部快照端点的前提下拿到了可复现性**，而且顺带省掉
   11.1 分钟的 dnf 时间。必须设强制失效上限（如每周），否则安全更新进不来。
   这是 Tier B 事实上的正解。
2. **记录而非固定（成本 S，尚未实施）** —— 在构建阶段把 `rpm -qa --qf` 的完整
   NEVRA 清单写进证据包。不能让构建可复现，但能让两次构建**可 diff、可归因**：
   「昨天绿今天红」至少能立刻回答「哪些包变了」。
   注意实施时要与 `test-containerfile-layer-budget.sh` 的层预算协调。

## Tier C / Tier D 现状

- **Tier C（记录）**：CI 证据包目前**没有**记录 podman/qemu/OVMF/bootc 版本
  （GCP 路径的 `host-environment.txt` 有）。对应 P2 #11，未实施。
- **Tier D（签名）**：`ghcr.io/oratis/andromeda:edge` 仍是可变且未签名的 tag，
  且被写进每台安装机的 `--target-imgref`。对应 security-review #4 / 评审 Tier D。
  **在有真实用户跟随 `:edge` 之前必须配 sigstore / `containers-policy.json` 签名策略。**
  未实施。

## 横切：定时探测

上游漂移应该由定时任务先撞上，而不是由下一个倒霉的 PR 作者撞上
（fedora-bootc 那次正是被一个无关 PR 撞上的）。对应评审 P0 #5 的
`schedule:` 触发器，由 `opt/e2e-p0-hardening` 系列负责，不在本文范围内。
新鲜度检查每个 PR 都会跑，已经提供了一部分同样的早期信号。
