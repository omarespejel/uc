#!/usr/bin/env python3
"""Random Cairo integer programs with a reference semantics in Python.

Each generated function works over one unsigned type (u8..u128). Expressions mix checked
`+ - *`, `/ %` (panic on zero), bitwise `& | ^`, `if` on comparisons, calls to functions
generated earlier, `Checked*` traits matched on `Option`, struct round trips, arithmetic in the
next wider type narrowed back with `try_into().unwrap()`, `felt252` arithmetic (which wraps
modulo the field prime) narrowed back the same way, and `assert`. Functions add `let` chains and
short bounded `while` loops. The Python evaluator mirrors Cairo exactly: operands left to right,
the first overflow, underflow, failed unwrap, failed assert or division by zero panics, and `if`
or a `None` arm evaluates only the chosen branch. It is the oracle the runner compares uc and
snforge against.
"""
import random

TYPES = {"u8": 8, "u16": 16, "u32": 32, "u64": 64, "u128": 128, "i8": 8, "i16": 16}
WIDER = {
    "u8": "u16",
    "u16": "u32",
    "u32": "u64",
    "u64": "u128",
    "u128": "u256",
    "i8": "i16",
    "i16": "i32",
}
WIDTH = dict(TYPES, u256=256, i32=32)
BIN_OPS_UNSIGNED = ["+", "-", "*", "/", "%", "&", "|", "^"]
BIN_OPS_SIGNED = ["+", "-", "*"]
CMP_OPS = ["<", "<=", ">", ">=", "==", "!="]
CHECKED = {"+": "CheckedAdd::checked_add", "-": "CheckedSub::checked_sub", "*": "CheckedMul::checked_mul"}
FIELD_PRIME = 2**251 + 17 * 2**192 + 1


class Panic(Exception):
    pass


def is_signed(ty):
    return ty.startswith("i")


def tmin(ty):
    return -(1 << (WIDTH[ty] - 1)) if is_signed(ty) else 0


def tmax(ty):
    return (1 << (WIDTH[ty] - 1)) - 1 if is_signed(ty) else (1 << WIDTH[ty]) - 1


