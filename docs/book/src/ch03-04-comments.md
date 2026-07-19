# Comments

A comment runs from `---` to the end of the line. There is no closing
delimiter, so an unclosed comment can never swallow code:

```crow
--- this whole line is a comment
x = 5    --- and this trails a statement
log[x]
```

```text
5
```

Multi-line commentary is just multiple `---` lines. That's the entire
comment system.
