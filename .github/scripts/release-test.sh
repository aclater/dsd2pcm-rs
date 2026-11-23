#!/bin/bash
# The local parts of the release test plan (docs/release-test-plan.md), one item per call,
# from the repository root of the release candidate, on Linux x86_64 with AVX2 and FMA:
#
#     .github/scripts/release-test.sh ITEM [ARGS...]
#
#   simd                  1, 2: the workspace's tests at every x86_64 SIMD level
#   i686                  1, 2: the same for i686-unknown-linux-gnu, in a Fedora container
#   aarch64               1, 2: the same for aarch64-unknown-linux-gnu, in a Fedora
#                         container, run under qemu-user
#   fuzz SECONDS          3: every fuzz target for SECONDS, all targets at once, one process each
#   soak SECONDS          4: the leak soak (tests/soak.rs) for SECONDS
#   miri                  5: Miri on the C interface and the core crate
#   corpus LIST WORKDIR   6: the real-file corpus (LIST: one input path per line)
#   capi                  7: the C interface under ASan, LSan and UBSan
#   quickstart            8: the README's Quick start in a fresh Fedora container, with the
#                         tool built from this tree in place of the download
#   docs                  9: the docs checks and the docs' commands
#   safety                10: the file-safety regression checks
#
# Every item prints PASS or FAIL as its last line and exits non-zero on a failure. Nothing
# here takes a benchmark lock: no item is timed. Each heavy step runs under
# `systemd-run --user --scope -p MemoryMax=...` (RTP_MEM, default 8G) at nice 19, with
# RTP_JOBS build jobs and CPUs (default 8). Containers use FEDORA_IMAGE (default
# registry.fedoraproject.org/fedora:45) and the host's rustup and cargo homes.
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
item=${1:-}
shift || true
mem=${RTP_MEM:-8G}
jobs=${RTP_JOBS:-8}
image=${FEDORA_IMAGE:-registry.fedoraproject.org/fedora:45}
nightly=${RTP_NIGHTLY:-nightly-2026-11-01}
export CARGO_BUILD_JOBS=$jobs RUST_TEST_THREADS=$jobs
rustup_home=${RUSTUP_HOME:-$HOME/.rustup}
cargo_home=${CARGO_HOME:-$HOME/.cargo}

# Memory and CPU capped (the CPU quota also sets the threads the library and the test
# harness start: Rust's available_parallelism follows the cgroup's quota).
capped() {
    nice -n 19 systemd-run --user --scope -q -p MemoryMax="$mem" -p MemorySwapMax=0 \
        -p CPUQuota="$((jobs * 100))%" "$@"
}
verdict() { if [[ $1 == 0 ]]; then echo "PASS $item"; else echo "FAIL $item"; fi; exit "$1"; }

# A command in a fresh Fedora container with the tree at /src, the host's toolchains, and
# PACKAGES installed first.
in_container() {
    local packages=$1
    shift
    capped podman run --rm --security-opt label=disable --cpus "$jobs" \
        -v "$root:/src" -v "$rustup_home:/rustup" -v "$cargo_home:/cargo" \
        -e RUSTUP_HOME=/rustup -e CARGO_HOME=/cargo -e PATH=/cargo/bin:/usr/bin:/bin \
        -e CARGO_BUILD_JOBS="$jobs" -e RUST_TEST_THREADS="$jobs" -e CARGO_TARGET_DIR="/src/target/rtp-$item" -w /src "$image" \
        bash -euo pipefail -c "for try in 1 2 3; do dnf -y -q --setopt=install_weak_deps=False install $packages > /tmp/dnf.log 2>&1 && break; cat /tmp/dnf.log; [[ \$try == 3 ]] && exit 1; sleep 30; done; $*"
}

case "$item" in
simd)
    rc=0
    CARGO_TARGET_DIR=target/rtp-simd capped .github/scripts/test-simd-levels.sh || rc=1
    verdict $rc
    ;;
i686)
    rc=0
    in_container "gcc glibc-devel.i686 libgcc.i686 libatomic.i686 file" \
        '.github/scripts/test-simd-levels.sh --target i686-unknown-linux-gnu &&
         cargo build --locked -p dsd2pcm-cli --target i686-unknown-linux-gnu &&
         file "$CARGO_TARGET_DIR/i686-unknown-linux-gnu/debug/dsd2pcm" | grep "ELF 32-bit.*Intel"' || rc=1
    verdict $rc
    ;;
