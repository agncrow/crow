# Files and directories

```crow
mkdir['ex_tmp']
write['ex_tmp/a.txt', 'one\ntwo']
log[exists['ex_tmp/a.txt']]
log[read['ex_tmp/a.txt']]
cp['ex_tmp/a.txt', 'ex_tmp/b.txt']
mv['ex_tmp/b.txt', 'ex_tmp/c.txt']
log[ls['ex_tmp']]
missing = read['ex_tmp/nope.txt']
log[missing.ok]
log[missing.kind]
rm['ex_tmp']
log[exists['ex_tmp']]
```

```text
true
one
two
['a.txt', 'c.txt']
false
io
false
```

Reference: spec section 11 (I/O) and section 10 (Fallible Builtins).
