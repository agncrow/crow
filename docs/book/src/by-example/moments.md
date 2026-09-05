# Moments and freshness

```crow
bell = moment[2026, 7, 14, hour = 9, minute = 30, tz = 'America/New_York']
log[fmt[bell, time = 'iso']]
log[fmt[bell, time = 'date']]
c = calendar[bell, tz = 'UTC']
log[c.hour]
log[c.weekday]
later_on = bell + 90.0
log[later_on - bell]
p = parse_time['2026-07-14 09:30', tz = 'America/New_York']
log[p eq bell]
t = now[]
log[t gt bell]

write['ex_moment.txt', 'tick']
px = read['ex_moment.txt']
age = now[] - captured[px]
log[age lt 5.0]
computed = 2.0 * 2.0
log[captured[computed].kind]
rm['ex_moment.txt']
```

```text
2026-07-14T13:30:00Z
2026-07-14
13
2
90
true
true
true
no_capture
```

Reference: spec section 11 (Moments, Freshness).