aarch64)
    rc=0
    sysroot=/usr/aarch64-redhat-linux/sys-root/fc45
    in_container "gcc gcc-aarch64-linux-gnu sysroot-aarch64-fc45-glibc qemu-user-static-aarch64 file cpio python3" \
        "# The sysroot has glibc only: add aarch64 libgcc_s, which Rust's std links.
         dnf -q download --forcearch=aarch64 --destdir /tmp/rpm libgcc
         (cd $sysroot && rpm2cpio /tmp/rpm/libgcc-*.aarch64.rpm | cpio -idm --quiet)
         cp -a $sysroot/lib64/libgcc_s* $sysroot/usr/lib64/
         ln -sf libgcc_s.so.1 $sysroot/usr/lib64/libgcc_s.so
         # The loader's path, /lib/ld-linux-aarch64.so.1, for qemu -L.
         ln -sfn usr/lib $sysroot/lib
         # A linker wrapper with the sysroot, so doctests (built by rustdoc, which gets no
         # target RUSTFLAGS) link too.
         printf '#!/bin/sh\nexec aarch64-linux-gnu-gcc --sysroot=$sysroot \"\$@\"\n' > /usr/local/bin/aarch64-cc
         chmod +x /usr/local/bin/aarch64-cc
         export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=/usr/local/bin/aarch64-cc
         export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUNNER='qemu-aarch64-static -L $sysroot'
         # Under emulation, optimised, with the debug build's checks kept.
         export CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true CARGO_PROFILE_RELEASE_OVERFLOW_CHECKS=true
         # The CLI's signal test spawns the tool, which a container without binfmt_misc
         # cannot exec under qemu-user: skipped here (native aarch64 CI runs it) and
         # replaced by the same check run through qemu below.
         .github/scripts/test-simd-levels.sh --target aarch64-unknown-linux-gnu --release -- --skip sigint_and_sigterm_leave_no_partial_output &&
         cargo build --locked -p dsd2pcm-cli --target aarch64-unknown-linux-gnu &&
         file \"\$CARGO_TARGET_DIR/aarch64-unknown-linux-gnu/debug/dsd2pcm\" | grep 'ELF 64-bit.*ARM aarch64' &&
         cargo build --locked --release -p dsd2pcm-cli --target aarch64-unknown-linux-gnu &&
         tool=\"qemu-aarch64-static -L $sysroot \$CARGO_TARGET_DIR/aarch64-unknown-linux-gnu/release/dsd2pcm\" &&
         d=\$(mktemp -d) && python3 -I .github/scripts/make-test-dsf.py --music 120 6 \$d/in.dsf &&
         for sig in INT TERM; do
             printf precious > \$d/out.wav
             env --default-signal=INT \$tool \$d/in.dsf \$d/out.wav --rate 352800 --single-thread 2> \$d/err & pid=\$!
             sleep 3; kill -0 \$pid; kill -\$sig \$pid; st=0; wait \$pid || st=\$?
             echo \"aarch64 under qemu, SIG\$sig: exit \$st: \$(cat \$d/err); left: \$(ls -A \$d | tr '\\n' ' ')\"
             [[ \$st == \$([[ \$sig == INT ]] && echo 130 || echo 143) && \$(cat \$d/out.wav) == precious && \$(ls -A \$d | wc -l) == 3 ]]
         done" || rc=1
    verdict $rc
    ;;
