#!/usr/bin/env python3
"""The right answer for crow check --self.

Each line of the input names a case — a builtin called, or an operator, and
the self-check's values it is given, by name — and what both modes showed
for it (they agree: a case they disagree on is a finding already). This
script computes the answer from the language spec (docs/language-spec.md),
independently of Crow's runtime, and writes one line per case it answers
differently. A call it has no reference for is skipped, never guessed.

The answers follow the spec's words, not Python's: an int divided by an int
is rounded toward zero, a remainder has the sign of the left side, a half
is rounded away from zero, a float shows as Rust shows it (a whole float as
an int, no exponent), text is quoted only inside a list or a map.

Usage: reference.py CASES.jsonl — each line {"id", "what", "args", and
"seen" (the case's own lines) with "end" (how it stopped, or ""), or
"refused" (what refused it)}. Prints a JSON line per case answered
otherwise (`kind` "wrong"), per refused case it answers (`kind` "refused":
a refusal may be the spec's), then `{"answered": n, "skipped": m}`.
"""

import json
import math
import sys
import warnings
from decimal import Decimal

warnings.simplefilter("ignore")  # (numpy's warnings on an empty mean: the answer is NaN, as Crow's)

try:  # (the spec holds math's reductions to numpy's; without it they are not asked)
    import numpy
except ImportError:  # pragma: no cover
    numpy = None


class Err(Exception):
    """The call fails: an error value or a stop, either shows as one."""


class Skip(Exception):
    """No reference for this call."""


class L(list):
    """A list."""


class T:
    """A tensor: its numbers (floats), its shape, whether written as ints."""

    def __init__(self, data, shape):
        self.data = [float(x) for x in data]
        self.shape = list(shape)

    def rows(self):
        if len(self.shape) == 1:
            return self.data
        width = self.shape[1]
        return [T(self.data[i * width:(i + 1) * width], self.shape[1:]) for i in range(self.shape[0])]


class M(dict):
    """A map, its keys in order as a BTreeMap holds them."""


class C(list):
    """A frame's column: its cells, None where one is missing."""


FAIL = object()

VALUES = {
    "int": 3,
    "int0": 0,
    "intneg": -2,
    "int99": 99,
    "intmax": 2**63 - 1,
    "big": 2**100,
    "float": 2.5,
    "float0": 0.0,
    "floatneg": -1.5,
    "text": "abc",
    "text0": "",
    "textnum": "42",
    "textkey": "a",
    "textuni": "héllo wörld",
    "bool": True,
    "boolf": False,
    "ints": L([3, 1, 2]),
    "texts": L(["b", "a", "c"]),
    "floats": L([2.5, 0.5, 1.5]),
    "bools": L([True, False, True]),
    "empty": L([]),
    "vector": T([3.0, 1.0, 2.0], [3]),
    "vectorint": T([3, 1, 2], [3]),
    "matrix": T([1.0, 2.0, 3.0, 4.0], [2, 2]),
    "map": M({"a": 1, "b": 2}),
    "maptext": M({"a": "x", "b": "y"}),
    "mapint": M({1: 10, 2: 20}),
    "colnames": L(["a", "b"]),
    # the self-check's frame: a = [1, 2, 3], b = ['x', 'y', 'z'],
    # on = [true, false, true], f = [1.5, 2.5, 3.5]; and a left join's
    # column with a missing cell
    "colint": C([1, 2, 3]),
    "coltext": C(["x", "y", "z"]),
    "colbool": C([True, False, True]),
    "colfloat": C([1.5, 2.5, 3.5]),
    "colmissing": C([5, None, 7]),
    "maskcmp": C([False, True, True]),
    "scheme": "base64",
    "thousands": 1234567.891,
    "fraction8": 0.12345678,
    "whole7": 1234567,
}


def value(name):
    if name.startswith("fail:"):
        return FAIL
    if name.startswith("may:"):
        name = name[4:]
    if name not in VALUES:
        raise Skip()
    return VALUES[name]


# ── how a value shows ───────────────────────────────────────────────


def rust_float(x):
    """A float as Rust's Display writes it: the shortest digits that read
    back, never an exponent, a whole float without its point."""
    if x != x:
        return "NaN"
    if x == math.inf:
        return "inf"
    if x == -math.inf:
        return "-inf"
    r = repr(x)
    s = format(Decimal(r), "f") if ("e" in r or "E" in r) else r
    if s.endswith(".0"):
        s = s[:-2]
    return s


def show(v, top=True):
    if v is FAIL:
        raise Err()
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, int):
        return str(v)
    if isinstance(v, float):
        return rust_float(v)
    if isinstance(v, str):
        return v if top else "'" + v + "'"
    if isinstance(v, C):
        return "[" + ", ".join("error: missing" if x is None else show(x, False) for x in v) + "]"
    if isinstance(v, L):
        return "[" + ", ".join(show(x, False) for x in v) + "]"
    if isinstance(v, T):
        if len(v.shape) == 1:
            return "[" + ", ".join(rust_float(x) for x in v.data) + "]"
        return "[" + ", ".join(show(r, False) for r in v.rows()) + "]"
    if isinstance(v, M):
        return "[" + ", ".join("{} = {}".format(k, show(v[k], False)) for k in sorted(v)) + "]"
    if isinstance(v, Dec):
        return format(v.d, "f")
    raise Skip()


