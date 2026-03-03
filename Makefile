SHELL := /bin/zsh

.PHONY: bootstrap benchmark-local benchmark-smoke compare-local gh-bootstrap

bootstrap:
	@mkdir -p benchmarks/results benchmarks/baselines
	@chmod +x benchmarks/scripts/run_local_benchmarks.sh benchmarks/scripts/run_dual_run_comparator.sh scripts/github/bootstrap_github_stack.sh
	@echo "Bootstrap complete."

benchmark-local:
	@./benchmarks/scripts/run_local_benchmarks.sh --matrix research

benchmark-smoke:
	@./benchmarks/scripts/run_local_benchmarks.sh --matrix smoke

compare-local:
	@./benchmarks/scripts/run_dual_run_comparator.sh

gh-bootstrap:
	@./scripts/github/bootstrap_github_stack.sh