fuzz)
    seconds=${1:?fuzz SECONDS}
    cd fuzz
    export RUSTUP_TOOLCHAIN=$nightly CARGO_TARGET_DIR=$root/target/rtp-fuzz
    capped cargo fuzz build -O
    capped cargo run --release --locked --bin make_seeds
    mapfile -t targets < <(cargo fuzz list | while read -r t; do [[ -f fuzz_targets/$t.rs ]] && echo "$t"; done)
    mkdir -p "$root/target/rtp-fuzz-logs"
    pids=()
    for t in "${targets[@]}"; do
        mkdir -p "corpus/$t"
        work=$(mktemp -d "$root/target/rtp-fuzz-corpus-$t.XXXX")
        cp -r "corpus/$t/." "$work/"
        # One process per target (no -fork), each under its own memory cap.
        RTP_MEM=3G capped cargo fuzz run -O "$t" "$work" -- -max_total_time="$seconds" \
            -max_len=32768 -rss_limit_mb=2048 -malloc_limit_mb=2048 -timeout=10 \
            -print_final_stats=1 > "$root/target/rtp-fuzz-logs/$t.log" 2>&1 &
        pids+=("$!")
    done
    rc=0
    for i in "${!targets[@]}"; do
        t=${targets[$i]}
        if wait "${pids[$i]}"; then
            echo "$t: no failures in $seconds s, $(grep -m1 -o 'stat::number_of_executed_units: [0-9]*' "$root/target/rtp-fuzz-logs/$t.log")"
        else
            echo "$t: FAILED (log target/rtp-fuzz-logs/$t.log, input in fuzz/artifacts/$t/)"
            rc=1
        fi
    done
    verdict $rc
    ;;
soak)
    seconds=${1:?soak SECONDS}
    rc=0
    DSD2PCM_SOAK_SECONDS=$seconds CARGO_TARGET_DIR=target/rtp-soak capped \
        cargo test --release --locked -p dsd2pcm --test soak -- --ignored --nocapture || rc=1
    verdict $rc
    ;;
miri)
    rc=0
    export RUSTUP_TOOLCHAIN=$nightly CARGO_TARGET_DIR=target/rtp-miri
    capped cargo miri test --locked -p dsd2pcm-capi || rc=1
    capped .github/scripts/miri-core.sh || rc=1
    verdict $rc
    ;;
capi)
    rc=0
    export CARGO_TARGET_DIR=target/rtp-capi
    capped env CBINDGEN=cbindgen LEAKLOOP_ITERATIONS=1000 crates/dsd2pcm-capi/check.sh || rc=1
    capped env ASAN_CC=clang LEAKLOOP_ITERATIONS=1000 .github/scripts/capi-check.sh crates/dsd2pcm-capi || rc=1
    verdict $rc
    ;;
