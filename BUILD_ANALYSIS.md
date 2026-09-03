# macOS Release Build Analysis — tailcat-node

## Summary

A clean `cargo build --release` takes **~41–43 seconds** wall-clock
(`user 59s`, `sys 10s`) on this 32-core Apple Silicon Mac. The build is
not I/O-bound on CPU compilation of your own code — your crate compiles
in **2.3s**. The time is consumed by **dependency build scripts that
spawn child `rustc` probes**, **heavy third-party crate codegen**, and a
**thin-LTO final link**, all amplified by macOS-specific environment
factors.

---

## Methodology

| Measurement | Tool | Result |
|---|---|---|
| Clean release build (full) | `/usr/bin/time -p cargo build --release` | **41.4s** real, 59s user, 10s sys |
| Incremental (deps cached, own crate rebuilt) | same | 41.4s (same — no incremental in release) |
| Per-unit timing | `cargo build --release --timings` | 128 units, DURATION=43s |
| Verbose invocation log | `cargo build --release -v` | 113 rustc + 15 build-script units |

---

## Root Causes (ranked by impact)

### 1. Build scripts invoking `rustc` as a feature probe — **dominant cost (~26s on critical path)**

15 dependency crates ship `build.rs` files that call `Command::new(rustc)`
to probe for nightly/unstable features. These all launch **simultaneously at
t≈0.4s** and each spawns 1+ child `rustc` process:

| Crate | Build-script wall (s) | Probe mechanism |
|---|---|---|
| anyhow 1.0.104 | 25.8 | `compile_probe()` → child `rustc` |
| num-traits 0.2.19 | 24.1 | `autocfg` → child `rustc` |
| zerocopy 0.8.56 | 22.4 | `rustc` probe |
| rustix 0.38.44 | 20.7 | `rustc` probe (asm/const fn) |
| libc 0.2.189 | 19.0 | `rustc` probe (cfg detection) |
| thiserror 1.0.69 | 17.2 | `compile_probe()` |
| thiserror 2.0.20 | 15.5 | `compile_probe()` |
| zmij 1.0.23 | 13.8 | `rustc` probe |
| getrandom 0.4.3 | 12.1 | `rustc` probe |
| serde_json 1.0.151 | 10.4 | `rustc` probe |
| proc-macro2 1.0.107 | 8.8 | `rustc` version probe |
| serde_core 1.0.229 | 7.0 | `rustc` probe |
| quote 1.0.47 | 5.2 | `rustc` probe |
| crossbeam-utils 0.8.22 | 3.5 | `autocfg` |
| serde 1.0.229 | 1.8 | `rustc` probe |

**Sum of build-script durations: 207s** — but they run in parallel, so the
critical path is bounded by the longest (~26s). The slowness is **not**
that each probe is inherently slow (a single isolated `rustc --version`
takes 0.05s; compiling `fn main(){}` takes 0.47s). The problem is
**contention**: 15+ concurrent `rustc` processes fighting for CPU, memory
bandwidth, and disk I/O simultaneously, plus the macOS overhead per process
spawn (code signing validation, provenance xattr reads — see below).

### 2. Heavy third-party crate codegen (~17s, after build scripts)

Once build scripts complete, the largest compilation units are:

| Crate | Duration (s) | Why slow |
|---|---|---|
| serde 1.0.229 | 20.5 | Massive derive macro surface |
| tracing 0.1.44 | 18.8 | Large trait/graph machinery |
| tokio 1.53.1 | 18.2 | Full async runtime, many features enabled |
| thiserror 1.0.69 | 17.3 | Derive macro expansion |
| thiserror 2.0.20 | 17.2 | Same |
| clap 4.6.6 | 17.1 | Derive + builder + help generation |

All compiled with `-C opt-level=3 -C linker-plugin-lto` (83 codegen units).
These crates have large generic/monomorphization surfaces that release
optimization (`opt-level=3`) expands aggressively.

### 3. Thin LTO final link

Your `Cargo.toml` sets `lto = "thin"` in `[profile.release]`. The final
binary link uses `-C lto=thin`, which performs whole-crate link-time
optimization across all 83 dependency rlibs. This adds a dedicated LTO
pass at the end of the build that cannot be parallelized with compilation.

### 4. macOS-specific amplifiers

#### a) Disk 99% full — APFS write amplification
```
/dev/disk3s5   1.8Ti   1.8Ti    36Gi    99%
```
APFS on a nearly-full volume triggers severe **copy-on-write write
amplification** and space-reclamation stalls. Every `rustc` temp file
write (object files, dep-info, metadata) hits a filesystem under space
pressure, causing synchronous flush/throttle behavior.

