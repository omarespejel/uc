SHELL := /bin/zsh

.PHONY: bootstrap benchmark-local benchmark-uc benchmark-smoke benchmark-delta perf-fast perf-fast-semantic compare-local gh-bootstrap

bootstrap:
	@mkdir -p benchmarks/results benchmarks/baselines
	@chmod +x benchmarks/scripts/run_local_benchmarks.sh benchmarks/scripts/run_dual_run_comparator.sh benchmarks/scripts/run_fast_perf_check.sh scripts/github/bootstrap_github_stack.sh
	@echo "Bootstrap complete."

benchmark-local:
	@./benchmarks/scripts/run_local_benchmarks.sh --matrix research --tool scarb

benchmark-uc:
	@./benchmarks/scripts/run_local_benchmarks.sh --matrix research --tool uc

benchmark-smoke:
	@./benchmarks/scripts/run_local_benchmarks.sh --matrix smoke --tool scarb

benchmark-delta:
	@echo "Use benchmarks/scripts/compare_benchmark_results.sh with explicit baseline/candidate JSON files."

perf-fast:
	@./benchmarks/scripts/run_fast_perf_check.sh

perf-fast-semantic:
	@./benchmarks/scripts/run_fast_perf_check.sh --scenario build.warm_edit_semantic

compare-local:
	@./benchmarks/scripts/run_dual_run_comparator.sh

gh-bootstrap:
	@./scripts/github/bootstrap_github_stack.sh