def edge_values(ty):
    lo, hi = tmin(ty), tmax(ty)
    pts = {0, 1, 2, 3, hi, hi - 1, hi // 2, hi // 2 + 1, 7, 15 if hi >= 15 else 1}
    if lo < 0:
        pts |= {lo, lo + 1, -1, -2, -3, -7, lo // 2}
    return sorted(v for v in pts if lo <= v <= hi)


def rand_value(rng, ty):
    lo, hi = tmin(ty), tmax(ty)
    pick = rng.random()
    if pick < 0.45:
        return rng.choice(edge_values(ty))
    if pick < 0.75:
        return rng.randint(max(lo, -300), min(hi, 300))
    return rng.randint(lo, hi)


def prelude():
    """Struct declarations the `pair` expressions use; emit once per package."""
    return "\n".join(f"#[derive(Drop, Copy)]\npub struct P{ty} {{\n    pub x: {ty},\n    pub y: {ty},\n}}" for ty in TYPES)


def arith(op, a, b, ty):
    """Checked `+ - *` in `ty`: the result, or None on overflow or underflow."""
    r = {"+": a + b, "-": a - b, "*": a * b}[op]
    return r if tmin(ty) <= r <= tmax(ty) else None


# Expression AST:
#   ("var", name) | ("const", v) | ("bin", op, l, r) | ("if", cmp, a, b, then, else)
#   ("call", function, [args]) | ("checked", op, a, b, fallback) | ("pair", a, b, first)
#   ("wide", op, a, b) | ("felt", op, a, b) | ("assert", cmp, a, b, value)
def gen_expr(rng, ty, names, depth, helpers=()):
    if depth <= 0 or rng.random() < 0.3:
        if names and rng.random() < 0.75:
            return ("var", rng.choice(names))
        return ("const", rand_value(rng, ty))
    sub = lambda d=1: gen_expr(rng, ty, names, depth - d, helpers)  # noqa: E731
    r = rng.random()
    callable_helpers = [h for h in helpers if h.ty == ty]
    if r < 0.12:
        return ("if", rng.choice(CMP_OPS), sub(2), sub(2), sub(), sub())
    if r < 0.20 and callable_helpers:
        h = rng.choice(callable_helpers)
        return ("call", h, [sub() for _ in h.params])
    if r < 0.27:
        checked_ops = ["+", "-"] if is_signed(ty) else list(CHECKED)
        return ("checked", rng.choice(checked_ops), sub(), sub(), sub())
    if r < 0.32:
        return ("pair", sub(), sub(), rng.random() < 0.5)
    if r < 0.38:
        return ("wide", rng.choice(["+", "-", "*"]), sub(), sub())
    if r < 0.43:
        return ("felt", rng.choice(["+", "-", "*"]), sub(), sub())
    if r < 0.47:
        return ("assert", rng.choice(CMP_OPS), sub(2), sub(2), sub())
    ops = BIN_OPS_SIGNED if is_signed(ty) else BIN_OPS_UNSIGNED
    return ("bin", rng.choice(ops), sub(), sub())


def render(e, ty):
    k = e[0]
    if k == "var":
        return e[1]
    if k == "const":
        return f"({e[1]}_{ty})" if e[1] < 0 else f"{e[1]}_{ty}"
    if k == "bin":
        return f"({render(e[2], ty)} {e[1]} {render(e[3], ty)})"
    if k == "if":
        return f"(if {render(e[2], ty)} {e[1]} {render(e[3], ty)} {{ {render(e[4], ty)} }} else {{ {render(e[5], ty)} }})"
    if k == "call":
        return f"{e[1].name}({', '.join(render(a, ty) for a in e[2])})"
    if k == "checked":
        return (f"(match core::num::traits::{CHECKED[e[1]]}({render(e[2], ty)}, {render(e[3], ty)}) "
                f"{{ Option::Some(v) => v, Option::None => {render(e[4], ty)} }})")
    if k == "pair":
        field = "x" if e[3] else "y"
        return f"({{ let p = P{ty} {{ x: {render(e[1], ty)}, y: {render(e[2], ty)} }}; p.{field} }})"
    if k == "wide":
        w = WIDER[ty]
        return (f"({{ let wa: {w} = ({render(e[2], ty)}).into(); let wb: {w} = ({render(e[3], ty)}).into(); "
                f"let n: {ty} = (wa {e[1]} wb).try_into().unwrap(); n }})")
    if k == "felt":
        return (f"({{ let fa: felt252 = ({render(e[2], ty)}).into(); let fb: felt252 = ({render(e[3], ty)}).into(); "
                f"let n: {ty} = (fa {e[1]} fb).try_into().unwrap(); n }})")
    return f"({{ assert({render(e[2], ty)} {e[1]} {render(e[3], ty)}, 'chk'); {render(e[4], ty)} }})"


def compare(cmp, x, y):
    return {"<": x < y, "<=": x <= y, ">": x > y, ">=": x >= y, "==": x == y, "!=": x != y}[cmp]


def evaluate(e, ty, env):
    k = e[0]
    if k == "var":
        return env[e[1]]
    if k == "const":
        return e[1]
    if k == "bin":
        op = e[1]
        a = evaluate(e[2], ty, env)
        b = evaluate(e[3], ty, env)
        if op in ("+", "-", "*"):
            r = arith(op, a, b, ty)
            if r is None:
                raise Panic(f"{op} overflow")
            return r
        if op in ("/", "%"):
            if b == 0:
                raise Panic("division by 0")
            return a // b if op == "/" else a % b
        return {"&": a & b, "|": a | b, "^": a ^ b}[op]
    if k == "if":
        x = evaluate(e[2], ty, env)
        y = evaluate(e[3], ty, env)
        return evaluate(e[4] if compare(e[1], x, y) else e[5], ty, env)
    if k == "call":
        return e[1].run([evaluate(a, ty, env) for a in e[2]])
    if k == "checked":
        a = evaluate(e[2], ty, env)
        b = evaluate(e[3], ty, env)
        r = arith(e[1], a, b, ty)
        return evaluate(e[4], ty, env) if r is None else r
    if k == "pair":
        a = evaluate(e[1], ty, env)
        b = evaluate(e[2], ty, env)
        return a if e[3] else b
    if k == "wide":
        a = evaluate(e[2], ty, env)
        b = evaluate(e[3], ty, env)
        r = arith(e[1], a, b, WIDER[ty])
        if r is None:
            raise Panic(f"{WIDER[ty]} {e[1]} overflow")
        if not (tmin(ty) <= r <= tmax(ty)):
            raise Panic("Option::unwrap failed")
        return r
    if k == "felt":
        a = evaluate(e[2], ty, env)
        b = evaluate(e[3], ty, env)
        fa = a % FIELD_PRIME
        fb = b % FIELD_PRIME
        r = {"+": fa + fb, "-": fa - fb, "*": fa * fb}[e[1]] % FIELD_PRIME
        if r >= FIELD_PRIME + tmin(ty):
            r -= FIELD_PRIME
        if not (tmin(ty) <= r <= tmax(ty)):
            raise Panic("Option::unwrap failed")
        return r
    x = evaluate(e[2], ty, env)
    y = evaluate(e[3], ty, env)
    if not compare(e[1], x, y):
        raise Panic("chk")
    return evaluate(e[4], ty, env)


class Function:
    """A generated function: params, `let` statements, an optional loop, a result. `helpers` are
    functions generated earlier that this one may call (same type only)."""

    def __init__(self, rng, name, ty, n_params, max_depth=3, helpers=()):
        self.name, self.ty = name, ty
        self.params = ["a", "b", "c"][:n_params]
        names = list(self.params)
        self.lets = []
        for i in range(rng.randint(0, 3)):
            v = f"x{i}"
            self.lets.append((v, gen_expr(rng, ty, names, rng.randint(1, max_depth), helpers)))
            names.append(v)
        self.loop = None
        if rng.random() < 0.2:
            body = gen_expr(rng, ty, names + ["acc"], 2, helpers)
            self.loop = (rng.randint(0, 4), gen_expr(rng, ty, names, 1, helpers), body)
            names.append("acc")
        self.result = gen_expr(rng, ty, names, rng.randint(1, max_depth), helpers)

    def cairo(self):
        ty = self.ty
        lines = [f"pub fn {self.name}({', '.join(f'{p}: {ty}' for p in self.params)}) -> {ty} {{"]
        for v, e in self.lets:
            lines.append(f"    let {v}: {ty} = {render(e, ty)};")
        if self.loop:
            k, init, body = self.loop
            lines.append(f"    let mut acc: {ty} = {render(init, ty)};")
            lines.append("    let mut i: u32 = 0;")
            lines.append(f"    while i < {k} {{")
            lines.append(f"        acc = {render(body, ty)};")
            lines.append("        i += 1;")
            lines.append("    }")
        lines.append(f"    {render(self.result, ty)}")
        lines.append("}")
        return "\n".join(lines)

    def run(self, args):
        """The result, or raises Panic, exactly as Cairo would."""
        env = dict(zip(self.params, args))
        for v, e in self.lets:
            env[v] = evaluate(e, self.ty, env)
        if self.loop:
            k, init, body = self.loop
            env["acc"] = evaluate(init, self.ty, env)
            for _ in range(k):
                env["acc"] = evaluate(body, self.ty, env)
        return evaluate(self.result, self.ty, env)

    def outcome(self, args):
        try:
            return ("ok", self.run(args))
        except Panic as p:
            return ("panic", str(p))
        except RecursionError:
            return ("panic", "recursion")
