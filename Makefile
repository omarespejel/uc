SHELL := /bin/zsh

.PHONY: bootstrap doctor agent-map agent-validate validate-fast validate-native benchmark-local benchmark-uc benchmark-smoke benchmark-delta benchmark-strict-smoke benchmark-strict-research perf-fast perf-fast-semantic compare-local gh-bootstrap

bootstrap:
	@mkdir -p benchmarks/results benchmarks/baselines
	@chmod +x benchmarks/scripts/run_local_benchmarks.sh benchmarks/scripts/run_dual_run_comparator.sh benchmarks/scripts/run_fast_perf_check.sh benchmarks/scripts/run_stability_benchmarks.sh scripts/github/bootstrap_github_stack.sh scripts/doctor.sh scripts/refresh_repo_map.sh scripts/validate_agent_surface.sh
	@echo "Bootstrap complete."

doctor:
	@./scripts/doctor.sh

agent-map:
	@./scripts/refresh_repo_map.sh

agent-validate:
	@./scripts/validate_agent_surface.sh

validate-fast:
	@./scripts/validate_agent_surface.sh
	@cargo fmt --all --check
	@cargo test -p uc-core --quiet

validate-native:
	@./scripts/validate_agent_surface.sh
	@cargo test -p uc-cli main_tests::native_ -- --nocapture
	@cargo test -p uc-cli commands::build::tests::native_ -- --nocapture

benchmark-local:
	@./benchmarks/scripts/run_local_benchmarks.sh --matrix research --tool scarb

benchmark-uc:
	@./benchmarks/scripts/run_local_benchmarks.sh --matrix research --tool uc

benchmark-smoke:
	@./benchmarks/scripts/run_local_benchmarks.sh --matrix smoke --tool scarb

benchmark-delta:
	@echo "Use benchmarks/scripts/compare_benchmark_results.sh with explicit baseline/candidate JSON files."

benchmark-strict-smoke:
	@./benchmarks/scripts/run_stability_benchmarks.sh --matrix smoke --runs 12 --cold-runs 12 --cycles 5 --cpu-set 0 --strict-pinning --host-preflight warn --uc-daemon-mode require

benchmark-strict-research:
	@./benchmarks/scripts/run_stability_benchmarks.sh --matrix research --runs 12 --cold-runs 12 --cycles 5 --cpu-set 0 --strict-pinning --host-preflight warn --uc-daemon-mode require

perf-fast:
	@./benchmarks/scripts/run_fast_perf_check.sh

perf-fast-semantic:
	@./benchmarks/scripts/run_fast_perf_check.sh --scenario build.warm_edit_semantic

compare-local:
	@./benchmarks/scripts/run_dual_run_comparator.sh

gh-bootstrap:
	@./scripts/github/bootstrap_github_stack.sh
