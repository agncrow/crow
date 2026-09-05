# Formatting

```crow
x = 1234.5678
log[fmt[x, decimals = 2]]
log[fmt[x, decimals = 1, commas = true]]
log[fmt[0.256, percent = true, decimals = 1]]
log[fmt[42, width = 6]]
log[fmt[42, width = 6, fill = '0']]
log[fmt['ab', width = 5, align = 'right']]
```

```text
1234.57
1,234.6
25.6%
    42
000042
   ab
```

Reference: spec section 11 (Formatting).
