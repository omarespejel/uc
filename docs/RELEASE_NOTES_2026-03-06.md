# Release Notes (2026-03-06)

## `uc metadata` daemon output behavior

- Behavioral change: `uc metadata` now replays captured daemon `stdout/stderr` in daemon `auto|require` modes, even when `--report-path` is not provided.
- Rationale: daemon-mode metadata output should be visible to users/scripts in the same way command output is visible in non-daemon flows.
- Reference: `docs/COMMAND_SURFACE.md` documents this behavior under the `uc metadata` command section.
