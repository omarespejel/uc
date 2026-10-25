#!/usr/bin/env python3
"""Head-to-head timings: uc vs scarb/snforge on real projects.

Usage:
    benchmarks/scripts/run_head_to_head.py --uc target/release/uc \
        --project /path/to/erc8004 --project /path/to/betshare [--runs 3]

Each project is copied to a scratch directory (never touching the original tree),
then timed cold and warm with both toolchains. Results are written to
benchmarks/results/head-to-head-<stamp>.json and printed as a Markdown table.

Rules (see benchmarks/README.md): same host, same window, both tools fully warm
before warm numbers, no daemon for uc (`--daemon-mode off`), snforge_std pinned to
the installed snforge version so the baseline actually runs.
"""
import argparse
import json
import os
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path


def run(cmd, cwd, env=None, timeout=1800):
    t0 = time.perf_counter()
    proc = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, env=env, timeout=timeout)
    return proc.returncode, (time.perf_counter() - t0) * 1000.0, proc.stdout + proc.stderr


def wipe(path: Path):
    if path.exists():
        shutil.rmtree(path)


def timed(cmd, cwd, runs, env=None, prepare=None):
    samples = []
    ok = True
    for _ in range(runs):
        if prepare:
            prepare()
        code, ms, _ = run(cmd, cwd, env=env)
        ok = ok and code == 0
        samples.append(ms)
    return {"ok": ok, "runs": runs, "median_ms": round(statistics.median(samples), 1), "samples_ms": [round(s, 1) for s in samples]}


def bench_project(src: Path, uc: str, runs: int, scratch: Path) -> dict:
    work = scratch / src.name
    shutil.copytree(src, work, ignore=shutil.ignore_patterns("target", ".uc", ".snfoundry_cache"))
    env = dict(os.environ)
    res = {"project": src.name, "manifest": str(src / "Scarb.toml")}

    # scarb build: cold (no target) and warm (unchanged tree)
    res["scarb_build_cold"] = timed(["scarb", "build"], work, runs, env, prepare=lambda: wipe(work / "target"))
    run(["scarb", "build"], work, env)
    res["scarb_build_warm"] = timed(["scarb", "build"], work, runs, env)

    # uc build: cold (no target, no .uc) and warm
    res["uc_build_cold"] = timed([uc, "build", "--daemon-mode", "off"], work, runs, env,
                                 prepare=lambda: (wipe(work / "target"), wipe(work / ".uc")))
    run([uc, "build", "--daemon-mode", "off"], work, env)
    res["uc_build_warm"] = timed([uc, "build", "--daemon-mode", "off"], work, runs, env)

    # artifact parity after both builds
    wipe(work / "target")
    run(["scarb", "build"], work, env)
    scarb_art = {p.name: p.read_bytes() for p in (work / "target" / "dev").glob("*.json")}
    wipe(work / "target")
    wipe(work / ".uc")
    run([uc, "build", "--daemon-mode", "off"], work, env)
    uc_art = {p.name: p.read_bytes() for p in (work / "target" / "dev").glob("*.json")}
    identical = sorted(n for n in scarb_art if uc_art.get(n) == scarb_art[n])
    differing = sorted(n for n in scarb_art if n not in identical)
    res["artifact_parity"] = {"identical": identical, "differing": differing}

    # tests: snforge (cold = fresh snfoundry cache) and uc
    has_tests = (work / "tests").is_dir() or any("#[test]" in p.read_text(errors="ignore") for p in (work / "src").rglob("*.cairo"))
    if has_tests:
        res["snforge_test_cold"] = timed(["snforge", "test"], work, runs, env,
                                         prepare=lambda: (wipe(work / "target"), wipe(work / ".snfoundry_cache")))
        run(["snforge", "test"], work, env)
        res["snforge_test_warm"] = timed(["snforge", "test"], work, runs, env)
        res["uc_test_cold"] = timed([uc, "test"], work, runs, env, prepare=lambda: (wipe(work / "target"), wipe(work / ".uc")))
        run([uc, "test"], work, env)
        res["uc_test_warm"] = timed([uc, "test"], work, runs, env)
    _, prove_ms, prove_out = run([uc, "prove", "--json"], work, env)
    try:
        prove = json.loads(prove_out[prove_out.index("{"):])
        res["uc_prove"] = {"ms": round(prove_ms, 1), "functions": prove.get("total_functions_analyzed"),
                           "findings": prove.get("total_caracal_findings"), "counterexamples": prove.get("total_counterexamples")}
    except (ValueError, json.JSONDecodeError):
        res["uc_prove"] = {"ms": round(prove_ms, 1), "error": prove_out[-300:]}
    return res


def md_table(results: list) -> str:
    rows = ["| project | scarb build cold | uc build cold | scarb build warm | uc build warm | snforge test warm | uc test warm | artifacts identical | uc prove |",
            "|---|---|---|---|---|---|---|---|---|"]
    for r in results:
        def cell(key):
            v = r.get(key)
            if not v:
                return "n/a"
            return f"{v['median_ms']/1000:.2f} s" + ("" if v["ok"] else " (failed)")
        par = r["artifact_parity"]
        parity = f"{len(par['identical'])}/{len(par['identical']) + len(par['differing'])}"
        pv = r.get("uc_prove", {})
        prove = f"{pv.get('ms', 0)/1000:.2f} s, {pv.get('findings', '?')} findings" if "error" not in pv else "error"
        rows.append(f"| {r['project']} | {cell('scarb_build_cold')} | {cell('uc_build_cold')} | {cell('scarb_build_warm')} | {cell('uc_build_warm')} | {cell('snforge_test_warm')} | {cell('uc_test_warm')} | {parity} | {prove} |")
    return "\n".join(rows)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--uc", required=True)
    ap.add_argument("--project", action="append", required=True)
    ap.add_argument("--runs", type=int, default=3)
    ap.add_argument("--results-dir", default="benchmarks/results")
    args = ap.parse_args()
    uc = str(Path(args.uc).resolve())
    scratch = Path(tempfile.mkdtemp(prefix="uc-head-to-head-"))
    versions = {}
    for tool in [[uc, "--version"], ["scarb", "--version"], ["snforge", "--version"]]:
        code, _, out = run(tool, scratch)
        versions[Path(tool[0]).name] = out.strip().splitlines()[0] if code == 0 else "missing"
    results = [bench_project(Path(p).resolve(), uc, args.runs, scratch) for p in args.project]
    stamp = time.strftime("%Y%m%d-%H%M%S")
    out_dir = Path(args.results_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    doc = {"stamp": stamp, "host": os.uname().machine + " " + os.uname().sysname, "versions": versions, "runs": args.runs, "results": results}
    (out_dir / f"head-to-head-{stamp}.json").write_text(json.dumps(doc, indent=2))
    print(json.dumps(versions))
    print(md_table(results))
    shutil.rmtree(scratch, ignore_errors=True)
    return 0 if all(r.get("uc_build_warm", {}).get("ok") for r in results) else 1


if __name__ == "__main__":
    sys.exit(main())