# ── the kinds, as the spec reads them ───────────────────────────────


def is_int(x):
    return isinstance(x, int) and not isinstance(x, bool)


def is_num(x):
    return is_int(x) or isinstance(x, float)


def numbers(x):
    """A list of numbers or a tensor's numbers, or None."""
    if isinstance(x, T):
        return x.data
    if isinstance(x, L) and all(is_num(i) for i in x):
        return [float(i) for i in x]
    return None


def trunc_div(a, b):
    if b == 0:
        raise Err()
    q = abs(a) // abs(b)
    return q if (a >= 0) == (b >= 0) else -q


def rem(a, b):
    if b == 0:
        raise Err()
    return a - b * trunc_div(a, b)


def round_half_away(x):
    if x != x:
        return 0
    if x in (math.inf, -math.inf):
        raise Err()
    return int(math.floor(abs(x) + 0.5)) * (1 if x >= 0 else -1)


def position(i, n):
    if not is_int(i):
        raise Skip()
    small(i)
    j = i + n if i < 0 else i
    if j < 0 or j >= n:
        raise Err()
    return j


def small(i):
    """A position, a size or a count: an int of 64 bits, or the program stops."""
    if is_int(i) and not -(2**63) <= i < 2**63:
        raise Err()
    return i


def clamp(i, n):
    j = i + n if i < 0 else i
    return max(0, min(n, j))


# ── the builtins, by name ───────────────────────────────────────────


def f_len(x):
    if isinstance(x, (str, L, M)):
        return len(x)
    if isinstance(x, T):
        return x.shape[0]
    raise Skip()


def one_kind(items):
    """Whether a list may hold these: one kind, ints and floats one."""
    return all(same_kind(items[0], x) for x in items[1:]) if items else True


def f_push(xs, v):
    if isinstance(xs, L):
        items = list(xs) + [v]
        if not one_kind(items):
            raise Skip()
        if any(isinstance(i, float) for i in items) and all(is_num(i) for i in items):
            items = [float(i) for i in items]
        return L(items)
    if isinstance(xs, T) and len(xs.shape) == 1 and is_num(v):
        return T(xs.data + [float(v)], [xs.shape[0] + 1])
    raise Skip()


def tensor_of_rows(rows, shape):
    """A tensor of these items (numbers, or rows), the rest of its shape kept."""
    if len(shape) == 1:
        return T(rows, [len(rows)])
    return T([x for r in rows for x in r.data], [len(rows)] + shape[1:])


def f_pop(xs):
    if isinstance(xs, L):
        return L(xs[:-1])
    if isinstance(xs, T):
        rows = xs.data if len(xs.shape) == 1 else xs.rows()
        return tensor_of_rows(rows[:-1], xs.shape)
    raise Skip()


def f_concat(a, b, *rest):
    if rest:
        raise Skip()
    if isinstance(a, str) and isinstance(b, str):
        return a + b
    if isinstance(a, L) and isinstance(b, L):
        return f_push_all(a, b)
    if isinstance(a, T) and isinstance(b, T) and len(a.shape) != len(b.shape):
        raise Err()
    na, nb = numbers(a), numbers(b)
    if na is not None and nb is not None:
        if isinstance(a, T) and len(a.shape) == 1:
            return T(na + nb, [len(na) + len(nb)])
        if isinstance(a, L):
            return L(list(a) + nb)
    raise Skip()


def f_push_all(a, b):
    items = list(a) + list(b)
    if not one_kind(items):
        raise Skip()
    if any(isinstance(i, float) for i in items) and all(is_num(i) for i in items):
        items = [float(i) for i in items]
    return L(items)


def sort_key(x):
    if isinstance(x, float) and x != x:
        return (1, 0.0)
    return (0, x)


def f_sort(xs):
    if isinstance(xs, L) and all(is_num(i) for i in xs):
        return L(sorted(xs, key=sort_key))
    if isinstance(xs, L) and all(isinstance(i, str) for i in xs):
        return L(sorted(xs, key=lambda s: s.encode("utf-8")))
    if isinstance(xs, T) and len(xs.shape) == 1:
        return T(sorted(xs.data, key=sort_key), xs.shape)
    # (a matrix's rows have no order: refused)
    raise Skip()


def f_reverse(x):
    if isinstance(x, str):
        return x[::-1]
    if isinstance(x, L):
        return L(reversed(x))
    if isinstance(x, T):
        rows = x.data if len(x.shape) == 1 else x.rows()
        return tensor_of_rows(list(reversed(rows)), x.shape)
    raise Skip()


def equal(a, b):
    if is_num(a) and is_num(b):
        return float(a) == float(b) if (isinstance(a, float) or isinstance(b, float)) else a == b
    if type(a) is not type(b):
        raise Skip()
    return a == b


def f_contains(coll, item):
    if isinstance(coll, str) and isinstance(item, str):
        return item in coll
    if isinstance(coll, L):
        return any(equal(x, item) for x in coll)
    if isinstance(coll, M):
        return item in coll
    if isinstance(coll, T) and len(coll.shape) == 1 and is_num(item):
        return float(item) in coll.data
    raise Skip()


