# Refine Edges parallelism

Candidate scores are independent until a move is accepted. The native search
parallelizes candidate generation, speculative pair rankings, rescoring of
stale pairs, and full-image finalist checks. It consumes results in the original
order. Accepted moves remain ordered because they change subsequent source-fit,
neighbour and full-image comparisons.

Completed-render reuse, boundary-profile reuse and combination-score reuse are
separate caches. Within a search, full-image scores for repeated identical edits
are memoized while the committed image stays unchanged. Any accepted edit clears
that memo table, since the global score depends on the rest of the image too.

`glyph_search/rank_cache.rs` also caches local trial scores using the source
window, surrounding output pixels, colours, masks and candidate pools as its
exact key. Image rendering enables exact reuse by default, with a 256 MiB native
budget and 8 MiB WASM budget. The image TUI and browser renderer own this cache
across render requests; the native owner also survives render-task thread
migration. Video retains its configurable budget and optional RGB tolerance.
Lookups run before speculative scoring, and a hit skips patch preparation and
candidate scoring. If a neighbour changes before consumption, the search checks
the updated context and recomputes on a miss. Full-image acceptance remains
fresh. A universal score indexed only by two glyph IDs would not capture the
source, colours or neighbouring glyphs.

A November 4 release-build check isolated this local score cache by bypassing
completed-output reuse. Medians over three runs, four CPU workers, 48 columns,
six candidates, two passes, three finalists and blending enabled:

| Case | Cache disabled | Empty score cache | Reused score cache |
| --- | ---: | ---: | ---: |
| dexter3, large + space | 3.547 s | 3.835 s | 1.096 s |
| dexter3, smooth | 4.793 s | 4.423 s | 1.246 s |
| dexter1, foreground-only, large + space | 3.704 s | 2.967 s | 0.894 s |

All 27 outputs and final scores matched their uncached equivalents. Reused-cache
runs had no local-ranking misses. These are search times, not whole-render
times. Other applications were active, so wall times varied. Key construction
and storage have a cost: the empty-cache large-glyph case was about 8% slower
than disabling reuse. Repeating identical settings in the TUI normally uses its
completed output directly; local score reuse also helps when settings change
but some source/pool/neighbourhood contexts remain identical.

Changing the same large-glyph case from two to three passes exercises that
settings-change path: an uncached three-pass search took a median 5.557 s,
versus 2.816 s when reusing scores from the two-pass render. The completed-output
cache was bypassed. Each changed-settings run reused 5,520 local rankings and
missed 1,473; all output bytes and scores matched the uncached three-pass result.
This comparison used three repetitions with the same thread and render settings.

Initial batches parallelize across pairs. A pair invalidated by an accepted
move can instead distribute its candidate trials across workers. Small trial
sets stay serial to avoid dispatch and cache-copy overhead. Source-reference
filters are immutable during a search: a shared cache reuses them across pair
jobs and passes. Retained source-reference data is bounded to 32 MiB natively
and 8 MiB in WASM; output caches still follow accepted changes.

Within one score, pixel transforms and independent rows/columns of filters
also offer parallel work. Tiny patches generally make candidate-level CPU
parallelism a better place to spend thread-dispatch overhead. Floating-point
reductions and each running filter scan retain their existing arithmetic order.

For the October 31 source-reference and CPU-parallelism changes, four
release-build benchmarks using four CPU threads gave these median search times
over three runs:

| Case | Before | After |
| --- | ---: | ---: |
| Blending enabled, 32 columns | 2.195 s | 1.636 s |
| Blending enabled, 64 columns | 8.073 s | 6.221 s |
| Foreground-only with blending, 48 columns | 5.488 s | 3.989 s |
| Blending disabled, 64 columns | 5.215 s | 4.360 s |

Every before/after comparison used the same explicit blending setting and
search parameters (six candidates, two passes, three finalists). Rendered bytes,
candidate counts, move counts and reported search scores matched in all 24 runs.
These are input-dependent examples, not a guaranteed speedup for every image.

The November 4 search changes cache boundary-profile equivalence classes for
all rendered glyph masks, including inversions. Glyphs with the same boundary
pixels share the same connection calculation. Source gradients and actual
colours still determine the cost of a connection: an intentional gap or line
ending must remain eligible. The cache is linear in the glyph count rather than
a quadratic table of glyph pairs. Source-reference filters are also deduplicated
across identical source windows, with pixel and dimension checks after hashing.

Speculative ranking batches now shrink when accepted edits frequently invalidate
the prepared work, and grow when those rankings remain valid. This uses observed
search results to avoid scoring candidates twice. An explicit
`IMG2IRC_REPAIR_BATCH_SIZE` still selects a fixed batch size, including zero.
Scoring workspaces retain their buffers across batches, so shrinking the batch
does not repeatedly allocate worker caches. Precision-order resolution skips
its buffer reset when no trial needs an exact check.

