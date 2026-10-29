#!/usr/bin/env python3
"""Check `uc prove --emit-smt` obligations with an independent SMT solver.

Usage:
    benchmarks/scripts/check_smt_obligations.py --dir target/smt [--solver z3|cvc5|<path>] [--timeout 10]

Every `.smt2` file written by `uc prove --emit-smt <dir>` holds one `(check-sat)` per panic path,
each preceded by `; obligation N: expected unsat|sat|unknown`, plus a second `(check-sat)` per
`sat` path with the witness values pinned. The script runs the solver on each file, pairs its
answers with the expectations and prints the disagreements: an obligation uc called `unsat` that
the solver finds `sat` means uc's PROVEN_SAFE or GUARDED_SAFE claim for that path is wrong; a
pinned witness the solver rejects means the counterexample is not a model of its own path.

The solver may be the `z3` or `cvc5` binary on PATH, any path to one, or the `z3-solver` Python
package (used when no binary is found). Exit status 1 on any disagreement.
"""
import argparse
import re
import shutil
import subprocess
import sys
from pathlib import Path

EXPECT_RE = re.compile(r"^; (obligation \d+(?: witness)?): expected (\w+)$")


def expectations(text):
    return [(m.group(1), m.group(2)) for m in (EXPECT_RE.match(l) for l in text.splitlines()) if m]


def run_binary(solver, path, timeout):
    if "cvc5" in Path(solver).name:
        cmd = [solver, "--lang", "smt2", "--incremental", f"--tlimit={timeout * 1000}", str(path)]
    else:
        cmd = [solver, "-smt2", f"-T:{timeout}", str(path)]
    proc = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout * 20 + 30)
    return [l.strip() for l in proc.stdout.splitlines() if l.strip() in ("sat", "unsat", "unknown", "timeout")]


def run_python_z3(path, timeout):
    import z3
    text = path.read_text()
    head, *blocks = text.split("(push 1)")
    answers = []
    for block in blocks:
        body = block.split("(pop 1)")[0].replace("(check-sat)", "")
        s = z3.Solver()
        s.set("timeout", timeout * 1000)
        s.from_string(head + body)
        r = s.check()
        answers.append("sat" if r == z3.sat else "unsat" if r == z3.unsat else "unknown")
    return answers


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dir", required=True)
    ap.add_argument("--solver", default="z3")
    ap.add_argument("--timeout", type=int, default=10)
    args = ap.parse_args()
    files = sorted(Path(args.dir).rglob("*.smt2"))
    if not files:
        sys.exit(f"no .smt2 files under {args.dir}")
    binary = args.solver if Path(args.solver).is_file() else shutil.which(args.solver)
    agree = disagree = unknown = 0
    problems = []
    for f in files:
        text = f.read_text()
        exp = expectations(text)
        got = run_binary(binary, f, args.timeout) if binary else run_python_z3(f, args.timeout)
        if len(got) != len(exp):
            problems.append((f, "answer count", f"{len(got)} answers for {len(exp)} obligations"))
            continue
        for (label, expected), answer in zip(exp, got):
            if answer in ("unknown", "timeout") or expected == "unknown":
                unknown += 1
            elif answer == expected:
                agree += 1
            else:
                disagree += 1
                problems.append((f, label, f"uc expected {expected}, solver says {answer}"))
    total = agree + disagree + unknown
    print(f"{len(files)} files, {total} obligations: {agree} agree, {disagree} disagree, {unknown} unknown")
    for f, label, msg in problems:
        print(f"  {f.name}: {label}: {msg}")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
