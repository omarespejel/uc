# Benchmark Scenarios

## `build.cold`

Remove workspace build artifacts and run a fresh build.

## `build.warm_noop`

Run a build twice without changing source files. The second run measures unchanged-input behavior.

## `build.warm_edit`

Apply a reversible source edit, then run build.

## `build.warm_edit_semantic`

Apply a reversible semantic source edit, then run build.

## `support.native`

Run support classification without compiling. This measures pre-build routing cost and correctness.
