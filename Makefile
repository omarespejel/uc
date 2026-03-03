SHELL := /bin/zsh

.PHONY: bootstrap benchmark-local benchmark-smoke gh-bootstrap

bootstrap:
	@mkdir -p benchmarks/results benchmarks/baselines
	@chmod +x benchmarks/scripts/run_local_benchmarks.sh scripts/github/bootstrap_github_stack.sh
	@echo "Bootstrap complete."

benchmark-local:
	@./benchmarks/scripts/run_local_benchmarks.sh --matrix research

benchmark-smoke:
	@./benchmarks/scripts/run_local_benchmarks.sh --matrix smoke

gh-bootstrap:
	@./scripts/github/bootstrap_github_stack.sh
