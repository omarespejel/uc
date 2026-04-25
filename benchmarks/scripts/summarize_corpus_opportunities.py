#!/usr/bin/env python3
"""Summarize benchmark/support artifacts into an agent-actionable opportunity log."""

from __future__ import annotations

import argparse
import datetime as dt
import json
import math
from pathlib import Path
from typing import Any

SCHEMA_VERSION = 1
REQUIRED_DIAGNOSTIC_FIELDS = {
    "schema_version",
    "code",
    "category",
    "severity",
    "title",
    "docs_url",
    "what_happened",
    "why",
    "how_to_fix",
    "next_commands",
    "safe_automated_action",
    "retryable",
    "fallback_used",
}
GENERIC_DIAGNOSTIC_TEXT = {
    "compilation failed",
    "native failed",
    "native compilation failed",
    "uc auto build failed",
    "uc auto build failed before backend classification completed",
    "uc auto build fell back to the scarb backend",
}


def load_json(path: Path) -> dict[str, Any]:
    with path.open("r", encoding="utf-8") as handle:
        data = json.load(handle)
    if not isinstance(data, dict):
        raise SystemExit(f"expected object JSON at {path}")
    return data


def detect_benchmark(data: dict[str, Any]) -> tuple[str, dict[str, Any], dict[str, Any] | None]:
    if isinstance(data.get("benchmark"), dict) and isinstance(data["benchmark"].get("cases"), list):
        return "deployed_contract_corpus", data["benchmark"], data.get("corpus") if isinstance(data.get("corpus"), dict) else None
    if isinstance(data.get("cases"), list):
        return "real_repo_benchmark", data, None
    raise SystemExit("input must be a real-repo benchmark JSON or deployed-contract corpus benchmark JSON")


def item_metadata_by_tag(corpus: dict[str, Any] | None) -> dict[str, dict[str, Any]]:
    if not corpus:
        return {}
    items = corpus.get("items")
    if not isinstance(items, list):
        return {}
    out: dict[str, dict[str, Any]] = {}
    for item in items:
        if isinstance(item, dict) and isinstance(item.get("tag"), str):
            out[item["tag"]] = item
    return out


def as_float(value: Any) -> float | None:
    if isinstance(value, bool) or value is None:
        return None
    if isinstance(value, (int, float)) and math.isfinite(float(value)):
        return float(value)
    return None


def pct(part: float | None, total: float | None) -> float | None:
    if part is None or total is None or total <= 0:
        return None
    return part / total


def round3(value: Any) -> float | None:
    number = as_float(value)
    if number is None:
        return None
    return round(number, 3)


def speedup(case: dict[str, Any], stage: str) -> float | None:
    benchmarks = case.get("benchmarks")
    if not isinstance(benchmarks, dict):
        return None
    scarb = benchmarks.get("scarb", {}).get("build", {}).get(stage, {}).get("stats", {}).get("p95_ms")
    uc = benchmarks.get("uc", {}).get("build", {}).get(stage, {}).get("stats", {}).get("p95_ms")
    scarb_f = as_float(scarb)
    uc_f = as_float(uc)
    if scarb_f is None or uc_f is None or uc_f <= 0:
        return None
    return scarb_f / uc_f


def get_toolchain(case: dict[str, Any]) -> dict[str, Any]:
    matrix = case.get("support_matrix") if isinstance(case.get("support_matrix"), dict) else {}
    build_report = matrix.get("build_report") if isinstance(matrix.get("build_report"), dict) else {}
    native_support = case.get("native_support") if isinstance(case.get("native_support"), dict) else {}
    toolchain = build_report.get("native_toolchain") if isinstance(build_report.get("native_toolchain"), dict) else None
    if toolchain is None:
        toolchain = native_support.get("toolchain") if isinstance(native_support.get("toolchain"), dict) else {}
    return dict(toolchain or {})