When a pass stalls, remaining passes can try source-fit glyphs absent from the
failed pools. The incumbent and strongest source-fit alternative remain
available; successful changes reset that history because the context has changed.
The candidate/pass budgets and full-image improvement checks still apply. This
can improve a result after the previous search would have stopped early.

A more aggressive shortlist based on possible connections to proposed neighbours
was tested and rejected because it worsened final scores in several cases.
SIMD matching and a cache of individual edge comparisons also failed to provide
consistent runtime improvements and were removed.

Against the preceding optimized build, the retained changes gave these median
search times over three alternating runs per build, with four CPU threads and
process affinity restricted to four physical cores:

| Case | Before | After | CPU time saved | Invalidated rankings avoided |
| --- | ---: | ---: | ---: | ---: |
| dexter1, large + space, 48 columns | 2.598 s | 2.625 s | 3.6% | 23% |
| dexter3, large + space, 48 columns | 4.377 s | 3.721 s | 5.1% | 26% |
| dexter3, smooth, 48 columns | 3.691 s | 3.479 s | 5.8% | 30% |
| dexter1, foreground-only, large + space, 48 columns | 3.463 s | 3.390 s | 6.5% | 23% |
| ralph, smooth, 40 columns | 5.302 s | 4.796 s | 2.4% | 25% |

All 30 rendered outputs, final scores, trial counts and move counts matched.
CPU time is process user + system time across all threads; the Before/After
columns are wall time for the search. These runs used six alternatives, two
passes, three finalists and blending enabled.
Other renders were active during measurement, and wall times varied substantially.
CPU time fell in every case; wall-time medians ranged from about 1% slower to
15% faster. The invalidation counts are deterministic for these settings.

With eight passes allowed, four smaller cases also exercised the stalled-search
fallback. Scores decreased from 31.043862 to 29.765513 (dexter1, 8 columns),
26.078073 to 25.518254 (dexter3, 8 columns), 30.579622 to 29.939503 (porygon,
10 columns), and 26.016306 to 25.310454 (didi, 8 columns). Each used large +
space. These 2–4% score improvements required additional passes and more time
than stopping early. The fallback improves quality within the requested budget.

## GPU feasibility benchmark

The application scorer currently runs on the CPU. The standalone
[CUDA benchmark](../examples/gpu_blur_bench.cu) tests a separable blur used by
the scoring pipeline, using the same scan order and double-precision running
sums. It checks CPU/GPU output bytes for equality. It does not implement the
complete scorer, its incremental caches, or search acceptance.

On Linux with CUDA and an OpenMP-capable host compiler:

```sh
nvcc -O3 --fmad=false -Xcompiler=-fopenmp,-ffp-contract=off \
  examples/gpu_blur_bench.cu -o /tmp/img2irc-gpu-blur-bench
/tmp/img2irc-gpu-blur-bench
```

An illustrative run on an RTX 4090, CUDA 13.3, with up to four CPU threads:

| 24×40 patches | CPU | GPU, resident data | GPU, pixel transfers included |
| ---: | ---: | ---: | ---: |
| 1 | 0.0059 ms | 0.0513 ms | 0.0672 ms |
| 64 | 0.1757 ms | 0.0785 ms | 0.3174 ms |
| 2048 | 8.5252 ms | 0.3433 ms | 7.0001 ms |

Values are average time per batch over repeated iterations. Buffers are allocated
before measurement. Resident timing uses CUDA events; round-trip timing includes
input and output pixel transfers. CPU/GPU outputs matched bit for bit in this
run. The benchmark recomputes a complete blur, whereas the application's CPU
scorer also has incremental updates. These numbers are not end-to-end search
speedups.

A GPU implementation should keep source data and intermediate features resident,
batch many candidate windows, and return compact scores. This also suits dense
colour transforms, blur/gradient stages and independent video frames or render
trials. Sending individual tiny windows back and forth can erase the benefit;
NVIDIA's [transfer guidance](https://docs.nvidia.com/cuda/cuda-c-best-practices-guide/index.html#data-transfer-between-host-and-device)
recommends batching transfers and minimizing host/device traffic.

Preserving search decisions also requires controlling arithmetic order and
precision. Fused operations and parallel reductions can change floating-point
results, as NVIDIA's [floating-point guide](https://docs.nvidia.com/cuda/floating-point/index.html)
explains. A complete GPU scorer needs validated error bounds and CPU resolution
of ambiguous rankings/acceptances, or an arithmetic implementation proven to
match the existing scorer. A CPU check of only the GPU's selected winner would
not protect against an incorrectly discarded candidate.
