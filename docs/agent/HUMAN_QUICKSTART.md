# Human Quickstart

Use this path when you are debugging `uc` directly.

## Probe Native Support

```sh
uc support native --manifest-path Scarb.toml
uc support native --manifest-path Scarb.toml --format json | jq
```

## Build With Native First

```sh
uc build --engine uc --daemon-mode off --manifest-path Scarb.toml
```

## Write A Build Report

```sh
uc build --engine uc --daemon-mode off --manifest-path Scarb.toml --report-path /tmp/uc-build-report.json
jq . /tmp/uc-build-report.json
```

## Older Cairo Lane

For Cairo `2.14` projects:

```sh
./scripts/build_native_toolchain_helper.sh --lane 2.14
export UC_NATIVE_TOOLCHAIN_2_14_BIN=/absolute/path/printed/by/the/script
uc support native --manifest-path Scarb.toml --format json | jq
```

## Benchmark Discipline

Use same-window comparisons only. Do not compare today's `uc` run to yesterday's Scarb run.

Publish benchmark numbers only when the artifact directory includes host metadata, binary identity, lanes, flags, sample counts, support classifications, and logs.