def collect_diagnostics(case: dict[str, Any]) -> list[dict[str, Any]]:
    diagnostics: list[dict[str, Any]] = []
    native_support = case.get("native_support") if isinstance(case.get("native_support"), dict) else {}
    support_diags = native_support.get("diagnostics") if isinstance(native_support.get("diagnostics"), list) else []
    diagnostics.extend(diag for diag in support_diags if isinstance(diag, dict))
    matrix = case.get("support_matrix") if isinstance(case.get("support_matrix"), dict) else {}
    build_report = matrix.get("build_report") if isinstance(matrix.get("build_report"), dict) else {}
    build_diags = build_report.get("diagnostics") if isinstance(build_report.get("diagnostics"), list) else []
    diagnostics.extend(diag for diag in build_diags if isinstance(diag, dict))
    return diagnostics


def phase_telemetry(case: dict[str, Any]) -> dict[str, Any]:
    matrix = case.get("support_matrix") if isinstance(case.get("support_matrix"), dict) else {}
    build_report = matrix.get("build_report") if isinstance(matrix.get("build_report"), dict) else {}
    telemetry = build_report.get("phase_telemetry")
    return dict(telemetry) if isinstance(telemetry, dict) else {}


def add_opportunity(opps: list[dict[str, Any]], code: str, severity: str, title: str, why: str, next_action: str) -> None:
    opps.append(
        {
            "code": code,
            "severity": severity,
            "title": title,
            "why": why,
            "next_action": next_action,
        }
    )


def is_generic_diagnostic_text(value: Any) -> bool:
    if not isinstance(value, str):
        return False
    normalized = value.strip().rstrip(".").lower()
    return normalized in GENERIC_DIAGNOSTIC_TEXT


def diagnostic_quality_issues(diag: dict[str, Any]) -> list[str]:
    issues = [f"missing {field}" for field in sorted(REQUIRED_DIAGNOSTIC_FIELDS) if field not in diag]
    for field in ("severity", "title", "docs_url", "safe_automated_action"):
        value = diag.get(field)
        if field in diag and (not isinstance(value, str) or not value.strip()):
            issues.append(f"{field} is empty")
    for field in ("how_to_fix", "next_commands"):
        value = diag.get(field)
        if field in diag and (
            not isinstance(value, list)
            or not value
            or any(not isinstance(item, str) or not item.strip() for item in value)
        ):
            issues.append(f"{field} is empty")
    for field in ("what_happened", "why"):
        raw = diag.get(field)
        if not isinstance(raw, str):
            if field in diag:
                issues.append(f"{field} is not a string")
            continue
        text = raw.strip()
        if not text or is_generic_diagnostic_text(text):
            issues.append(f"{field} is generic")
    return issues


def unstable_lanes_by_tag(summary: dict[str, Any]) -> dict[str, list[dict[str, Any]]]:
    lanes = summary.get("unstable_lanes") if isinstance(summary.get("unstable_lanes"), list) else []
    out: dict[str, list[dict[str, Any]]] = {}
    for lane in lanes:
        if isinstance(lane, dict) and isinstance(lane.get("tag"), str):
            out.setdefault(lane["tag"], []).append(lane)
    return out


def classify_phase_hotspots(telemetry: dict[str, Any]) -> list[dict[str, Any]]:
    compile_ms = as_float(telemetry.get("compile_ms"))
    hotspot_specs = [
        ("native_frontend_compile_ms", "UCO3001", "Native frontend compile dominates"),
        ("native_casm_ms", "UCO3002", "CASM generation is material"),
        ("native_artifact_write_ms", "UCO3003", "Artifact write is material"),
        ("fingerprint_ms", "UCO3004", "Fingerprinting is material"),
        ("native_session_prepare_ms", "UCO3007", "Native session prepare is material"),
    ]
    hotspots: list[dict[str, Any]] = []
    for field, code, title in hotspot_specs:
        value = as_float(telemetry.get(field))
        share = pct(value, compile_ms)
        if value is None:
            continue
        if value >= 1000 or (share is not None and share >= 0.20):
            hotspots.append(
                {
                    "code": code,
                    "field": field,
                    "title": title,
                    "elapsed_ms": round3(value),
                    "compile_share": round3(share) if share is not None else None,
                }
            )
    hotspots.sort(key=lambda item: (item["elapsed_ms"] or 0), reverse=True)
    return hotspots


