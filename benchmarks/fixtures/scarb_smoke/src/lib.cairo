mod math;

fn smoke() -> felt252 {
    let seed = math::fib(8);
    math::weighted_sum(seed)
}
