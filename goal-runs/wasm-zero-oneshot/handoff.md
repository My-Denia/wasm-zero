# Handoff: wasm-zero-oneshot

<!-- gah:auto -->
Status: completed | Risk: high | Audit: independent-subagents
Gates: plan=pass execution=pass
Latest checkpoint: merged + reproduced from fresh checkout.
<!-- /gah:auto -->

## 最终状态

- 目标交付：从零实现的 WebAssembly 2.0 解释器，官方 test/core（wg-2.0 快照，SHA fffc6e12）全量 54,006 条命令 = PASS 52,915 + FAIL 0 + UNSUPPORTED 1,091（后者恰为全部 module_type=="text" 的 assert_malformed，逐条归因，零语义豁免）
- 仓库：https://github.com/My-Denia/wasm-zero （公开）
- main = merge SHA `3d06be164522a7cd1716b21dd5709add02fe1751`（PR #1，普通 merge，2026-07-18T00:53:39Z）
- CI：GitHub Actions 全绿（最终 HEAD run 29623532773；main merge 后自动 run 亦应绿）
- 本地 `C:\Files\wasm-zero` main 与 origin/main 同步；工作树差异仅本 goal-runs 收尾记录

## 复现方式

fresh checkout 全管线（实测 25 秒，Windows Git Bash / Linux 同构）：

```sh
git clone https://github.com/My-Denia/wasm-zero.git && cd wasm-zero
scripts/fetch_spec.sh && scripts/fetch_wabt.sh && scripts/convert_corpus.sh
cargo run --release -p spec-runner -- --expect-total 54006 --expect-unsupported 1091 --ledger-out build/ledger.json
python scripts/enum_corpus.py --ledger build/ledger.json
```

两条命令 exit 0 即复现成功。oracle（官方 OCaml 参照解释器）复核路径见 README "Oracle cross-check" 一节（WSL switch `wasm-zero`，OCaml 5.2.1 + dune + menhir，同 SHA 构建）。

## 证据索引

见同目录 evidence.md（AC1-AC7 全部闭环）、ledger-final.json（逐行归因台账）、oracle-sweep.log、wast2json.log、negctl-{a,b,c}.log、contract.md（冻结验收）、plan.md、intervention-log.md、execution-log.md。

## 未来边界（非目标，未做）

- Wasm 3.0 特性（GC/EH/memory64/multi-memory/tail-call/relaxed SIMD）
- 文本格式前端（.wat/.wast 解析——由 WABT wast2json 承担）
- 生产级沙箱加固与性能优化（正确性优先的解释器）
- 结论表述必须始终带 wg-2.0 限定词（plan-audit 裁决）

## 遗留风险

- 引擎实现限制（TABLE_IMPL_LIMIT=2^27、MAX_LOCALS=1M、MAX_CALL_DEPTH=2000、MAX_VALUE_STACK=4M values）为 spec 允许的 embedder 限制，已注释；超限模块表现为拒绝/−1/trap 而非崩溃
- 32 位平台仅做了溢出安全推理与定点修复，无 32 位 CI 覆盖
- oracle 复核不在 CI 内（本机 WSL 证据 + README 第三方复核路径）
