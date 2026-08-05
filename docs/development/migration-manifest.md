# Windows/macOS 只读迁移扫描器与 Migration Manifest v1

## 交付边界

`andromeda migration scan` 已实现一个跨平台、只读的迁移 inventory。它从用户明确指定的
profile root 中扫描 `Desktop`、`Documents`、`Downloads`、`Pictures`、`Music`、
`Videos`/`Movies`，为每个普通文件计算 SHA-256，并列出有限的用户级应用候选：

- macOS/profile `Applications` 目录中的 bundle 或目录；
- Windows `AppData/Local/Programs` 的一级候选；
- Linux `~/.local/share/applications/*.desktop` 的文件名候选。

它生成 [`migration-manifest.schema.json`](../../schemas/migration-manifest.schema.json)
定义的 v1 JSON。该清单是“发现了什么、什么没能读取”的证据，**不是**“已经迁移成功”的
证据。

```bash
# 当前系统的默认用户 profile（Windows 用 USERPROFILE，其他平台用 HOME）
cargo run --locked --bin andromeda -- migration scan --output migration.json

# 在 Linux/Andromeda 上只读扫描一份挂载的 Windows profile
cargo run --locked --bin andromeda -- migration scan \
  --profile-root /run/media/source/Users/Alice \
  --source-platform windows \
  --output migration.json
```

输出文件使用 `create_new` 语义，已存在就拒绝，绝不覆盖另一份迁移证据。未指定
`--output` 时 JSON 写到 stdout，摘要写到 stderr，便于接 `jq`。

## 当前成立的不变式

- 源 profile 和其中的文件不会被写入；
- 只扫描固定的用户数据目录，不进入浏览器 cookie、密码库、SSH 密钥、系统目录或隐藏的
  应用数据树；
- 符号链接会写入 `skipped[]`，但绝不跟随；profile root 或顶层数据目录本身为符号链接时
  也不会被当作真实目录进入；
- 文件路径相对 profile root 输出，不在 manifest 中泄露绝对挂载点或用户名路径前缀；
- 每个可迁移文件记录大小、mtime 和 SHA-256；哈希期间元数据变化会记为
  `changed_during_scan`，不会把一个不稳定快照伪装成可导入项；
- 默认最多哈希 100,000 个文件、访问 250,000 个目录项、进入 64 层目录；命中任何硬上限
  会令 `summary.truncated=true`，并在 `skipped[]` 留下原因；
- 权限错误、读取错误、特殊文件和所有其他跳过项都进入 `skipped[]`，迁移报告不能静默隐藏
  已观察到但未清点的数据；
- 文件与候选应用按相对路径稳定排序，方便 diff 和后续可恢复导入。

Installable OS E2E 会在 first boot、revision 2 更新启动和 revision 1 回滚启动三个阶段，以
普通测试用户运行扫描器，并验证持久化文件的相对路径与 SHA-256。它证明 scanner 已进入镜像且
跨系统部署切换可读到同一用户数据；它仍不证明任何物理机器或真实 Windows/macOS 源盘。

Migration Manifest 本身含文件名、大小和哈希，仍属于敏感本地数据。CLI 不上传它，调用方也
不应把它当普通遥测发送。

## 尚未实现

- 文件复制、空间预留、暂停/恢复、导入后逐项复验和事务回滚；
- OneDrive/iCloud placeholder 与云端导出；
- 浏览器书签、邮件、日历、联系人及经用户授权的密码导出；
- Windows Registry、系统级已安装应用、macOS LaunchServices/receipt 的完整应用清单；
- Windows Known Folder 重定向与 macOS 非默认/本地化数据目录的系统 API 解析；
- 应用到 Native/Web/Wine/Windows Workspace 的兼容数据库映射；
- NTFS/APFS 休眠/加密状态检测、BitLocker/FileVault 解锁和安全共存安装；
- P2V、Windows 激活/许可证处理和 macOS 受授权环境保留。

下一阶段 importer 必须把源卷保持只读，把 manifest 作为输入，为每个复制项记录目标路径、
复验哈希、状态与错误，并让中断后的重跑幂等。没有这些证据前，产品只能说“完成 inventory”，
不能说“完成迁移”。

`source_platform` 由调用方指定（默认当前主机），manifest v1 也没有签名，因此它不是来源身份
证明。对可能被对手并发修改的源 profile，未来 importer 还必须运行在独立只读 sandbox 中并以
文件句柄/卷快照重新验证；当前的 size + mtime 稳定性检查只用于发现普通扫描期间的变化，不是
对抗性 TOCTOU 防线。
