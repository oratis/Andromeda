# Andromeda 开发者入门

## 1. 环境

- Rust 1.85 或更新版本；
- Git；
- Linux、macOS 或 Windows。

仓库通过 `rust-toolchain.toml` 固定最低工具链，CI 在三种宿主系统运行测试和硬件探测。

## 2. 验证工作区

```bash
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

## 3. 运行硬件预检

```bash
cargo run --locked --bin andromeda -- hardware probe
```

输出是 JSON，包含兼容性相关的整机型号、CPU、内存、启动能力和可枚举设备。它不收集序列号，也不自动上传。

示例 HCM：

```bash
cargo run --locked --bin andromeda -- hardware check \
  examples/hcm/developer-x86_64-pc.json
```

`effective_tier` 只有在 selector 和 requirements 都满足时才等于 manifest 声明等级，否则固定降为 `blocked`。正式产品还需要 HCM 签名和 CI evidence 验证。

## 4. 创建任务

```bash
cargo run --locked --bin andromeda -- \
  --state-dir .andromeda/state \
  task create-inspection . --requested-by local-user
```

该命令：

1. canonicalize 目标目录；
2. 创建只读文件 capability；
3. 创建 L1 inspection action；
4. 校验 schema、风险、依赖 DAG 和 capability subject；
5. 原子写入任务记录。

它不会遍历或修改目录。当前 runtime 不包含 tool executor。

## 5. 评估与转换

从创建结果取得 `plan.task_id` 后：

```bash
cargo run --locked --bin andromeda -- \
  task evaluate TASK_ID --isolation sandbox

cargo run --locked --bin andromeda -- \
  task transition TASK_ID --to running \
  --expected-revision 0 --actor local-runner
```

`--isolation` 是策略输入模拟，不证明进程真的运行在 sandbox。真实执行器必须由未来的 attestation 接口提供不可伪造的隔离证明。

## 6. 启动 API

```bash
RUST_LOG=info cargo run --locked --bin andromeda-taskd
```

默认地址是 `127.0.0.1:7777`，状态目录是 `.andromeda/state`：

```bash
curl http://127.0.0.1:7777/healthz
curl http://127.0.0.1:7777/v1/tasks
```

当前 API 无远程认证，不得改为公网监听。
