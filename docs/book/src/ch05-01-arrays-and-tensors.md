# Arrays and Tensors

Arrays are ordered and homogeneous. Indexing is zero-based; negative
indices count from the end:

```crow
nums = [10, 20, 30]
log[nums[0]]
log[nums[-1]]
log[len[nums]]
more = push[nums, 40]
log[more]
log[concat[[1, 2], [3, 4]]]
```

```text
10
30
3
[10, 20, 30, 40]
[1, 2, 3, 4]
```

`push` and `concat` return **new** arrays — Crow collections are
immutable-by-rebuild, which is part of what keeps parallel loops safe.

## Numeric Arrays Are Tensors

An all-numeric array literal is backed by a real tensor, so math works on
it wholesale — `+` adds element-wise against another tensor or broadcasts
a scalar; the dotted operators `.*`, `./`, `.^` are explicit element-wise
multiply/divide/power:

```crow
a = [1.0, 2.0, 3.0]
b = [10.0, 20.0, 30.0]
log[a .* b]
log[a + b]
log[a * 2.0]
```

```text
[10, 40, 90]
[11, 22, 33]
[2, 4, 6]
```

Nest literals and you have a matrix — `m[0]` is a row, `.shape` tells you
the dimensions, `*` of two matrices multiplies them, and `use math`
unlocks the full toolkit (`zeros`, `eye`, `transpose`, reductions along
axes — chapter 10 puts it to work):

```crow
m = [[1, 2], [3, 4]]
log[m[0]]
log[m.shape]
```

```text
[1, 2]
[2, 2]
```
