#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Runs each decoder of decoders.toml, as a black box, on the generated DSD files.

    python decode.py --signals-dir SIG --out DEC --tool dsd2pcm=PATH --tool sbpcm=PATH
                     [--decoders dsd2pcm,pysacd] [--jobs N]

Writes DEC/<decoder>/<setting>/<signal file>.wav and DEC/runs.json (per output: the
command, its exit status and output, and the SHA-256 of the input and output files).
Imported outputs (decoders run elsewhere, e.g. the foobar2000 SACD plug-in) are listed by
import.py instead.
"""

import argparse
import hashlib
import json
import os
import subprocess
import tempfile
import tomllib
from concurrent.futures import ThreadPoolExecutor

import numpy as np

import dsdio

HERE = os.path.dirname(os.path.abspath(__file__))


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for b in iter(lambda: f.read(1 << 20), b""):
            h.update(b)
    return h.hexdigest()


def fill(arg, vals, tools):
    if arg.startswith("{tool:"):
        return tools[arg[6:-1]]
    return arg.format(**vals)


def run_one(dec_name, dec, setting, dsf, sig_dir, out_dir, tools, force):
    stem = os.path.basename(dsf)[:-4]
    out = os.path.join(out_dir, dec_name, setting["name"], stem + ".wav")
    if os.path.exists(out) and not force:
        return None
    os.makedirs(os.path.dirname(out), exist_ok=True)
    with tempfile.TemporaryDirectory(dir=out_dir) as tmp:
        inp = dsf
        if dec["input"] == "raw":
            chans, rate, n = dsdio.read_dsf(dsf)
            inp = os.path.join(tmp, "in.raw")
            dsdio.write_raw_frames(inp, chans)
        target = out + ".tmp.wav" if dec["output"] == "wav" else os.path.join(tmp, "out.s24")
        vals = dict(setting, input=inp, output=target)
        cmd = [fill(a, vals, tools) for a in dec["command"]]
        p = subprocess.run(cmd, capture_output=True, text=True)
        rec = {"decoder": dec_name, "setting": setting["name"], "input": os.path.relpath(dsf, sig_dir),
               "command": " ".join([os.path.basename(cmd[0])] + [os.path.basename(c) if c in (inp, target) else c for c in cmd[1:]]),
               "status": p.returncode, "stdout": p.stdout.strip()[-2000:], "stderr": p.stderr.strip()[-2000:],
               "input_sha256": sha256(dsf)}
        if p.returncode != 0:
            rec["output"] = None
            return rec
        if dec["output"] == "s24":
            raw = np.frombuffer(open(target, "rb").read(), np.uint8).reshape(-1, 3).astype(np.int32)
            v = raw[:, 0] | (raw[:, 1] << 8) | (raw[:, 2] << 16)
            v = np.where(v >= 1 << 23, v - (1 << 24), v)
            dsdio.write_wav24(out + ".tmp.wav", v.reshape(-1, 2), int(setting["rate"]))
        os.replace(out + ".tmp.wav", out)
    rec["output"] = os.path.relpath(out, out_dir)
    rec["output_sha256"] = sha256(out)
    return rec


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--signals-dir", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--config", default=os.path.join(HERE, "decoders.toml"))
    ap.add_argument("--tool", action="append", default=[], help="NAME=PATH")
    ap.add_argument("--decoders", default="")
    ap.add_argument("--only", default="", help="signal ids")
    ap.add_argument("--jobs", type=int, default=min(os.cpu_count(), 8))
    ap.add_argument("--force", action="store_true")
    a = ap.parse_args()
    cfg = tomllib.load(open(a.config, "rb"))["decoder"]
    tools = dict(t.split("=", 1) for t in a.tool)
    want = set(filter(None, a.decoders.split(","))) or set(cfg)
    only = set(filter(None, a.only.split(",")))
    manifest = json.load(open(os.path.join(a.signals_dir, "signals.json")))
    os.makedirs(a.out, exist_ok=True)
    runs_path = os.path.join(a.out, "runs.json")
    runs = json.load(open(runs_path)) if os.path.exists(runs_path) else {}
    work = []
    for name, dec in cfg.items():
        if name not in want:
            continue
        for key, meta in manifest.items():
            if key.startswith("_") or meta["dsd_rate"] // 44100 not in dec["dsd_rates"]:
                continue
            if only and meta["id"] not in only:
                continue
            if "signals" in dec and meta["id"] not in dec["signals"]:
                continue
            for s in dec["settings"]:
                work.append((name, dec, s, os.path.join(a.signals_dir, meta["file"])))
    with ThreadPoolExecutor(a.jobs) as ex:
        for rec in ex.map(lambda w: run_one(*w, a.signals_dir, a.out, tools, a.force), work):
            if rec:
                key = f"{rec['decoder']}/{rec['setting']}/{os.path.basename(rec['input'])[:-4]}"
                runs[key] = rec
                if rec["status"]:
                    print("FAILED", key, rec["stderr"][-300:])
    json.dump(runs, open(runs_path, "w"), indent=1)


if __name__ == "__main__":
    main()
