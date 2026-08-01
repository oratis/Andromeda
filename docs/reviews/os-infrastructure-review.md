# OS 镜像 / 安装器 / CI 基础设施评审

> 评审范围：`os/Containerfile`、`os/installer/*`、`os/scripts/*` E2E harness、`os/files/**`、`.github/workflows/{ci,os-e2e}.yml`。仅覆盖 OS 镜像 / 安装器 / CI 基础设施，不含 Rust 内部实现与 policy 语义。
> 评审基线：merged `main`（PR #7 merge）。
> 工具说明：本评审主机未安装 `shellcheck`，无法在本地复跑；CI（`os-e2e.yml`）在每次运行时对 `os/scripts/*.sh os/installer/*.sh os/files/usr/libexec/*` 强制执行 `shellcheck`。本评审主机也无 `/dev/kvm`，无法实跑 QEMU/nested-KVM E2E。

## 概览与总评

**评级：A− / "生产级基础设施，带一个突出的可观测性缺口"。**

这是一套罕见地成熟、以证据为中心的 bootc OS 交付流水线：安全边界（destructive install 严格 CI-only、`*-ci.iso` 命名）、marker 序列校验、SELinux karg 加固、平台守卫、云资源生命周期与成本控制都做得很扎实，且大量"防御性断言"能把静默失败转成可诊断的硬失败。

**当前 os-e2e 状态：green。** 关键证据：`os/` 与 `.github/workflows/` 目录树在 `44da8ed`（最后一次通过 os-e2e 的提交）与当前 merged `main` 之间 **逐字节相同**。即先前引发 `bootc install to-filesystem … exited with status 1` 的"过度合并 payload 层"已被回退，payload DNF 层数为 **23**，满足新守卫 `test-containerfile-layer-budget.sh` 的下限 **≥20**（`os/scripts/test-containerfile-layer-budget.sh:7`）。因此本分支的 payload 分层结构与已知 green 的提交内容等价。

**唯一必须重点整改的问题**：E2E harness 在 install 失败路径上无法可靠捕获 `bootc` 自身 stderr——这正是上次 root-cause 分析异常困难的根因，属于一等（first-class）可观测性缺口，详见问题 #1。

## 优点

