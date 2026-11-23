#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Generates the test signals of signals.toml: PCM test files (WAV) and DSD files (DSF).

    python gen_signals.py --out OUT [--only ID,ID] [--jobs N] [--sox PATH] [--no-sox]

Writes, for each signal and source rate:
    OUT/pcm/<id>-p88.wav           the PCM test file (24-bit; pcm sources only)
    OUT/dsd/<id>-<src>-dsd64-ref.dsf  the reference modulator's DSD (src: p88, p96, dsd, bits)
    OUT/dsd/<id>-<src>-dsd64-sox.dsf  SoX-DSD's DSD (signals with modulators = ["ref", "sox"])
and OUT/signals.json (the resolved definitions: tones, levels, segment times) for the
analysers. Everything is deterministic; existing outputs are kept unless --force.
"""

import argparse
import json
import math
import os
import subprocess
import sys
import tomllib
from concurrent.futures import ProcessPoolExecutor

import numpy as np

import dsdio
import modulator

HERE = os.path.dirname(os.path.abspath(__file__))
DSD64 = 2_822_400
FRAME = 1 / 75


def amp(dbfs):
    return 10 ** (dbfs / 20)


def fnv(s):
    h = 2166136261
    for c in s.encode():
        h = ((h ^ c) * 16777619) & 0xFFFFFFFF
    return h


class Resolver:
    """Turns components into sample arrays at a rate, and records what was generated."""

    def __init__(self, settings):
        self.bin = settings["coherent_bin_hz"]

    def coh(self, f):
        return round(f / self.bin) * self.bin

    def render(self, comps, t, fs, info):
        """Sum of components at times t (seconds from the start of the steady part)."""
        x = np.zeros_like(t)
        for c in comps:
            if "sine" in c:
                f = self.coh(c["sine"])
                x += amp(c["dbfs"]) * np.sin(2 * math.pi * f * t)
                info.setdefault("tones", []).append({"hz": f, "dbfs": c["dbfs"]})
            elif "tones" in c:
                fl = [self.coh(f) for f in c["tones"] if f < 0.45 * fs]
                n = len(fl)
                for k, f in enumerate(fl, 1):
                    x += amp(c["dbfs"]) * np.sin(2 * math.pi * f * t + math.pi * k * (k - 1) / n)
                info["tones"] = [{"hz": f, "dbfs": c["dbfs"]} for f in fl]
            elif "multitone" in c:
                n = c["multitone"]
                fl = [self.coh(c["low"] * (c["high"] / c["low"]) ** (k / (n - 1))) for k in range(n)]
                # Newman phases (pi k^2 / n): a low crest factor, deterministic.
                y = np.zeros_like(t)
                for k, f in enumerate(fl):
                    y += np.sin(2 * math.pi * f * t + math.pi * k * k / n)
                # Scale on a long, fine grid so the peak does not depend on the rate.
                tt = np.arange(0, 2.0, 1 / 2_822_400)
                yy = np.zeros_like(tt)
                for k, f in enumerate(fl):
                    yy += np.sin(2 * math.pi * f * tt + math.pi * k * k / n)
                peak = np.max(np.abs(yy))
                crest = 20 * math.log10(peak / math.sqrt(np.mean(yy**2)))
                g = amp(c["peak_dbfs"]) / peak
                x += g * y
                info["tones"] = [{"hz": f, "dbfs": 20 * math.log10(g)} for f in fl]
                info["crest_db"] = crest
            elif "jtest" in c:
                # fs/4 tone at the given level (samples at +-45 degrees), and a square wave of
                # one 24-bit LSB at fs/192, added in integers by the caller (pcm only).
                n = np.round(t * fs).astype(np.int64)
                x += amp(c["jtest"]) * np.sin(math.pi * n / 2 + math.pi / 4)
                info["tones"] = [{"hz": fs / 4, "dbfs": c["jtest"]}]
                info["jtest_lsb_square_hz"] = fs / 192
            elif "impulse" in c:
                n = np.round(t * fs).astype(np.int64)
                x += np.where(n == round(c["impulse"] * fs), amp(c["dbfs"]), 0.0)
                info["impulse_at"] = round(c["impulse"] * fs) / fs
            elif "step" in c:
                x += np.where(t >= c["step"], amp(c["dbfs"]), 0.0)
                info["step_at"] = c["step"]
            elif "halfsine" in c:
                n = np.round(t * fs).astype(np.int64) - round(c["halfsine"] * fs)
                m = c["samples"]
                # Rising half of a sine over m samples, then back to zero.
                x += np.where((n >= 0) & (n < m), amp(c["dbfs"]) * np.sin(0.5 * math.pi * (n + 1) / m), 0.0)
                info["pulse_at"] = round(c["halfsine"] * fs) / fs
            elif "steps" in c:
                f = self.coh(c["steps"])
                lv = np.arange(c["from_dbfs"], c["to_dbfs"] - 1e-9, -c["step_db"])
                d = c["step_seconds"]
                k = np.minimum((t // d).astype(int), len(lv) - 1)
                x += amp(lv[k]) * np.sin(2 * math.pi * f * t)
                info["steps"] = [{"start": i * d, "seconds": d, "hz": f, "dbfs": float(v)} for i, v in enumerate(lv)]
            elif "freq_steps" in c:
                fl = [self.coh(f) for f in c["freq_steps"]]
                d = c["step_seconds"]
                k = np.minimum((t // d).astype(int), len(fl) - 1)
                fk = np.array(fl)[k]
                # Phase continuous within each step, starting at zero.
                x += amp(c["dbfs"]) * np.sin(2 * math.pi * fk * (t - k * d))
                info["steps"] = [{"start": i * d, "seconds": d, "hz": f, "dbfs": c["dbfs"]} for i, f in enumerate(fl)]
            elif "ramp_sine" in c:
                f = self.coh(c["ramp_sine"])
                dur = c["seconds"]
                lv = c["from_dbfs"] + (c["to_dbfs"] - c["from_dbfs"]) * t / dur
                x += amp(lv) * np.sin(2 * math.pi * f * t)
                info["ramp"] = {"hz": f, "from_dbfs": c["from_dbfs"], "to_dbfs": c["to_dbfs"], "seconds": float(dur)}
            else:
                raise ValueError(f"unknown component {c}")
        return x


def steady_seconds(sig):
    if "seconds" in sig:
        return sig["seconds"]
    for c in sig.get("left", []) if isinstance(sig.get("left"), list) else sig.get("right", []):
        if "steps" in c:
            n = len(np.arange(c["from_dbfs"], c["to_dbfs"] - 1e-9, -c["step_db"]))
            return n * c["step_seconds"]
        if "freq_steps" in c:
            return len(c["freq_steps"]) * c["step_seconds"]
    raise ValueError(sig["id"])


def layout(sig):
    lead, ramp = sig.get("lead_silence", 0), sig.get("ramp", 0)
    total = lead + ramp + steady_seconds(sig)
    total = math.ceil(total / FRAME - 1e-9) * FRAME
    return lead, ramp, total


def channel_comps(sig):
    left = sig.get("left", [])
    right = sig.get("right", "same")
    left = [] if left == "silent" else left
    right = left if right == "same" else ([] if right == "silent" else right)
    return [left, right]


def render_signal(res, sig, fs, i0=0, i1=None):
    """Float stereo (2, i1 - i0): samples i0 .. i1 at fs (default: all), and the info dict."""
    lead, ramp, total = layout(sig)
    if i1 is None:
        i1 = round(total * fs)
    n = i1 - i0
    t = np.arange(i0, i1) / fs
    rng = np.random.default_rng([fnv(sig["id"]), fs, i0])
    out, info = [], {"lead_silence": lead, "ramp": ramp, "seconds": total, "steady_start": lead + ramp}
    for ch, comps in enumerate(channel_comps(sig)):
        ci = {}
        x = res.render(comps, t - (lead + ramp), fs, ci)
        env = np.clip((t - lead) / ramp, 0, 1) if ramp else np.ones(n)
        x = x * env
        if lead:
            x = np.where(t < lead, rng.standard_normal(n) * amp(-100), x)
        out.append(x)
        info.setdefault("channels", []).append(ci)
    return np.stack(out), info


def quantise24(x, sig, fs):
    """24-bit integers (2, n): TPDF dither (fixed seed) unless dither = false; J-Test adds
    its LSB square wave."""
    rng = np.random.default_rng([fnv(sig["id"]), fs, 24])
    s = x * 2.0**23
    if sig.get("dither", True):
        d = rng.random(s.shape) - rng.random(s.shape)
        if sig.get("right", "same") == "same":
            d[1] = d[0]  # identical channels stay identical (channel matching)
        s = s + d
    q = np.round(s)
    if any("jtest" in c for c in channel_comps(sig)[0]):
        lead, ramp, _ = layout(sig)
        n = np.arange(x.shape[1])
        sq = np.where((n // 96) % 2 == 0, 1.0, -1.0)  # period 192 samples: fs/192, +-1 LSB
        sq = np.where(n >= round((lead + ramp) * fs), sq, 0.0)
        q = q + sq
    return np.clip(q, -(2**23), 2**23 - 1).astype(np.int32)


def bits_signal(sig):
    n = round(layout(sig)[2] * DSD64)
    bits = np.unpackbits(np.full(n // 8, 0x69, np.uint8)).reshape(1, -1).repeat(2, 0)
    at = round(0.5 * DSD64) // 8 * 8
    if sig["bits"] == "impulse":
        bits[0, at] = 1  # 0x69 = 0110 1001: its first bit is 0
    elif sig["bits"] == "step":
        bits[:, at:] = np.unpackbits(np.full((n - at) // 8, 0xEE, np.uint8))
    return bits


def job_list(cfg, only):
    jobs = []
    for sig in cfg["signal"]:
        if only and sig["id"] not in only:
            continue
        if sig["source"] == "bits":
            jobs.append((sig, "bits", None, 64, "ref"))
            continue
        srcs = [("p88" if r == 88200 else "p96", r) for r in sig.get("pcm_rates", [])] if sig["source"] == "pcm" else [("dsd", None)]
        for src, fs in srcs:
            for dr in sig.get("dsd_rates", [64]):
                for m in sig.get("modulators", ["ref"]):
                    jobs.append((sig, src, fs, dr, m))
    return jobs


def run_job(job, cfg, out, sox, force):
    sig, src, fs, dr, mod = job
    name = f"{sig['id']}-{src}-dsd{dr}-{mod}"
    path = os.path.join(out, "dsd", name + ".dsf")
    res = Resolver(cfg["settings"])
    rate = DSD64 * dr // 64
    meta = {"id": sig["id"], "desc": sig["desc"], "source": src, "pcm_rate": fs, "dsd_rate": rate, "modulator": mod, "file": os.path.relpath(path, out)}
    if src == "bits":
        meta.update({"seconds": layout(sig)[2], "steady_start": 0.0, "lead_silence": 0, "ramp": 0})
        if force or not os.path.exists(path):
            dsdio.write_dsf(path, bits_signal(sig), DSD64)
        return name, meta
    if src == "dsd":
        _, info = render_signal(res, sig, rate, 0, 1)
    else:
        xp, info = render_signal(res, sig, fs)
        q = quantise24(xp, sig, fs)
        wav = os.path.join(out, "pcm", f"{sig['id']}-{src}.wav")
        if not os.path.exists(wav):
            dsdio.write_wav24(wav + ".tmp", q.T, fs)
            os.replace(wav + ".tmp", wav)
        meta["pcm_file"] = os.path.relpath(wav, out)
    meta.update(info)
    if not force and os.path.exists(path):
        return name, meta
    gain = sig.get("ntf_gain", 1.3)
    if mod == "sox":
        cmd = [sox, "-D", os.path.join(out, meta["pcm_file"]), "-t", "dsf", path + ".tmp", "rate", "-v", str(rate)] + cfg["sox"]["args"][str(dr)]
        meta["command"] = " ".join(["sox"] + cmd[1:]).replace(out + "/", "")
        subprocess.run(cmd, check=True, stderr=subprocess.DEVNULL)
        os.replace(path + ".tmp", path)
        return name, meta
    meta["ntf_gain"] = gain
    zs, ps, b, a, corner = modulator.design(rate, gain)
    meta["ntf_corner_hz"] = corner
    n_out = round(layout(sig)[2] * rate)
    block = 1 << 20
    packed = []
    if src != "dsd":
        interp = dsdio.Interpolator(fs, rate)
        meta["interpolator"] = {"taps": int(len(interp.h)), "kaiser_beta": float(interp.beta), "up": interp.up, "down": interp.down}
    for c in range(2):
        m = modulator.Modulator(b, a)
        if src != "dsd":
            gen = interp.blocks(q[c].astype(np.float64) * 2.0**-23, n_out, block)
        else:
            gen = (render_signal(res, sig, rate, i, min(i + block, n_out))[0][c] for i in range(0, n_out, block))
        parts = [np.packbits(m.process(blk)).tobytes() for blk in gen]
        packed.append(b"".join(parts))
    dsdio.write_dsf(path + ".tmp", packed, rate, packed=True)
    os.replace(path + ".tmp", path)
    return name, meta


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", required=True)
    ap.add_argument("--signals", default=os.path.join(HERE, "signals.toml"))
    ap.add_argument("--only", default="")
    ap.add_argument("--jobs", type=int, default=min(os.cpu_count(), 8))
    ap.add_argument("--sox", default="sox", help="SoX with DSD support (mansr's SoX-DSD)")
    ap.add_argument("--no-sox", action="store_true")
    ap.add_argument("--force", action="store_true")
    a = ap.parse_args()
    cfg = tomllib.load(open(a.signals, "rb"))
    os.makedirs(os.path.join(a.out, "pcm"), exist_ok=True)
    os.makedirs(os.path.join(a.out, "dsd"), exist_ok=True)
    only = set(filter(None, a.only.split(",")))
    jobs = [j for j in job_list(cfg, only) if not (a.no_sox and j[4] == "sox")]
    # PCM files first (shared by several jobs), then everything in parallel, longest first.
    jobs.sort(key=lambda j: -(layout(j[0])[2] * j[3] * (20 if j[4] == "sox" else 1)))
    manifest_path = os.path.join(a.out, "signals.json")
    manifest = json.load(open(manifest_path)) if os.path.exists(manifest_path) else {}
    for j in jobs:  # write the PCM files serially so parallel jobs never race on them
        if j[1] in ("p88", "p96"):
            run_pcm_only(j, cfg, a.out)
    with ProcessPoolExecutor(a.jobs) as ex:
        futs = [ex.submit(run_job, j, cfg, a.out, a.sox, a.force) for j in jobs]
        for f in futs:
            name, meta = f.result()
            manifest[name] = meta
            print(name, file=sys.stderr)
    manifest["_settings"] = {"signals_toml": os.path.basename(a.signals), "ref_modulator": modulator.__doc__.strip().splitlines()[0]}
    json.dump(manifest, open(manifest_path, "w"), indent=1, default=float)


def run_pcm_only(job, cfg, out):
    sig, src, fs, _, _ = job
    wav = os.path.join(out, "pcm", f"{sig['id']}-{src}.wav")
    if os.path.exists(wav):
        return
    xp, _ = render_signal(Resolver(cfg["settings"]), sig, fs)
    dsdio.write_wav24(wav + ".tmp", quantise24(xp, sig, fs).T, fs)
    os.replace(wav + ".tmp", wav)


if __name__ == "__main__":
    main()
