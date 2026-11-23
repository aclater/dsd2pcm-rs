"""Summarise the rounds of .github/workflows/bench.yml as a markdown table.

    python3 -I .github/scripts/bench-summary.py LOGDIR TITLE

LOGDIR holds bench-CH-ROUND.log (examples/bench.rs) and path-CH-ROUND.log
(examples/sacdfs_path.rs, all modes) for CH in 2ch and 6ch. Per row: the median speed of
each crate over the rounds (x real time, one thread), the median of the per-round ratios
dsd2pcm / scarletbook-pcm 0.7.0, and their lowest and highest. Standard library only.
"""

import pathlib
import statistics
import sys

CHANNELS = ("2ch", "6ch")


def parse(path, kind):
    rows = {}
    for line in path.read_text().splitlines():
        f = line.split()
        if not f or not f[0].isdigit():
            continue
        if kind == "bench":  # rate cutoff sb ours-1 ours-threads
            key = (int(f[0]), f[1])
            sb, ours = float(f[2]), float(f[3])
        else:  # rate cutoff mode... sb ours ratio; mode is "run N" or "area"
            mode = "area" if f[2] == "area" else f"run {f[3]}"
            key = (int(f[0]), f[1], mode)
            sb, ours = float(f[-3]), float(f[-2])
        rows[key] = (sb, ours)
    return rows


def collect(logdir, kind):
    data = {}  # key -> ch -> list of (sb, ours)
    for ch in CHANNELS:
        for p in sorted(logdir.glob(f"{kind}-{ch}-*.log")):
            for key, v in parse(p, kind).items():
                data.setdefault(key, {}).setdefault(ch, []).append(v)
    return data


def cell(vals):
    if not vals:
        return "", "", ""
    sb = statistics.median(v[0] for v in vals)
    ours = statistics.median(v[1] for v in vals)
    ratios = [v[1] / v[0] for v in vals]
    spread = f"{min(ratios):.2f}–{max(ratios):.2f}"
    return f"{sb:.1f} / {ours:.1f}", f"**{statistics.median(ratios):.2f}x**", spread


def rate(hz):
    return f"{hz / 1000:g} kHz"


def main():
    logdir = pathlib.Path(sys.argv[1])
    title = sys.argv[2]
    out = [f"### {title}", ""]
    meta = logdir / "runner.txt"
    if meta.exists():
        out += ["```", meta.read_text().strip(), "```", ""]
    rounds = len(list(logdir.glob("bench-2ch-*.log")))
    out += [
        f"GitHub-hosted runner, indicative. {rounds} rounds, crate order alternating; "
        "speed in x real time, one thread (median, scarletbook-pcm 0.7.0 / dsd2pcm); "
        "ratio = dsd2pcm / scarletbook-pcm, median and lowest–highest over the rounds. "
        "Every round checked that both crates give the same bytes.",
        "",
        "Whole file (`examples/bench.rs`, DSF to 24-bit WAV in memory):",
        "",
        "| Rate | Cutoff | 2 ch speed | 2 ch ratio | 2 ch spread "
        "| 6 ch speed | 6 ch ratio | 6 ch spread |",
        "|---|---|---|---|---|---|---|---|",
    ]
    for key, by_ch in collect(logdir, "bench").items():
        cells = [c for ch in CHANNELS for c in cell(by_ch.get(ch, []))]
        out.append(f"| {rate(key[0])} | {key[1]} | " + " | ".join(cells) + " |")
    out += [
        "",
        "Frame runs (`examples/sacdfs_path.rs ... all`):",
        "",
        "| Rate | Cutoff | Mode | 2 ch speed | 2 ch ratio | 2 ch spread "
        "| 6 ch speed | 6 ch ratio | 6 ch spread |",
        "|---|---|---|---|---|---|---|---|---|",
    ]
    for key, by_ch in collect(logdir, "path").items():
        cells = [c for ch in CHANNELS for c in cell(by_ch.get(ch, []))]
        out.append(f"| {rate(key[0])} | {key[1]} | {key[2]} | " + " | ".join(cells) + " |")
    print("\n".join(out))


if __name__ == "__main__":
    main()
