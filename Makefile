SHELL := /bin/zsh

.PHONY: bootstrap benchmark-local gh-bootstrap

bootstrap:
	@mkdir -p benchmarks/results
	@chmod +x benchmarks/scripts/run_local_benchmarks.sh scripts/github/bootstrap_github_stack.sh
	@echo "Local bootstrap done."

benchmark-local:
	@./benchmarks/scripts/run_local_benchmarks.sh

gh-bootstrap:
	@./scripts/github/bootstrap_github_stack.sh