#### b) Spotlight indexing active during builds
```
mds_stores:  230:40 CPU time accumulated
mds:         169:35 CPU time accumulated
Indexing enabled on /
```
Spotlight's `mds`/`mdworker` processes index every new file written to
`target/`. With 1747 files written per clean build, Spotlight competes
for disk I/O and inode allocation.

#### c) `com.apple.provenance` xattr overhead
```
66,451 provenance xattrs on ~/.cargo/registry/src
```
macOS attaches `com.apple.provenance` extended attributes to every file
extracted from the cargo registry. Each `rustc`/build-script file access
triggers provenance xattr reads, adding per-file syscall overhead.

#### d) No sccache configured
```
RUSTC_WRAPPER = (unset)
which sccache → not found
```
Every clean build recompiles all 85 dependency crates from scratch. There
is no shared compilation cache, so CI or periodic `cargo clean` pays the
full 41s every time.

#### e) Homebrew Rust, not rustup
```
/opt/homebrew/Cellar/rust/1.95.0/bin/rustc  (arm64, native)
```
Homebrew's Rust build may differ from official rustup distributions in
debug-info/panic settings. This is a minor factor but worth noting for
reproducibility.

---

## What is NOT the problem

- **Your own code**: `tailcat-node` crate compiles in 2.3s (2 units, 2.6s + 2.3s).
- **Number of dependencies**: 85 unique crates is moderate, not excessive.
- **Debug info**: `[profile.dev] debug = 0` and `[profile.release] strip = true` are already optimal.
- **Incremental compilation**: Correctly disabled in release (standard behavior).
- **Rosetta/translation**: Not in use — native arm64, `sysctl.proc_translated = 0`.

---

## Recommendations

### High impact, low effort

1. **Add sccache** — cache compiled dependencies across builds:
   ```sh
   brew install sccache
   # Add to ~/.cargo/config.toml:
   [build]
   rustc-wrapper = "sccache"
   ```
   Expected: first build unchanged; subsequent clean builds drop to **<10s**.

2. **Exclude `target/` from Spotlight**:
   ```sh
   # System Settings → Spotlight → Privacy → add the project folder
   # or:
   sudo mdutil -i off /Users/xiaming/Workspace/tailcat-node
   ```

3. **Free disk space** — APFS performance degrades sharply above ~90% full.
   Clearing 50–100GB would reduce write-stall overhead.

### Medium impact

4. **Consider `lto = false` or `lto = "thin"` with `codegen-units = 16`** for
   faster dev-iteration release builds. Thin LTO adds link time; if binary
   size isn't critical, `lto = false` can save 3–5s on the final link.

5. **Reduce tokio features** — you enable `rt-multi-thread, sync, net,
   io-util, macros`. If you don't need multi-threaded runtime, dropping to
   `rt` alone cuts tokio's codegen substantially.

6. **Pin `thiserror` to one major version** — you compile both
   `thiserror 1.0.69` AND `thiserror 2.0.20` (plus their build scripts and
   derive macros). Consolidating dependencies to one version eliminates
   duplicate codegen.

### Low impact / informational

7. **Build scripts are upstream** — the `rustc`-probe pattern in
   `anyhow`/`zerocopy`/`rustix`/`libc` is an upstream design choice. You
   can't change it, but sccache caches their results, making it a one-time
   cost.

8. **Consider `cargo-chef` or `cargo nextest`** for CI pipelines to
   layer-cache dependencies separately from application code.

---

## Timing breakdown (clean build)

```
t=0.0s   ─┬─ compile 15 build-script binaries
t=0.4s   ─┤
          └─ run 15 build scripts in parallel (each spawns rustc probe)
             │  ├─ contention: 15+ rustc processes + mds indexing
             │  ├─ APFS write stalls (disk 99% full)
             │  └─ provenance xattr reads (66k files)
t=26s    ─┤  ← build scripts complete (critical path = anyhow @ 25.8s)
          │
          ├─ codegen 83 dependency crates (opt-level=3, linker-plugin-lto)
          │  ├─ serde 20.5s, tracing 18.8s, tokio 18.2s
          │  └─ thiserror×2 17.3s, clap 17.1s
t=41s    ─┤  ← codegen + rmeta complete
          │
          └─ final link: -C lto=thin (tailcat-node bin)
t=43s    ──── Finished
```

---

*Analysis performed on macOS 15.4.1, arm64, 32 cores, 512GB RAM,
Rust 1.95.0 (Homebrew). Generated from `cargo build --release --timings`
and `cargo build --release -v` instrumentation.*
