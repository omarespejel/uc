mod math;
mod contract_patterns;

const BENCH_EDIT_SEED_BIAS: felt252 = 0;

fn smoke() -> felt252 {
    let seed = math::fib(8);
    math::weighted_sum(seed + BENCH_EDIT_SEED_BIAS)
}