corpus)
    list=${1:?corpus LIST WORKDIR}
    work=${2:?corpus LIST WORKDIR}
    mkdir -p "$work"
    export CARGO_TARGET_DIR=$root/target/rtp-corpus
    capped cargo build --release --locked -p dsd2pcm-cli
    capped cargo build --release --locked -p dsd2pcm --example real_identity
    tool=$CARGO_TARGET_DIR/release/dsd2pcm
    ident=$CARGO_TARGET_DIR/release/examples/real_identity
    rc=0
    # The checker must catch a difference: one flipped byte in a correct output fails it.
    first=$(grep -v '^#' "$list" | sed -n 1p)
    capped "$tool" "$first" "$work/out.wav" --rate 88200 2>/dev/null
    capped "$ident" "$first" "$work/out.wav" 88200 40 > /dev/null || { echo "FAIL checker on a correct output"; rc=1; }
    python3 -I -c 'import sys; f=open(sys.argv[1],"r+b"); f.seek(-100000,2); b=f.read(1); f.seek(-100000,2); f.write(bytes([b[0]^1]))' "$work/out.wav"
    if capped "$ident" "$first" "$work/out.wav" 88200 40 > "$work/selftest.txt"; then
        echo "FAIL the checker missed a flipped byte"; rc=1
    else
        echo "checker self-test: a flipped byte is caught ($(cut -c1-120 "$work/selftest.txt"))"
    fi
    settings=("88200 30" "88200 40" "176400 30" "176400 40" "176400 wide" "352800 30" "352800 40" "352800 wide")
    while IFS= read -r f; do
        [[ -z "$f" || "$f" == \#* ]] && continue
        echo "== $f"
        out=$work/out.wav
        for s in "${settings[@]}"; do
            read -r r c <<< "$s"
            rm -f "$out"
            if ! capped "$tool" "$f" "$out" --rate "$r" --cutoff "$c" 2> "$work/cli.log"; then
                # The one refusal the docs promise: WAV output over 4 GiB.
                if grep -q -i '4 GiB\|too large\|too long' "$work/cli.log" && [[ ! -e "$out" ]]; then
                    echo "refused (over 4 GiB), no output: $r $c: $(tail -n 1 "$work/cli.log")"
                    continue
                fi
                echo "FAIL convert $r $c: $(cat "$work/cli.log")"
                rc=1
                continue
            fi
            line=$(capped "$ident" "$f" "$out" "$r" "$c") || rc=1
            echo "$r $c: $line"
        done
        # pysacd's filter: a valid WAV of the right length, the same threaded and not.
        for r in 88200 176400; do
            rm -f "$out" "$work/out1.wav"
            if capped "$tool" "$f" "$out" --rate "$r" --filter pysacd 2> "$work/cli.log" &&
                capped "$tool" "$f" "$work/out1.wav" --rate "$r" --filter pysacd --single-thread 2>> "$work/cli.log" &&
                cmp -s "$out" "$work/out1.wav" &&
                python3 -I - "$out" "$r" <<'EOF'
import struct, sys
d = open(sys.argv[1], "rb").read(4096)
assert d[:4] == b"RIFF" and d[8:12] == b"WAVE"
EOF
            then
                echo "pysacd $r: valid WAV, threaded = single-thread ($(stat -c %s "$out") bytes)"
            else
                echo "FAIL pysacd $r: $(cat "$work/cli.log")"
                rc=1
            fi
        done
        # The default filter threaded and on one thread: the same bytes.
        rm -f "$out" "$work/out1.wav"
        if capped "$tool" "$f" "$out" 2>/dev/null && capped "$tool" "$f" "$work/out1.wav" --single-thread 2>/dev/null &&
            cmp -s "$out" "$work/out1.wav"; then
            echo "default 176400 40: threaded = single-thread"
        else
            echo "FAIL default 176400 40: threaded and single-thread differ"
            rc=1
        fi
        rm -f "$out" "$work/out1.wav"
    done < "$list"
    # The same audio in DSF and DFF (RTP_PAIRS: lines "A B"): the same WAV, byte for byte.
    if [[ -n "${RTP_PAIRS:-}" ]]; then
        while read -r a b; do
            [[ -z "$a" || "$a" == \#* ]] && continue
            rm -f "$work/a.wav" "$work/b.wav"
            if capped "$tool" "$a" "$work/a.wav" 2>/dev/null && capped "$tool" "$b" "$work/b.wav" 2>/dev/null &&
                cmp -s "$work/a.wav" "$work/b.wav"; then
                echo "same WAV: $a = $b"
            else
                echo "FAIL different WAV: $a, $b"
                rc=1
            fi
        done < "$RTP_PAIRS"
        rm -f "$work/a.wav" "$work/b.wav"
    fi
    verdict $rc
    ;;
quickstart)
    rc=0
    in_container "gcc python3 gawk git-core" \
        'cargo build --release --locked -p dsd2pcm-cli
         block=$(awk "/^## /{on=(\$0==\"## Quick start\");next} on&&!b&&/^\`\`\`/{b=1;next} on&&b&&/^\`\`\`/{exit} on&&b{print}" README.md)
         echo "$block"
         ver=$(sed -n "s/^version = \"\(.*\)\"/\1/p" Cargo.toml | sed -n 1p)
         name=dsd2pcm-$ver-x86_64-linux
         grep -q "$name.tar.gz" <<< "$block"
         # The archive the release will hold, from this tree, in place of the download.
         stage=$(mktemp -d); mkdir "$stage/$name"
         cp "$CARGO_TARGET_DIR/release/dsd2pcm" README.md LICENSE-MIT LICENSE-APACHE "$stage/$name/"
         run=$(mktemp -d); tar -C "$stage" -czf "$run/$name.tar.gz" "$name"
         python3 -I .github/scripts/make-test-dsf.py "$run/01 - Track.dsf"
         cd "$run"
         grep -v "^curl " <<< "$block" | bash -e -x
         python3 -I -c "import struct,sys; d=open(\"01 - Track.wav\",\"rb\").read(64); print(d[:4], d[8:12]); i=d.index(b\"fmt \"); ch,sr=struct.unpack_from(\"<HI\",d,i+10); bits=struct.unpack_from(\"<H\",d,i+22)[0]; print(ch,sr,bits); sys.exit(0 if (sr,bits)==(176400,24) else 1)"' || rc=1
    verdict $rc
    ;;
docs)
    rc=0
    python3 -I .github/scripts/check-doc-versions.py || rc=1
    .github/scripts/readme-quick-start.sh || rc=1
    lychee=${LYCHEE:-lychee}
    "$lychee" --no-progress --offline --config .github/lychee.toml \
        README.md CHANGELOG.md SECURITY.md PROVENANCE.md AGENTS.md 'docs/**/*.md' || rc=1
    verdict $rc
    ;;
safety)
    export CARGO_TARGET_DIR=$root/target/rtp-safety
    capped cargo build --release --locked -p dsd2pcm-cli
    tool=$CARGO_TARGET_DIR/release/dsd2pcm
    d=$(mktemp -d)
    trap 'rm -rf "$d"' EXIT
    rc=0
    check() { if eval "$2"; then echo "ok   $1"; else echo "FAIL $1"; rc=1; fi; }
    python3 -I .github/scripts/make-test-dsf.py "$d/in.dsf"
    printf 'precious' > "$d/keep.wav"
    # 1. Output is a hard link to the input: refused, input unchanged.
    ln "$d/in.dsf" "$d/link.wav"
    sum=$(sha256sum < "$d/in.dsf")
    check "hard link to the input refused (exit 1)" '"$tool" "$d/in.dsf" "$d/link.wav" 2>/dev/null; [[ $? -eq 1 ]]'
    check "input unchanged after the hard-link refusal" '[[ "$(sha256sum < "$d/in.dsf")" == "$sum" ]]'
    check "same path refused" '! "$tool" "$d/in.dsf" "$d/in.dsf" 2>/dev/null'
    ln -s in.dsf "$d/sym.wav"
    check "symbolic link to the input refused" '! "$tool" "$d/in.dsf" "$d/sym.wav" 2>/dev/null'
    check "input unchanged" '[[ "$(sha256sum < "$d/in.dsf")" == "$sum" ]]'
    # 2. Failures leave an existing output untouched.
    printf 'not DSD at all' > "$d/bad.dsf"
    check "bad input fails, exit 1" '"$tool" "$d/bad.dsf" "$d/keep.wav" 2>/dev/null; [[ $? -eq 1 ]]'
    check "missing input fails" '! "$tool" "$d/missing.dsf" "$d/keep.wav" 2>/dev/null'
    check "gain out of range fails" '! "$tool" "$d/in.dsf" "$d/keep.wav" --gain 99 2>/dev/null'
    check "unknown option, exit 2" '"$tool" "$d/in.dsf" "$d/keep.wav" --bogus 2>/dev/null; [[ $? -eq 2 ]]'
    check "wide at 88.2 kHz refused" '! "$tool" "$d/in.dsf" "$d/keep.wav" --rate 88200 --cutoff wide 2>/dev/null'
    check "existing output untouched by every failure" 'cmp -s "$d/keep.wav" <(printf precious)'
    check "no temporary file left" '[[ -z "$(find "$d" -name ".*.tmp")" ]]'
    # 3. An interrupted conversion leaves no partial WAV at the output path.
    big=$d/big.dsf
    python3 -I - "$big" <<'EOF'
import struct, sys
# 6 channels, 300 s of a busy pattern: long enough to interrupt.
ch, n = 6, 2822400 // 8 * 300
groups = -(-n // 4096)
with open(sys.argv[1], "wb") as o:
    data = groups * 4096 * ch
    o.write(b"DSD " + struct.pack("<QQQ", 28, 92 + data, 0))
    o.write(b"fmt " + struct.pack("<QIIIIIIQII", 52, 1, 0, 7, ch, 2822400, 1, n * 8, 4096, 0))
    o.write(b"data" + struct.pack("<Q", 12 + data))
    block = bytes((i * 37 + 11) & 0xFF for i in range(4096))
    for _ in range(groups * ch):
        o.write(block)
EOF
    # A background job of a script starts with SIGINT ignored: give the tool the default.
    interrupt() { # SIGNAL OUTPUT [SECOND]: start a conversion, signal it after 1 s, report
        env --default-signal=INT "$tool" "$big" "$2" --rate 352800 --single-thread 2> "$d/sig.log" &
        local pid=$!
        sleep 1
        alive=no; kill -0 "$pid" 2>/dev/null && alive=yes
        kill "-$1" "$pid" 2>/dev/null || true
        [[ -n "${3:-}" ]] && kill "-$1" "$pid" 2>/dev/null
        local t0=$(date +%s%N)
        wait "$pid" && st=0 || st=$?
        waited=$(( ($(date +%s%N) - t0) / 1000000 ))
        echo "$1: running when signalled: $alive; exit $st after $waited ms: $(tail -n 1 "$d/sig.log")"
    }
    interrupt INT "$d/keep.wav"
    check "SIGINT while running: exit 130, existing output untouched" '[[ $alive == yes && $st -eq 130 ]] && cmp -s "$d/keep.wav" <(printf precious)'
    check "SIGINT: \"interrupted; no output written\"" 'grep -q "interrupted; no output written" "$d/sig.log"'
    interrupt TERM "$d/new.wav"
    check "SIGTERM while running: exit 143, no output file" '[[ $alive == yes && $st -eq 143 && ! -e "$d/new.wav" ]]'
    left=$(find "$d" -name '.*.tmp' -printf '%f ')
    echo "temporary files left by the interrupted conversions: ${left:-none}"
    check "SIGINT and SIGTERM leave no temporary file" '[[ -z "$left" ]]'
    # A second signal ends the process at once, like SIGKILL (documented).
    interrupt INT "$d/new2.wav" twice
    left=$(find "$d" -name '.*.tmp' -printf '%f ')
    echo "after two SIGINTs: ${left:-no temporary file}"
    check "two SIGINTs: exit 130 at once, no output file; any leftover is .dsd2pcm-PID-N.tmp" '[[ $alive == yes && $st -eq 130 && $waited -lt 2000 && ! -e "$d/new2.wav" ]] && [[ -z "$left" || "$left" =~ ^\.dsd2pcm-[0-9]+-[0-9]+\.tmp\ $ ]]'
    find "$d" -name '.*.tmp' -delete
    # SIGKILL cannot be caught: at most a hidden .dsd2pcm-PID-N.tmp remains (documented).
    "$tool" "$big" "$d/killed.wav" --rate 352800 --single-thread 2>/dev/null &
    pid=$!
    sleep 1
    kill -KILL "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
    left=$(find "$d" -name '.*.tmp' -printf '%f ')
    echo "after SIGKILL: ${left:-no temporary file}"
    check "SIGKILL: no output file; any leftover is named .dsd2pcm-PID-N.tmp" '[[ ! -e "$d/killed.wav" ]] && [[ -z "$left" || "$left" =~ ^\.dsd2pcm-[0-9]+-[0-9]+\.tmp\ $ ]]'
    find "$d" -name '.*.tmp' -delete
    # 3b. Output targets that are not regular writable files are refused up front.
    mkdir "$d/sub"
    check "directory as output refused" '! "$tool" "$d/in.dsf" "$d/sub" 2>/dev/null && [[ -d "$d/sub" && -z "$(ls -A "$d/sub")" ]]'
    check "path ending in / refused" '! "$tool" "$d/in.dsf" "$d/nothere/" 2>/dev/null && [[ ! -e "$d/nothere" ]]'
    mkfifo "$d/fifo.wav"
    check "FIFO refused (without blocking)" '! timeout 10 "$tool" "$d/in.dsf" "$d/fifo.wav" 2>/dev/null && [[ -p "$d/fifo.wav" ]]'
    check "device (/dev/null) refused" '! "$tool" "$d/in.dsf" /dev/null 2>/dev/null'
    python3 -I -c 'import socket,sys; s=socket.socket(socket.AF_UNIX); s.bind(sys.argv[1])' "$d/sock.wav"
    check "socket refused" '! "$tool" "$d/in.dsf" "$d/sock.wav" 2>/dev/null && [[ -S "$d/sock.wav" ]]'
    printf 'readonly' > "$d/ro.wav"; chmod 444 "$d/ro.wav"
    check "unwritable existing output refused, unchanged" '! "$tool" "$d/in.dsf" "$d/ro.wav" 2>/dev/null && cmp -s "$d/ro.wav" <(printf readonly)'
    mkdir "$d/rodir"; chmod 555 "$d/rodir"
    check "unwritable directory refused, nothing created" '! "$tool" "$d/in.dsf" "$d/rodir/out.wav" 2>/dev/null && [[ -z "$(ls -A "$d/rodir")" ]]'
    chmod 755 "$d/rodir"
    printf 'old' > "$d/mode.wav"; chmod 640 "$d/mode.wav"
    check "replacing an output keeps its permissions (0640)" '"$tool" "$d/in.dsf" "$d/mode.wav" 2>/dev/null && [[ $(stat -c %a "$d/mode.wav") == 640 ]] && [[ $(head -c 4 "$d/mode.wav") == RIFF ]]'
    long=$(printf 'L%.0s' $(seq 1 251)).wav
    check "an output name of 255 bytes works" '"$tool" "$d/in.dsf" "$d/$long" 2>/dev/null && [[ $(head -c 4 "$d/$long") == RIFF ]]'
    check "no temporary file left by the path checks" '[[ -z "$(find "$d" -name ".*.tmp")" ]]'
    CARGO_TARGET_DIR=$root/target/rtp-safety capped cargo test --release --locked -p dsd2pcm --test output --test chunk_walk || rc=1
    CARGO_TARGET_DIR=$root/target/rtp-safety capped cargo test --release --locked -p dsd2pcm-cli || rc=1
    # 4. Crafted many-chunk files are refused fast (DFF with 100 000 empty chunks before PROP).
    python3 -I - "$d/many.dff" <<'EOF'
import struct, sys
ck = lambda i, b: i + struct.pack(">Q", len(b)) + b
body = b"".join(ck(b"JUNK", b"") for _ in range(100000))
body += ck(b"FVER", struct.pack(">I", 0x01050000))
body += ck(b"PROP", b"SND " + ck(b"FS  ", struct.pack(">I", 2822400)) + ck(b"CHNL", struct.pack(">H", 2) + b"SLFTSRGT") + ck(b"CMPR", b"DSD \x0enot compressed\x00"))
body += ck(b"DSD ", bytes([0x69]) * 9408)
open(sys.argv[1], "wb").write(ck(b"FRM8", b"DSD " + body))
EOF
    # The cap: 4096 top-level chunks up to and including the audio chunk open; 4097 do not.
    for junk in 4093 4094; do
        python3 -I - "$d/cap$junk.dff" "$junk" <<'EOF'
import struct, sys
ck = lambda i, b: i + struct.pack(">Q", len(b)) + b
body = b"".join(ck(b"JUNK", b"") for _ in range(int(sys.argv[2])))
body += ck(b"FVER", struct.pack(">I", 0x01050000))
body += ck(b"PROP", b"SND " + ck(b"FS  ", struct.pack(">I", 2822400)) + ck(b"CHNL", struct.pack(">H", 2) + b"SLFTSRGT") + ck(b"CMPR", b"DSD \x0enot compressed\x00"))
body += ck(b"DSD ", bytes([0x69]) * 9408)
open(sys.argv[1], "wb").write(ck(b"FRM8", b"DSD " + body))
EOF
    done
    check "DFF with 4096 chunks (audio included) converts" '"$tool" "$d/cap4093.dff" "$d/cap.wav" 2>/dev/null'
    check "DFF with 4097 chunks refused" '! "$tool" "$d/cap4094.dff" "$d/cap.wav" 2>/dev/null'
    start=$(date +%s%N)
    "$tool" "$d/many.dff" "$d/keep.wav" 2> "$d/many.log" && st=0 || st=$?
    ms=$(( ($(date +%s%N) - start) / 1000000 ))
    echo "many-chunk DFF: exit $st in $ms ms: $(cat "$d/many.log")"
    check "many-chunk DFF refused (exit 1) within 2 s" '[[ $st -eq 1 && $ms -lt 2000 ]]'
    check "existing output untouched" 'cmp -s "$d/keep.wav" <(printf precious)'
    verdict $rc
    ;;
*)
    sed -n '2,/^set -euo/p' "$0" | sed '$d'
    exit 2
    ;;
esac
