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

Multi-line commentary is just multiple `---` lines. A line that holds
only a comment opens and closes no block, whatever its indentation, as a
blank line does: a comment that goes on from a trailing one can sit
aligned under it, and a note at the margin inside a function leaves the
function open:

```crow
fn hello[name]
    words = 'hi `name`'     --- the greeting, as the
                            --- page shows it
--- a note at the margin, inside the function
    return words
log[hello['ada']]
```

```text
hi ada
```

That's the entire comment system.
