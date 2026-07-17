# Execution Log

## 2026-07-17T16:06:13+00:00

- Status: planning
- Event: run initialized for `wasm-zero-oneshot`.

## 2026-07-17T16:17:56+00:00

- M0 done: oracle 148/148 rc=0; wast2json 148/148; INDEPENDENT_ENUM total=54006 text=1091 baseline_fail=52915; plan audit pass (2 rounds, 6 findings fixed)

## 2026-07-17T16:24:56+00:00

- M1 done: baseline sweep total=54006 PASS=719 FAIL=52196 UNSUPPORTED=1091 accounting=ok exit=1; fmt+clippy clean; commit 5dc797b

## 2026-07-17T16:38:28+00:00

- M2 done: malformed(binary)+invalid FAIL=0; unit tests 5 pass; commit d3a7181

## 2026-07-17T16:47:01+00:00

- M3 done: core 90 files FAIL=0 (PASS 27437); remaining 24066 FAIL all SIMD; commit 4fd3f20

## 2026-07-17T16:50:58+00:00

- M4 done: FAIL=0 PASS=52915 UNSUPPORTED=1091 == frozen constants; lints green; commits 2931f12+d71cfd7
