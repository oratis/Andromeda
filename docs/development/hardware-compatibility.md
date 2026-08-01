# Hardware Compatibility Manifest 开发说明

## 探测不等于支持

硬件 probe 只能回答“系统看到了什么”。Supported/Certified 必须同时具备：

- 精确整机、主板和固件 selector；
- kernel/firmware/driver 版本组合；
- 实机启动、安装、更新、回滚、休眠、GPU、网络、音频和外设测试；
- 签名 HCM；
- 可追溯 CI evidence；
- 安全更新责任人与支持期限。

## v0 报告

报告包含：

- OS family 与 CPU 架构；
- manufacturer/model/board/firmware；
- CPU、逻辑核心和内存；
- UEFI、Secure Boot、TPM2、虚拟化；
- Linux PCI/USB ID 与绑定驱动；
- probe 无法验证的警告。

为保护隐私，v0 不采集序列号、磁盘 UUID、MAC 地址、Windows machine GUID 或 Apple platform UUID。

## Manifest 规则

- `selectors` 使用 OR：至少一个 selector 完整匹配；
- 同一个 selector 内字段使用 AND；
- 空 selector 列表永远不匹配，避免误把所有硬件升级为支持；
- `requirements` 全部满足才保留声明 tier；
- 任意 selector/requirement 失败，`effective_tier` 降为 `blocked`；
- ID 比较忽略 `0x` 前缀和大小写；
- 需要驱动的设备必须有 bound driver evidence。

Schema 位于 [`schemas/hardware-compatibility-manifest.schema.json`](../../schemas/hardware-compatibility-manifest.schema.json)，示例位于 [`examples/hcm/developer-x86_64-pc.json`](../../examples/hcm/developer-x86_64-pc.json)。

## 下一步

1. 对 HCM 使用离线根密钥和轮换签名；
2. 把 kernel channel、HEP、firmware hash 与测试 evidence 变成强制字段；
3. 从 Windows/macOS source agent 导入更完整但经用户同意的设备 inventory；
4. 建立 QEMU、参考 PC、Intel Mac、T2、M1/M2 分离的实验室队列；
5. 只有 CI 达到 SLO 后才允许 tier promotion。
