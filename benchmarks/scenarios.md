# Scenario Matrix

## Build Scenarios
1. `build.cold`
- Remove workspace build artifacts and run `scarb build`.

2. `build.warm_noop`
- Build once, then rerun without changes.

3. `build.warm_edit`
- Build once, modify one source file, rebuild.

4. `build.warm_edit_semantic`
- Build once, apply a semantic source edit, rebuild.

## Metadata Scenarios
5. `metadata.online_cold`
- Run metadata with empty global cache.

6. `metadata.offline_warm`
- Warm cache once, then run metadata with `--offline`.

## Outputs
Each run generates:
- JSON with raw samples + summary stats.
- Markdown summary table.
