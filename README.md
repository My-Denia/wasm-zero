# wasm-zero

A from-scratch WebAssembly 2.0 interpreter in Rust, acceptance-tested
against the **complete official `test/core` suite at the W3C `wg-2.0`
snapshot** with fail-closed accounting, and cross-checked against the
**official OCaml reference interpreter** built from the same commit.

## Results (frozen acceptance, verified in CI)

| Metric | Value |
| --- | --- |
| Corpus | `WebAssembly/spec` tag `wg-2.0` (`fffc6e12`), `test/core` 90 files + `test/core/simd` 58 files |
| Total commands | **54,006** |
| PASS | **52,915** |
| FAIL | **0** |
| UNSUPPORTED | **1,091** (all `module_type == "text"`; see method boundary below) |

Accounting is *no-silent-skip*: every command receives exactly one
verdict in {PASS, FAIL, UNSUPPORTED}, the runner's ledger is
cross-checked by an independent enumeration script
(`scripts/enum_corpus.py`), and unknown command/action/value shapes are
FAIL, never skipped.

## Architecture

- `crates/wasm-core` — the engine (zero dependencies):
  - `decode.rs` — strict wg-2.0 binary decoder (LEB128 width/sign
    checks, section order/size enforcement, full opcode space incl.
    `0xFC` and the SIMD `0xFD` space)
  - `validate.rs` — the spec-appendix type checker (operand/control
    stacks with bottom values), constant-expression rules, declared
    function references, import/limit checks
  - `store.rs` / `exec.rs` / `simd.rs` — instantiation per wg-2.0
    semantics and an iterative interpreter (explicit frame stack, so
    call-stack exhaustion is a deterministic trap); floats are carried
    as raw bits end-to-end to preserve NaN payloads
- `crates/spec-runner` — drives the wast2json-converted corpus and
  produces the ledger. Judgement rules live here (NaN patterns,
  trap-message matching, malformed-vs-invalid stage separation).

## Method boundary (frozen)

The implementation surface is the **binary format**. The text-format
frontend is delegated to the official WABT toolchain (`wast2json`
1.0.41, SHA256-pinned): `.wast` scripts are converted to JSON commands
plus binary modules. The 1,091 `assert_malformed` assertions whose
module is given as quoted *text* target the text parser itself and are
counted UNSUPPORTED with per-row attribution in the ledger — this is a
method boundary, not a semantic exemption. All 2,146 `assert_invalid`
and 719 binary `assert_malformed` assertions are in the driven set and
pass.

Trap-message matching rule: an `assert_trap` passes iff the action traps
and one message is a prefix of the other (spec-canonical messages, e.g.
`uninitialized element` vs `uninitialized element 7`). Invalid/malformed
expectations require the correct failure *stage* (decode vs validation),
not message equality.

## Reproduce (Linux or Windows Git Bash)

Prerequisites: Rust (stable), Python 3, `curl`, `git`.

```sh
scripts/fetch_spec.sh        # clone spec pinned at wg-2.0 (fffc6e12)
scripts/fetch_wabt.sh        # WABT 1.0.41 release, SHA256-verified
scripts/convert_corpus.sh    # 148 .wast -> build/wast-json/
cargo run --release -p spec-runner -- \
  --expect-total 54006 --expect-unsupported 1091 \
  --ledger-out build/ledger.json
python scripts/enum_corpus.py --ledger build/ledger.json
```

Both commands exit nonzero on any FAIL or accounting mismatch. CI runs
exactly this sequence plus `cargo fmt --check`, `clippy -D warnings`,
and unit tests.

## Oracle cross-check (reference interpreter)

The corpus-oracle consistency proof runs the official OCaml reference
interpreter — built from the *same* spec commit — over all 148 `.wast`
files (each exits 0). This is not part of the CI gate; to re-run it:

```sh
# any Linux/WSL with opam >= 2:
opam switch create wasm-zero ocaml-base-compiler.5.2.1
eval $(opam env --switch=wasm-zero)
opam install -y dune menhir
cd third_party/spec/interpreter && make   # produces ./wasm
cd .. && for f in test/core/*.wast test/core/simd/*.wast; do
  ./interpreter/wasm "$f" || echo "ORACLE_FAIL: $f"
done
```

During development, disputed decode/validate classifications were
adjudicated against the suite + oracle (never by editing expectations);
the three adjudicated nuances are documented in `decode.rs` comments and
`goal-runs/wasm-zero-oneshot/evidence.md`.

## Scope and non-claims

- Implements **WebAssembly 2.0** (wg-2.0 snapshot): SIMD, bulk memory,
  reference types, multi-value, sign-extension, saturating truncation,
  multiple tables, import/export of mutable globals.
- **Not** implemented: Wasm 3.0 features (GC, exception handling,
  memory64, multi-memory, tail calls, relaxed SIMD), the text format
  (delegated to WABT), threads, and any embedder API beyond what the
  spec-runner needs.
- This is a correctness-first interpreter, not a sandbox hardened for
  untrusted production workloads, and not performance-tuned.
- "Full test/core" always means the wg-2.0 snapshot of the suite.

## License

Apache-2.0.
