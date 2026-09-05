# The system: shell, environment, time, arguments

```crow
r = shell['echo hello']
log[r.status]
log[r.stdout]
home = env['HOME']
log[home.ok]
unset = env['CROW_NO_SUCH_VAR']
log[unset.ok]
log[unset.kind]
log[env['CROW_NO_SUCH_VAR', 'fallback']]
log[time[] gt 0.0]
log[len[args[]]]
log[hash['crow', 'sha256']]
```

```text
0
hello
true
false
env
fallback
true
0
851d49d3087657e971533b2fd8b28be2ff490c34411c2e3cd2b400fe66f3a3c3
```

`exit[code]` ends the program with a status code and `sleep[ms]` pauses;
this block builds but is not run:

```crow,norun
sleep[10]
log['stopping']
exit[0]
```

Reference: spec section 11 (System, Shell, Hashing).