def summarize_case(
    case: dict[str, Any],
    unstable_by_tag: dict[str, list[dict[str, Any]]],
    item_by_tag: dict[str, dict[str, Any]],
) -> dict[str, Any]:
    tag = str(case.get("tag") or "<unknown>")
    matrix = case.get("support_matrix") if isinstance(case.get("support_matrix"), dict) else {}
    native_support = case.get("native_support") if isinstance(case.get("native_support"), dict) else {}
    toolchain = get_toolchain(case)
    diagnostics = collect_diagnostics(case)
    telemetry = phase_telemetry(case)
    phase_hotspots = classify_phase_hotspots(telemetry)
    unstable = unstable_by_tag.get(tag, [])
    classification = str(matrix.get("classification") or "unknown")
    benchmark_status = str(case.get("benchmark_status") or "unknown")
    opps: list[dict[str, Any]] = []

    if classification == "native_unsupported":
        add_opportunity(
            opps,
            "UCO1001",
            "blocker",
            "Native support gap",
            str(matrix.get("reason") or native_support.get("reason") or "native support probe returned unsupported"),
            "Fix toolchain selection or add the missing native lane before benchmarking this case.",
        )
    elif classification == "fallback_used":
        add_opportunity(
            opps,
            "UCO1002",
            "blocker",
            "Fallback path used",
            str(matrix.get("reason") or "uc auto build fell back to Scarb"),
            "Treat this as unsupported for launch speed claims; capture the fallback diagnostic and fix the native failure class.",
        )
    elif classification == "build_failed":
        add_opportunity(
            opps,
            "UCO1003",
            "blocker",
            "Auto-build classification failed",
            str(matrix.get("reason") or "uc auto build failed"),
            "Use the recorded log/report path to create a replay bundle and add a regression fixture.",
        )

    if benchmark_status == "failed":
        add_opportunity(
            opps,
            "UCO2001",
            "blocker",
            "Benchmark lane failed",
            "At least one benchmark lane did not complete successfully.",
            "Inspect the failed lane log before making speed or support claims for this case.",
        )
    if unstable:
        add_opportunity(
            opps,
            "UCO2002",
            "high",
            "Benchmark lane unstable",
            f"{len(unstable)} lane(s) exceeded stability thresholds.",
            "Rerun in the same host window after removing noise; do not use this case for headline speed copy yet.",
        )

    cold_speedup = speedup(case, "cold")
    warm_speedup = speedup(case, "warm_noop")
    if cold_speedup is not None and cold_speedup < 1.0:
        add_opportunity(
            opps,
            "UCO3005",
            "high",
            "UC cold build slower than Scarb",
            f"Cold p95 speedup is {cold_speedup:.3f}x, below parity.",
            "Profile native compile phases and compare against the same-window Scarb lane before adding cache work.",
        )
    elif cold_speedup is not None and cold_speedup < 1.25:
        add_opportunity(
            opps,
            "UCO3006",
            "medium",
            "UC cold speedup is weak",
            f"Cold p95 speedup is {cold_speedup:.3f}x, below the material-launch threshold.",
            "Profile semantic/native frontend hotspots on this case before claiming material speedup.",
        )

    for hotspot in phase_hotspots:
        if hotspot["code"] == "UCO3001":
            next_action = "Profile semantic/native frontend work for this case; this is the best acceleration target."
        else:
            next_action = f"Inspect {hotspot['field']} before optimizing unrelated cache paths."
        add_opportunity(
            opps,
            hotspot["code"],
            "medium",
            hotspot["title"],
            f"{hotspot['field']} took {hotspot['elapsed_ms']}ms"
            + (f" ({hotspot['compile_share']} of compile_ms)." if hotspot.get("compile_share") is not None else "."),
            next_action,
        )

    for diag in diagnostics:
        quality_issues = diagnostic_quality_issues(diag)
        if quality_issues:
            add_opportunity(
                opps,
                "UCO5001",
                "medium",
                "Diagnostic is not agent-grade",
                f"Diagnostic {diag.get('code', '<missing-code>')} has weak remediation detail: {', '.join(quality_issues)}.",
                "Extend the diagnostic payload before relying on agents to remediate this class automatically.",
            )

    if classification == "native_supported" and benchmark_status == "ok" and not unstable:
        add_opportunity(
            opps,
            "UCO4001",
            "info",
            "Launch evidence candidate",
            "Case is native-supported, benchmarked successfully, and has no unstable lanes.",
            "Keep it in the support matrix and consider it for bounded launch evidence if claim guards also pass.",
        )
    if warm_speedup is not None and warm_speedup >= 10.0:
        add_opportunity(
            opps,
            "UCO4002",
            "info",
            "Strong warm no-op speedup",
            f"Warm p95 speedup is {warm_speedup:.3f}x.",
            "Use only with lane/sample/host caveats and stability guard status.",
        )

    item = item_by_tag.get(tag, {})
    top_opps = [opp["code"] for opp in opps]
    return {
        "tag": tag,
        "manifest_path": case.get("manifest_path"),
        "source_kind": item.get("source_kind"),
        "contract_address": item.get("contract_address"),
        "class_hash": item.get("class_hash"),
        "cairo_version": native_support.get("package_cairo_version") or item.get("cairo_version"),
        "classification": classification,
        "benchmark_status": benchmark_status,
        "compile_backend": matrix.get("compile_backend"),
        "fallback_used": bool(matrix.get("fallback_used")),
        "toolchain": {
            "requested_version": toolchain.get("requested_version"),
            "requested_major_minor": toolchain.get("requested_major_minor"),
            "request_source": toolchain.get("request_source"),
            "source": toolchain.get("source"),
            "compiler_version": toolchain.get("compiler_version"),
        },
        "speedups": {
            "cold_p95": round3(cold_speedup),
            "warm_noop_p95": round3(warm_speedup),
        },
        "phase_telemetry": telemetry,
        "phase_hotspots": phase_hotspots,
        "unstable_lanes": unstable,
        "diagnostics": diagnostics,
        "opportunities": opps,
        "opportunity_codes": top_opps,
    }


