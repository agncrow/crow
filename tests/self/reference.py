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
"refused" (what refused it)}. Prints a JSON line per case it answers
(`kind` "answered"), per case answered otherwise (`kind` "wrong"), per
refused case it answers (`kind` "refused": a refusal may be the spec's),
per case it would answer with a library this machine lacks (`kind`
"needs"), then `{"answered": n, "skipped": m}`.
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


class Needs(Skip):
    """A reference this machine lacks the library for (numpy, scipy): the
    case is not answered here, and its builtin is not one with no answer."""

    def __init__(self, library):
        super().__init__(library)
        self.library = library


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


class J:
    """A parsed value (spec, JSON): read as the program's use implies; a
    position past its list, a negative one, or a key it lacks is an error
    of reason null; never written."""

    def __init__(self, v):
        self.v = v


def json_shown(v, top):
    if isinstance(v, dict):
        return "[" + ", ".join("{} = {}".format(k, json_shown(v[k], False)) for k in sorted(v)) + "]"
    if isinstance(v, list):
        return "[" + ", ".join(json_shown(x, False) for x in v) + "]"
    if v is None:
        raise Skip()
    return show(v, top)


MOST_MS = 8_640_000_000_000_000


class Mo:
    """A moment: milliseconds from 1970 and the offset (minutes east of UTC)
    it was written in (spec, Moments)."""

    def __init__(self, ms, offset=0):
        if abs(ms) > MOST_MS:
            raise Err()
        self.ms, self.offset = ms, offset

    def shown(self):
        import datetime
        local = self.ms + self.offset * 60_000
        secs, ms = divmod(local, 1000)
        try:
            dt = datetime.datetime(1970, 1, 1) + datetime.timedelta(seconds=secs)
        except OverflowError:
            raise Skip()
        if self.offset == 0 and local % 86_400_000 == 0:
            return dt.strftime("%Y-%m-%d") if 1000 <= dt.year <= 9999 else _skip()
        if not 1000 <= dt.year <= 9999:
            raise Skip()
        text = dt.strftime("%Y-%m-%dT%H:%M:%S")
        if ms:
            text += ".%03d" % ms
        if self.offset == 0:
            return text + "Z"
        sign = "+" if self.offset > 0 else "-"
        h, m = divmod(abs(self.offset), 60)
        return text + "%s%02d:%02d" % (sign, h, m)


def _skip():
    raise Skip()


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
    "moment": Mo(1767323045000),
    "json": J({"a": 1, "b": [1, 2], "c": "x"}),
    "jsonarr": J([1, 2, 3]),
    "tensors": L([T([1.0, 2.0], [2]), T([3.0, 4.0], [2])]),
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
    if isinstance(v, Mo):
        return v.shown()
    if isinstance(v, J):
        return json_shown(v.v, top)
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
    if isinstance(x, J):
        if isinstance(x.v, (list, dict, str)):
            return len(x.v)
        raise Err()
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
        if isinstance(a, Mo) and isinstance(b, Mo):
            return pick(a, b, key=lambda m: m.ms)
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
    if isinstance(x, Mo):
        return trunc_div(x.ms, 1000)
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
    if isinstance(x, Mo):
        return x.ms / 1000.0
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
    if isinstance(x, J):
        return json_shown(x.v, True)
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
        if numpy is None:
            raise Needs("numpy")
        if isinstance(x, bool):
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
    if not (is_num(a) and is_num(b) and is_int(n)) or n < 0 or n > 1000:
        raise Skip()
    if numpy is None:
        raise Needs("numpy")
    return T([float(v) for v in numpy.linspace(float(a), float(b), n)], [n])


def f_arange(a, b, step):
    if not all(is_num(v) and not isinstance(v, bool) for v in (a, b, step)):
        raise Skip()
    if numpy is None:
        raise Needs("numpy")
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


def f_moment(x, *rest):
    """`moment[…]`: of seconds, in UTC; of a moment, itself; of text, ISO
    8601 (the catalog's texts write none); of parts in a zone."""
    if rest:
        if len(rest) == 6 and all(is_int(v) for v in (x,) + rest[:4]) and is_num(rest[4]) and isinstance(rest[5], str):
            if rest[5] not in ("UTC", "Z") and not rest[5].startswith(("+", "-")):
                import zoneinfo
                try:
                    zoneinfo.ZoneInfo(rest[5])
                except Exception:
                    raise Err()
            raise Skip()
        if len(rest) == 1 and isinstance(rest[0], str):
            # (a zone: one it does not know is an error value)
            if rest[0] in ("", "abc", "a", "42") or " " in rest[0]:
                raise Err()
        raise Skip()
    if isinstance(x, Mo):
        return x
    if isinstance(x, bool):
        raise Skip()
    if is_num(x):
        ms = float(x) * 1000.0
        if ms != ms or abs(ms) > MOST_MS:
            raise Err()
        return Mo(int(round_half_away(ms)) if isinstance(x, float) else int(x) * 1000)
    if isinstance(x, str):
        import re
        if not re.fullmatch(r"\d{4}-\d{2}-\d{2}([T ]\d{2}:\d{2}(:\d{2}(\.\d+)?)?(Z|[+-]\d{2}:\d{2})?)?", x):
            raise Err()
        raise Skip()
    raise Skip()


def f_bucket(t, span):
    if not isinstance(t, Mo) or not is_num(span) or isinstance(span, bool):
        raise Skip()
    if span <= 0:
        raise Err()
    span_ms = float(span) * 1000.0
    if 86_400_000 % span_ms != 0:
        raise Skip()  # (a span that does not divide a day: aligned as the spec words it, not asked)
    local = t.ms + t.offset * 60_000
    day = (local // 86_400_000) * 86_400_000
    start = day + int((local - day) // span_ms * span_ms)
    return Mo(start - t.offset * 60_000, t.offset)


def f_moment_any(x, *rest):
    if isinstance(x, C) and not rest:
        out = []
        for c in x:
            try:
                out.append(None if c is None else f_moment(c))
            except Err:
                out.append(None)
        return C(out)
    return f_moment(x, *rest)


def f_fma(a, b, c):
    """a × b + c rounded once: exact in fractions, then the nearest float."""
    from fractions import Fraction
    if not all(is_num(v) and not isinstance(v, bool) for v in (a, b, c)):
        raise Skip()
    a, b, c = float(a), float(b), float(c)
    if not all(math.isfinite(v) for v in (a, b, c)):
        raise Skip()
    r = Fraction(a) * Fraction(b) + Fraction(c)
    try:
        return float(r)
    except OverflowError:
        return math.inf if r > 0 else -math.inf


def f_clip(t, lo, hi):
    tt = as_tensor(t) if not isinstance(t, bool) else None
    if tt is None or not all(is_num(v) and not isinstance(v, bool) for v in (lo, hi)) or lo > hi:
        raise Skip()
    return T([min(max(v, float(lo)), float(hi)) for v in tt.data], tt.shape)


def f_stack(xs, *rest):
    if rest or not isinstance(xs, L) or not xs or not all(isinstance(v, T) and len(v.shape) == 1 for v in xs):
        raise Skip()
    if len({v.shape[0] for v in xs}) != 1:
        raise Err()
    return T([n for v in xs for n in v.data], [len(xs), xs[0].shape[0]])


def f_mod(t, d):
    tt = as_tensor(t) if not isinstance(t, bool) else None
    if tt is None or not is_num(d) or isinstance(d, bool):
        dd = as_tensor(d) if not isinstance(d, bool) else None
        if tt is None or dd is None:
            raise Skip()
        if any(v < 0 for v in tt.data + dd.data):
            raise Skip()  # (a negative's remainder: not said)
        return elementwise(lambda x, y: math.fmod(x, y) if y != 0 else math.nan, tt, dd)
    if float(d) <= 0 or any(v < 0 for v in tt.data):
        raise Skip()
    return T([math.fmod(v, float(d)) for v in tt.data], tt.shape)


def f_where_math(cond, a, b):
    c = cond
    # (a mask — a list of bools, a column of them, a missing cell false — is
    # numbers where numbers are asked: true 1, false 0)
    if isinstance(c, (L, C)) and c and all(isinstance(v, bool) or v is None for v in c):
        c = T([1.0 if v else 0.0 for v in c], [len(c)])
    ct = as_tensor(c) if not isinstance(c, bool) else None
    # (math's `where` takes numbers held together on both sides: a single
    # number is refused, `0.0 * v` writes zeros)
    if ct is None or not all(isinstance(v, (L, C, T)) for v in (a, b)):
        raise Skip()
    at, bt = as_tensor(a), as_tensor(b)
    if at is None or bt is None:
        raise Skip()
    def pick(i, v):
        return v.data[i] if isinstance(v, T) else float(v)
    for v in (at, bt):
        if isinstance(v, T) and v.shape != ct.shape:
            raise Err()
    return T([pick(i, at) if x != 0 else pick(i, bt) for i, x in enumerate(ct.data)], ct.shape)


def f_len_json(x):
    return f_len(x)


BUILTINS["moment"] = f_moment_any
BUILTINS["bucket"] = f_bucket
BUILTINS["math.fma"] = lambda a, b, c: f_fma(a, b, c)
BUILTINS["math.clip"] = f_clip
BUILTINS["math.stack"] = f_stack
BUILTINS["math.mod"] = f_mod
BUILTINS["math.where"] = f_where_math


def f_type(x):
    if isinstance(x, J):
        return {dict: "map", list: "array", str: "str", bool: "bool", int: "int", float: "float"}.get(type(x.v), "none")
    if isinstance(x, Mo):
        return "moment"
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
    if isinstance(b, J):
        if isinstance(b.v, list) and is_int(i):
            if 0 <= i < len(b.v):
                return J(b.v[i])
            raise Err()
        if isinstance(b.v, dict) and isinstance(i, str):
            if i in b.v:
                return J(b.v[i])
            raise Err()
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


def moment_op(op, a, b):
    """A moment on or back by seconds, the seconds between two, compared by
    instant (spec, Moments)."""
    num = lambda x: is_num(x) and not isinstance(x, bool)
    if op == "+" and isinstance(a, Mo) and num(b):
        return Mo(a.ms + round_half_away(float(b) * 1000.0), a.offset)
    if op == "+" and num(a) and isinstance(b, Mo):
        return Mo(b.ms + round_half_away(float(a) * 1000.0), b.offset)
    if op == "-" and isinstance(a, Mo) and num(b):
        return Mo(a.ms - round_half_away(float(b) * 1000.0), a.offset)
    if op == "-" and isinstance(a, Mo) and isinstance(b, Mo):
        return (a.ms - b.ms) / 1000.0
    if op in ("eq", "ne", "gt", "lt", "gte", "lte") and isinstance(a, Mo) and isinstance(b, Mo):
        x, y = a.ms, b.ms
        return {"eq": x == y, "ne": x != y, "gt": x > y, "lt": x < y, "gte": x >= y, "lte": x <= y}[op]
    raise Skip()


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
    if isinstance(a, Mo) or isinstance(b, Mo):
        return moment_op(op, a, b)
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


# ── a route's answer (spec, Responses) ──────────────────────────────
#
# An answer is read as the self-check reads one (served_parity::answer_of):
# the status line, the content type and location, the body — text, or
# bytes that are not UTF-8 as `<n bytes: hex>` — the parts joined by ` | `,
# a case's answers by a line `—`. Where a case's route sits in its batch
# (`/c3`) is read past, as `/cN`.

import http as _http
import re as _re
import struct as _struct


class Exact(str):
    """A decimal or a fraction: JSON holds it as the text it shows."""


class Rec(dict):
    """A record: JSON holds it as an object of its fields."""


class Frame(list):
    """A frame: its rows, each a dict of its cells."""


ROUTE_VALUES = {
    "path": "self_text.txt",
    "csvpath": "self_data.csv",
    "jsonpath": "self_data.json",
    "dirpath": "self_dir",
    "url": "http://127.0.0.1:9/self",
    "sql": "SELECT 1 AS n",
    "decimal": Exact("1.25"),
    "fraction": Exact("1/3"),
    "record": Rec({"x": 1, "y": 2}),
    "frame": Frame([{"a": 1, "b": "x", "on": True, "f": 1.5}, {"a": 2, "b": "y", "on": False, "f": 2.5}, {"a": 3, "b": "z", "on": True, "f": 3.5}]),
}

STOPPED = "HTTP/1.1 500 Internal Server Error | content-type: text/plain | the request could not be answered — the server's log has why, as <id>"


def route_value(name):
    bare = name[4:] if name.startswith("may:") else name
    if bare in ROUTE_VALUES and not name.startswith("fail:"):
        return ROUTE_VALUES[bare]
    return value(name)


def answered(status, kinds, body):
    if isinstance(body, bytes):
        try:
            body = body.decode("utf-8")
        except UnicodeDecodeError:
            body = "<{} bytes: {}>".format(len(body), body.hex())
    return "HTTP/1.1 {} {} | {} | {}".format(status, _http.HTTPStatus(status).phrase, " | ".join(sorted(kinds)), body)


def held(v):
    """A value as JSON holds it, as `encode` writes it: an int past 64 bits
    the text of its digits, a float that is not finite null, a moment its
    ISO text, a decimal or a fraction its text, a tensor's whole numbers
    ints (a matrix a list of its rows), a map's keys text, a failed value
    null."""
    if v is FAIL:
        return None
    if isinstance(v, bool):
        return v
    if isinstance(v, int):
        return v if -2**63 <= v < 2**63 else str(v)
    if isinstance(v, float):
        return v if math.isfinite(v) else None
    if isinstance(v, Exact):
        return str(v)
    if isinstance(v, str):
        return v
    if isinstance(v, Mo):
        return v.shown()
    if isinstance(v, T):
        if len(v.shape) >= 2:
            return [held(r) for r in v.rows()]
        return [int(f) if math.isfinite(f) and f == int(f) and -2**63 <= f < 2**63 else held(f) for f in v.data]
    if isinstance(v, (L, C)):
        return [held(x) for x in v]
    if isinstance(v, (M, Rec)):
        return {str(k): held(x) for k, x in v.items()}
    if isinstance(v, J):
        return v.v
    raise Skip()


def json_text(j):
    """JSON as the runtime writes it (serde_json): compact, an object's keys
    in order, a float as its shortest digits with an exponent past 1e16 or
    under 1e-4 (`1e-7`, `1e+21`)."""
    if j is None:
        return "null"
    if j is True:
        return "true"
    if j is False:
        return "false"
    if isinstance(j, int):
        return str(j)
    if isinstance(j, float):
        r = repr(j)
        if "e" in r:
            m, e = r.split("e")
            r = "{}e{}{}".format(m, "-" if int(e) < 0 else "+", abs(int(e)))
        return r
    if isinstance(j, str):
        return json.dumps(j, ensure_ascii=False)
    if isinstance(j, list):
        return "[" + ",".join(json_text(x) for x in j) + "]"
    if isinstance(j, dict):
        return "{" + ",".join(json.dumps(k, ensure_ascii=False) + ":" + json_text(j[k]) for k in sorted(j)) + "}"
    raise Skip()


def packed(j):
    """A JSON value packed as MessagePack, each piece in its smallest form
    (the format's spec, "Formats"): a float always in 8 bytes."""
    def counted(fixed, n, small, big):
        if n <= 15:
            return bytes([fixed + n])
        return bytes([small]) + n.to_bytes(2, "big") if n < 65536 else bytes([big]) + n.to_bytes(4, "big")
    if j is None:
        return b"\xc0"
    if j is True:
        return b"\xc3"
    if j is False:
        return b"\xc2"
    if isinstance(j, int):
        if 0 <= j < 128 or -32 <= j < 0:
            return (j & 0xFF).to_bytes(1, "big")
        if j >= 0:
            for tag, n in ((0xCC, 1), (0xCD, 2), (0xCE, 4), (0xCF, 8)):
                if j < 2 ** (8 * n):
                    return bytes([tag]) + j.to_bytes(n, "big")
        for tag, n in ((0xD0, 1), (0xD1, 2), (0xD2, 4), (0xD3, 8)):
            if j >= -(2 ** (8 * n - 1)):
                return bytes([tag]) + j.to_bytes(n, "big", signed=True)
        raise Skip()
    if isinstance(j, float):
        return b"\xcb" + _struct.pack(">d", j)
    if isinstance(j, str):
        b = j.encode("utf-8")
        n = len(b)
        if n <= 31:
            return bytes([0xA0 + n]) + b
        for tag, w in ((0xD9, 1), (0xDA, 2), (0xDB, 4)):
            if n < 2 ** (8 * w):
                return bytes([tag]) + n.to_bytes(w, "big") + b
    if isinstance(j, list):
        return counted(0x90, len(j), 0xDC, 0xDD) + b"".join(packed(x) for x in j)
    if isinstance(j, dict):
        return counted(0x80, len(j), 0xDE, 0xDF) + b"".join(packed(k) + packed(j[k]) for k in sorted(j))
    raise Skip()


def read_as_text(v):
    """A body a response takes: text, a parsed value read as text (a number
    its text; anything else stops), or a refusal."""
    if isinstance(v, J):
        if isinstance(v.v, str):
            return v.v
        if isinstance(v.v, (int, float)) and not isinstance(v.v, bool):
            return json_text(v.v)
        return None
    if isinstance(v, str) and not isinstance(v, Exact):
        return v
    raise Err()


def route_answer(form, names):
    """`return F[v]` in a route's handler, asked once by GET. A kind the form
    does not take is refused, whether or not the value failed; a value
    that failed, of a kind it takes, is the handler's failure."""
    name = names[0]
    failing = name.startswith("fail:")
    v = route_value("may:" + name[5:] if failing else name)
    if form in ("text", "html", "redirect", "bytes", "respond"):
        body = read_as_text(v)
        if failing or body is None:
            return STOPPED
        if form == "text":
            return answered(200, ["content-type: text/plain"], body)
        if form == "html":
            return answered(200, ["content-type: text/html"], body)
        if form == "bytes":
            return answered(200, ["content-type: application/octet-stream"], body.encode("utf-8"))
        if form == "respond":
            return answered(201, ["content-type: text/plain"], body)
        # (a response that names no type of its own is served as text)
        return answered(302, ["content-type: text/plain", "location: " + body.strip()], "")
    if form == "json":
        # a map, a record, a list or a frame; text is parsed, and anything
        # else refused
        if isinstance(v, Frame):
            rows = [{k: held(x) for k, x in row.items()} for row in v]
            return STOPPED if failing else answered(200, ["content-type: application/json"], json_text(rows))
        # (a parsed value answers as it is)
        if isinstance(v, (L, T, M, Rec, J)):
            return STOPPED if failing else answered(200, ["content-type: application/json"], json_text(held(v)))
        raise Err()
    if form == "msgpack":
        if isinstance(v, (Frame, C)):
            raise Err()
        body = packed(held(v))
        return STOPPED if failing else answered(200, ["content-type: application/msgpack"], body)
    if form == "arrow":
        # a frame as an Arrow IPC stream, read back by pyarrow: its columns,
        # in order, hold the frame's cells
        if not isinstance(v, Frame):
            raise Err()
        if failing:
            return STOPPED
        return ArrowOf(v)
    if form == "stream":
        # (the feed puts the value's text, then a bar)
        try:
            shown = f_str(v)
        except (Err, TypeError):
            raise Skip()
        if not isinstance(shown, str):
            # (`str` of a column is a column of text, which put does not take)
            raise Err()
        return STOPPED if failing else answered(200, ["content-type: text/plain"], shown + "|")
    raise Skip()


class ArrowOf(str):
    """An answer whose body is a frame's Arrow IPC stream: held to the frame
    by reading the bytes back (pyarrow's reader), since a writer's padding
    and metadata are its own."""

    def __new__(cls, frame):
        try:
            import pyarrow  # noqa: F401
        except ImportError:
            raise Needs("pyarrow")
        self = str.__new__(cls, "the frame's Arrow stream")
        self.frame = frame
        return self

    def holds(self, seen):
        import pyarrow.ipc
        head = "HTTP/1.1 200 OK | content-type: application/vnd.apache.arrow.stream | "
        m = _re.fullmatch(_re.escape(head) + r"<(\d+) bytes: ([0-9a-f]*)>", seen)
        if not m:
            return False
        table = pyarrow.ipc.open_stream(bytes.fromhex(m.group(2))).read_all()
        names = list(self.frame[0].keys())
        return table.column_names == names and table.to_pydict() == {n: [row[n] for row in self.frame] for n in names}


def asked(*answers):
    return "\n—\n".join(answers)


def read_answer(name):
    """A route that reads its request, each request it is asked."""
    ok = lambda kind, body: answered(200, ["content-type: " + kind], body)
    jsoned = lambda j: ok("application/json", json_text(j))
    said = {
        "params": asked(jsoned({"name": "ada", "n": "3"}), jsoned({"name": "héllo w", "n": "0"})),
        "query": asked(jsoned({"q": "hello world", "n": "3", "e": ""}), jsoned({"q": "a+b"}), jsoned({})),
        "method-path": asked(jsoned({"m": "GET", "p": "/cN"}), jsoned({"m": "GET", "p": "/cN"})),
        "body-text": asked(ok("text/plain", "a nest"), ok("text/plain", "héllo"), ok("text/plain", "")),
        "body-json": asked(ok("text/plain", json_text({"a": [1, 2], "b": "x"})), STOPPED),
        "body-field": asked(ok("text/plain", "x"), ok("text/plain", "3"), STOPPED),
        "status-read": asked(answered(404, ["content-type: text/plain"], "x"), STOPPED, STOPPED),
        "header": asked(ok("text/plain", "hello"), STOPPED),
        "form": asked(jsoned({"a": "1", "b": "two words", "c": "é"})),
        "files": asked(jsoned({"n": "1", "f": "field value", "name": "a.txt", "size": "10"})),
        "method-named": asked(ok("text/plain", "made"), answered(405, [], "")),
        # (limit = 2: a request a minute's share, 30 s, after the second)
        "limit": asked(ok("text/plain", "ok"), ok("text/plain", "ok"), answered(429, ["content-type: text/plain; charset=utf-8"], "too many requests: try again in 30 s")),
        "nowhere": asked(answered(404, ["content-type: text/html; charset=utf-8"], "<!DOCTYPE html><title>Not found</title><p>no page at /cN/nowhere/else</p>")),
        "stopped": asked(STOPPED),
    }
    if name in ("falls-off", "failed", "failed-in-json", "bad-status"):
        # refused before either mode runs: a handler answers on every path,
        # a response's body is text, a map holds one kind, a status is HTTP's
        raise Err()
    if name not in said:
        raise Skip()
    return said[name]


# ── a view's markup (spec, Views) ──────────────────────────────────
#
# A page's answer is its status and the markup its root holds; the markup is
# read back with Python's own HTML parser, and held to what the spec says:
# an element draws as its tag (a container as a div, a divider as hr, a
# dropdown as details), its text — entities decoded — is what the value
# shows (a list's items each, a column's cells, a failed value an id the
# log holds), an attribute reads back as it was written, a boolean one is
# there or not.

from html.parser import HTMLParser as _HTMLParser

CONTAINERS = ("stack", "row", "grid", "overlay", "scroll", "modal", "drawer", "toast")
FAILED_IN_VIEW = "error: the server's log has why, as <id>"


def html_tag(name):
    if name in CONTAINERS:
        return "div"
    return {"divider": "hr", "dropdown": "details"}.get(name, name)


class _Node:
    def __init__(self, tag, attrs):
        self.tag, self.attrs, self.children = tag, dict(attrs), []

    def text(self):
        return "".join(c if isinstance(c, str) else c.text() for c in self.children)


class _Reader(_HTMLParser):
    VOID = {"area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track", "wbr"}

    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.root = _Node("", [])
        self.stack = [self.root]

    def handle_starttag(self, tag, attrs):
        node = _Node(tag, [(k, "" if v is None else v) for k, v in attrs])
        self.stack[-1].children.append(node)
        if tag not in self.VOID:
            self.stack.append(node)

    def handle_endtag(self, tag):
        if len(self.stack) > 1 and self.stack[-1].tag == tag:
            self.stack.pop()

    def handle_data(self, data):
        self.stack[-1].children.append(data)


def drawn(seen):
    """The element a page's root holds, read back: None for an answer that
    is no page."""
    head = "HTTP/1.1 200 OK | "
    if not seen.startswith(head):
        return None
    r = _Reader()
    # (a failure's id, read past as `<id>`, is text the page wrote)
    r.feed(seen[len(head):].replace("<id>", "&lt;id&gt;"))
    r.close()
    elements = [c for c in r.root.children if not isinstance(c, str)]
    return elements[0] if len(elements) == 1 else None


def child_text(v):
    """What a value shows as an element's child (spec, Views): a list's
    items each (nested lists too), a column's cells (a missing one failed),
    text as it is, anything else as `log` writes it."""
    if v is FAIL:
        return FAILED_IN_VIEW
    if isinstance(v, C):
        return "".join(FAILED_IN_VIEW if c is None else child_text(c) for c in v)
    if isinstance(v, L):
        return "".join(child_text(x) for x in v)
    if isinstance(v, Rec):
        return "SelfP[" + ", ".join("{} = {}".format(k, show(x, False)) for k, x in v.items()) + "]"
    if isinstance(v, Frame):
        raise Skip()
    if isinstance(v, J):
        return json_shown(v.v, True)
    if isinstance(v, Exact):
        return str(v)
    return show(v, True)


class Drawn(str):
    """An answer held by reading its markup back: `holds(seen)`."""

    def __new__(cls, what, check):
        self = str.__new__(cls, what)
        self.check = check
        return self

    def holds(self, seen):
        node = drawn(seen)
        return node is not None and self.check(node)


def view_answer(element, names):
    v = route_value(names[0])
    if isinstance(v, Frame) or names[0] in ("bytes", "fn"):
        raise Skip()
    want = child_text(v)
    tag = html_tag(element)
    # (a record's kind is the case's own copy of it: `SelfP_45`)
    return Drawn("<{}> showing {!r}".format(tag, want), lambda n: n.tag == tag and _re.sub(r"SelfP_\d+", "SelfP", n.text()) == want)


def attr_answer(attr, names):
    v = route_value(names[0])
    if not isinstance(v, str):
        raise Skip()
    if attr == "class":
        classes = v.split()
        return Drawn("class {!r}".format(" ".join(classes)), lambda n: n.attrs.get("class") == (" ".join(classes) if classes else None))
    return Drawn("{} = {!r}".format(attr, v), lambda n: n.attrs.get(attr) == v and n.text() == "x")


def class_answer(names):
    v = route_value(names[0])
    classes = ["stack"] + v.split()
    return Drawn("class {!r}".format(" ".join(classes)), lambda n: n.tag == "div" and n.attrs.get("class") == " ".join(classes) and n.text() == "x")


def bool_answer(attr, names):
    on = names[0] == "bool"
    return Drawn("{} {}".format(attr, "present" if on else "absent"), lambda n: n.tag == "button" and ((n.attrs.get(attr) == "") if on else (attr not in n.attrs)))


# ── a page's state (spec, Reactive State and Events) ───────────────
#
# Each case is a page whose first paragraph shows a fact (or a derivation
# over it); the page after each event is read back, and the paragraph's
# text held to what the spec says the fact holds then. A handler that
# stops lands nothing and the page says so; an event the page did not
# offer changes nothing.

STATE = {
    "int-inc": ["0", "1", "2"],
    "int-sub": ["0", "-3", "-6"],
    "float-half": ["1", "0.5", "0.25"],
    "text-grow": ["a", "a<b>", "a<b><b>"],
    "bool-flip": ["false", "true", "false"],
    "list-push": ["[7]", "[7, 1]", "[7, 1, 2]"],
    "map-set": ["[a = 1]", "[a = 1, k1 = 1]", "[a = 1, k1 = 1, k2 = 2]"],
    "big-grow": [str(2**62), str(2**63), str(2**64)],
    "moment-step": ["2026-01-02T03:04:05Z", "2026-01-02T04:04:05Z", "2026-01-02T05:04:05Z"],
    "record-field": ["SelfQ[x = 1, y = 2]", "SelfQ[x = 2, y = 2]", "SelfQ[x = 3, y = 2]"],
    "list-item": ["[1, 2]", "[10, 2]", "[100, 2]"],
    "map-key": ["[a = 1]", "[a = 2]", "[a = 3]"],
    "args-int": ["0", "10", "20"],
    "args-text": ["", "x&y", "x&yx&y"],
    "derived": ["10", "20", "30"],
    "derived-chain": ["11", "21", "31"],
    "helper": ["0", "5", "10"],
    "stops": ["1", "1", "1"],
    "later": ["0", "1", "2"],
    "field-text": ["", "Ada <b> & co", ""],
    "field-number": ["1", "42", "-2"],
    "checkbox": ["false", "true", "false"],
    "select": ["a", "b", "c"],
    "unoffered": ["a", "a", "a"],
    "slider": ["5", "7", "0"],
    # (the page opened again starts a session of its own, from the facts'
    # first values)
    "reopened": ["0", "1", "0", "1"],
}

# (the cases whose handler stops: the page says so after the event)
STOPS = {"stops"}


class Pages(str):
    """A page's answers after each event, held by reading each back."""

    def __new__(cls, name, want):
        self = str.__new__(cls, "pages showing " + repr(want))
        self.name, self.want = name, want
        return self

    def holds(self, seen):
        pages = seen.split("\n—\n")
        if len(pages) != len(self.want):
            return False
        for i, (page, want) in enumerate(zip(pages, self.want)):
            html, _, rest = page.partition(" | title: ")
            if "address: /cN" not in rest:
                return False
            r = _Reader()
            r.feed(html.replace("<id>", "&lt;id&gt;"))
            r.close()
            paras = []

            def walk(n):
                for c in n.children:
                    if isinstance(c, _Node):
                        if c.tag == "p":
                            paras.append(c.text())
                        walk(c)
            walk(r.root)
            shown = _re.sub(r"SelfQ_\d+", "SelfQ", paras[0]) if paras else None
            if shown != want:
                return False
            stopped = "stopped, and nothing changed" in html
            if stopped != (self.name in STOPS and i > 0):
                return False
        return True


def state_answer(name):
    if name not in STATE:
        raise Skip()
    return Pages(name, STATE[name])


def placed_past(seen):
    """An answer with where its case's route sits in its batch read past."""
    return _re.sub(r"/c\d+", "/cN", seen)


def answer(what, names):
    if what.startswith("route:"):
        return route_answer(what[6:], names)
    if what.startswith("route-read:"):
        return read_answer(what[11:])
    if what.startswith("view:"):
        return view_answer(what[5:], names)
    if what.startswith("view-attr:"):
        return attr_answer(what[10:], names)
    if what == "view-class:stack":
        return class_answer(names)
    if what.startswith("view-bool:"):
        return bool_answer(what[10:], names)
    if what.startswith("view-layout:"):
        raise Skip()
    if what.startswith("state:"):
        return state_answer(what[6:])
    args = [value(n) for n in names]
    # (a parsed value is read as each use implies: beyond showing, measuring
    # and indexing it, not answered here)
    if any(isinstance(a, J) for a in args) and what not in ("form:show", "len", "form:index", "type", "str", "form:spliced"):
        raise Skip()
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
            except Needs as n:
                print(json.dumps({"kind": "needs", "id": case["id"], "library": n.library}))
                skipped += 1
                continue
            except Skip:
                skipped += 1
                continue
            except Err:
                expected = None
            answered += 1
            print(json.dumps({"kind": "answered", "id": case["id"]}))
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
            if case["what"].startswith(("route", "state")) and "seen" in case:
                case["seen"] = placed_past(case["seen"])
            if isinstance(expected, (ArrowOf, Drawn, Pages)) and "seen" in case and not case.get("end"):
                if not expected.holds(case["seen"]):
                    print(json.dumps({"kind": "wrong", "id": case["id"], "expected": str(expected), "seen": case["seen"]}))
                continue
            if case.get("end") or case.get("seen") != expected:
                seen = case.get("seen", "")
                if case.get("end"):
                    seen = (seen + " " if seen else "") + "[" + case["end"] + "]"
                print(json.dumps({"kind": "wrong", "id": case["id"], "expected": expected, "seen": seen}))
    print(json.dumps({"answered": answered, "skipped": skipped}))


if __name__ == "__main__":
    main()
