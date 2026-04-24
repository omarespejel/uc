SHELL := /bin/sh

.PHONY: bootstrap install-hooks doctor agent-map agent-validate validate-local-ci validate-bench-scripts validate-fast validate-native local-ci benchmark-local benchmark-uc benchmark-smoke benchmark-delta benchmark-strict-smoke benchmark-strict-research perf-fast perf-fast-semantic compare-local gh-bootstrap

bootstrap:
	@mkdir -p benchmarks/results benchmarks/baselines
	@chmod +x benchmarks/scripts/run_local_benchmarks.sh benchmarks/scripts/run_dual_run_comparator.sh benchmarks/scripts/run_fast_perf_check.sh benchmarks/scripts/run_stability_benchmarks.sh benchmarks/scripts/run_native_only_gate.sh benchmarks/scripts/run_native_real_repo_smoke.sh benchmarks/scripts/tests/benchmark_host_preflight_test.sh benchmarks/scripts/tests/native_ci_gate_test.sh scripts/github/bootstrap_github_stack.sh scripts/doctor.sh scripts/refresh_repo_map.sh scripts/validate_agent_surface.sh scripts/install_git_hooks.sh scripts/local_ci_gate.sh scripts/tests/local_ci_gate_test.sh .githooks/pre-push
	@$(MAKE) install-hooks
	@echo "Bootstrap complete."

install-hooks:
	@./scripts/install_git_hooks.sh

doctor:
	@./scripts/doctor.sh

agent-map:
	@./scripts/refresh_repo_map.sh

agent-validate:
	@./scripts/validate_agent_surface.sh

validate-local-ci:
	@./scripts/tests/local_ci_gate_test.sh

validate-bench-scripts:
	@./benchmarks/scripts/tests/benchmark_host_preflight_test.sh
	@./benchmarks/scripts/tests/native_ci_gate_test.sh

validate-fast:
	@$(MAKE) agent-validate
	@$(MAKE) validate-local-ci
	@$(MAKE) validate-bench-scripts
	@cargo fmt --all --check
	@cargo test -p uc-core --quiet

validate-native:
	@$(MAKE) agent-validate
	@$(MAKE) validate-local-ci
	@$(MAKE) validate-bench-scripts
	@cargo test -p uc-cli main_tests::native_ -- --nocapture
	@cargo test -p uc-cli commands::build::tests::native_ -- --nocapture

local-ci:
	@./scripts/local_ci_gate.sh

benchmark-local:
	@./benchmarks/scripts/run_local_benchmarks.sh --matrix research --tool scarb

benchmark-uc:
	@./benchmarks/scripts/run_local_benchmarks.sh --matrix research --tool uc

benchmark-smoke:
	@./benchmarks/scripts/run_local_benchmarks.sh --matrix smoke --tool scarb

benchmark-delta:
	@echo "Use benchmarks/scripts/compare_benchmark_results.sh with explicit baseline/candidate JSON files."

benchmark-strict-smoke:
	@./benchmarks/scripts/run_stability_benchmarks.sh --matrix smoke --runs 12 --cold-runs 12 --cycles 5 --cpu-set 0 --strict-pinning --host-preflight warn --uc-daemon-mode require --gate-config benchmarks/gates/perf-gate-smoke.json

benchmark-strict-research:
	@: "$${UC_RESEARCH_ROOT:?set UC_RESEARCH_ROOT to the cloned research workspace root}"
	@./benchmarks/scripts/run_stability_benchmarks.sh --matrix research --workspace-root "$$UC_RESEARCH_ROOT" --runs 12 --cold-runs 12 --cycles 5 --cpu-set 0 --strict-pinning --host-preflight require --uc-daemon-mode require --gate-config benchmarks/gates/perf-gate-research.json

perf-fast:
	@./benchmarks/scripts/run_fast_perf_check.sh

perf-fast-semantic:
	@./benchmarks/scripts/run_fast_perf_check.sh --scenario build.warm_edit_semantic

compare-local:
	@./benchmarks/scripts/run_dual_run_comparator.sh

gh-bootstrap:
	@./scripts/github/bootstrap_github_stack.sh
