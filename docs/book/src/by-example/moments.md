# Moments and freshness

```crow
bell = moment[2026, 7, 14, hour = 9, minute = 30, tz = 'America/New_York']
log[bell]
log[fmt[bell, style = 'date']]
log[fmt[bell, style = 'iso', tz = 'UTC']]
utc = moment[bell, tz = 'UTC']
log[utc.hour]
log[bell.weekday]
later_on = bell + 90.0
log[later_on - bell]
p = moment['2026-07-14 09:30', tz = 'America/New_York']
log[p eq bell]
t = now[]
log[t gt bell]
log[type[t]]

write['ex_moment.txt', 'tick']
px = read['ex_moment.txt']
age = now[] - captured[px]
log[age lt 5.0]
computed = 2.0 * 2.0
log[captured[computed].kind]
rm['ex_moment.txt']
```

```text
2026-07-14T09:30:00-04:00
2026-07-14
2026-07-14T13:30:00Z
13
2
90
true
true
moment
true
no_capture
```

A moment is a kind of its own: the instant, and the offset it was written
in, which its fields and `str[t]` read in. `moment[…]` makes one from
text, from its parts in a zone, or from a moment in another zone; `fmt`
renders it in a style; `t2 - t1` is the seconds between, `t + 60` a
moment. Reference: spec section 11 (Moments, Freshness).