def count_by(items: list[dict[str, Any]], key: str) -> dict[str, int]:
    counts: dict[str, int] = {}
    for item in items:
        value = item.get(key)
        if not isinstance(value, str):
            value = "<unknown>"
        counts[value] = counts.get(value, 0) + 1
    return dict(sorted(counts.items()))


def opportunity_counts(items: list[dict[str, Any]]) -> dict[str, int]:
    counts: dict[str, int] = {}
    for item in items:
        for code in item.get("opportunity_codes", []):
            counts[code] = counts.get(code, 0) + 1
    return dict(sorted(counts.items()))


def md_escape(value: Any) -> str:
    if value is None:
        return "<none>"
    text = str(value).replace("\n", " ").replace("|", "\\|")
    return text if text else "<none>"


def render_markdown(summary: dict[str, Any]) -> str:
    lines: list[str] = []
    lines.append("# Corpus Opportunity Summary")
    lines.append("")
    lines.append(f"- Generated at: {summary['generated_at']}")
    lines.append(f"- Source JSON: {summary['source_json']}")
    lines.append(f"- Source kind: {summary['source_kind']}")
    corpus = summary.get("corpus") or {}
    if corpus:
        lines.append(f"- Corpus: {corpus.get('corpus_id', '<unknown>')}")
        lines.append(f"- Chain: {corpus.get('chain', '<unknown>')}")
    lines.append("")
    lines.append("## Summary")
    lines.append("")
    s = summary["summary"]
    lines.append(f"- Cases: {s['case_count']}")
    lines.append(f"- Launch evidence candidates: {s['launch_candidate_count']}")
    lines.append(f"- Blocker opportunities: {s['blocker_opportunity_count']}")
    lines.append(f"- Unstable lanes: {s['unstable_lane_count']}")
    lines.append("")
    lines.append("## Support Matrix")
    lines.append("")
    lines.append("| Classification | Count |")
    lines.append("|---|---:|")
    for key, value in (s.get("support_matrix") or {}).items():
        lines.append(f"| {md_escape(key)} | {value} |")
    lines.append("")
    lines.append("## Opportunity Counts")
    lines.append("")
    lines.append("| Code | Count |")
    lines.append("|---|---:|")
    for key, value in s["opportunity_counts"].items():
        lines.append(f"| {md_escape(key)} | {value} |")
    lines.append("")
    lines.append("## Cases")
    lines.append("")
    lines.append("| Tag | Classification | Benchmark | Cairo | Cold p95 speedup | Warm p95 speedup | Opportunities |")
    lines.append("|---|---|---|---|---:|---:|---|")
    for case in summary["cases"]:
        cold = case["speedups"].get("cold_p95")
        warm = case["speedups"].get("warm_noop_p95")
        lines.append(
            "| "
            + " | ".join(
                [
                    md_escape(case.get("tag")),
                    md_escape(case.get("classification")),
                    md_escape(case.get("benchmark_status")),
                    md_escape(case.get("cairo_version")),
                    md_escape(cold),
                    md_escape(warm),
                    md_escape(", ".join(case.get("opportunity_codes", []))),
                ]
            )
            + " |"
        )
    lines.append("")
    lines.append("## Next Actions")
    lines.append("")
    lines.append("| Tag | Code | Severity | Action |")
    lines.append("|---|---|---|---|")
    for case in summary["cases"]:
        for opp in case.get("opportunities", []):
            if opp.get("severity") == "info":
                continue
            lines.append(
                "| "
                + " | ".join(
                    [
                        md_escape(case.get("tag")),
                        md_escape(opp.get("code")),
                        md_escape(opp.get("severity")),
                        md_escape(opp.get("next_action")),
                    ]
                )
                + " |"
            )
    return "\n".join(lines) + "\n"