- **安全边界严密且多重冗余。** `INSTALLER_DEFAULT` 在三处独立校验并互相印证：`build-iso.sh:21-33`（仅 `1` 产出 `-ci.iso`）、`Containerfile:360-368`（校验 0/1、`sed` 改写 `iso.yaml` 默认项、再 `grep -q` 回读确认）、`iso.yaml` 默认项 `default: 0` 为交互式非破坏性安装。破坏性 `zerombr/clearpart --all`（`andromeda-ci.ks:12-13`）只存在于 CI kickstart，只能经第二个 GRUB 条目触达。`README.md:7-14` 明确记录该边界。
- **destructive install 的运行时守卫。** `check-platform-compatibility.sh:65-79` 在 `ci` 模式下强制 `systemd-detect-virt --vm` 非空/非 `none`，从机制上阻止自动化破坏性安装落到真实机器；`test-installer-platform-guard.sh` 以 14 个用例覆盖 Apple 机型、架构不匹配、非法 manifest、VM 守卫等。
- **安装器 boot-path 加固到位。** `install-uefi-fallback.sh` 校验 ESP 分区类型 GUID、vfat、挂载点，复制 shim/grub/mm 到 `EFI/BOOT` fallback，删除重复 NVRAM 条目再新建（`:66-79`），并把安装后 target 的 kargs 强制为 `selinux=1 enforcing=1`，随后 **grep 断言** target loader entries 里不残留 `selinux=0` 且保留 `root=`/`boot=`（`:111-125`）。安装器环境 `selinux=0` 与安装结果 enforcing 的"permissive 装机、enforcing 首启 + `restorecon`"模式是刻意且正确的。
- **marker 序列校验严谨。** `test-install.sh:272-301` 的 Python 校验器先 strip `\0`、`\r`、ANSI 转义，再对 10 个 marker 做"恰好出现一次 + 顺序单调"双重断言；`gcp-run-e2e.sh:82-85` 以 `^ANDROMEDA_E2E_OK` 前缀锚定，明确防"同一物理串口行尾游标控制残留"导致的假阴性。
- **fw_cfg SHA 校验闭环干净。** host 经 `-fw_cfg name=opt/andromeda/update-sha256`（`test-install.sh:256`）注入期望 SHA，guest 从 `/sys/firmware/qemu_fw_cfg/.../raw` 读取并强校验 64 位 hex（`andromeda-ci-verify:39-53`）；CI-only 且 fw_cfg 缺失即硬失败（无回退）。
- **`tee /dev/ttyS0` 守卫得当。** 所有 `emit()` 与 `exec > >(tee …)` 均以 `[[ -c /dev/ttyS0 … ]]` 守卫（`andromeda-hardware-report:29`、`andromeda-first-boot-labels:8`、`installer-preflight.sh:9` 等），保证真实无串口硬件上单元不因证据管线失败。
- **云资源生命周期与成本控制优秀。** `gcp-run-e2e.sh` 单实例、`--max-run-duration`（默认 6h，平台侧硬上限）、`--instance-termination-action DELETE`、`--no-restart-on-failure`、EXIT trap 强制 `instances delete`、`disposable=true` 标签；即便脚本被 kill，平台侧 run-duration 上限也会删除实例。GCP 路径为算子手动执行、**未接入任何 workflow**，正确地把云成本挡在 CI 之外。
- **CI 去重与镜像卫生。** `ci.yml:49-52` 显式跳过 ubuntu 上的重复 `cargo test`；`Containerfile` 每个 dnf 层内 `dnf clean all && rm -rf /var/cache/dnf`，payload 末尾 `bootc container lint`（`:324`）作为构建门禁；`os/files` 无任何 `/usr/local` 内容（已核对），规避了常见的 `/usr/local` 只读 lint 告警。
- **runtime 加固与证据留存。** `andromeda-taskd.service` 全套 systemd sandbox（DynamicUser、ProtectSystem=strict、RestrictAddressFamilies…）；journald 持久化并有界（`Storage=persistent`、`SystemMaxUse=512M`、`MaxRetentionSec=14day`），与 artifact `retention-days:14` 对齐。
- **Firefox Wayland 真实性校验。** `andromeda-daily-driver-verify:307-348` 不满足于"进程存活"，而是以 peer inode 匹配确认到 compositor wayland socket 的 established 连接，杜绝静默回退 Xwayland 的假通过。

## 问题与风险（按严重度排序）

### 1.〔严重〕bootc 自身 stderr 在 install 失败路径上无法可靠捕获（一等可观测性缺口）
**这是导致上次 `bootc install to-filesystem … exited with status 1` root-cause 极难定位的根因。** 经确认，harness 没有任何一条路径能保证捕获 `bootc` 子进程自己的 stderr，问题分两层，互相叠加：

- **(a) 失败路径提前 `exit`，跳过 ESP 诊断收割。** `test-install.sh` 在 install 失败时于 `:166-170` 打印 `install-serial.log` 末 100 行后 **立即 `exit`**；而把 anaconda 在 ESP 上写下的诊断（`collect-anaconda-diagnostics.sh:36-55` 拷贝的 `program.log`、以及 `journalctl --boot` → `journal.log`）收割进上传目录的代码在 `:195-203`（挂载 ESP 后 `cp -a .../EFI/Andromeda/diagnostics`），**位于该 `exit` 之后、不可达**。同理 `:176-180` 的 partition-probe 失败也在收割前退出。结果：**恰恰在失败时**，包含子进程重定向输出的 `program.log` 与完整 `journal.log` 明明已落盘在磁盘 ESP 上，却从不进入上传 artifact。
- **(b) 没有对 `bootc` 输出做一等捕获。** `collect-anaconda-diagnostics.sh` 只收集通用 anaconda 日志（`/tmp/{anaconda,program,storage,packaging}.log`、`syslog`）+ `journalctl --boot`，**依赖这些通用日志"恰好"包含 bootc stderr**。Anaconda 44 原生 `bootc` payload 由独立的 Payloads DBus 模块驱动，其子进程输出是否完整落入 `program.log`／哪个 unit 的 journal 并不确定；上次实际失败中上传证据只留下被 anaconda 包装后的 "exited with status 1"，即为这层间接性丢失真实 stderr 的经验证据。此外 `program.log` 经 `tail -n 2000`（`:23`）截断，超大 payload 阶段的错误可能被截掉；且该串口回显路径还依赖 `%onerror` 确实触发（`%onerror` 仅存在于 `andromeda-ci.ks:25-27`，`interactive-defaults.ks` 无）。

