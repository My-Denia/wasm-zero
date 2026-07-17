# Plan: wasm-zero-oneshot

Status: ready-for-audit
Created: 2026-07-17T16:06:13+00:00

## Goal

从零实现 WebAssembly 解释器：官方 test/core (wg-2.0) 全量为验收语料、官方参照解释器为外部 oracle，fail-closed 核算 FAIL=0

## Scope

C:/Files/wasm-zero 全部；My-Denia 名下本次新建远端仓库

## Non-Goals

禁读 wasm-oracle 仓库(本地+远端)；禁读 C:/Files/ablation-2arm；不改 C:/Files/wasm；Wasm 3.0 特性(GC/EH/memory64等)不在本次验收内；不引入付费服务；unsigned 提交

## Risk Level

high

## 技术路线

实现语言 Rust（原生 IEEE754、性能、CI 成熟）。实现面 = Wasm 2.0 二进制格式解码 + 验证 + 解释执行。文本前端 = WABT wast2json 1.0.41（官方 WebAssembly 工具链，版本+SHA256 锁定）。oracle = 官方 OCaml 参照解释器（同 wg-2.0 SHA，WSL/OCaml 5.2.1 构建，已验证可构建，`wasm 2.0.2 reference interpreter`）。

## Milestones

### M0 — 契约与 oracle 基础设施
交付：contract.md/plan.md；oracle 二进制；scripts/fetch-spec + scripts/fetch-wabt（SHA 锁定）；wast2json 转换 148 wast → build/wast-json/
验证（二进制）：oracle 对 148 wast 全部 exit 0（oracle-sweep.log）；wast2json 148/148 转换成功；命令总数落账
回滚：third_party/ 与 build/ gitignored，可整目录重建

### M1 — Rust workspace + spec-runner 骨架（fail-closed 基线）
交付：crates/wasm-core、crates/spec-runner；runner 解析全部 JSON 命令，零引擎状态下除 text-format UNSUPPORTED 外全部 FAIL
验证（精确等式）：ledger sum=54,006 且 FAIL=52,196、UNSUPPORTED=1,091、PASS=719（fail-closed 基线；719=assert_malformed(binary) 在全拒 stub 解码器下的空真通过，原计划"PASS=0"系推导错误——该空转风险由 M5 判定路径负向对照专项覆盖；最终态 AC1 常量 PASS=52,915 不变）；cargo fmt --check、clippy -D warnings 通过
回滚：git

### M2 — 二进制解码器 + 验证器
交付：LEB128/全 section/全指令（含 SIMD/bulk/reftype）解码；类型检查验证器
验证：assert_malformed(binary) 与 assert_invalid 断言 FAIL=0；解码单元测试
回滚：git

### M3 — 执行引擎核心（含全部非 SIMD 2.0 特性）
交付：栈式解释器（数值全指令+NaN、控制流、call/call_indirect、memory/table/global/start、trap、spectest 宿主、register 链接、栈深限制）+ 多值、sign-ext、饱和截断、bulk memory、reference types
验证：test/core 顶层 90 文件（非 SIMD 全部）FAIL=0
回滚：git

### M4 — SIMD 收尾
交付：v128 全指令（解码/验证已在 M2，本里程碑补执行语义）
验证：148/148 文件 FAIL=0（AC1）
回滚：git

### M5 — 核算台账 + 正负对照
交付：机器可读 ledger、UNSUPPORTED 逐条归因表（恰 1,091 条，逐条 file+line）、负向对照三件套（执行路径：注入 i32.add 缺陷→FAIL>0；判定路径：decoder/validator 注入"错误接受"→malformed/invalid FAIL>0；runner fail-closed：未知命令类型→FAIL）、oracle 裁决记录
验证：AC1/AC2/AC4 二进制通过；对照证据存档后恢复绿色
回滚：对照注入临时 patch（先确保干净树），验证后 git checkout 恢复

### M6 — CI + 文档 + 远端
交付：GitHub Actions（build+fmt+clippy+unit+fetch spec/wabt+全量 sweep FAIL=0 门禁）；README；My-Denia/wasm-zero 远端；push
验证：CI 全绿（AC5）
回滚：远端为本次新建，可整体废弃

### M7 — PR + review 闭环 + 合并 + 报告
交付：PR、完整 review loop、execution audit、合并、fresh-checkout 复现、evidence/handoff/autonomy report
验证：AC6/AC7；merge SHA；fresh checkout FAIL=0
回滚：不可逆点仅 merge，全部门禁先行

## Assumptions

| Assumption | Evidence | Status |
| --- | --- | --- |
| wg-2.0 interpreter 可在 OCaml 5.2.1+dune 构建并对语料自洽 | oracle-sweep.log：148 行 rc=0 + SUMMARY pass=148 fail=0 | verified |
| wast2json 1.0.41 可无损转换 wg-2.0 语料 | wast2json.log：148 行 rc=0 + SUMMARY ok=148 bad=0 + INDEPENDENT_ENUM total=54006 assert_return=45734 module_type_text=1091 baseline_fail=52915 | verified |
| Rust f32/f64 + 显式 NaN 处理匹配 spec | M3/M4 断言 + oracle 裁决 | pending |
| 全量 sweep 秒~分钟级 | M1 实测 | pending |

## Validation Matrix

| Milestone | Check | Status |
| --- | --- | --- |
| M0 | oracle 148/148 rc=0（oracle-sweep.log）；wast2json 148/148（wast2json.log） | done |
| M1 | ledger sum=54,006 且 FAIL=52,915 / UNSUPPORTED=1,091 / PASS=0 | pending |
| M2 | malformed(binary) 719 + invalid 2,146 断言 FAIL=0 | pending |
| M3 | test/core 顶层 90 文件 FAIL=0 | pending |
| M4 | 148 文件 FAIL=0，PASS=52,915 | pending |
| M5 | 三路负向对照 FAIL>0 证据 + 恢复绿色 | pending |
| M6 | CI 全绿 | pending |
| M7 | merge SHA + fresh checkout FAIL=0 | pending |

## Rollback

代码回滚走 git；third_party/build 可重建；负向对照用临时 patch 并恢复；唯一不可逆点为 PR merge，置于全部门禁之后。远端仓库为本次新建，最坏可整体废弃不影响任何既有资源。
