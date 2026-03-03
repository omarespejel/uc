pub fn fib(n: felt252) -> felt252 {
    if n == 0 {
        return 0;
    }
    if n == 1 {
        return 1;
    }
    fib(n - 1) + fib(n - 2)
}

pub fn weighted_sum(value: felt252) -> felt252 {
    value * 3 + mix(value)
}

fn mix(value: felt252) -> felt252 {
    ((value + 17) * 5) - 11
}