def build_summary(source_path: Path, data: dict[str, Any]) -> dict[str, Any]:
    source_kind, benchmark, corpus = detect_benchmark(data)
    summary = benchmark.get("summary") if isinstance(benchmark.get("summary"), dict) else {}
    item_by_tag = item_metadata_by_tag(corpus)
    unstable_by_tag = unstable_lanes_by_tag(summary)
    cases = [summarize_case(case, unstable_by_tag, item_by_tag) for case in benchmark["cases"] if isinstance(case, dict)]
    blocker_count = sum(
        1
        for case in cases
        for opp in case.get("opportunities", [])
        if opp.get("severity") == "blocker"
    )
    launch_candidates = sum(1 for case in cases if "UCO4001" in case.get("opportunity_codes", []))
    return {
        "schema_version": SCHEMA_VERSION,
        "generated_at": dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z"),
        "source_json": str(source_path),
        "source_kind": source_kind,
        "corpus": {
            "corpus_id": corpus.get("corpus_id"),
            "chain": corpus.get("chain"),
            "selection": corpus.get("selection"),
            "summary": corpus.get("summary"),
        }
        if corpus
        else None,
        "summary": {
            "case_count": len(cases),
            "support_matrix": summary.get("support_matrix") or count_by(cases, "classification"),
            "unstable_lane_count": int(summary.get("unstable_lane_count") or 0),
            "classification_counts": count_by(cases, "classification"),
            "benchmark_status_counts": count_by(cases, "benchmark_status"),
            "opportunity_counts": opportunity_counts(cases),
            "blocker_opportunity_count": blocker_count,
            "launch_candidate_count": launch_candidates,
        },
        "cases": cases,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--benchmark-json", required=True, type=Path, help="real-repo or deployed-contract benchmark JSON")
    parser.add_argument("--out-json", type=Path, help="write structured opportunity summary JSON")
    parser.add_argument("--out-md", type=Path, help="write Markdown opportunity summary")
    args = parser.parse_args()

    source_path = args.benchmark_json.resolve()
    summary = build_summary(source_path, load_json(source_path))
    json_text = json.dumps(summary, indent=2, sort_keys=True) + "\n"

    if args.out_json:
        args.out_json.parent.mkdir(parents=True, exist_ok=True)
        args.out_json.write_text(json_text, encoding="utf-8")
    if args.out_md:
        args.out_md.parent.mkdir(parents=True, exist_ok=True)
        args.out_md.write_text(render_markdown(summary), encoding="utf-8")
    if not args.out_json and not args.out_md:
        print(json_text, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