**场景：** payload 安装期任何 bootc 失败（正是本次事件）。**严重度：高**——直接决定 MTTR，且会诱导误判（团队最初把它归到分层，实际需要 bootc 原始报错才能确认）。
**建议修复（具体）：**
1. 把 ESP/root 诊断收割移到 `install_status`/`partition_probe` 提前退出 **之前**（无条件 `mount -o ro` ESP → `cp -a EFI/Andromeda/diagnostics` → 再判定退出码）；失败路径必须与成功路径拿到同样的磁盘侧证据。
2. 在 `collect-anaconda-diagnostics.sh` 里 **一等地** 采集 bootc 输出：`journalctl -u 'org.fedoraproject.Anaconda.Modules.Payloads*' --no-pager -b` 单独落一份文件，并去掉/放宽 `program.log` 的 2000 行截断（或对 `program.log` 全量拷贝到 ESP）。
3. 更稳妥地，在 kickstart 侧显式以 tee 落盘 bootc 日志（或让 Payloads 模块以 debug 记录），确保无论 anaconda 版本如何演进都不丢原始 stderr。

### 2.〔中〕KVM→TCG 静默回退 + 超时预算，在无 `/dev/kvm` 时会以"困惑的超时"而非清晰失败告终
`test-install.sh:80-85` 与 `test-hardware-matrix.sh:31-44` 在缺 `/dev/kvm` 时静默回退 TCG。TCG 下 matrix 每 profile 超时提升到 **3600s ×3 = 3h**，加上 install 阶段 `timeout 45m`（`test-install.sh:109`）+ boot deadline `2700s`（`:261`），**已超过 job `timeout-minutes: 150`**（`os-e2e.yml:33`）。当前 `ubuntu-latest` 提供 KVM 故走快路径；但一旦 runner 机队变更取消 KVM，os-e2e 会从 green 直接翻成一堆难懂的超时红，而非一句明确的"需要 KVM"。相较之下 `gcp-run-e2e` 的 `test-gcp-nested.sh:41` 用 `test -c /dev/kvm` 强制前置校验，做法更好。
**场景：** GitHub 机队变更 / self-hosted 无 KVM。**严重度：中。**
**建议：** 在 CI 环境（或 `test-install.sh`）显式断言 `/dev/kvm` 存在（可用 env 开关允许本地 TCG 调试），把静默回退变成明确的前置失败；或让 TCG 分支的总超时预算与 job `timeout-minutes` 自洽。

### 3.〔中〕`ci.yml` 第三方 action 未按声明的"SHA 钉死"策略钉死
`os-e2e.yml:35-36` 声明策略：first-party `actions/*` 钉 major tag，third-party 钉完整 commit SHA。`os-e2e.yml` 自身合规（`jlumbroso/free-disk-space@54081f…` 用 SHA）。但 `ci.yml` **违反该策略**：`dtolnay/rust-toolchain@1.85.0`（`ci.yml:25,47`）与 `Swatinem/rust-cache@v2`（`:28,48`）均为 **可变 tag** 的第三方 action，未钉 SHA。
**场景：** 上游 tag 被重打 → 供应链投毒面。**严重度：中。**
**建议：** 将这两个第三方 action 钉到完整 commit SHA（配合 Dependabot 更新），与 os-e2e 的策略统一。

### 4.〔中/低〕`test-install.sh` 内联 trigger grep 未做 CR/ANSI 加固，可能引发 45 分钟假超时
项目已在 `gcp-run-e2e.sh:82-85` 明确修过"串口行尾游标控制残留导致假阴性"，但 `test-install.sh:263-268` 的 **触发用** grep 仍是对 **原始** 串口日志的 `grep -q ANDROMEDA_E2E_OK` / `grep -qE 'ANDROMEDA_.*_FAILED'`（严格的 CR/ANSI strip 只发生在触发之后的 Python 校验里）。若 marker token 被控制字符切断，触发 grep 永不命中，即使 marker 已发出也会白等到 `2700s` 超时。这是同一类、项目他处已修、此处仍潜伏的不一致。
**严重度：中/低。**
**建议：** 让 `test-install.sh` 的触发 grep 复用与 `gcp-run-e2e.sh` 一致的 `LC_ALL=C grep -aoE … | grep -a` 加固方式（或先 strip 到临时文件再 grep）。

