pub fn fib(n: felt252) -> felt252 {
    fib_linear(n, 0, 1)
}

fn fib_linear(remaining: felt252, prev: felt252, curr: felt252) -> felt252 {
    if remaining == 0 {
        return prev;
    }
    fib_linear(remaining - 1, curr, prev + curr)
}

pub fn weighted_sum(value: felt252) -> felt252 {
    value * 3 + mix(value)
}

fn mix(value: felt252) -> felt252 {
    ((value + 17) * 5) - 11
}
