# Contract — wasm-zero one-shot

## 初始业务输入（原样，唯一目标）

> 目标：从零实现一个 WebAssembly 解释器，以官方 test/core 套件全量为验收语料、官方参照解释器为外部 oracle，fail-closed 核算：所有可驱动断言 PASS、FAIL=0；UNSUPPORTED 仅允许以工具链或文本格式等方法边界为由并逐条归因，不允许以"语义未实现"为由——任何可驱动而未实现的语义按 FAIL 计。

## 冻结的验收标准（在核心实现开始前锁定）

以下标准一经冻结，不因实现困难而降低。变更需数据证据 + 独立 plan audit + 明确记录。

### AC1 — 全量清零（硬常量，独立枚举自 build/wast-json/*.json，见 wast2json.log INDEPENDENT_ENUM 行）
`spec-runner` 对验收语料生成的全部命令逐条核算，最终 ledger 满足：
- 命令总数 = **54,006**（其中 assert_return 45,734 / assert_trap 2,408 / assert_invalid 2,146 / assert_malformed 1,810 / module 1,599 / action 155 / assert_unlinkable 83 / assert_uninstantiable 34 / register 22 / assert_exhaustion 15）
- FAIL = 0
- PASS = **52,915**（= 54,006 − 1,091）
- 每条命令必属 {PASS, FAIL, UNSUPPORTED} 之一；sum = 54,006（no-silent-skip）
- 计数来源：runner ledger 必须与对 JSON 的独立枚举脚本一致（双重核算）

### AC2 — UNSUPPORTED 白名单（精确上限）
UNSUPPORTED 仅允许以下归因类，且逐条记录（文件、行号、类别）：
- `text-format`: `module_type == "text"` 的命令（assert_malformed 中带引号文本模块），属文本格式方法边界（实现面为二进制格式，文本前端由官方工具链 wast2json 承担）
- UNSUPPORTED 计数必须 **恰等于 1,091** = 全量 JSON 中 module_type=="text" 命令数（独立枚举，客观判定不由实现者裁量）；assert_invalid 全部 2,146 条为 binary，无一可入白名单
- 不允许任何 "semantics-not-implemented" 类。可驱动（有二进制模块与明确期望）而未实现的语义 = FAIL。

### AC3 — 外部 oracle 一致性
官方参照解释器（OCaml，构建自与语料完全相同的 spec SHA）对全部 wast 文件运行且 exit 0，证明语料-oracle 自洽。争议断言以 oracle 行为裁决并记录于 evidence。禁止修改 expected 迎合实现、禁止以自身实现替代 oracle。

### AC4 — 正负对照
- 正向：全量 sweep FAIL=0 且 PASS 计数与命令枚举一致。
- 负向（至少三个，覆盖执行与判定两条路径）：(a) 执行路径——向引擎注入已知语义缺陷（如篡改 i32.add），sweep 必须 FAIL>0；(b) 判定路径——向 decoder/validator 注入"错误地接受"缺陷（如放行某类非法模块），assert_malformed(binary)/assert_invalid 断言必须产生 FAIL>0；(c) runner fail-closed——面对未知命令类型/未知期望模式必须计 FAIL。对照以临时 patch 实施（先 stash/干净树），验证后恢复并留存证据。

### AC5 — CI 全绿
GitHub Actions：build + rustfmt + clippy(-D warnings) + 单元测试 + 全量 sweep（FAIL=0 为门禁硬条件）全部成功。

### AC6 — 工程化交付
fresh checkout 按 README 复现全量 sweep 结果（复现范围 = fetch spec/wabt + 构建 + 全量 sweep FAIL=0；不含 OCaml oracle 重建）；oracle 复核路径在 README 单列（WSL/opam、switch 名 wasm-zero、同 SHA 构建步骤），使 AC3 对第三方可复核；README/文档无私有路径、无凭据；PR 按授权合并、main 与远端一致且 worktree clean。

### AC7 — 审计闭环
plan audit pass；execution audit（独立子代理）pass；review loop 闭环（所有 threads resolved，真实 findings 修复+回归）。

## 语料锚定（范围决策 D1）

- 仓库：https://github.com/WebAssembly/spec
- 版本：tag `wg-2.0`，SHA `fffc6e12fa454e475455a7b58d3b5dc343980c10`（W3C Wasm 2.0 工作组官方冻结快照）
- 范围：`test/core/*.wast`（90 文件）+ `test/core/simd/*.wast`（58 文件），共 148 文件全量
- 理由：目标未指明版本；wg-2.0 为 W3C 官方 tagged 冻结快照，语料稳定、可引用、oracle（同 SHA interpreter/）可构建；主干 HEAD 含 Wasm 3.0（GC/EH/memory64 等）属后续标准版本。此为普通模糊点自主决策，非目标降级：所选快照内 100% 语义均在验收内，无任何语义豁免。
- 非目标：Wasm 3.0 特性。在 README 与最终报告中显著披露。

## 方法边界决策（D2）

- 文本格式前端：官方 WebAssembly 工具链 WABT `wast2json`（锁定版本+校验和）将 .wast 转为 JSON 命令 + 二进制模块。本工程实现面 = 二进制格式解码/验证/执行 + JSON 驱动。纯文本模块的 malformed 断言归 UNSUPPORTED(text-format)。
- 判分期望值来自官方语料本身（wast2json 保真转换）；oracle 为官方 OCaml 参照解释器（AC3）。

## 授权边界（owner 裁决记录）

- 工作区：C:\Files\wasm-zero（本次创建）
- 禁读：wasm-oracle 仓库（本地+GitHub 远端全部内容）；C:\Files\ablation-2arm
- 从零推导：实现、测试脚手架、判分核算均从官方规范与官方套件推导
- 签名：全程 unsigned 提交（owner 裁决，运行记录披露一次，不以 Verified 为合并条件）
- 预授权：依赖安装/构建、My-Denia 名下新建本次专用远端仓库、分支/提交/push/PR/CI/review 回复/满足条件后合并
- 禁止：force-push、改写共享历史、admin override、删除非本次创建的远端资源、修改伪造 expected、隐藏 FAIL/UNSUPPORTED、提交凭据或私有路径

## 环境事实（Phase 0 审计）

- Windows 11 + Git Bash；git 2.47.1；gh 2.83.1 已认证 My-Denia（repo/workflow scope）
- Rust 1.96.1（实现语言决策 D3：Rust——原生 IEEE754 f32/f64、性能足够跑 5 万断言、cargo 测试与 CI 成熟）
- WSL Ubuntu：opam 2.5.1，新建独立 switch `wasm-zero`（OCaml 5.2.1）用于构建 oracle；不复用旧项目 switch
- 开始时间（UTC）：2026-07-17T16:01:48Z
