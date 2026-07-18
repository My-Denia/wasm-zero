# Evidence Index — wasm-zero one-shot

所有证据可由 fresh checkout 复现（见 README）；本索引映射到冻结验收 AC1–AC7。

## AC1 — 全量清零（硬常量逐位匹配）

- 命令：`cargo run --release -p spec-runner -- --expect-total 54006 --expect-unsupported 1091 --ledger-out build/ledger-final.json`
- 结果：`TOTAL files=148 commands=54006 PASS=52915 FAIL=0 UNSUPPORTED=1091 accounting=ok`，exit 0
- 台账：`ledger-final.json`（本目录，1,091 条 UNSUPPORTED 逐行 file+line+reason；FAIL 行为空）
- 双重核算：`python scripts/enum_corpus.py --ledger build/ledger-final.json` → 全部 CROSS-CHECK ok（commands/unsupported/files/per-file/逐行归因类），exit 0
- 确定性：连续两次 sweep 产出 ledger 逐字节相等（DETERMINISM: identical）

## AC2 — UNSUPPORTED 白名单

- UNSUPPORTED = 1,091 = 全量 JSON 中 `module_type=="text"` 命令数（独立枚举，见 wast2json.log INDEPENDENT_ENUM 与 enum_corpus.py 输出一致）
- 逐条归因：ledger-final.json 每条 UNSUPPORTED reason 均为 `text-format module (method boundary)`；off-class=0（enum_corpus.py 校验）
- assert_invalid 2,146 条全部 binary、全部 PASS（无一进入白名单）

## AC3 — 外部 oracle 一致性

- oracle-sweep.log：官方 OCaml 参照解释器（构建自同一 wg-2.0 SHA `fffc6e12`，WSL/OCaml 5.2.1 switch `wasm-zero`）对 148 个 wast 逐文件 rc=0，SUMMARY pass=148 fail=0
- 语料期望值未做任何修改；判分期望全部来自 wast2json 保真转换
- 裁决记录（开发中以语料+oracle 判定的两个解码口径，未改任何 expected）：
  1. memarg align ≥ 32 → 解码期拒绝（"malformed memop flags"）：align.wast L892-968 五条 assert_malformed 期望如此，oracle 对 align.wast exit 0 佐证
  2. datacount 缺失 + 使用 memory.init/data.drop：仅当 data section 存在时为 malformed（binary.wast L509/L530），无 data section 时留给验证期报 unknown data segment（memory_init.wast L190/L227 期望 invalid）；oracle 对两文件均 exit 0 佐证
  3. 类似：typed select 元数 ≠1 为验证期错误（select.wast L328 期望 invalid "invalid result arity"）

## AC4 — 正负对照

- 正向：AC1 全量绿 + exit 0。
- 负向 A（执行路径）：注入 f32.copysign 符号源取反 → `negctl-a.log`：FAIL=328；恢复后复绿。
  （注：最初尝试注入 i32.add/f64.sub 导致部分测试辅助循环失去终止性而超时——该学习记录于此；最终选择不参与循环控制的 copysign。）
- 负向 B（判定路径）：注入 validator 无条件接受 → `negctl-b.log`：FAIL=2,146（恰为全部 assert_invalid）；恢复后复绿 FAIL=0。
- 负向 C（runner fail-closed）：合成 JSON（未知命令类型/未知 action 类型/未知值类型/未知 module_type）→ `negctl-c.log`：4/4 FAIL 且原因逐条为 fail-closed 拒绝，RUNNER_EXIT=1。
- 全部注入均在 clean tree 上进行，验证后 `git checkout --` 恢复并重跑全量确认绿色。

## AC5 — CI

- 首次 CI 抓出真实跨平台缺陷：Linux glibc 路径 ceil/floor/trunc/nearest 对 SNaN 不置 quiet 位 → 32 条 nan:arithmetic FAIL；修复（显式 quiet + 回归单测）后双平台 FAIL=0
- 最终 HEAD（9a0622f）绿色 run：https://github.com/My-Denia/wasm-zero/actions/runs/29623532773（全量 sweep 硬门禁 + 独立枚举核对）；此前每轮 review 修复后 CI 均绿（29620342062 / 29620854353 / 29621496838 / 29622204333 / 29622836515）
- 前两次 CI 失败（脚本执行位 rc=126、NaN 分歧）及修复过程记录于 execution-log

## AC6 — 工程化交付

- fresh-checkout 复现（合并前预演，远端分支克隆到独立目录）：`git clone → scripts/fetch_spec.sh → scripts/fetch_wabt.sh → scripts/convert_corpus.sh → cargo run --release -p spec-runner -- --expect-total 54006 --expect-unsupported 1091` → SWEEP_EXIT=0；`enum_corpus.py` → ENUM_EXIT=0
- 合并后对 main（merge SHA 3d06be16）重复：见本文件末尾 "AC6 final" 记录

## AC7 — 审计闭环

- plan audit：independent plan-auditor，2 轮（needs-replan→pass），6 findings 全闭合（见 execution-log）
- execution audit #1（pre-push）：pass（重跑 sweep/枚举/lint、抽查 ledger 12 行对 wast 源、脱敏扫描）
- 内部代码评审 #1：1×P2 + 4×P3，全修复（commit 4c7996d 时代）
- 外部评审（Codex GitHub bot，跨家族）：6 轮，findings 8→10→5→5→1→0，共 29 条全部修复/回复/resolve；第 6 轮 clean（"Didn't find any major issues", reviewed 9a0622f）
- execution audit #2（pre-merge final）：pass（8/8 项亲自验证；修复真实性抽查 3/3 属实）
- 合并：PR #1 merge SHA 3d06be164522a7cd1716b21dd5709add02fe1751，2026-07-18T00:53:39Z，普通 merge（无 admin override、无 force push）

## AC6 final — 合并后 fresh-checkout 复现（2026-07-18）

- `git clone https://github.com/My-Denia/wasm-zero.git`（HEAD = merge SHA 3d06be16）→ fetch_spec / fetch_wabt / convert_corpus → sweep → enum
- 结果：`TOTAL files=148 commands=54006 PASS=52915 FAIL=0 UNSUPPORTED=1091 accounting=ok`，SWEEP_EXIT=0，ENUM_EXIT=0，全管线耗时 25s
