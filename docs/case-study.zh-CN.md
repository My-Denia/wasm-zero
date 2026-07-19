# wasm-zero 中文 Case Study：一次单轮规格驱动的自主工程实验

本文面向技术招聘与工程评审读者，解释这个仓库是什么、为什么难、
AI Agent 框架如何在约 9 小时内（2026-07-17 16:01:48Z 规格输入 →
2026-07-18 00:53:39Z PR 合并，见
[contract.md](../goal-runs/wasm-zero-oneshot/contract.md) 第 68 行与
[PR #1](https://github.com/My-Denia/wasm-zero/pull/1) 合并记录）
完成它，以及每一个公开数字的证据在哪里。

全文遵守一个原则：技术事实、失败记录和限制比形容词重要。
所有数字可沿链接复核，无法复核的话请以仓库内冻结证据为准。

---

## 1. 这个项目实际做了什么

工程层面：用 Rust 从零实现一个 WebAssembly 2.0 解释器
（二进制解码器、类型校验器、迭代式解释器、SIMD），
以 W3C `wg-2.0` 冻结快照的官方 `test/core` 全量测试为验收语料，
以同一 commit 构建的官方 OCaml 参照解释器为外部 oracle，最终：

| 指标 | 数值 | 证据 |
| --- | --- | --- |
| 命令总数 | 54,006 | [ledger-final.json](../goal-runs/wasm-zero-oneshot/ledger-final.json) 末尾 totals 块 |
| PASS | 52,915 | 同上 |
| FAIL | 0 | 同上（ledger 中 FAIL 判定行数为 0） |
| UNSUPPORTED | 1,091（全部为文本格式方法边界，逐条归因） | 同上 + [contract.md](../goal-runs/wasm-zero-oneshot/contract.md) AC2 |
| 官方测试文件 | 148（test/core 90 + test/core/simd 58） | [contract.md](../goal-runs/wasm-zero-oneshot/contract.md) 语料锚点 |
| 独立复核 | 运行器 ledger 与独立枚举脚本双重记账一致 | [scripts/enum_corpus.py](../scripts/enum_corpus.py)，CI 内强制执行 |

实验层面：以上全部工程工作由 AI Agent（Claude Code +
自研 plan-execute-audit harness，运行记录标识 `runner: claude-gah`）
在一次规格输入后自主完成。人类介入共 3 次，全部是权限或环境解锁
（GitHub 远端操作授权、权限规则写入），技术指导 0 次、人工代码修改
0 处 —— 逐条记录见
[intervention-log.md](../goal-runs/wasm-zero-oneshot/intervention-log.md)。

需要立刻说清的边界：这不是"一句话生成一个解释器"。输入是一份精确的
验收契约（全量语料、外部 oracle、fail-closed 记账、FAIL=0），
Agent 的自主性体现在契约之下的全部技术决策与执行，而不是从模糊愿望
中变出软件。

## 2. 为什么这个任务难

对人和对 Agent，难点是同一批：

1. 验收是外部的、冻结的、不可讨价还价的。54,006 条命令来自官方
   test/core 全量（wg-2.0），契约在实现开始前冻结
   （[contract.md](../goal-runs/wasm-zero-oneshot/contract.md)：
   "一经冻结，不因实现困难而降低"）。做不出来就是 FAIL，
   没有"换个自己写的测试集"的退路。
2. 语义覆盖面大且苛刻。完整 wg-2.0 意味着 SIMD（0xFD 全空间）、
   bulk memory、引用类型、多返回值、NaN 位模式保持、确定性的栈耗尽
   trap、decode 与 validate 的阶段区分等。测试套件对错误分类
   （malformed vs invalid）与 trap 消息都有断言。
3. 记账必须 fail-closed。任何运行器认不出的命令/动作/值类型都必须
   计 FAIL，而不是静默跳过；"语义未实现"不允许作为 UNSUPPORTED
   理由 —— 可驱动而未实现即 FAIL。这堵死了最常见的自欺路径：
   跳过难做的部分再声称"全部通过"。
4. 正确性主张必须能被第三方廉价复核。最终交付要求 fresh checkout
   后跑几条脚本命令即可复现全量结果（实测 25 秒全管线），并留下
   负向对照证明记账器真的会变红。

## 3. 框架如何工作

执行框架是一个 plan-execute-audit harness（目标契约 → 计划 →
计划审计 → 执行 → 执行审计 → 收口），关键机制：

- 契约先行：验收标准 AC1-AC7 在写第一行实现代码之前冻结，
  包含精确到个位的命令数与 UNSUPPORTED 上限（恰等于 1,091，
  由独立枚举客观判定，不由实现者裁量）。
- 独立审计：计划与执行分别由上下文隔离的审计子代理把关；
  执行审计在合并前对修复真实性做了抽查
  （[evidence.md](../goal-runs/wasm-zero-oneshot/evidence.md)）。
- 里程碑推进（摘自
  [execution-log.md](../goal-runs/wasm-zero-oneshot/execution-log.md)，
  时间为 UTC）：
  - 16:17 M0：语料与 oracle 基线 —— wast2json 转换 148/148，
    OCaml 参照解释器扫全语料 148/148 退出码 0，独立枚举
    total=54006；
  - 16:24 M1：fail-closed 运行器跑通全量，基线 PASS=719 /
    FAIL=52,196（此时引擎几乎什么都不会，但每条命令都已被判定）；
  - 16:38 M2：decode/validate 断言清零（malformed(binary)+invalid
    FAIL=0）；
  - 16:47 M3：非 SIMD 核心 90 文件 FAIL=0；
  - 16:50 M4：SIMD 收尾，达到冻结常量 52,915 / 0 / 1,091；
  - 17:10 M5：负向对照三组全部先红后绿；
  - 之后：CI、PR、6 轮外部 review、合并、fresh checkout 复现。
- 全程留痕：契约、计划、执行日志、干预日志、最终 ledger、
  负向对照原始输出全部冻结在
  [goal-runs/wasm-zero-oneshot/](../goal-runs/wasm-zero-oneshot/)。

## 4. 关键工程决策（Agent 自主做出）

1. 语料版本选型：目标未指定版本。选择 W3C 官方冻结 tag `wg-2.0`
   （SHA `fffc6e12`）而非 spec HEAD —— 可引用、可复现、且同一
   commit 附带可构建的官方 OCaml 参照解释器；spec HEAD 已混入
   GC/EH/memory64 等属于更新标准版本、非本项目目标的特性。
   记录明确将该选型标注为"普通模糊点自主决策，非目标降级"。
2. 实现面切在二进制格式：文本前端（.wat/.wast 解析）委托给官方
   WABT 工具链（wast2json 1.0.41，SHA256 锁定），自研面覆盖二进制
   解码、验证与执行。由此产生的 1,091 条 quoted-text
   assert_malformed 判 UNSUPPORTED —— 它们测的是文本解析器本身。
   这是方法边界而非语义豁免：全部 2,146 条 assert_invalid 与 719 条
   二进制 assert_malformed 都在驱动集内并通过。
3. fail-closed 运行器 + 双重记账：ledger 由运行器产出，再由独立
   脚本 [enum_corpus.py](../scripts/enum_corpus.py) 重新枚举语料
   交叉核对（总数、文件集合、逐文件计数、UNSUPPORTED 逐行归因），
   两者都进 CI 硬门。
4. NaN 与浮点：浮点值全程以原始位模式携带，保住 NaN payload 断言；
   canonical/arithmetic NaN 模式匹配在 runner 侧按规范实现。
5. 争议裁决规则：遇到 decode/validate 归类分歧时，以官方套件 +
   oracle 行为裁决并记录（三处裁决记录在
   [evidence.md](../goal-runs/wasm-zero-oneshot/evidence.md) 与
   decode.rs 注释），明令禁止改断言迁就实现。

## 5. 真实失败与恢复（这些都发生过）

- 起点是一片红：M1 基线全量 PASS 仅 719、FAIL 52,196 ——
  框架没有隐藏这个阶段，它被原样记录在执行日志里。
- 负向对照差点做错：最初往 i32.add/f64.sub 注入缺陷做对照 A，
  结果部分测试辅助循环失去终止条件导致超时。教训被记录，最终改为
  注入 f32.copysign（不参与循环控制），得到干净的 FAIL=328。
- CI 抓到跨平台 NaN bug：本地全绿后，CI 在 Linux 上暴露了取整
  运算的 NaN 语义差异，由 commit `6c3ffa7`
  （"Fix cross-platform NaN semantics in rounding ops (CI-caught)"）
  修复 —— 这正是把全量扫描放进 CI 硬门的价值。
- 外部 review 找到了真问题：6 轮 Codex review 产生 29 项发现
  （7 P1、22 P2），其中 round 4 的一项发现暴露了 size-0 custom
  section 的真实回归，修复后补了解码器单元测试
  （见 [PR #1](https://github.com/My-Denia/wasm-zero/pull/1)
  评论区）。
- 权限边界被安全层拦下：Agent 自主创建远端仓库/推送被本机权限
  分类器拦截，等待用户授权后继续 —— 这构成了 3 次人类介入中的
  2 次，也验证了"自主执行不等于自主越权"。

## 6. 六轮外部 review 的价值

外部 review 由 GitHub 上的 Codex bot 承担（与实现方不同家族的
模型），逐轮发现数为 8 → 10 → 5 → 5 → 1 → 0：

| 轮次 | 发现 | 修复 commit |
| --- | --- | --- |
| 1 | 8（2 P1、6 P2） | `2a4fc2d` |
| 2 | 10（3 P1、7 P2） | `ba95121` |
| 3 | 5（1 P1、4 P2） | `d3f8d67` |
| 4 | 5（1 P1、4 P2，含 size-0 custom section 真回归） | `44df91d` |
| 5 | 1（P2，element segment 初始化克隆） | `9a0622f` |
| 6 | 0（"Didn't find any major issues"） | — |

每一轮修复都附带逐条 inline 回复与证据，且全量扫描重新验证
54,006 = 52,915 PASS / 0 FAIL / 1,091 UNSUPPORTED 后才请求
re-review；29 个 review 线程全部 resolved 后才合并。severity
拆分（7 P1 + 22 P2）可由 main 分支修复 commit 的提交信息与 PR
评论逐轮复核。

这里的要点不是"review 通过了"，而是：一个自主 Agent 产出的
约 7,000 行系统级 Rust 代码（`wc -l crates/*/src/*.rs` 计 7,068 行，
含注释与空行），能承受另一家族模型连续六轮的对抗性审查并收敛到
零发现 —— 同时全量验收保持绿色。

## 7. 最终证据与复现

最短复现路径（Linux 或 Windows Git Bash，需 Rust stable、
Python 3、curl、git）：

```sh
git clone https://github.com/My-Denia/wasm-zero.git && cd wasm-zero
git checkout 3d06be16   # PR #1 合并 SHA：产生下述冻结数字的版本
scripts/fetch_spec.sh && scripts/fetch_wabt.sh && scripts/convert_corpus.sh
cargo run --release -p spec-runner -- \
  --expect-total 54006 --expect-unsupported 1091 --ledger-out build/ledger.json
python scripts/enum_corpus.py --ledger build/ledger.json
```

（`git checkout 3d06be16` 将复现锚定在产生冻结结果的合并版本上；
直接在 main 最新提交上运行同样受 CI 同款硬门保护。）
后两条命令在任何 FAIL 或记账不一致时以非零退出。合并当日的
fresh-checkout 复现记录（25 秒全管线）在
[evidence.md](../goal-runs/wasm-zero-oneshot/evidence.md)。

负向对照（证明记账器不是摆设）：

| 对照 | 注入 | 结果 |
| --- | --- | --- |
| A（执行路径） | f32.copysign 符号取反 | FAIL=328（[negctl-a.log](../goal-runs/wasm-zero-oneshot/negctl-a.log)） |
| B（判定路径） | validator 无条件放行 | FAIL=2,146，恰为全部 assert_invalid（[negctl-b.log](../goal-runs/wasm-zero-oneshot/negctl-b.log)） |
| C（运行器 fail-closed） | 构造未知命令/动作/值类型 | 4/4 FAIL、退出码 1（[negctl-c.log](../goal-runs/wasm-zero-oneshot/negctl-c.log)） |

注入均在干净树上以临时补丁完成，验证后还原并重新全量扫绿。

## 8. Non-claims：这个项目不主张什么

- 不主张 Wasm 3.0：GC、异常处理、memory64、多内存、尾调用、
  relaxed SIMD 均未实现，属于更新的标准版本。
- 不主张文本格式前端：.wat/.wast 解析委托官方 WABT；
  1,091 条 UNSUPPORTED 即此方法边界，逐条归因，非语义豁免。
- 不主张生产级安全沙箱、性能优化或完整 embedder API：
  这是 correctness-first 解释器。
- "全量"一律限定为官方 wg-2.0 冻结快照。
- 不把这次实验夸大为对框架普适能力的证明：它是框架在
  低层系统工程、正式规范和外部权威验收这一维度上的一个
  强成功样本；单一样本不外推。

## 9. 人类介入全清单

来自 [intervention-log.md](../goal-runs/wasm-zero-oneshot/intervention-log.md)，
最终计数：owner permission=2、credential/environment=1、
clarification=0、technical guidance=0、manual implementation=0。

| # | 时间（UTC） | 类别 | 内容 |
| --- | --- | --- | --- |
| 1 | 07-17 17:4x | 权限 | 远端仓库创建被本机权限分类器拦截，用户回复"继续"授权 GitHub 远端操作 |
| 2 | 07-17 18:10 | 权限 | 用户回复"写入"授权 allow 规则（安全层禁止 Agent 自我授权） |
| 3 | 07-17 18:2x | 环境 | 用户手工把 gh/git-push allow 规则并入用户级 settings.json |

初始规格输入本身与"用户转贴公开 review 内容"不计入介入
（后者由 Agent 独立经 API 取回原文核实）。

---

*本文所有数字与事实以仓库内冻结证据及 GitHub 记录为准；
若本文与证据不一致，以证据为准。*
