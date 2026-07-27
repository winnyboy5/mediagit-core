# Perf baselines

`perf.tsv` is the comparison point for `08_perf.ps1`. It is **not** written automatically:
a baseline that re-created itself every run would happily adopt whatever regression it just
measured.

To promote a baseline after a campaign you trust:

```powershell
Copy-Item dev-tests\qa-suite\logs\<run-id>\perf-bench.tsv dev-tests\qa-suite\baselines\perf.tsv
```

`run_all.ps1` passes this file to `08_perf.ps1` automatically when it exists. Without it the
perf gate records `WARN` (informational) and the phase stays green.