def f_slice(x, start, end):
    if not (is_int(start) and is_int(end)):
        raise Skip()
    small(start)
    small(end)
    if isinstance(x, (str, L)):
        n = len(x)
        s, e = clamp(start, n), clamp(end, n)
        part = x[s:e] if s < e else x[0:0]
        return L(part) if isinstance(x, L) else part
    if isinstance(x, T):
        n = x.shape[0]
        s, e = clamp(start, n), clamp(end, n)
        rows = x.data if len(x.shape) == 1 else x.rows()
        return tensor_of_rows(rows[s:e] if s < e else [], x.shape)
    raise Skip()


def f_range(*args):
    if not all(is_int(a) for a in args):
        raise Skip()
    for a in args:
        small(a)
    if len(args) == 1:
        start, stop, step = 0, args[0], 1
    elif len(args) == 2:
        start, stop, step = args[0], args[1], 1
    else:
        start, stop, step = args
    if step == 0:
        raise Err()
    count = max(0, -((start - stop) // step)) if step > 0 else max(0, -((stop - start) // -step))
    if count > 2**36:
        raise Err()
    return L(range(start, stop, step))


def f_keys(m):
    if isinstance(m, M):
        return L(sorted(m))
    raise Skip()


def f_values(m):
    if isinstance(m, M):
        return L(m[k] for k in sorted(m))
    raise Skip()


def key_fits(m, k):
    """Whether `k` is a key of the map's kind (text or ints)."""
    if not m:
        return isinstance(k, str) or is_int(k)
    if all(isinstance(x, str) for x in m):
        return isinstance(k, str)
    return is_int(k)


def f_set(m, k, v):
    if isinstance(m, M) and key_fits(m, k) and (not m or same_kind(next(iter(m.values())), v)):
        out = M(m)
        out[k] = v
        if any(isinstance(x, float) for x in out.values()):
            out = M({a: float(b) for a, b in out.items()})
        return out
    raise Skip()


def f_del(m, k):
    if isinstance(m, M) and key_fits(m, k) and k in m:
        out = M(m)
        del out[k]
        return out
    raise Skip()


def f_abs(x):
    if is_num(x):
        return abs(x)
    raise Skip()


def f_sqrt(x):
    if is_num(x):
        return math.sqrt(x) if x >= 0 else math.nan
    raise Skip()


def f_floor(x):
    if is_int(x):
        return x
    if isinstance(x, float):
        if x != x:
            return 0
        if x in (math.inf, -math.inf):
            raise Err()
        return math.floor(x)
    raise Skip()


def f_ceil(x):
    if is_int(x):
        return x
    if isinstance(x, float):
        if x != x:
            return 0
        if x in (math.inf, -math.inf):
            raise Err()
        return math.ceil(x)
    raise Skip()


def f_round(x, *rest):
    if rest:
        raise Skip()
    if is_int(x):
        return x
    if isinstance(x, float):
        return round_half_away(x)
    raise Skip()


def f_minmax(pick):
    def f(a, b):
        if is_num(a) and is_num(b):
            r = pick(a, b)
            return float(r) if isinstance(a, float) or isinstance(b, float) else r
        raise Skip()

    return f


def column_of(f, x):
    """A conversion over a frame's column: each cell converted, one that
    cannot be (or is missing) missing (spec, Frames)."""
    out = []
    for c in x:
        try:
            out.append(None if c is None else f(c))
        except Err:
            out.append(None)
    return C(out)


def f_int(x):
    if isinstance(x, C):
        return column_of(f_int, x)
    if isinstance(x, bool):
        return 1 if x else 0
    if is_int(x):
        return x
    if isinstance(x, float):
        if x != x:
            return 0
        if x in (math.inf, -math.inf):
            raise Err()
        return int(x)
    if isinstance(x, str):
        body = x[1:] if x[:1] in "+-" else x
        if body and body.isascii() and body.isdigit():
            return int(x)
        raise Err()
    raise Skip()


def f_float(x):
    if isinstance(x, C):
        return column_of(f_float, x)
    if isinstance(x, bool):
        return 1.0 if x else 0.0
    if is_num(x):
        return float(x)
    if isinstance(x, str):
        # (text as Rust reads a float: digits, a point, an exponent; the
        # catalog's texts are a number or none)
        body = x[1:] if x[:1] in "+-" else x
        if body and all(ch.isdigit() or ch == "." for ch in body) and body.count(".") <= 1 and body != ".":
            return float(x)
        raise Err()
    raise Skip()


class Dec:
    """A decimal: exact base ten, its places kept (spec, Numbers That Keep
    Every Digit); shown as written."""

    def __init__(self, d):
        self.d = d


def f_decimal(x, *rest):
    if rest:
        raise Skip()
    if isinstance(x, bool):
        raise Skip()
    if is_int(x):
        return Dec(Decimal(x))
    if isinstance(x, float):
        if x != x or x in (math.inf, -math.inf):
            raise Skip()
        return Dec(Decimal(rust_float(x)))
    if isinstance(x, str):
        body = x[1:] if x[:1] in "+-" else x
        if body and all(ch.isdigit() or ch == "." for ch in body) and body.count(".") <= 1 and body != ".":
            return Dec(Decimal(x))
        raise Err()
    raise Skip()


def f_join(xs, sep):
    if not isinstance(sep, str):
        raise Skip()
    if isinstance(xs, C):
        if any(c is None for c in xs):
            raise Err()
        return sep.join(show(c, True) for c in xs)
    if isinstance(xs, T):
        if len(xs.shape) != 1:
            raise Skip()
        return sep.join(rust_float(v) for v in xs.data)
    if isinstance(xs, L) and all(isinstance(v, (str, bool, int, float)) for v in xs):
        return sep.join(show(v, True) for v in xs)
    raise Skip()


def f_str(x):
    if isinstance(x, C):
        return C(None if c is None else show(c, True) for c in x)
    return show(x, True)


def f_upper(s):
    if isinstance(s, str):
        return s.upper()
    raise Skip()


def f_lower(s):
    if isinstance(s, str):
        return s.lower()
    raise Skip()


def f_trim(s):
    if isinstance(s, str):
        return s.strip()
    raise Skip()


def f_split(s, d):
    if isinstance(s, str) and isinstance(d, str):
        if d == "":
            # (the spec does not say what an empty separator answers)
            raise Skip()
        return L(s.split(d))
    raise Skip()





def f_replace(s, old, new):
    if all(isinstance(a, str) for a in (s, old, new)):
        return s.replace(old, new)
    raise Skip()


def f_starts(s, p):
    if isinstance(s, str) and isinstance(p, str):
        return s.startswith(p)
    raise Skip()


def f_ends(s, p):
    if isinstance(s, str) and isinstance(p, str):
        return s.endswith(p)
    raise Skip()


def f_repeat(s, n):
    if isinstance(s, str) and is_int(n):
        small(n)
        if n < 0 or len(s) * n > 2**36:
            raise Err()
        return s * n
    raise Skip()


def f_occurs(s, p):
    if isinstance(s, str) and isinstance(p, str):
        if p == "":
            raise Err()
        return s.count(p)
    raise Skip()


def f_translate(s, a, b):
    if all(isinstance(x, str) for x in (s, a, b)):
        if len(a) != len(b):
            raise Err()
        table = {}
        for x, y in zip(a, b):
            table.setdefault(x, y)
        return "".join(table.get(c, c) for c in s)
    raise Skip()


def f_has(m, k):
    if isinstance(m, M) and key_fits(m, k):
        return k in m
    raise Skip()


# ── the math module: number by number, and reduced ──────────────────


def libm(f):
    """A function of a float as Rust's (the system's libm) answers it: a
    domain it does not cover NaN, past the largest float an infinity."""
    def g(x):
        x = float(x)
        if x != x:
            return math.nan
        try:
            return f(x)
        except ValueError:
            return math.nan
        except OverflowError:
            return math.inf
    return g


def ln(x):
    if x == 0.0:
        return -math.inf
    if x == math.inf:
        return math.inf
    return math.log(x)


def log2(x):
    if x == 0.0:
        return -math.inf
    if x == math.inf:
        return math.inf
    return math.log2(x)


def log10(x):
    if x == 0.0:
        return -math.inf
    if x == math.inf:
        return math.inf
    return math.log10(x)


def sinh(x):
    try:
        return math.sinh(x)
    except OverflowError:
        return math.copysign(math.inf, x)


def sign(x):
    return 0.0 if x == 0 else math.copysign(1.0, x)


UNARY = {
    "exp": libm(math.exp),
    "ln": libm(ln),
    "log2": libm(log2),
    "log10": libm(log10),
    "sin": libm(math.sin),
    "cos": libm(math.cos),
    "tan": libm(math.tan),
    "asin": libm(math.asin),
    "acos": libm(math.acos),
    "atan": libm(math.atan),
    "sinh": libm(sinh),
    "cosh": libm(math.cosh),
    "tanh": libm(math.tanh),
    "sqrt": libm(lambda x: math.sqrt(x) if x >= 0 else math.nan),
    "erf": libm(math.erf),
}


class Near(str):
    """An answer whose numbers are compared within a few units in the last
    place: a function Crow computes itself rather than through libm."""


def math_unary(name):
    f = UNARY[name]
    def g(x):
        if isinstance(x, bool):
            raise Skip()
        if is_num(x):
            return f(x)
        t = as_tensor(x)
        if t is None:
            raise Skip()
        return T([f(v) for v in t.data], t.shape)
    return g


def reduced(name):
    def g(x):
        if numpy is None or isinstance(x, bool):
            raise Skip()
        if isinstance(x, L) and x and all(isinstance(b, bool) for b in x):
            x = T([1.0 if b else 0.0 for b in x], [len(x)])
        t = as_tensor(x) if not (isinstance(x, L) and not x) else T([], [0])
        if t is None:
            raise Skip()
        a = numpy.array(t.data, dtype=numpy.float64).reshape(t.shape)
        if name in ("min", "max") and a.size == 0:
            raise Skip()  # (an empty tensor's least and greatest: a question for the spec)
        with numpy.errstate(all="ignore"):
            r = {"sum": numpy.sum, "mean": numpy.mean, "prod": numpy.prod, "min": numpy.min, "max": numpy.max,
                 "norm": lambda v: numpy.linalg.norm(v.ravel())}[name](a)
        r = float(r)
        if name in ("min", "max") and r != r:
            raise Skip()  # (numpy's NaN against Crow's: a question for the spec)
        return r
    return g


def f_linspace(a, b, n):
    if not (is_num(a) and is_num(b) and is_int(n)) or numpy is None or n < 0 or n > 1000:
        raise Skip()
    return T([float(v) for v in numpy.linspace(float(a), float(b), n)], [n])


def f_arange(a, b, step):
    if not all(is_num(v) and not isinstance(v, bool) for v in (a, b, step)) or numpy is None:
        raise Skip()
    if step == 0:
        raise Err()
    count = (float(b) - float(a)) / float(step)
    if count != count or abs(count) > 1000:
        raise Skip()
    v = numpy.arange(float(a), float(b), float(step))
    if v.size > 1000:
        raise Skip()
    return T([float(x) for x in v], [v.size])


MATH = {"math." + k: math_unary(k) for k in UNARY}
MATH.update({"math." + k: reduced(k) for k in ("sum", "mean", "prod", "min", "max", "norm")})
MATH["math.linspace"] = f_linspace
MATH["math.arange"] = f_arange


BUILTINS = {
    "len": f_len,
    "push": f_push,
    "pop": f_pop,
    "concat": f_concat,
    "sort": f_sort,
    "reverse": f_reverse,
    "contains": f_contains,
    "slice": f_slice,
    "range": f_range,
    "keys": f_keys,
    "values": f_values,
    "set": f_set,
    "del": f_del,
    "has": f_has,
    "abs": f_abs,
    "sqrt": f_sqrt,
    "floor": f_floor,
    "ceil": f_ceil,
    "round": f_round,
    "min": f_minmax(min),
    "max": f_minmax(max),
    "int": f_int,
    "decimal": f_decimal,
    "float": f_float,
    "str": f_str,
    "upper": f_upper,
    "lower": f_lower,
    "trim": f_trim,
    "split": f_split,
    "join": f_join,
    "replace": f_replace,
    "starts": f_starts,
    "ends": f_ends,
    "repeat": f_repeat,
    "occurs": f_occurs,
    "translate": f_translate,
}
BUILTINS.update(MATH)


def f_type(x):
    if isinstance(x, bool):
        return "bool"
    if is_int(x):
        return "int"
    if isinstance(x, float):
        return "float"
    if isinstance(x, str):
        return "str"
    if isinstance(x, (L, C, T)):
        return "array"
    if isinstance(x, M):
        return "map"
    raise Skip()


def items(x):
    """What a loop over the value walks, or Skip: a map, text or a number
    has no items (refused before the program runs)."""
    if isinstance(x, C):
        if any(c is None for c in x):
            raise Skip()
        return list(x)
    if isinstance(x, L):
        return list(x)
    if isinstance(x, T):
        return x.data if len(x.shape) == 1 else x.rows()
    raise Skip()


def is_mask(i):
    return isinstance(i, (L, C)) and len(i) > 0 and all(isinstance(b, bool) for b in i)


def f_index(b, i):
    if isinstance(i, bool):
        raise Skip()
    if isinstance(b, (str, L, C, T)):
        n = b.shape[0] if isinstance(b, T) else len(b)
        if is_int(i):
            j = position(i, n)
            if isinstance(b, T):
                return b.rows()[j] if len(b.shape) > 1 else b.data[j]
            if isinstance(b, C) and b[j] is None:
                raise Err()
            return b[j]
        if is_mask(i) and not isinstance(b, str):
            if len(i) != n:
                raise Err()
            if isinstance(b, T):
                picked = [r for r, keep in zip(b.rows() if len(b.shape) > 1 else b.data, i) if keep]
                if len(b.shape) == 1:
                    return T(picked, [len(picked)])
                return T([x for r in picked for x in r.data], [len(picked)] + b.shape[1:])
            kept = [x for x, keep in zip(b, i) if keep]
            return C(kept) if isinstance(b, C) else L(kept)
        raise Skip()
    if isinstance(b, M):
        if not b:
            raise Skip()
        text_keys = all(isinstance(k, str) for k in b)
        if (text_keys and not isinstance(i, str)) or (not text_keys and not is_int(i)):
            raise Skip()
        if i not in b:
            raise Err()
        return b[i]
    raise Skip()


def same_kind(old, new):
    """Whether a list holding `old` items takes `new`, and as what."""
    if is_num(old) and is_num(new):
        return True
    return type(old) is type(new) and not isinstance(old, (L, T, M, C))


def f_write(b, i, x):
    if isinstance(i, bool) or isinstance(x, bool) and not isinstance(b, (L, M)):
        raise Skip()
    if isinstance(b, L):
        if not is_int(i) or not b or not same_kind(b[0], x):
            raise Skip()
        j = position(i, len(b))
        out = list(b)
        out[j] = x
        if any(isinstance(v, float) for v in out):
            out = [float(v) for v in out]
        return L(out)
    if isinstance(b, T):
        if not is_int(i):
            raise Skip()
        j = position(i, b.shape[0])
        if len(b.shape) == 1:
            if not is_num(x):
                raise Skip()
            data = list(b.data)
            data[j] = float(x)
            return T(data, b.shape)
        row = numbers(x)
        if row is None or len(b.shape) != 2 or len(row) != b.shape[1]:
            raise Skip()
        data = list(b.data)
        data[j * b.shape[1]:(j + 1) * b.shape[1]] = row
        return T(data, b.shape)
    if isinstance(b, M):
        text_keys = all(isinstance(k, str) for k in b)
        if (text_keys and not isinstance(i, str)) or (not text_keys and not is_int(i)):
            raise Skip()
        old = next(iter(b.values()))
        if not same_kind(old, x):
            raise Skip()
        out = M(b)
        out[i] = x
        if any(isinstance(v, float) for v in out.values()):
            out = M({k: float(v) for k, v in out.items()})
        return out
    raise Skip()


def f_fmt(x, options):
    """`fmt[x, …]` by the spec's table (Formatting): fixed decimals, a
    thousands separator, a percent (times 100, then `%`), a width filled
    with `fill` — numbers to the right, text to the left unless `align`
    says."""
    opts = {}
    for part in options.split(", "):
        k, v = part.split(" = ")
        opts[k] = v.strip("'")
    decimals = int(opts["decimals"]) if "decimals" in opts else None
    if isinstance(x, bool):
        raise Skip()
    if isinstance(x, str):
        if decimals is not None or "percent" in opts or "commas" in opts:
            raise Err()
        body = x
        right = opts.get("align") == "right"
    elif is_num(x):
        if is_int(x) and not -(2**63) <= x < 2**63:
            raise Skip()
        if opts.get("percent") == "true":
            x = x * 100
        commas = opts.get("commas") == "true"
        if decimals is not None:
            body = format(float(x) if isinstance(x, float) else x, ("," if commas else "") + "." + str(decimals) + "f")
        elif isinstance(x, float):
            body = rust_float(x)
            if commas:
                if "e" in body or "inf" in body or "NaN" in body:
                    raise Skip()
                whole, _, frac = body.partition(".")
                sign = "-" if whole.startswith("-") else ""
                whole = format(int(whole.lstrip("-")), ",")
                body = sign + whole + ("." + frac if frac else "")
        else:
            body = format(x, "," if commas else "d")
        if opts.get("percent") == "true":
            body += "%"
        right = opts.get("align") != "left"
    else:
        raise Skip()
    width = int(opts.get("width", "0"))
    fill = opts.get("fill", " ")
    if len(fill) != 1:
        raise Skip()
    if fill != " " and body.startswith("-"):
        raise Skip()  # (a fill before a sign: not said)
    pad = fill * max(0, width - len(body))
    return pad + body if right else body + pad


# ── the operators ───────────────────────────────────────────────────


def fpow(x, y):
    """A float to a power, as Rust's powf answers it: past the largest
    float an infinity, a negative number to a fraction NaN."""
    try:
        return math.pow(x, y)
    except OverflowError:
        odd = float(y).is_integer() and int(y) % 2 == 1
        return -math.inf if (x < 0 and odd) else math.inf
    except ValueError:
        return math.nan


def arith(op, a, b):
    if isinstance(a, bool) or isinstance(b, bool):
        raise Skip()
    if is_num(a) and is_num(b):
        if is_int(a) and is_int(b):
            if op == "+":
                return a + b
            if op == "-":
                return a - b
            if op == "*":
                return a * b
            if op == "/":
                return trunc_div(a, b)
            if op == "%":
                return rem(a, b)
            if op == "^":
                if b < 0:
                    raise Err()
                if abs(a) > 1 and b > 100000:
                    raise Skip()
                return a**b
        x, y = float(a), float(b)
        if op == "+":
            return x + y
        if op == "-":
            return x - y
        if op == "*":
            return x * y
        if op == "/":
            if y == 0.0:
                return math.copysign(math.inf, x) * (1 if math.copysign(1.0, y) > 0 else -1) if x != 0 else math.nan
            return x / y
        if op == "^":
            return fpow(x, y)
    raise Skip()


def compare(op, a, b):
    if is_num(a) and is_num(b):
        x, y = (float(a), float(b)) if (isinstance(a, float) or isinstance(b, float)) else (a, b)
    elif isinstance(a, str) and isinstance(b, str):
        x, y = a.encode("utf-8"), b.encode("utf-8")
    elif isinstance(a, bool) and isinstance(b, bool) and op in ("eq", "ne"):
        x, y = a, b
    else:
        raise Skip()
    return {"eq": x == y, "ne": x != y, "gt": x > y, "lt": x < y, "gte": x >= y, "lte": x <= y}[op]


# ── numbers held together: a tensor, a list of numbers, a column ────


def as_tensor(x):
    """A tensor of the value's numbers where arithmetic takes it as one (a
    tensor, a list of numbers, a frame's column of numbers — whose missing
    cell stops the program, as one of text or bools does), or None."""
    if isinstance(x, T):
        return x
    if isinstance(x, C):
        if any(c is None for c in x) or not all(is_num(c) for c in x):
            raise Err()
        return T(x, [len(x)])
    if isinstance(x, L) and x and all(is_num(i) for i in x):
        return T(x, [len(x)])
    return None


def broadcast(a, b):
    """The shape two shapes make, aligned from the innermost: each pair
    equal or one of them 1; otherwise the program stops."""
    n = max(len(a), len(b))
    a = [1] * (n - len(a)) + a
    b = [1] * (n - len(b)) + b
    out = []
    for x, y in zip(a, b):
        if x != y and x != 1 and y != 1:
            raise Err()
        out.append(max(x, y))
    return out


def at(t, shape, k):
    """The number of tensor `t` at flat position `k` of `shape`, broadcast."""
    idx = []
    for d in reversed(shape):
        idx.append(k % d)
        k //= d
    idx = list(reversed(idx))[len(shape) - len(t.shape):]
    flat = 0
    for i, d in zip(idx, t.shape):
        flat = flat * d + (0 if d == 1 else i)
    return t.data[flat]


def elementwise(f, a, b):
    """`f` of two tensors, or a tensor and a number, number by number."""
    if is_num(a):
        a = T([a], [])
    if is_num(b):
        b = T([b], [])
    shape = broadcast(a.shape, b.shape)
    n = 1
    for d in shape:
        n *= d
    return T([f(at(a, shape, k), at(b, shape, k)) for k in range(n)], shape)


def fdiv(x, y):
    """A float divided, as IEEE divides: by zero an infinity, 0/0 NaN."""
    if y == 0.0:
        if x == 0.0 or x != x:
            return math.nan
        return math.copysign(math.inf, x) * math.copysign(1.0, y)
    return x / y


FLOAT_OPS = {
    "+": lambda x, y: x + y,
    "-": lambda x, y: x - y,
    ".*": lambda x, y: x * y,
    "./": fdiv,
    ".^": lambda x, y: fpow(x, y),
}


def tensor_arith(op, a, b):
    ta, tb = as_tensor(a), as_tensor(b)
    if isinstance(a, bool) or isinstance(b, bool):
        raise Skip()
    if ta is None and not is_num(a) or tb is None and not is_num(b):
        raise Skip()
    x = ta if ta is not None else float(a)
    y = tb if tb is not None else float(b)
    if op in FLOAT_OPS:
        return elementwise(FLOAT_OPS[op], x, y)
    if op == "*":
        if ta is not None and tb is not None:
            # matrix multiply, of two matrices whose inner sizes meet
            if len(ta.shape) != 2 or len(tb.shape) != 2:
                raise Skip()
            (n, k), (k2, m) = ta.shape, tb.shape
            if k != k2:
                raise Err()
            ra, rb = ta.rows(), tb.rows()
            data = []
            for i in range(n):
                for j in range(m):
                    total = 0.0
                    for q in range(k):
                        total += ra[i].data[q] * rb[q].data[j]
                    data.append(total)
            return T(data, [n, m])
        return elementwise(lambda p, q: p * q, x, y)
    if op == "/":
        if ta is not None and tb is not None:
            raise Skip()
        return elementwise(fdiv, x, y)
    raise Skip()


def mask(t):
    return L(t)


def compare_items(op, a, b):
    """A list, a tensor or a column compared with one value: a mask, item
    by item — a missing cell false for every operator."""
    coll, one, flip = (a, b, False) if not is_num(a) and not isinstance(a, str) else (b, a, True)
    if isinstance(coll, T):
        if len(coll.shape) != 1:
            raise Skip()
        cells = coll.data
    elif isinstance(coll, (L, C)):
        cells = list(coll)
    else:
        raise Skip()
    out = []
    for c in cells:
        if c is None:
            out.append(False)
            continue
        out.append(compare(op, one, c) if flip else compare(op, c, one))
    return L(out)


def whole_equal(a, b):
    """Two lists, tensors or columns compared whole: the same items in the
    same shape (a missing cell equal to nothing)."""
    def cells(x):
        if isinstance(x, T):
            return x.shape, x.data
        return [len(x)], list(x)
    (sa, ca), (sb, cb) = cells(a), cells(b)
    if len(sa) != len(sb):
        # (a list of numbers and a list of lists are two kinds: refused)
        raise Skip()
    if sa != sb:
        return False
    for x, y in zip(ca, cb):
        if x is None or y is None:
            return False
        if (is_num(x) and is_num(y)) or type(x) is type(y):
            if not equal(x, y):
                return False
        else:
            raise Skip()
    return True


def operator(op, args):
    if op in ("neg",):
        (a,) = args
        if is_num(a):
            return -a
        t = as_tensor(a) if not isinstance(a, bool) else None
        if t is not None:
            return T([-x for x in t.data], t.shape)
        raise Skip()
    if op in ("not",):
        (a,) = args
        if isinstance(a, bool):
            return not a
        raise Skip()
    a, b = args
    collection = lambda x: isinstance(x, (L, C, T))
    if op in ("+", "-", "*", "/", "%", "^", ".*", "./", ".^"):
        if collection(a) or collection(b):
            return tensor_arith(op, a, b)
        if op in (".*", "./", ".^"):
            raise Skip()
        return arith(op, a, b)
    if op in ("eq", "ne", "gt", "lt", "gte", "lte"):
        if collection(a) and collection(b):
            if op not in ("eq", "ne"):
                raise Skip()
            same = whole_equal(a, b)
            return same if op == "eq" else not same
        if collection(a) or collection(b):
            return compare_items(op, a, b)
        return compare(op, a, b)
    if op in ("and", "or"):
        if isinstance(a, bool) and isinstance(b, bool):
            return (a and b) if op == "and" else (a or b)
        raise Skip()
    raise Skip()


# ── the cases ───────────────────────────────────────────────────────


def answer(what, names):
    args = [value(n) for n in names]
    if what == "type" and args[0] is FAIL:
        return "error"
    # (what is shown of a failed value names its line, and `str` of one is
    # not said: not answered here)
    if what in ("form:show", "form:spliced", "form:for", "form:collect", "form:count", "str") and any(a is FAIL for a in args):
        raise Skip()
    # a value that failed is the call's value, as a call hands it on
    known = what in ("form:index", "form:write", "type") or what.startswith("op:") or what in BUILTINS
    if any(a is FAIL for a in args):
        if known:
            raise Err()
        raise Skip()
    # (the values are written into the case as they are: `2 ^ 100 ^ y` is
    # `2 ^ (100 ^ y)`, as `^` groups from the right)
    if what == "op:^" and names[0] == "big":
        b = args[1]
        if is_int(b) and not isinstance(b, bool) and 0 <= b <= 3:
            return show(2 ** (100**b), True)
        if isinstance(b, float):
            return show(fpow(2.0, fpow(100.0, b)), True)
        raise Skip()
    if what == "op:^" and len(names) > 1 and names[1] == "big":
        raise Skip()
    # (`2 ^ 100 .^ y` is `2 ^ (100 .^ y)`, a power of a tensor: refused)
    if what in ("op:.^", "op:.*", "op:./") and names[0] == "big":
        if what == "op:.^":
            raise Skip()
    if what == "form:show":
        (a,) = args
        return show(a, True)
    if what.startswith("fmt:"):
        return f_fmt(args[0], what[4:])
    if what == "type":
        return f_type(args[0])
    if what == "form:spliced":
        return "v=" + show(args[0], True)
    if what == "form:for":
        return "\n".join(show(x, True) for x in items(args[0]))
    if what == "form:collect":
        return show(L(items(args[0])), True)
    if what == "form:count":
        return str(len(items(args[0])))
    if what == "form:index":
        return show(f_index(*args), True)
    if what == "form:write":
        return show(f_write(*args), True)
    if what.startswith("op:"):
        return show(operator(what[3:], args), True)
    f = BUILTINS.get(what)
    if what == "math.erf" and f is not None:
        return Near(show(f(*args), True))
    if f is None:
        raise Skip()
    try:
        r = f(*args)
    except TypeError:
        raise Skip()
    return show(r, True)


def failed(case):
    """Whether the case's value was a failure: a failed value shown, or a stop."""
    seen = case.get("seen", "")
    last = seen.split("\n")[-1] if seen else ""
    return case.get("end", "").startswith("stop") or last.startswith("error")


NUMBER = None


def near(seen, expected):
    """Whether two shown values are the same but for their numbers' last
    few places (8 units in the last place, relatively)."""
    import re
    pat = re.compile(r"-?(?:inf|NaN|\d+(?:\.\d+)?)")
    a, b = pat.split(seen), pat.split(expected)
    if a != b:
        return False
    for x, y in zip(pat.findall(seen), pat.findall(expected)):
        fx, fy = float(x), float(y)
        if fx != fx and fy != fy:
            continue
        if fx == fy:
            continue
        if abs(fx - fy) > 8 * sys.float_info.epsilon * max(abs(fx), abs(fy)):
            return False
    return True


def main():
    answered = skipped = 0
    with open(sys.argv[1], encoding="utf-8") as cases:
        for line in cases:
            case = json.loads(line)
            try:
                expected = answer(case["what"], case["args"])
            except Skip:
                skipped += 1
                continue
            except Err:
                expected = None
            answered += 1
            if isinstance(expected, Near) and "seen" in case and not case.get("end"):
                if not near(case["seen"], expected):
                    print(json.dumps({"kind": "wrong", "id": case["id"], "expected": str(expected), "seen": case["seen"]}))
                continue
            if "refused" in case:
                # a refusal the spec may make (a kind it does not mix); listed
                if expected is not None:
                    print(json.dumps({"kind": "refused", "id": case["id"], "expected": expected, "seen": case["refused"]}))
                continue
            if expected is None:
                if not failed(case):
                    print(json.dumps({"kind": "wrong", "id": case["id"], "expected": "a failure", "seen": case.get("seen", "")}))
                continue
            if case.get("end") or case.get("seen") != expected:
                seen = case.get("seen", "")
                if case.get("end"):
                    seen = (seen + " " if seen else "") + "[" + case["end"] + "]"
                print(json.dumps({"kind": "wrong", "id": case["id"], "expected": expected, "seen": seen}))
    print(json.dumps({"answered": answered, "skipped": skipped}))


if __name__ == "__main__":
    main()