### 5.〔低〕layer-budget 守卫是"层数代理"，挡不住"众多小层 + 单个超大层"
`test-containerfile-layer-budget.sh` 仅统计 payload 阶段 `^RUN dnf -y install \` 的**行数**（当前 23 ≥ 20），本质是以"层数"代理"单层 blob 体积"。它无法阻止未来把 20 个小层 + 1 个巨层混排——既过守卫又重新触发安装器 `/var/tmp` overlay 撑爆（正是本次事故成因）。守卫注释也自述这只是经验下限。
**严重度：低（当前无违规）。**
**建议：** 补一个对 `podman history`（`build-iso.sh:78-82` 已产出 `andromeda-v1-history.json`）单层 `Size` 的上界断言，直接约束真正的风险量。

### 6.〔低〕交互式安装失败无串口/ESP 证据
`%onerror` 诊断收集仅接在 `andromeda-ci.ks`，`interactive-defaults.ks` 无。交互式（面向人的默认 ISO）安装若失败不产出 `collect-anaconda-diagnostics` 证据。鉴于交互式由人驱动、可现场排查，属可接受取舍，但值得记录。
**严重度：低。** 建议：至少为交互式也挂上 `%onerror`，把诊断写入 ESP（人可事后取盘查看）。

### 7.〔低/信息〕镜像层数多带来 pull/元数据开销（刻意取舍）
23 个 payload 层 + 完整 KDE + LibreOffice + 全套 firmware + i686 gaming 库 + Firefox，镜像体量与层数都很大。多层有利于安装器逐 blob 暂存（守卫的初衷），但增加 pull 次数与 OCI 元数据开销。对 daily-driver 预览属固有取舍，已被守卫注释与 `README` 说明覆盖，无需改动，仅记录。

## E2E 可靠性与可复现性建议

1. **（最高优先）修复失败路径的证据收割顺序**：无条件先挂 ESP/root、`cp -a EFI/Andromeda/diagnostics` 与 root 侧 `var/log/anaconda`，再做退出码判定（问题 #1a）。这一步单独就能把上次那类 root-cause 从"数小时"降到"看一份文件"。
2. **一等捕获 bootc 输出**：`collect-anaconda-diagnostics.sh` 增加 Payloads 模块 journal（`journalctl -u 'org.fedoraproject.Anaconda.Modules.Payloads*'`），并放宽/去除 `program.log` 的 2000 行截断（问题 #1b）。
3. **把隐式 TCG 回退变成显式契约**：CI 前置 `test -c /dev/kvm`（对齐 `test-gcp-nested.sh:41`），或令 TCG 超时预算与 `timeout-minutes:150` 自洽，避免机队变更引发困惑超时（问题 #2）。
4. **统一串口 marker 检测的健壮性**：`test-install.sh` 触发 grep 采用与 `gcp-run-e2e.sh` 一致的 CR/ANSI 加固（问题 #4）。
5. **补强守卫的语义**：用 `andromeda-v1-history.json` 增加单层 `Size` 上界断言，直接守住"安装器 overlay 撑爆"的真实约束（问题 #5）。
6. **统一 action 钉死策略**：`ci.yml` 的第三方 action 钉 SHA（问题 #3）。
7. **可复现性增强（可选）**：`gcp-run-e2e.sh` 已把 `ANDROMEDA_SOURCE_REVISION` 贯穿到 evidence；建议 os-e2e 也把 `podman`/`qemu`/`ovmf`/`bootc` 版本与 ISO SHA 一并落入串口证据首部（`installer-preflight.sh` 已打印 bootc/podman 版本，可扩展到 host 侧），使 CI artifact 自证复现环境。

---

**关键结论**：基础设施整体为生产级（A−）。os-e2e 在 merged `main` 上 green（`os/` 与 workflows 与最后一次通过的 `44da8ed` 逐字节相同，layer 守卫 23≥20 通过）。**唯一必须整改的一等问题是 install 失败路径无法可靠捕获 `bootc` 原始 stderr**（问题 #1，成因二重：失败时提前退出跳过 ESP 诊断收割 + 无对 bootc 输出的一等采集），这正是上次事故 root-cause 困难的直接原因，修复成本低、收益高。

*Reviewed by Claude Code multi-agent review (OS infrastructure dimension).*
