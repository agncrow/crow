# Changelog

Crow is in alpha: the syntax may change between releases, `crow fix`
migrates what it can, and a breaking change bumps the minor version.
Each release lists what breaks first. This file summarises; the commit
history says the rest.

## Unreleased

### Fixed

- A float's bytes are the same on every machine: `sys.float_bytes` and
  `sys.ieee` write a NaN as the one quiet NaN (0x7ff8…, 0x7fc0… at single
  precision), whatever bits the arithmetic left in it. x86's `0.0 / 0.0`
  sets the sign bit where ARM's does not, so a Magpie file holding a NaN
  was other bytes on the Linux runner than on a Mac, and the file
  module's recorded digests failed there (CI's first run of Magpie in
  Crow). The bytes a Mac wrote are unchanged.
- A function `any`, `all` and `find` apply to each item of a list takes
  the items as they are in compiled programs too: given a list holding
  growing ints, its parameter grows, and a list of 64-bit ints or a
  vector handed to it elsewhere is converted. The width pass had passed
  no item's range to the parameter, so a list of growing ints was handed,
  item by item, to a parameter held in 64 bits, and the program did not
  build (parity 438; fuzz seed 1793331019).
- A Magpie log removed while its flush ran keeps none of its rows in
  compiled programs too. The flush runs on a thread of its own there, and
  one that had claimed the rows before an `rm` wrote them a moment later
  into the log made again at the path: 50,000 removed rows read back
  beside the new log's one, and once its one row was lost, since the old
  flush's settling trimmed the new log's pending rows (parity 434; the
  interpreter flushes inline). A flush now writes a file of its own
  claim's and links it under the tail's lock only while the claim holds,
  which `rm` lets go with the tail; and `rm` removes a directory again
  where a writer added a file as it emptied it (parity 302 had left its
  directory behind).
- `sys.tokens` reads ASCII text a byte at a time, where Unicode's
  letters and lowercase are ASCII's (the words the same): Magpie's
  scanned search, a quarter of whose time had gone to decoding characters
  and lowercasing words through Unicode's tables, takes 57 ms where it
  took 62 (the Rust kernel's 11).
- A compiled program reads a field of a value that can fail where it
  lies once the line has checked the value, as it reads a record's: the
  record had been copied out whole, its lists and all, at each field read
  (Magpie's scanned search copied every hit at each of its fields).
- Magpie's summary counts each group's rows in 64 bits: the count was
  added through a local (`k = ns[g] + 1`, then `ns[g] = k`), which the
  width pass does not read as a sum bounded by its rounds, so it grew,
  and every row's mean divided by a growing int made a float. 200,000
  rows by 100 groups summarised in 7.5 ms, where they had taken 8.6 (the
  Rust kernel's 2).
- Magpie's writes: a number column's distinct values for its filter are
  found by sorting them, where a map had taken a lookup and an insert
  for every cell (the filter and the count the same, byte for byte), and
  a bloom filter's slots are computed in 64 bits — 2^64 modulo the
  filter's size had been any 64-bit int to the width pass, so each slot's
  sum went growing. 200,000 rows by 102 columns written in 2.9 s and
  merged in 3.1 s, where they had taken 5.3 and 5.6 (the Rust kernel's
  2.7 and 2.3).
- Magpie's `merge` folds a lone batch into a segment too, where it had
  answered '' for fewer than two live files: whether a key's older
  versions were folded had depended on whether a flush cut a `put` and
  the `batch` after it into one file or two — which a compiled program
  decides by its flush thread's timing, so parity 303 counted a summary
  differently in one run of three (the interpreter flushes inline). A
  merge now answers the same however the rows reached files; a time
  series merged keeps its rows with `every = true`, as the as-of test
  now says. And the unit test lowering the parity corpus opens the
  harness's door for its thread alone, as the parity harness opens it
  for its programs (it had exited the test process on a program that
  imports process memory).
- A column read by a computed name of a frame that can fail — `g.`c``
  of a function's frame, as a summary's is — propagates the frame's
  unwrap, as a pick does: compiled, the call had read the frame as it
  was held, and the program did not build (parity 381).
- Magpie's summary groups each unit's rows once — each row's group as
  its place among the groups, through a map of ints where the key is one
  int column — and keeps a column of numbers' partials per group as
  numbers, made records once the rows are read: a group's record had
  been read out of a map and written back for every row (a summary of
  two columns by key over 200,000 rows 96 → 9.8 ms; the answers the
  same to the digit, held by the kernel's recorded digests and a new
  test of the int key).
- A search that scans a batch's texts places only the words the query
  asks in each text's map of words to positions, unless a prefix or a
  fuzzy word asks of every word: every word of every text had been
  placed, a list copied out of the map and back for each (a common
  word's top 10 over 100,000 batched texts 108 → 70 ms).
- Bytes appended from a range are one copy: the lowering fuses
  `concat[a, slice[b, i, j]]` into `core.__concat_slice`, and the split
  it makes of `a = concat[a, slice[a, i, j]]` — the slice held, then the
  append — into `core.__extend_within` (fuse.rs; both modes, the ends
  and the refusal `slice`'s). Magpie's decompressor had copied each
  literal run and each match into a slice and that slice again: 50 MB
  decompressed in 34 ms, now 17 (the Rust kernel's 9; parity 380). An
  overlapping match — a run — is copied as whole repeats, doubling each
  time, where it had been copied a byte at a time. And `unpack` reads its
  bytes where they lie, where a copy reversed had been an allocation at
  every field a decoder read.
- Magpie's reads made cheap, as Vijay chose over keeping anything
  between calls. The bit-packed decoders are 64-bit throughout: each
  width has a loop of its own, the cursor is bounded by the bytes, and
  the bits held are written so the width pass bounds them — one function
  holding both loops had lost the bound, and three quarters of a
  column's decode was growing-int arithmetic (a scan of one column over
  finch's 11.3 million bars 224 → 83 ms). An int `unpack` reads from 1,
  2 or 4 bytes is within what those bytes spell, of either sign
  (`IntRange::Unpacked`): every such field had been any 64-bit int to
  the pass. And the graph package reads a hop at a time: a frontier of
  more than four nodes is one range of the edges log, read once, where
  each node had been a read of the log's files of its own (a walk of
  three hops over 1,022 nodes read the files 1,022 times).
- What writing Rosetta Code's tasks in Crow found (rosetta/; each fix
  with its parity program, 331–357). Maps: an empty map written at an
  int key holds ints as keys, and `keys` of a function's int-keyed map
  is a list of ints (331); a key the width pass cannot bound (`10 ^
  xs[0]`) is no value the map holds, and `values` of a map of growing
  ints is a list of them — the lowered program had disagreed with itself
  (350). A function that can fail and answers nothing ends well on a
  bare `return` or on a last `if`, `while` or `for` (332), and a
  recursive one ending on a call of itself (339). `int[text]` items handed
  to `sort`, `sum` or `max` are numbers (334). `fmt` lays out an int past
  64 bits, a decimal and a fraction with every option (335, 347).
- Whole numbers written down and grown with `push`, or written by
  position, are a list of ints — every digit kept (`fib = [0, 1]` had
  rounded past 2^53 interpreted and overflowed compiled; 336) — and so
  when the pushed value can fail (351): the loop's shape pass had joined
  the failure into a tensor's variable. A list pushed as an item of a list
  of lists keeps its items' ranges (337); a loop over whole numbers
  written down ranges its variable by them, which had been read as any
  64-bit int and overflowed compiled (357); a growing field read through
  a record-or-none link keeps every digit (338); numbers held two ways
  (a literal list beside a list of growing ints) compare by value (342).
- A float written at a nested position of a holder of ints (`g[0][0] =
  2.5`) makes every row a row of floats, rows pushed or collected before
  converted where they meet the grid (352), and so does a float written
  into whole numbers written down at the top level (358); a literal written into a
  list's item, a map's value or a record's field is the list it holds
  (344). A local written into a list by position or into its own
  record's link is copied (343); a record from a call that can fail is
  handed where a link is taken (345); a call through a list of functions
  types its members (346); a function handed a list of vectors and a
  list of lists has a copy for each (341). An item of a list nothing is
  put into, read or written by position, stops at the first position in
  both modes — compiled it had been read as a function (353).
- Names: a variable named `bytes`, `match`, `from` or `print` — words
  another module or another language owns — is the program's (340); a
  record named like a type the compiled program names (`Vec`, `String`,
  `Self`, the runtime's `Crow…`) and a function named like its own
  helpers (`crow_stop`) are renamed out of Rust's way, shown as declared
  (354); a backslash before an interpolation (`'\\`e`'`) is a
  backslash; a view in a program without `use ui` is refused with the
  fix.
- A parallel loop's threads have the program's stack: a recursion
  inside one had aborted the process, 643 calls deep interpreted (355). A
  loop's variable written in its body is the round's own copy — compiled
  it had been bound immutably and not built (356). Indexing a top-level
  list that can fail reads the item where it lies (it had copied the list
  at every read: 300 times slower compiled), and `join` of a statement
  loop's item that is a list builds.
- More of what the Rosetta Code tasks found (parity 359–387). Lists:
  a list given `[]` that meets `join` before the push typing it takes the
  push's kind (359); a map filled by position at the top level and handed
  on is typed by its writes (360); a list literal holds its items in one
  form — numbers held as a tensor beside a list are the list, whole
  numbers beside floats floats (365) — and lists whose items hold numbers
  the other way, at any depth, are converted item by item (382); rows of
  whole numbers written down keep every digit (366, 371), a list of lists
  one of whose lists grew holds growing lists (376), and `concat` of a
  range and growing ints grows both (384). A literal returned beside a
  list of growing ints is that list (377); a function's answer follows
  its body once every item is known (378); a list of rows beside a matrix
  is the matrix, not a tensor of lists (379); one copy of a function
  takes a matrix of ints and one of floats as floats (400); a list pushed
  onto another before its own items are known (383), an item written
  through a record's field (385) and a local a collecting loop yields
  (387) are typed by what is written later; whole numbers a loop gives a
  list of ints are a list of ints from their declaration, read so on a
  line above that assignment too, and a tensor of floats given a list
  stays the tensor (372). Each had been an internal error in both modes
  or a compiled program that did not build.
- Failures: a value that can fail written into a field made plain holds
  the failure (362); written by position inside a function it is handed
  back (363); a loop over a list that can fail walks its items (364);
  `values` of a map that can fail (361); `error[…]` where an item goes —
  in a list literal, pushed — is a failure of an item (401).
- Ints: -2^63 divided by -1, and its `abs`, grow to 2^63 (367);
  arithmetic on an int-keyed map's key grows (368); a width past 64 bits
  handed to `fmt` beside a float stops alike in both modes, where it had
  chosen the growing form for the float (386; `crow check --self`).
- Compiled builds: loops reusing a name Rust reserves (369), a loop over
  moments handing each on (373), records compared where one is lent
  (374). Under `use math as m`, a bare name is the core's (`min`), the
  program's own function or a variable (`sum = [1, 2]`), never math's —
  each had been refused as math's (375).
- Refused before the program runs, with the fix (errors j88–j93): what a
  function answering nothing gives, read by position; an item of a list
  nothing is put into, computed with or read (parity 353 became j91); a
  collecting loop whose last line answers nothing; a record declared
  inside a body; a comparison word Crow does not have (`if k ge 0` is
  told `gte`, where it had been told the block has no body).
- A wrong answer, compiled only: a function answering a list whose items
  can fail (`[turned]` on one path) and returning a plain list on another
  (`split_up[turned]`) had answered an empty list — each item is now the
  success it is (399); the IR's verifier refuses a success handed on
  under another payload, the shape of that slip. Builds that had failed
  compiled: a view's markup returned by a plain function (388), a map of
  failures answered through a failing argument (389), `concat` of a list
  whose items can fail and a plain one (390), a moment a loop borrows
  handed to a function inside an operator (391), a call handed `error[…]`
  directly answers that failure (392), a record of no fields (393).
  Agreeing modes where the lowered program had disagreed with itself: a
  function handed as a value answers what it answers once its callees
  are copied (394), rows of ints written down and grown at the top level
  (395), a whole number written into growing ints by position (397), a
  map of text written a value that can fail by key — decided where it is
  written, as a list's item is (398), maps of ints and of floats handed
  to one function (402). An int and a growing int are one kind: `answer =
  0` then `answer = m + 1` of a growing `m` had been refused, "from int
  to int" (396). A body may end on `halt[…]`, which no path goes on past
  (403; the row's new `Prop::Ends`, beside `exit`'s).
- Refused before the program runs (errors j94–j97): a top-level variable
  read before the line that gives it its first value; two calls handing
  a function taking a function two functions of two answers (spec,
  Specialization); a collecting loop whose last line is a `for` or
  `while` statement, which is no value (the interpreter had collected the
  inner loop's last value, the compiled program nothing); a parameter
  returned beside a return of another kind, named at the returns — the
  parameter had taken the function's other answer as its type, and the
  refusal had landed in another function.
- Builds that had failed compiled, now agreeing (parity 404–414): a
  function of the program's named like a form of a module it does not
  import (`connect`, `fits`, `box`) is the program's — its calls had been
  checked against the form (404); a top-level variable's one assignment is
  judged against nothing an earlier round guessed (405); a tree's links
  pushed onto a stack of records (406); a map of maps written two levels
  down (407); whole numbers written down joined by `concat` with a list
  of growing ints keep every digit (408); a record-or-none field that can
  also hold a failure (409); a list or a map a loop borrows, compared
  (410); a local handed back before the line that types its items (411);
  a function value handed a value that can fail takes its payload (412);
  a function a record's field holds, called through it (413); a call
  through a member of a list of functions handed in answers the members'
  kind (414); a list of plain items given a literal whose items can fail
  holds the good ones and refuses a failed one at the assignment, where
  the reassignment had been refused as a change of kind (415); a list
  holding an int past 64 bits handed where numbers are taken is its
  numbers interpreted too, that int its nearest float (416); a call in a
  `while` condition is handed what the loop's body grows — the width
  pass had read it with the variable's range from before the loop, and
  `while not check[big]` stopped at the first round past 2^63 (417); a
  stop in a `while` condition names the `while` line in every round,
  where the interpreter had named the body's last line after the first
  (418). A decimal of 65,535 places or more shows, where Rust's
  formatter had stopped the program, and the lines `log` wrote before a
  stop inside its own formatting are no longer lost on a pipe. A
  fraction or a decimal of 0, 1 or -1 raised to any int is exact, as an
  int is — past 32 bits it had stopped as a number no memory holds, and
  past 64 the exponent had been refused naming no `^` (419).
- Three copies that made a loop quadratic, found writing a
  lambda-calculus machine (Rosetta Code, Universal lambda machine): a
  `return m` inside a loop's body moves `m`, since nothing reads it after
  (compiled, a record returned from inside a `while` had been copied at
  every call: 41.8 s, now 0.25); `x = f[x, n]` whose call can fail hands
  `x` to the call as `x = f[x]` does (a list grown by such calls had been
  copied at each: 40,000 calls 5.2 s, now 0.03 interpreted); and a vector
  held in a record's field or a list's item grows where it lies, as a
  list there does (`b.xs = push[b.xs, i]`: 200,000 pushes 4.9 s, now
  0.04 interpreted), so a matrix written through a field is written
  where it lies too. A gather by an index tensor and a matrix's row read
  the numbers they name where they lie: each had copied its whole source
  (1,000 gathers of 3 numbers from 4,000,000 took 2.3 s, now 0.001), and
  a walk of a matrix's rows had copied the matrix once per row (Rosetta
  Code, Facial detection in a photo). A parallel collecting loop works
  its first item where it stands and hands the rest to the cores only
  where their time is worth the pool's round trip (`cores::worth_sharing`,
  both modes): a loop over ten flags called 20,000 times had waited on
  the pool at every call, 1 to 4 s interpreted where the loop in order
  took 0.02 (Rosetta Code, Average loop length); a loop of heavy items
  still runs on every core. A `return` inside a `for` hands
  back what the loop walks beside its item as it did (420), and a loop
  walks what its body assigns anew as the loop began, where the compiled
  program had not built (421). A parsed JSON whole number past a signed
  64-bit int, within 2^64, is an int with every digit — `type` had said
  float and `int[…]` answered the float's digits (422). A window's sum,
  minimum and maximum over a column of ints are computed on the ints,
  every digit, where they had gone through floats (423). A moment's text
  outside the years 0 to 9999 is ISO 8601's expanded year — its sign and
  four digits or more, `-0005-06-01`, `+10000-01-01`, as `fmt`'s date
  style writes it — and reads back as the moment: the sign had counted
  among four digits and read back a day off, and a year past 9999 had not
  read back (424). A function answering a record on one path and `0` on
  another answers a record or none, as the spec says, where it had been
  refused as two kinds (425). A list of numbers met in arithmetic with a
  number is a vector of one axis, so a loop walks it and an index reads
  it, where its rank had been unknown and both were refused (426). A
  variable or a parameter named like web's or the test harness's forms
  (`stream`, `box`) is indexed, where its index had been read as the form
  (428).
  `eigen` answers two vectors, as its row now declares: the imaginary
  parts had been declared a matrix, and a read of one stopped (427). A
  list of no rows handed where a matrix is taken is a matrix of none, of
  the row shape the program's type states, where it had been a vector of
  no numbers whose rows the reader refused (429). A function handed as a
  value where a list of ints is answered answers that list, where a later
  round had reformed it to its body's vector and `concat` met the vector
  (430). A parsed value set into a map that holds nothing yet is held as
  it is, where a row of sys's — which no program imports — had made it an
  int (431). A function called with a list of ints and with a list of
  records is two copies, where one copy's list of records or none had
  taken the ints unconverted (432). A function taking a function, handed
  one answering a truth that can fail and, at another call, one answering
  a plain truth, holds the parameter both fill, where the plain one had
  made it its own and the program did not build compiled (433). A
  function held in a record's field is typed by the calls made through
  the field — there, through a local read from it, or where it is handed
  to a function taking one — where it had been refused as a function
  nothing calls (437); so are functions put in a list one at a time, by
  the calls through its items (436). A record made in a function of the
  answers of two calls of one function at two kinds holds each copy's
  answer, where the first round's answer, read with the kind the later
  call gave, had stayed in the field and the program was refused (435).
  Arithmetic on what a function handed a record and an int answers names
  it a record or none, with the way out, where it had named the record
  alone (k32); and a parsed value pushed onto a variable's list whose
  items nothing said yet, read as a number, says so where it holds text,
  and how to keep parsed values (457).
- Refused before the program runs (errors j98–k15, k30–k32): a field the record
  does not declare, written; a name declared twice (functions, records);
  a function value compared (`eq` had compared by name interpreted and
  by address compiled); a function named `contains` or `has`, which no
  call could reach; a predicate whose answer can fail handed to `all`,
  `any` or `find` (a failure had counted as not passing, silently); a
  member of a list of functions handed where another call hands a
  function by name; a program's own call of sys's `describe` or `apply`
  (the interpreter had stopped, "does not run 'describe' yet", and the
  compiled program did not build); a loop over an item of a list never
  given one (no rounds interpreted, no build compiled). A fault inside a
  string's interpolation is placed at the string's line (k07): it had
  been shown on line 1. A list a builtin takes no row of is named by its
  items' kind (k09): `sort` of fractions had said `got a list`. A value
  missing at an interpolation's end says the interpolation's end, not the
  file's (k10), and a call that answers nothing is named so in a
  refusal, where it had been `void` (k11); `concat` of a list of rows and
  a matrix names the matrix and says how to join one (k12); a function
  taking a function called with lists of two record kinds names the two
  kinds, where both had read "a list of records" (k13), and one called
  with a list of ints and a list of records is refused so too, where it
  had been an internal error (k14). A record's field given two kinds by a
  function's two copies is refused once, naming both — a list's items said
  as they would be alone, `a list of lists of fractions that can fail` —
  where its joined type had grown a level each round of inference, 79
  deep in the next message (k15).

- Magpie measured after the port to Crow (docs/magpie-in-crow.md, 4.2),
  and what the measures found. The sequencer's flush claims the pending
  rows under the tail's lock and writes them outside it, so a put waits
  for no disk and the puts that arrive meanwhile join the next flush:
  each put had waited out the last one's fsync and flushed alone, 2,000
  puts 72.8 s compiled, now 0.22 s. A put and `latest` revise the tail
  with a function that cannot fail, so no backup copy is made (`latest`
  104 → 14 µs). A search opens each file once and reads each group once
  for the rows it answers (an indexed search 200 → 21 ms). `nearest`
  walks the flat list a width at a time (51 → 22 ms over 100,000
  vectors). And `hex` writes its digits from a table, not a `format!` a
  byte.
- An item of a list that is itself an item, `cells[k][i]`, is read
  through the inner item where it lies: compiled, the inner list was
  copied whole for each read, and Magpie's summary copied a group's
  every cell once per row and column (19 s where the kernel took 2 ms;
  0.11 s now). A value a guard made known good, lent to a function that
  only reads it, is lent where it lies rather than copied (a file's
  reader, with its whole footer, had been copied at every call reading
  it).
- `bitand`, `bitor` and `bitxor` of 64-bit patterns stay in 64 bits: the
  width pass had read a side of [-2^63, 2^63 - 1] as 65 bits, so a hash
  over 64-bit patterns (Magpie's bloom filter: FNV and murmur over every
  cell a merge writes) ran on growing ints — a merge of 100,000 vectors
  2,157 → 531 ms.
- A growing int handed on through a form that calls a function with
  its own arguments (`revise[name, default, f, k]`) takes the
  function's parameter as a call's argument does: it had been narrowed
  to 64 bits as the form's own, so the revision stopped interpreted and
  did not build compiled (parity 349). A statement's parameters of two
  kinds (`['k`i`', i]`) beside a connection that can fail are bound item
  by item where the call binds its arguments in order: bound whole, as
  one Rust vector of text and an int, they did not build (parity 348).
  And a function copied per signature, handed a list of its other
  copy's answers, takes the list's items once a later call says them:
  the copy made while the list was a list of nothing known had kept it
  so, and indexed it as text compiled (parity 330).
- A function handed to a builtin that calls it with the call's own
  arguments — `revise[name, default, f, x]` calls `f[held, x]`,
  `later[f, a]` calls `f[a]` — is lowered at the version those arguments
  reach, as a direct call is: inference had made a version per
  signature and the lowering then refused the function as a value of
  several kinds, so a page calling magpie's `batch` with two frames
  typed apart did not start in either mode (parity 329,
  examples/magpie).
- A row chosen by name for a value of no known kind is text's, as the
  compiled program holds such a value: `len[p]` of an item of a list a
  record holds that nothing ever fills (magpie's pending parts, in a
  program that only follows a log) had printed a list's length of a
  text, and `follow` and `metrics` did not build compiled (parity 327).
  And `crow check --self` writes the directory a case ran in as `<run>`:
  each configuration runs in its own, and an absolute path
  (`canonical['self_data.csv']`) had made the five runs disagree.
- `remember` and `recall` take a record and nothing else: any other
  value had passed the kinds check and reached the verifier as an
  internal error (errors j85). And an answer of no value handed on as an
  argument — an early round's answer of a function whose first return is
  an error, narrowed by its guard — is noted as a value not known yet,
  as a call's own place is: `merge` of a parsed value had made a copy of
  magpie's `place` whose bytes were no value, and was an internal error
  (parity 328). The stop it makes now names the use without a line
  inside the module (`handed to starts`): the line had been none of the
  program's.
- A local a function returns, its kind not known yet, takes no answer of
  no value an early round of inference had for the function (its first
  return an error): joined, magpie's `_fixed` had written no value back
  to its parameter, and a program calling `backup` alone was an internal
  error. And a function value a standard module's function hands on is
  not refused for parameters nothing types where the program does not
  call that function: only the program's call could type them (parity
  326).
- A parameter is a variable of its function: one the body assigns a
  float (`x = x + 0.5`), or a list of ints the body gives floats, holds
  the wider kind and every call converts what it hands; one reassigned
  another kind (`t = t + 1` of a parsed value) is refused as a local is;
  and a function's answer is its body's, not a guess an earlier round
  made before its callee's copies existed — where the kinds differ, not
  where one answer only adds a failure, which a function value handed on
  keeps (parity 325). Each had been an internal error, the lowered
  program disagreeing with itself (parity 324, errors j84).
- A number handed to a parameter its body reads as text is refused as a
  direct call is, however the number got there: walked out of a list,
  the first rounds had handed the parameter nothing known, the body's
  guess of text had stood against the call's number, and the lowered
  program disagreed with itself (errors j83).
- A parameter every reachable call hands an item of a list of nothing —
  a function handed `[]` walking it into another — is text, as the
  compiled program holds such an item: `'/.pending' in p` of one had
  compiled as a list's `contains`, a field given one had been moved out
  of a lent record, a value of no known kind handed on had not been
  copied, and an effect whose row the item left unchosen had taken a
  growing int unconverted (parity 323).
- A list whose items no type says, read where every row of the name
  takes a list of one kind — a local given `[]`, a record's field given
  `[]` where the record was made, read by `stack` as frames or `join` as
  text — is a list of that kind; compiled, it had been a list of text and
  the program did not build (parity 322). A name whose rows take lists of
  several kinds (`sort`) types nothing by it, except where the row takes a
  tensor, whose numbers a list of nothing always was. Turned over with `not`, such a list is read as a
  mask, so deciding on it is refused in both modes, where `!` of a list of
  text had not built (errors j82).
- A column a function walks with `for` and reads again after is borrowed
  by the walk: compiled, the loop had moved the column and the read after
  did not build (parity 321).
- `json[…]` of a list of frames answers an array of each frame's records,
  in both modes: compiled it had not built, interpreted the route had
  stopped (served 25).

- A local nothing wrote a kind into, handed back where the function's
  other return answers a map of records, holds what the function
  answers: declared a map of text compiled, the program had not built
  (parity 319).
- A plain value written beside values that can fail — `m['b'] = 5` after
  `m['a'] = int[t]`, `set[m, 'c', 6]`, `xs[0] = 7` in a list of such
  values — is a success the place goes on holding, in both modes: the
  interpreter had refused the map ("this map holds done values"), `set`
  had been refused as "an int beside an int", and compiled the value had
  been printed bare; an int past 64 bits beside an int is one kind to a
  map in the interpreter, and a record's field written into a map from a
  loop's borrowed record is copied out of it (parity 318).
- `map[]` is a map of nothing, of the type it is held as: the width pass
  had converted an empty map into one of plain growing ints where the
  map held values that can fail (E0308, parity 317), and the conversion
  of an empty map to growing ints names the kind it reads (E0282,
  parity 316).
- `keys` of a map no key has been written into yet is a list of items
  not known yet: read as text, the guess had become the kind a list
  filled from them held, and the int keys `set` wrote later were refused
  against it (parity 315).
- A part's function names a column by its own loop's variable
  (`df.`k``) whatever the entry names its variables: the entry's `k` had
  the part's renamed apart and the name inside the backticks left as
  the entry's, refused as a module variable (parity 314).
- A function given its second signature only after an earlier round of
  copies is copied too: the rounds had stopped once no function's types
  moved, leaving it uncopied and each call of it answering nothing known,
  so the compiled program read a field of a value that can fail and did
  not build (parity 312; magpie's `group_bytes`, called by the manifest's
  writer and by `file_of`).
- A function compiled code calls is compiled even where it is handed only
  the items of an empty list (`notes[[], []]` walking its lists into
  `note`): the call had been printed and the function left out, so the
  program did not build; a parameter of no known kind is held as text, as
  the empty list's items are (parity 313).
- `select` of a frame in a `use ui` program is frame's, compiled as it is
  lowered: the emitter had printed the dropdown for any `select` of two
  arguments, and inside a standard module's function a `select[whole,
  names]` is never the control (examples/magpie did not build compiled).
- A computed column name inside a function reads the function's own
  variable (`df.`k`` in a loop over `k`): the scoping pass had left the
  name inside the backticks as the program's (parity 311).
- A failure pushed into a list whose items are not known yet is decided
  where the function is lowered: the verifier had found a value that can
  fail in a list of plain values in a function nothing called (parity
  308).
- A standard module's local named like one of its exports is the
  function's own: the inliner had renamed it to the export across the
  whole module (parity 309).
- A literal of items whose kinds are not known yet is typed when they
  are: it had been typed a list in the early rounds and a tensor in the
  last, and disagreed with itself (parity 310).
- A column made from its parts (`ints_column`, `floats_column`, …) keeps
  its kind, and an empty frame's column read by name is an empty column
  of its kind in the interpreter as compiled: both had been read as text.
- A generated program's lockfile follows the runtime's pins: a program
  built where the runtime's `Cargo.lock` had moved resolved its
  dependencies afresh, and an Arrow response differed between the modes
  (served 21).
- The kernel's merge orders a missing key and a NaN last, tied, as a
  frame's sort does.
- A list held in a list, under a map's key or in a record's field grows
  where it lies: `xs[i] = push[xs[i], v]`, `m[k] = push[m[k], v]` and
  `r.items = push[r.items, v]` take the value out, push onto it and put
  it back, in both modes, where reading it had copied the list at every
  push (200,000 pushes under eight keys: 650 ms compiled and 14 s
  interpreted, 5 ms and 72 ms now; the postings builder of magpie.file
  had run 100× the kernel's time — parity 306).
- A record's field given `[]` where the record is made takes the kind of
  what is written into it (`b.items = push[b.items, i]`), and a map of
  maps nothing has written into yet is declared one: both had run
  interpreted and not built compiled (parity 307).
- A function taking a column is copied once per kind of cell: an int
  column and a float column handed to one function had shared a copy
  typed by the first, and the second call disagreed with itself
  (parity 305).
- A list of numbers compared with one number is a mask in both modes'
  types (`3 gte ys`), so a condition of it is refused as a tensor's is:
  it had run interpreted and not built compiled.
- `bytes[n]` of a count the machine cannot hold is refused, where it had
  aborted the interpreter; a shift by a count past 100 bits no longer
  stops the width pass (both found by `crow check --self`).
- A function's branches bind their own locals whatever the program
  names its variables: with `x` at the top level, a function's `x = 1`
  in one branch and `x = 'a'` in the other had been refused ("cannot
  reassign 'x' from int to str — variable type cannot change"), since
  the scoping pass kept the function's `x` as the program's; a ui
  program's functions still write its facts (parity 304, found by
  magpie.file's `plain` beside the encoder's).
- A nested literal of numbers holding an int past 2^53 is refused as the
  rule says (a matrix holds its numbers as floats; a list of ints is built
  with push[…]), in both modes: the row with the big int had been typed a
  list of ints by the flat rule and the other rows tensors, and the
  verifier found "a list of List(Int) holds an item of type Tensor" (errors
  j81).
- A test block reading a top-level value the declarations leave out — a
  value a call made — is refused at the lowering, saying what a test sees
  and to make the value inside the test; the interpreter had stopped as
  the test ran on a bare "undefined variable" (checker::left_out_of,
  carried on the type context).
- An option given by position and by name in one row no longer refuses
  the call where another row of the name takes it: `ieee[bytes, at, width
  = 4]` is the reader's row, whose second position is `at`, where the
  writer's second is `width`; the conflict is the refusal only where no
  row takes the call.
- The width pass bounds a right shift by its least shift: `shift[v, -16]`
  of a 32-bit int is 16 bits, not 32, so a hash's multiply of it stays in
  64 bits (it had gone to the growing form, and a codec written in Crow
  ran its hash on big ints).
- A record's bytes field read into a variable is copied out of the
  record, as a text field is: it had been moved, and a record read again
  after did not build (E0382).
- A module row taking a list whole (`float_bytes[xs, …]`) clones the
  variable where it is read again and moves it at its last read, as a core
  row's call does: the module row's arguments had been printed without the
  owned-kind rule, and the compiled program did not build (E0382);
  `args_by_kind` is the one rule both paths read.
- A bytes parameter a function only reads is lent (`&Bytes`, read where
  the caller holds it), as a list's, a map's or a text's is: it had been
  passed by value, so a hash of four bytes called once per byte of a 50 MB
  input cloned the input at every call.
- A row of any module that takes its collection whole is written where it
  lies, in both modes: `Row::takes_in_place` is the one rule the lowering's
  split, the walker, the prepared form and the emitter read (each had asked
  for a core row with an owned first kind), and the emitter asks the taken
  path for a module row's call as for a core row's — sys's bytes rows had
  copied the buffer at every write, and a codec written in Crow was
  quadratic.

### Added

- `concat[a, b, c, …]` joins any number of pieces, as `concat[[a, b,
  c]]` joins a list written down (453); `sort` orders fractions and
  decimals by value (456); math's `sum` of a list of ints is an int with
  every digit, so `sum[xs] % 10` runs (451); and a list of 0s an item of
  which is written a record holds records or none, its 0s none — the
  empty buckets of a hash table (455). Vijay, 2026-10-29.

- `atan2[y, x]` of two numbers is a number, by the tensor's rule (two
  numbers had been refused, where `sin` of one is its number).

- `use memory` (`remember`, `recall`, `revise`, `forget`, `canonical`):
  process memory — a record a program holds for the process under a name,
  shared by its threads, one kind of record per name, the names a tree (a
  name forgotten with every name beneath it), a path's records forgotten
  when `rm` removes the path. `revise` runs a function under the name's
  lock and hands it the record, copying it only where the function's
  answer can fail. Reading is the `held` effect, every other call
  `held.write`; the compiled program holds each record as its own type
  (`crow_runtime::memory`), alike in both modes.
- `use magpie.tail` (`put_rows`, `flush_rows`, `pending_of`,
  `pending_rows`, `pending_at`, `newest_row`, `last_position`,
  `forget_log`): Magpie's sequencer in Crow over process memory — the rows
  put since the last flush, each key's last position and newest row held
  key by key, a flush asked for through `later` — on which `put`,
  `batch`, `sync`, `latest` of one key and the readers' pending rows run;
  the crash harness's `pending` cut is the module's. And `write_batch`,
  `numbered_bytes` in `use magpie.merge`: a batch file written in Crow, as
  the sequencer's flush writes one.
- `use magpie.exec` (`summary_of`, `asof_rows`), `use magpie.nearest`
  (`nearest_rows`), `use magpie.search` (`search_rows`, `terms_of`,
  `snippet_of`), and the Kafka door's bytes — `use magpie.inflate`
  (`inflated`, `gunzipped`, `gzip_stored`, `crc32`), `use magpie.snappy`
  (`unsnapped`, `snappy_literal`), `use magpie.records` (`message_rows`,
  `message_set`): the executor, the nearest rows, BM25 search and the
  record sets in Crow, held to the kernels' answers in both modes;
  magpie's `summary`, `prior`, `search` and `nearest` and the Kafka
  door run on them.
- `use magpie.merge` (`merge_files`, `merge_into`, `place`,
  `key_ranges`): the streaming merge in Crow — one row group per input
  held, the rows decided up to the least of the inputs' last loaded rows,
  a key's run held back until another key follows, groups and segments
  cut where the kernel cuts them — the kernel's segments and manifest
  byte for byte (the ninth kernel); `magpie`'s merge runs on it. And
  `shape_of` in `use magpie.file`.
- `use magpie.encode` (`chunk`, `unchunk`, `header`): a column chunk in
  Crow, every kind the kernel writes — ints by frame of reference or
  delta, floats at 64 and 32 bits, text plain or as a dictionary, bools,
  moments, vectors — through the codec where it shrinks, the kernel's
  bytes byte for byte (docs/magpie-in-crow.md, the third kernel). A
  missing cell reads as one placeholder through the column rows in both
  modes (0, 0.0, false, empty text, zeros), where the interpreter had
  handed a float back as NaN and the compiled program as what was
  written.
- `use magpie.manifest` (`listed`, `unlisted`, `note_of`, `with_note`, `with_segments`,
  `narrow_of`, `stems_of`, `counter`, `listed_bytes`, `write_listed`,
  `declare`, `catalog_of`, `segment_columns`, `batch_named`,
  `segment_named`, `number_of`, `counter_of`, `key_words`): a log's
  manifest in Crow, the kernel's file byte for byte, written beside and
  moved over the old one (the eighth kernel); and `magic[]` in
  `use magpie.file`.
- `use magpie.file` (`file_of`, `rows_per_group`, `columns_of`,
  `group_bytes`, `finish`, `opened`, `entries`, `wanted`, `groups_of`,
  `group_of`, `empty_of`, `chunk_of`, `value_filter`, `token_filter`,
  `index_bytes`, `about`): the Magpie file in Crow — row groups of chunks
  with their statistics and filters, a block per column, the footer —
  written as the kernel writes it byte for byte and read back by region,
  the groups decoded across the cores (the seventh kernel). With it,
  `stack[frames]` of a list of frames (the form `stack[tensors]` has: a
  file's groups stacked in one pass, where two at a time copied the
  whole each time) and `filled[filter, items]` in `use magpie.bloom` (a
  list added in one pass over the bits).
- `use magpie.postings` (`built`, `parsed`, `term_of`, `with_prefix`,
  `within`, `blocks_of`, `levenshtein`): a text column's search index
  over one row group in Crow, the kernel's bytes byte for byte (the
  sixth kernel).
- `use magpie.stem` (`stem`): the Porter stemmer in Crow, the kernel's
  answer word for word (the fifth kernel); and `terms[text]` on the sys
  layer, the words of a text as an index and a query split them.
- `use magpie.bloom` (`sized`, `add`, `admits`, `filter_bytes`,
  `filter_of`): the bloom filter in Crow, the kernel's bits bit for bit
  (the fourth kernel).
- `wrap[x]` in `use bits`: a whole number's low 64 bits as the int of
  those bits, with a growing form.
- `wrapmul`, `wrapadd` and `wrapsub` in `use bits`: arithmetic wrapped to
  64 bits — a hash's multiply per byte, a linear congruence, a checksum,
  an offset from a base — which ints that grow had no way to say; and
  `float_bytes`/`float_values` on the sys layer, a chunk's floats as their
  bits at once.
- Bytes written in place, on the sys layer (docs/magpie-in-crow.md, the
  plan to rewrite Magpie's kernels in Crow): `bytes[n]` (zero bytes to
  write into), `set[bytes, i, v]`, `push[bytes, v]`, and `concat[a, b]`
  appending where `a` lies — each in place where nothing else holds the
  bytes, in both modes, through the take bridges and the emitter's taken
  path the tensors and texts already use; the lowering's split of a
  self-reading write (`taken_in_place`) applies to any module's in-place
  row now, not the core's alone.
- Numbers in bytes, files by region and columns as their parts, on the
  sys layer (docs/magpie-in-crow.md, 2.2, 2.3 and 2.5): `pack[n, width,
  little]` and `unpack[bytes, at, width, little, signed]` (an int as its
  1, 2, 4 or 8 bytes and back), `ieee[x, width, little]` and its reader
  `ieee[bytes, at, width, little]`, `digest[bytes, 'crc32']` and `hash[…,
  'crc32']`; `file[path, from, n]` (a region read without the rest),
  `modified[path]`, `flushed[path]` (written through to the disk),
  `link[from, to]` (a name claimed once) and `move[from, to]`; and a
  column as its parts — `cell_kind`, `ints_of`, `floats_of`, `bools_of`,
  `millis_of`, `zones_of`, `vectors_of`, `vector_width`, `text_bytes_of`,
  `text_ends_of`, `present_of`, `missing_in` — and back, `ints_column`,
  `floats_column`, `bools_column`, `moments_column`, `vectors_column`,
  `text_column`, `text_dict_column`, with no value per cell.

### Breaking

- The Rosetta Code run's questions, decided (Vijay, 2026-10-29;
  rosetta/QUESTIONS.md). A function's local named like a name an import
  makes — `e` or `mean` under `use math`, `code` under `use bits` — is
  refused where it is written, as the same name at the top level, a
  parameter and a loop's variable were (errors k30, k31): it had shadowed
  the import silently. A computed whole number past 2^53 in a list written
  down as a vector stops where the list is made, naming `push` (454): it
  had been rounded silently, `[a, 3]` holding 2^60 for `a = 2 ^ 60 + 1`.
  `fmt`'s zero fill goes after a number's sign, `-0042` as printf pads,
  where it had been `00-42` (450). A page's top-level `seed[n]` runs before
  its declarations are set, so the facts' draws are seeded (452). The
  axis form of `concat` takes the axis as a number written down or named
  (`axis = k`): a third piece held in a variable is joined as a piece.
  The standard library's modules rename the 64 locals the first rule now
  refuses in their own tests (`start`, `flushed`, `e`, `head`, …), and
  Magpie's `read_var` answers the list of two ints its row says: written
  down, its pair had been a vector, which rounded a varint past 2^53 (the
  second rule's stop found it in the module's own test).

- `use memory` is the standard library's own, as `sys` is (Vijay,
  2026-10-28): a program's import is refused before it runs (`` `memory`
  is the standard library's own layer — a program may not import it ``),
  and process memory is what a module keeps between calls — a program's
  state is its variables, a page's its facts. The import check reads the
  module's `internal` flag where it had read the name `sys`, and the
  parity and errors harnesses open the door the book's pages had, so the
  programs about memory's rows run there.
- A call of a function that answers nothing, handed a value that can
  fail, answers that failure (spec, Auto-Propagation): dropped by its
  line, a function hands it back and the top level halts, `cannot drop
  an error value`. It had been typed as nothing, so the failure vanished
  in both modes (parity 370).
- `input[]` at the end of standard input is an error value, `.ok` false
  and reason `end of input` (333): it had answered empty text that did
  not fail, the same as an empty line, so a loop reading lines could not
  tell the two apart. Read lines with `line = input[]` / `while line.ok`.
- A variable, a parameter or a loop variable named like one of the
  program's own functions is refused where it is defined (spec, Names
  Are Never Ambiguous): `fn first[scores]` beside `fn scores[x]` had read
  `scores[0]` as the function's call, a silent wrong answer in both
  modes. A record's name read as a value (`p = Point`) is refused with
  the fix (it had been an internal error), and a number written with an
  exponent (`3.301e23`) is refused saying Crow writes none — `3.301 *
  10.0 ^ 23`, or `float['3.301e23']` — where it had been read as a
  second statement.
- One value for several names is refused before the program runs, in
  both modes (spec, Multiple Assignment; Design Note: No Destructuring):
  `a, b = xs` and `c, d = pair[]` had run interpreted and not built
  compiled. The refusal gives the form in the program's names (`r =
  pair[]`, then `c = r[0]`, `d = r[1]`). Values of another count than
  the names are refused too: a value past the names had been dropped,
  and a name past the values left unset.
- A loaded file's empty cell is missing in a column the program's reads
  type: read one at a time it is an error value of reason `missing`,
  `.ok` false, in both modes and for a page's data source too, as the
  spec says. The reads had typed the cells plain, and a missing cell had
  read as `''` or NaN with `.ok` true. A program that decides on one now
  meets the refusal a failed value meets, and a list of such cells used
  as a mask is refused: fill the column first. On the way, a call
  answering a list of values that can fail, given an argument that can
  fail, builds compiled, and an item read from such a list keeps its
  failure.
- A module of the program's takes no standard module's name (spec,
  Importing): `use math` where a `math.co` of the program's lies — or a
  package `math` under `crow_modules/` — names two modules and is
  refused at the import, naming the file and how to rename it. The
  import had reached the standard module silently, and the file's
  functions failed as the program ran. The standard library's own
  sources are not the program's.
- A column is written the way it is read (spec, Frames): `df.name =
  values` writes a column where the frame lies — a new column, or one
  replaced; a scalar repeated down the rows — and `with` is gone from
  the language (an unknown name now). After a dot a column's name reads
  as text does, a backtick inside it an interpolation: `df.lag_`k` = …`
  and `df.`name` = …` write a column by a computed name, and read one
  the same way; a header no dot can write is reached as `df.`'in px'``.
  `frame.new[a = xs, b = ys]` stays, each column such a write. Values of
  another length than the rows stop the program where `with` had
  answered an error value. `crow fix` has no rule: write `x = with[x,
  'n', v]` as `x.n = v`, a chain over `frame.new[]` as the literal, and
  a `with` in expression position as a write on the line before.
- One statement verb (spec, db, Statements): `query` runs every statement
  and answers a frame — the rows a SELECT answers, the rows a RETURNING
  clause reads back, an empty frame for a statement that answers none.
  `exec` is gone: write `query`, and a write that must say what it
  touched reads it back with `RETURNING`; the count of rows affected is
  not answered. Whether a call reads or writes is the statement's to say:
  a literal SELECT, VALUES, SHOW, EXPLAIN, DESCRIBE or PRAGMA asked is a
  read, anything else a write (RETURNING included), and a statement
  built as the program runs is a write — a derivation that hands one is
  refused, the fix being the literal with `?` for its parameters.
  `migrate[conn, dir]` is gone too: migrations are the tool's, `crow db
  apply <dir> <url>`, beside `crow db diff`. The db module is `connect`,
  `query`, `insert`, `transact` and `describe`. On the way: a ClickHouse
  statement that answers no rows had failed through `query` ("the rows
  end early"); it answers an empty frame now.

- Effects have kinds (spec, Effects Have Kinds): each builtin's row says
  what it reaches — `console`, `files`, `files.write`, `network`,
  `db.read`, `db.write`, `mail`, `clock`, `random`, `input`, `process`,
  `exit`, `shell`, `session`, `waits`, `serving` — and every rule about effects
  is stated by kind, once. What changes for a program: a view may reach
  nothing but the console, the clock (one moment per event) and a draw,
  and a derivation only reads (`files`, `network`, `db.read`, `clock`,
  `process`) —
  a view reading a file or querying a table (the book had taught "query
  at render time"; a derivation is where a page reads a table), a
  derivation sending a notice, had passed and were performed at every
  render; a call the context cannot hold is refused in one wording
  whatever the builtin, naming the function it is reached through
  (`mail`'s own wording goes). In a handler, a failed argument stops the
  handler only for an effect held to its commit (`session`, `db.write`,
  `mail`); a read, a request, a query or a command with a failed argument
  answers the failure, as any call does. `exists` is an effect (`files`),
  so a loop over it keeps its order and a derivation may read it; `args`
  and `env` are stamped with their capture moment, as reads of the world
  are. `crow check`'s reach names a file's effects by kind (`db.read`,
  `db.write`, `files.write`, `shell`, `input`, `waits` where it said
  `database`, `files`, `shell`, `process`).

- `chat`'s and `follow`'s `max` option is `tokens` (`chat[m, talk, tokens
  = 200]`): the module's options are its functions' defaults now, and a
  parameter may not be named like a builtin. A conversation's next user
  turn is `asked[talk, text]`, beside `said[talk, reply]` and `told[talk,
  call, text]`; `ask[text]` still begins one. A program that bound `asked`
  as a name of its own under `use llm` is refused, as any clash with an
  import is: rename it.
- A tensor compared with a number is a mask of its shape: a matrix's is
  its rows of bools (`[[1.0, 2.0], [3.0, 4.0]] gt 2.0` is `[[false,
  false], [true, true]]`), as numpy's is. `where` over it keeps the matrix
  (it had answered a vector), `not`, `and` and `or` keep its rows, `sum`
  counts it, and `m[m gt 2.0]` is the matrix's numbers where it is true
  (it had stopped). A tensor whose rank the program does not show,
  compared with a number, is refused, naming it.
- `min` and `max` answer NaN where a NaN is held — of a tensor, along an
  axis, of two numbers — as numpy's do and as `sum` carries one (a NaN had
  been passed over), and of no numbers the program stops (`max of
  nothing: an empty tensor has no largest number`; they had answered
  `-inf` and `inf`).
- `split[s, '']` is the text's characters, with no empty piece at either
  end (`['', 'a', 'b', 'c', '']` is `['a', 'b', 'c']`), and of empty text
  `[]`. A walk that skipped the ends by position (`slice[cs, 1, len[cs] -
  1]`) now drops real characters: walk `split[s, '']` as it is.
- `gamma_dist[α, β, n]`'s β is the rate, the mean α/β — the textbooks',
  R's, Stan's and PyTorch's (α, β) — so `gamma_dist[1.0, λ, n]` is
  `exponential[λ, n]`. It had been the scale, the mean α·β.
- A `later[f]` asked again while `f` is in flight runs `f` once more when
  it ends, from the state as it then stands, however many arrive
  meanwhile; it had been skipped, and a page could keep the earlier run's
  answer where the interpreter, running each inline, showed the latest.

### Added

- A column of vectors (spec, frame: "A `vector` column"; design
  rule 24's one exception this round, on Vijay's word): a frame's
  column holds vectors of one width — an embedding per row — written
  from a matrix or a list of vectors, read one cell at a time as a
  tensor and whole as its matrix (`tensor[df.e]`), shown as a tensor
  shows, carried through the verbs that need no order (a mask, `select`,
  `head`, `stack`, `join`, `group` with `count`/`first`/`last`, `where`,
  `unique`, `pivot`), refused by the ones that do (`sort`, a `sum` or
  `mean`, `rank`, `fill`, `unpivot`, a text function) in both modes with
  one wording, saved as JSON records or Parquet (a fixed-size list of
  float64) and read back, refused by CSV, and held by Magpie as a float
  chunk of rows × width (`narrow` 32 bits), never a key. The cell's type
  is a tensor inside the column's, so the compiler gained no type.
  `nearest[dir, column, vector, k = 10]` (magpie) finds the `k` rows
  nearest a query vector by the cosine — the vectors package's flat
  score, exact over every row, the pending rows included — through the
  door too (`/orders/nearest`); `packages/kg` gained `embed` and
  `similar` (an entity found by meaning, the stale versions a
  re-embedding leaves dropped), `packages/features` embeddings as
  features and `similar`. Parity 303, the module's, the kernel's and
  the packages' tests, the frame runtime's unit tests, the self-check's
  `colvec` value, and a book block each.

- Magpie, the data platform (`use magpie`; first built as `lake`, renamed
  2026-10-26 before any release: a lake is raw files other engines query,
  and this store holds its state and runs its own engines — the magpie is
  the corvid that gathers everything into one nest). Its second
  architecture is docs/magpie-design.md, built phase by phase below.
- `packages/kg` (phase 10 of the design, without its vector index, which
  waits on the column of vectors): a knowledge graph over Magpie — nodes
  and edges as logs under a root, the edges again in reverse so a node's
  in-edges are one key's run; `node`, `retire`, `link`, `unlink` stamped
  with their moment, tombstones every read skips; the graph as of a
  position (`upto` on every read), a node's `history`, `neighbors` and
  `inbound`, `walk` and `path` (breadth first, a node at a time), `match`
  (a pattern's triples read as edges and joined on their shared
  variables), `subgraph` (what `packages/graph`'s `of_edges` takes),
  `rule` (a function deriving edges, linked back with their provenance),
  `asof` (Magpie's `prior` over the edges).
- `packages/features` (phase 11, without embeddings as features, which
  wait on the column of vectors): a feature store over Magpie — a group is
  a log keyed by its entity with its time column (`declare`), one `append`
  is both stores (`online` from the sequencer's image, `history` by
  windows), one feature function for the `backfill` (windows of a source
  log) and the `stream` (its tail), `training` as the labels joined
  against each group as of their moments and of a position (held to
  pandas's `merge_asof` cell for cell, packages/features/held.co),
  `freshness`, and a registry log (`define`, `defined`). The design's
  `group`, `write` and `serve` are not names a program function may have
  (two core builtins and the web form): `declare`, `append`, `online`.
- The doors (phase 9 of the design): `magpie.door[root, port]` serves
  every log under the root over HTTP as JSON, a request's path naming the
  log and the verb (`/orders/latest`), and every verb given a URL
  (`since['http://host:9000/orders', 3]`) reaches it, so another process
  or language reaches a log; `use redis` is the RESP door — SET, GET, INCR, MGET, DEL, KEYS, DBSIZE over
  a log keyed by the key, XADD, XLEN, XRANGE and XREAD over a stream's
  log — so Redis clients speak to Magpie, beside the Kafka door.
- Aggregates pushed below the reader (phase 8 of the design): `summary[dir,
  by, of, how, on, ops, equal, lo, hi, upto]` runs filter, group and
  reduce as operators over each row group on its own core and combines
  the partials, answering what `group` answers over `scan` with the rows
  never held as one frame; `prior[dir, f, columns, upto]` is the as-of join
  over the log, each label's newest row at or before its moment (and at
  or before the position `upto`: the training set a position names), from
  each key's run. Sys rows `batch_summary`, `batch_asof`.
- Search's long tail (phase 7 of the design): the merge builds postings
  with positions and a sorted dictionary per text column per row group
  beside the token filter; `search` takes Lucene's forms inside the words
  (a phrase as a word with a space, a prefix `kit*`, a fuzzy word
  `kitchn~` within two edits), skips the blocks whose bound cannot reach
  the k-th score, stems where the log declared `stem = true` at its first
  batch (the Porter stemmer, English), and answers a `snippet` column of
  `snippet` characters around the first hit when asked. A batch, read
  whole, ranks and scores exactly as the merged segment does.
- The sequencer (phase 6 of the design): a memtable and its write-ahead
  log per log directory in the process (`crow_runtime::magpie::tail`).
  `put[dir, f]` appends rows visible to the process at once and flushed
  as a batch file by a thread every ten milliseconds; `batch` is a put
  and a flush; `latest[dir, equal = …]` answers the newest row of a key from
  the image in memory; `commit` over the log's keys is validated against
  the key index in memory; `sync`, `merge`, `backup` and `share` flush
  first. The pending rows are the catalog's `pending` row, a Magpie file
  in memory every reader opens as `<dir>/.pending@first-last` (the
  positions the listing saw; flushed in between, the file they went to is
  read instead, and a listing reads the pending rows before the files, so
  a flush between the two hides nothing); `rm` of a log's directory
  drops the rows pending in its tail with it (the thread had written
  them into the directory ten milliseconds after the rm). Sys rows `tail_put`,
  `tail_flush`, `tail_pending`, `tail_get`, `tail_last`, `tail_forget`;
  a third crash cut, `pending`, after a put.
- Reads bounded by what a frame holds (phase 5 of the design): `since[dir,
  from, upto, columns]` and `scan[dir, from, upto, columns]` take a
  position window, `latest[dir, upto, columns, lo, hi]` a range of the
  log's first key (through the groups the range covers; sys row
  `batch_between`), and `batch_since` an upper bound; a read over the
  whole log is a loop over windows the program writes.
- Indexes built by the merge (phase 4 of the design): `texts` leaves
  `batch` and `commit`; the merge indexes every text column of every
  segment per row group (an identifier-like column given up), and a
  search reads a batch, or an unindexed group, whole and scores it the
  same, so any text column is searchable the moment it lands and cheap
  once merged, and no producer declares what will be asked.
- The streaming merge with levels (phase 3 of the design): a k-way merge
  over the inputs, one row group per input in memory, segments cut at a
  gigabyte and named by a counter (`S000000000017.segment`), the newest
  version of each key the last of its run; `merge[dir, every]` folds the
  tail into level 1 and, while a level holds more than eight segments,
  its oldest eight into the next; a merge of a log larger than memory
  runs in constant memory. `narrow = [...]` on the first batch keeps the
  named float columns in 32 bits on disk. `sync` flushes the manifest
  too. The crash harness (tests/magpie_crash.rs) ends a merge after the
  segments are placed and after the manifest is switched, and the next
  read and merge answer whole either way.
- A manifest, and keys declared once (phase 1 of the design): `dir/manifest`,
  a Magpie file naming the log's live segments (level, position range,
  rows, bytes, key ranges) with the log's keys and its time column,
  replaced whole by a rename, so a merge is one atomic switch and a
  reader sees the old list or the new; the batches stay the tail the
  directory lists, and a segment the manifest does not name is a
  leftover the next merge removes. The first `batch` fixes the log's
  keys (`by`) and its time column (`time = 'ts'`); a later batch names
  the same keys or none, another set is refused naming both. `merge[dir,
  every]`, `latest[dir, upto, columns]` and `commit[dir, f, texts, seen,
  columns, equal]` take no `by`; `batches` gains `level`; a backup
  copies the manifest and writes a `listing`; `share` serves the
  manifest and `follow` takes it every time. New sys rows
  `batch_declared`, `batch_declare`, `batch_catalog`; `batch_merge[dir,
  paths, every]`. The Kafka door's group and offset logs declare their
  keys at their first write.
- Magpie's own file format (phase 2 of the design): a batch, a segment
  and, later, the manifest are one format of our own — row groups of
  about 4 MB, each column chunk encoded by its kind (ints by frame of
  reference or delta, text by dictionary or lengths and bytes, bools as a
  bitmap, floats as their bits) and compressed by Magpie's own LZ codec
  where that pays, the statistics per column per group (min, max,
  missing, distinct, a bloom filter; a token filter and counts per
  indexed text column) behind a two-level footer read per column. No
  Arrow, no Parquet and no conversion between the frame and the disk;
  `flate2` and `snap` leave the runtime, the Kafka door's gzip and snappy
  decoding written as our own (`crow_runtime/src/magpie/inflate.rs`,
  `snappy.rs`). Format 2; format 1 is not read (nothing was released).
  `batch_stats` answers the footer's facts (rows, positions, keys,
  groups, each column's kind and whether it is indexed, the token
  counts), not every column's bounds.
- The log as batch files, `use magpie` (spec, magpie): each `batch` one file
  written once by the kernel (sys, `batch_write`) — the log entry, the
  sorted level-0 segment with its statistics and the word index at once —
  `head`, `since`, `tail` (a view that follows the log), `scan`, `latest`
  (the table the log holds), `lookup` (every key at once, so a segment
  sorted by day then symbol reads one row group for one symbol's day;
  files pruned by their filters), `columns` on every reader (a segment
  decodes only the columns asked), `batch[…, flush = false]` and
  `sync[dir]` (Redis's default durability, when asked for),
  `commit` (the sequencer: a stale read refused), `merge` (the files
  folded into one Parquet segment of 8192-row groups, the latest version
  of each key kept, or every row with `every = true`), `batches` (the
  catalog's state), `search` (BM25 as Lucene scores it, over the whole
  log as one corpus, the files whose filters admit no word unread; held
  to the formula computed apart, parity 296), `metrics`, `backup` and
  `restore` (a directory or an `s3://` prefix, the manifest written
  last), `share` and `follow` (replication by log shipping, two
  processes in tests/magpie_share.rs). Every file's head carries the
  format's version, so an older Crow refuses a newer file by name. Held
  to SQLite (parity 294, 295), to a month of finch's ClickHouse and
  Postgres rows, and to the self-check. The fifth reference app,
  `examples/magpie`, is a page over a log.
- The Kafka door, `use kafka` (spec, kafka): `serve[root, port]` answers
  a 0.10.1 broker's protocol in Crow over the sys layer's listener
  (`listen`, `accept`, `address`), a topic a log under the root, a
  producer's set one batch, a consumer's fetch `since` its offset, groups
  of several members and committed offsets two logs of the door's own,
  a fetch with nothing to answer waiting up to the client's `max_wait_ms`;
  legacy message sets with their CRC-32 read and written by a kernel
  (sys, `records`, `records_of`, `unpack`), gzip and snappy sets decoded.
  Held to the pure-Python client in both modes (tests/kafka.rs).
- `sys.filesize[path]`: a file's bytes from its metadata.

- MCP over stdio (spec, mcp: The stdio transport): `server[command =
  'npx -y @modelcontextprotocol/server-filesystem /tmp']` names a server
  by the command that runs it, as `shell` takes a command; the process
  starts at the first message, stays while the thread that started it
  lives — one per command on each thread, so a served program's handlers
  never share one — and is started again if it ends; each request is one line on its input with
  the `_meta` every request carries, the response the line carrying the
  request's id, notifications and late answers passed over, a line that
  is no message refused with its words; the server's standard error is
  the program's. `discover`, `catalog`, `tool` and the loop's `servers =
  [s]` are the same calls over either transport. `listen[tools = [f,
  g]]` makes a Crow program such a server over its own standard streams,
  answering as `answer` answers a route — and a legacy client's
  `initialize` is refused naming the revision spoken, on both transports.
- A process the program talks to, in the sys layer (spec, sys):
  `start[command]` runs the command through `sh -c` with its standard
  input and output piped and answers a socket — `send` and `recv` on its
  stream, `leftover` and `unread` for what a read took past a message —
  one process per command on each thread; `close` of its stream ends
  it as the MCP stdio transport says a client ends its server: the input
  closed, its exit waited for, then TERM and KILL to its process group.
  No new handle kind: a process is a socket, as a WebSocket's connection
  is.
- The agent loop, `act` (spec, llm: The loop is the module's):
  `act[m, talk, tools = [f, g]]` runs the model's tools until it answers
  or a limit stops it — each reply's calls answered by the tool of its
  name through `apply`, a name no tool has, an argument of the wrong
  kind and a tool's own failure each told to the model as a failed
  result that says why, so every call is answered; `rounds` (8) and
  `budget` (tokens in all) as the limits, stopping after a reply's calls
  are answered so the conversation stays well-formed. Answers a record:
  `text`, `stop`, `rounds`, `usage`, `talk` and `steps`, a frame with a
  row per model call and tool call. `told[…, failed = true]` tells a
  failure by hand; Anthropic's wire carries it as `is_error`. Written in
  Crow in the module (docs/agents-design.md).
- The loop's gate: `allow = [g, h]` names functions of the call, each
  asked before a tool runs and answering a bool; a call one refuses is
  told to the model as a failed result (`remove is not allowed by the
  program`, `how` `refused` in the steps frame) rather than run, and a
  gate that takes another count of parameters or answers another kind is
  the call's error before any model is asked.
- A subagent is a tool that runs the loop (spec, llm): a function the
  parent model calls, whose body runs `act` with a model and a tool list
  of its own and answers the inner run's text — nothing new in the
  language, verified in both modes against a server that scripts a
  delegating run and against a local model. A run that failed fails the
  tool, so the parent model hears a failed result that says why; the
  parent's steps frame shows the delegation as one tool row whose `ms`
  covers the inner run, and the inner run's steps are the tool's own to
  read.
- The loop's context, skills and servers: `keep = n` cuts the tool
  results of rounds older than the last n to a stub naming their size,
  `compact = n` asks the model for a summary once a reply has read n
  tokens and goes on from the summary alone, `skills = folder` reads a
  folder of skills in the Agent Skills format (agentskills.io: each
  skill's `SKILL.md` frontmatter held to the standard's rules), names
  them in the system prompt and lets the model load one, and the files
  it refers to, through the tools `skill` and `skill_file`; `servers =
  [s]` adds the tools MCP servers offer, called on the server.
- `use mcp` (spec, section 13): the Model Context Protocol, revision
  2026-07-28, over Streamable HTTP — `server[url]` names an endpoint,
  `discover[s]`, `tools[s]` and `tool[s, name, given]` ask it, and
  `answer[req, tools = [f, g]]` in a POST route makes a Crow program a
  server over the functions themselves (`server/discover`, `tools/list`,
  `tools/call`, the errors the protocol names). Written in Crow on
  `request`; tests/mcp.rs drives both directions in both modes.
- An agent with traces, evals and a page, `examples/agent` (the fourth
  reference app; docs/agents-design.md, Traces, evals and a page): a
  support agent over a shop's orders in SQLite — three tools, the refund
  behind the program's gate — every run written to the database as a
  trace (a row per run with its question, model, answer, stop, rounds,
  tokens and milliseconds; a row per step, the loop's own frame handed to
  `insert`) and a page over the traces, the tokens and the latency of
  each run charted; evals as `test` blocks replayed from recorded
  fixtures in both modes — exact checks on the steps frame (the tools
  called in order, the gate's refusal, the stop) and a model-as-judge
  asked for PASS or FAIL — and a page that asks the agent and shows the
  run's steps. Nothing new in the module: the judge is a `chat` with a
  rubric, kept in the example.
- The package lock holds the bytes (spec, Package Manager): `crow.lock`
  records the hash of each dependency as installed beside its commit,
  every command that reads a program refuses a package changed by hand
  under `crow_modules/` until `crow install` restores it, and an install
  refuses a fetch of a locked commit that hashes otherwise. A requirer
  may attest the hash it verified after the version
  (`…@v0.1.2#sha256:…`), in a program's crowfile or a package's, and the
  install holds the bytes to every attestation it meets. Two requirements
  of one package at tags of one major take the higher, saying so, where
  any two sources had been refused; tags of two majors, branches and
  commits that differ still are.

### Changed

- Magpie is Crow: its Rust kernels — the codec, bit packing, chunk
  encodings, bloom filter, stemmer, postings, file format, manifest,
  sequencer, merge, executor, search, nearest, inflate, snappy and the
  Kafka records (`crow_runtime::magpie`, `kafka.rs`, 7,580 lines) — are
  deleted, with the sys rows that reached them (`batch_*`, `tail_*`,
  `records`, `records_of`, `stream_rs` and the test-only `*_rs` oracles).
  The modules' tests hold the Crow port to the bytes, frames and words the
  kernels answered, recorded before they went (docs/magpie-in-crow.md).
- `later[f]` in a compiled script coalesces where it had skipped: a
  schedule of `f` while `f` runs makes it run once more when it ends, so
  its last run reads every write made before it (spec §15; the sequencer's
  flush). The interpreter runs the work inline, as before.
- The merge indexes no category column (magpie): a text column with
  fewer than one distinct token per 64 documents — a symbol, a day —
  gets no token index, as an identifier-like column gets none, since
  every row would be a posting and the group's filter over its values
  already admits or refuses the group (`lookup` reads the rows); the
  position column gets no filter, its values all distinct and its
  bounds pruning by range. Measured 2026-10-27 on finch's month of
  bars: 187 MB of postings over `symbol` and `day` and 11 MB of filters
  over `_pos` on a segment whose column chunks are 266 MB, pruning
  nothing the filters did not.

- `crow fmt` indents at four spaces, the unit every document, the book
  and the standard modules write (it had normalized every file to two),
  and reads levels from the nesting: a line inside a bracket moves with
  its statement, keeping its alignment, and the lines a string written
  across lines runs onto are left as written. One continuation aligned
  at an odd width had multiplied every level of a file
  (`tests/layout/conformance.co` came out four times as deep), and a
  string's lines had been re-indented, changing its text.
- `crow help` lists every command and flag the binary takes, one line
  each, on standard output, held by a test to the flags the binary
  reads; `crow file.co` is documented as the short form of `crow run
  file.co`.
- `crow new --db` declares the program's tables as the db section does:
  `db/schema.co` holds a `setup[conn]` running `CREATE TABLE IF NOT
  EXISTS users (…)`, and `main.co` opens `sqlite://<name>.db` and hands
  it to `schema.setup`; `crow db diff` drafts what a database lacks of
  them. The scaffold lists no migrate job.
- `crow check`'s rules for a view that queries, reads a file or waits
  are gone — a remote query in a view, its `--- crow: remote-render`
  acknowledgment, and blocking rooted at a view — since the compiler
  refuses such a view first; blocking in a handler stays.
- A name beginning with `_` is one `crow check` asks nothing of: `_kept
  = rm[tmp]` keeps a failure it ignores, and the unused-variable advice
  names that form (it had said "use _", which parses nowhere).
- `.coc` bytecode is no longer read (nothing wrote it since `crow
  compile` went): a project's `main.coc` and `crow x.coc` are not Crow
  files.
- A call a view may not make is sent where its kind belongs (spec,
  Effects Have Kinds): a read a derivation may make — a file, the
  network, a database, the process's environment — to a derivation the
  view reads (``read it in a derivation the view reads — `derive x`
  above the view, returning it``), as the book teaches, and a write or
  any other kind to a handler, as before; a call of several kinds goes
  where the one a derivation cannot hold says (`recv`, which waits, is a
  handler's). Every refusal had sent the call to a handler.
- A method call written as another language writes one is taught the
  Crow form there is, never a function Crow lacks (spec, Error
  Messages): `datetime.now()` names `now[]` (it had said `now[datetime,
  …]`), `xs.includes(1)` and its kin name `1 in xs` (it had said
  `includes[xs, …]`), a call through an imported module is that call in
  square brackets, a value's method is the builtin of its name with the
  value first (`upper[s]`), and anything else is told that a call is
  `f[x]`.
- The examples reach `claude-sonnet-5-5` (they had named
  `claude-sonnet-5`): llm's hint, its tests, the spec, the book and the
  design notes.
- The core's names are held, not frozen (spec, Three namespace tiers;
  the book's chapter 8): until 1.0 a core name is added only as a
  breaking change the changelog lists first, since a name it reserves
  may be one a program used, and from 1.0 the core does not grow. The
  spec had said the core never grows while `occurs`, `translate`,
  `decimal`, `fraction`, `halt` and `bucket` joined it this month.
- `packages/models`: `search` answers `Search[best, score, scores]`, the
  best candidate's mean in `score`; the field had been `scorer`, a slip
  of the rename of the function parameter beside it.
- The comparison's ratings follow the measurements of 2026-10-21: Go's
  performance is 9 beside Crow's (comparable on the seven programs),
  Crow's parallelism 8 (data-parallel only: requests and queries never
  overlap) and its data structures 7 (frames, tensors, exact numbers,
  moments).
- The documentation no test reads describes the language as it is
  (2026-10-21): `docs/crow-for-llms.md` (moments as a kind, `in`, ints
  that grow, and the modules a generator reaches for — frames, db, web,
  llm, mcp — every block run), the feature map, the by-example index,
  Appendix B, the comparison (the measurements of 2026-10-21, a section
  on language models), every command and flag in `docs/usage.md` and the
  book's toolchain page, `crow check`'s and the language server's notes,
  the db, frame and math notes, the roadmap's status, and the docs index
  (ten notes it had not listed). In the spec: the error kinds the
  modules answer (`'http'`, `'socket'`, `'auth'`, `'mail'`, `'llm'`,
  `'mcp'`) and what answers `'time'`; a handler's stop by effect kind; a
  dynamic cell read as its use implies; the project layout as the
  toolchain reads it (no folder is discovered but `jobs/`, through the
  crowfile); the note "a moment was a number" marked superseded.
- A parameter read by its fields, and typed by nothing else, is a parsed
  value (spec, Tools are the functions): a function called by name —
  the loop's gate, a tool taking an object, an MCP server's — had been
  refused as "typed by nothing — read it in the body", which its body
  did. A parameter any call types keeps that type; the rule only fills
  the case the compiler refused.
- The book's standard-library pages (`docs/book/src/stdlib/`) are
  generated for every module the manifest lists that has a section of
  its own in the spec, in the manifest's order; a hand-kept list in the
  docs check had lacked a module's page while the book's index linked it.
- The shape of the tensor a builtin answers is declared on its row
  (`table::Shape`, a fixed vocabulary: an argument's own shape, a
  broadcast, a dimension per size, each dimension stated, a reduction, a
  join along an axis, a transpose, rows stacked, a decomposition's
  pieces), read where the call is typed; the compiler's rule by name is
  gone (docs/extending-crow.md, phase 6). What a program sees: the rank
  of every builtin's answer as before, and a list handed where a tensor
  is taken keeps the rank it had as a list. A row that answers a frame
  declares its columns where its kernel fixes them (doc's `chapters`,
  chart's `macd`), and `query`'s row names its provider: what a database
  says a literal statement answers — asked, kept, typed from the
  statement where SQLite does not say, marked where it can hold NULL —
  is crow_db's (`crow_db::describe`), handed the connection's literal
  URL, the statement and the tables the program declares; the form that
  opens a connection says so on its row (`Prop::Connects`). What a
  program sees is as before.

- A handle — a database connection and a transaction's, a socket, a
  stream, a mailer, a response, the server's app, a test's app session —
  is one kind to the language (spec, Type System: Handles), declared
  once beside its module's rows with its name, its word, the type the
  compiled program holds it as and whether a session may hold it; every
  rule about handles (moved not copied, not indexed, not JSON, equal to
  no other, shown as its word, a fact of one the process's) reads the
  one declaration where it had listed the kinds (docs/extending-crow.md,
  phase 5). What a program sees: `type[x]` and the words of a refusal are
  as before.

- A builtin that takes a function declares it on its row, with its
  signature (`any`, `all`, `find`, `later`, `running`, `stream`, `route`,
  `transact`, a chart's `seek`): inference hands the function its
  parameters from the declaration, the kinds check refuses anything else
  in the row's words, the width pass and the classification read the
  same declaration — each had been a list of the builtins' names in the
  compiler (docs/extending-crow.md, phase 4). A list literal of functions
  is a group, held by name: a call through one is a dispatch the compiler
  writes (the members take and answer one kind), and members of several
  signatures are described and called by name alone (spec,
  Specialization). The standard modules describe and call functions
  through sys's `describe[f]` and `apply[f, args]`; llm's tools are
  described in the module (`chat`'s and `follow`'s schemas were the
  compiler's), so a tools list may hold functions of several signatures.

- The compiler holds the language and nothing of any module (design rule
  24; docs/extending-crow.md): what a builtin is — its options and their
  defaults, its effects, its in-place and growing forms, the range of the
  int it answers, the crate feature it needs — is declared on its row, and
  what a module is on its manifest (`modules::MODULES`); the compiler's
  hand-kept lists of names, which had disagreed with each other, are
  gone, and `src/freeze.rs` fails when a compiler file learns a module's
  or a builtin's name. No program behaves differently, except where a
  call's options are refused: the wording comes from the rows now
  (`window[] takes 3 arguments, then n = 0, by = [], span = 0.0, on = ''
  by name`), and `fmt[t, 'HH:mm']` with the style given by position is
  the time style's, where it had been refused. `use web.ui` is a ui
  program to every check, as it was to the lowering.
- `crow test --modules` runs every standard module written in Crow
  against its own test blocks, from the manifest.
- A standard module written in Crow declares its functions by writing
  them (phase 2 of docs/extending-crow.md): a top-level function with no
  leading underscore is what a program calls, typed by its use as any
  function is, its options its defaults — which may now be `[]` or
  `map[]` — and a helper begins with an underscore. The statistics, the
  neural-network formulas, the token functions, a socket's `socket` and
  `close`, the model functions and the protocol's own are such functions
  now; 45 rows of the builtin table are gone with the JSON text the
  values had crossed them as. A misuse inside one is refused
  naming it and the program's line (`in relu: clip[] takes numbers, got
  an int`). The effects (`read`, `write`, `request`, `mail`, the readers,
  a socket's `send` and `recv`), the variadics (`randn`, `normal`,
  `anova`), `msgpack` (a value of any kind, kept as it is) and the
  database drivers keep their rows until their phases.

- The VS Code extension depends on no npm package: it speaks LSP to `crow
  lsp` itself, in one file of plain JavaScript over VS Code's own API, and
  is packaged by Python's standard library. No workflow runs Node.js: the
  release, CI and the book's site use git, rustup, `gh`, Python and curl
  where they used GitHub's JavaScript actions, npm and `vsce`. The
  extension is published to Open VSX with each release; the Marketplace
  takes the release's `.vsix` by its web page.
- `rsi`'s warm-up is the spec's: from the second row, the mean gain and
  loss of the deltas there are until `n` have come, then Wilder's
  smoothing — as it was, now written down and held to a reference.

### Fixed

- A refusal inside a standard module's helper two exports share is
  blamed on the export whose call made the refused copy, at that
  call's line (errors j80): `lookup`, valid, had answered for a list
  handed to `latest`'s `upto`, since the helper's copy was lowered
  before its caller's body and the first export reaching the helper was
  named. The parity harness fails a program refused in both modes that
  printed nothing (a refusal is an errors case; a program that stops as
  it runs is parity as before): parity 233 had passed for weeks refused
  (`abs[j.n]` of a parsed value; `abs[int[j.n]]` now). The freeze ratchet
  holds each compiler file to the set of name-shaped literals it held at
  the bless (`tests/freeze/names.txt`), not a count, and records the
  literals that are no name, so a module may later export a word the
  emitter already held (`get`): the count had made magpie's `get` and
  `asof` cost their names. A loaded cell handed to a builtin that takes
  numbers one at a time and a tensor whole (`max`) is a number: the
  tensor row types a cell only where no row of the name takes a scalar
  (the market package had lost its cells' kind). A package of one file
  is `main.co`, which `use kg` resolves to from inside the package and
  from a program that depends on it (kg, features). `latest[dir, columns
  = […]]` reads the log's keys and `_pos` beside the columns asked and
  answers with them: a list lacking them had stopped the fold (`sort:
  column '_pos' not found`).

- The left side of an operator runs before the right side's own
  statements: `put[d, f] eq 1 and latest[d].id eq ['a', 'b']` had read
  the log before the put — the read of a value that can fail is hoisted
  ahead of the statement, and the put had stayed in the expression —
  wherever both sides are computed (a mask's `and`, a comparison,
  arithmetic; `and`/`or` of two truths had already waited). A left side
  that acts is bound first; a quiet one (a name, a literal, a field) is
  not (parity 302). And a column compared with a list, a column or a
  tensor is one truth, as the runtime answers it, not a mask — typed as
  a mask, such an `and` had joined item by item and the compiled program
  had not built (`no method iter for bool`); a column compared with one
  value is a mask as before.
- Inside a standard module written in Crow, a helper's call of a row is
  held to the row's kinds as the program's own calls are: the check had
  read the program's imports, where the module's own imports answer, so
  a list the program handed as `prior`'s position reached `batch_asof`
  through the helper `_asof` — a stop interpreted, no build compiled —
  while a program's own helper chain was refused. The refusal inside a
  helper, which no program calls, names the module function the program
  called whose body reaches it, at the program's line (`in prior:
  batch_asof[] takes … an int, got … a list of ints`); the exported
  functions no longer need to say a parameter's kind in their own bodies
  for the helper's sake (`prior`'s `pack[upto, 8]` is gone).
- A value whose making was refused — an undefined function's answer
  handed to a function copied per signature, typed as the copy a call
  with nothing known reaches — is not judged at a write into it either:
  `v = relu[nosuch[…]]` beside a `relu` of a matrix, then `v[2] = 4.2`,
  had been refused twice, the second time as a matrix write (the
  mistakes fuzzer's seed 1793066246). One mistake, one message.
- `crow check --self` runs every package's test file in a directory of
  its own, as it runs a module's, the run holding a link to the
  repository's `packages` for the references the tests read: kg's four
  runs at once in the repository's root had removed each other's logs.
- Five shapes over a log's open frames that one mode had accepted and
  the other refused (parity 297, found by the magpie's benchmark): a frame
  literal built from an open frame's cells — `frame.new[last =
  [rows.close[i]]]` — builds compiled (the runtime's `Cells` takes a
  list of parsed values: whole numbers as ints, numbers as floats,
  bools, texts, a null a missing cell); a mask conjunction over a frame
  that can fail (`ds[ds.day eq day and ds.status eq 'ready']` of a
  `latest`) is joined truth by truth where the emitter had printed `&`
  of two lists; `sum[where[mask, column, 0.0]]` over a column that can
  fail converts the marked column for math's sum (the verifier had
  refused the call); a helper handed a frame begun as `frame.new[]` and
  filled in a branch is one copy with the open frame's (a frame of no
  known columns is no signature of its own, and the recorded frames are
  joined); and a fallible local assigned in a branch under its own
  guard (`if hit.ok` then `out = hit`) copies its payload out where the
  local is read again (compiled, the frame had been moved: E0382).
- The interpreter compares a column of one kind with a value in one
  loop — numbers, moments or text — where it had asked `holds` of each
  cell: a day's 871,527 timestamps against an instant 12 → 7 ms. A
  column is still read into its cells first (6 ns a row); a column
  value kind in the interpreter is the fix behind this one.

- Ending a process `start` began signals its own group and nothing else
  on Linux: the group had been written `kill -TERM -<pid>`, which
  procps's `kill` (4.0.4, Ubuntu's) reads as a signal to every process
  the user may signal when the pid begins with 1 — on CI it ended the
  unit tests' step and a coverage runner, where macOS's `kill` had read
  it right. The group is named after `--`, and a pid of 0 or 1 is
  signalled not at all.
- A line that holds only a comment opens and closes no block, whatever
  its indentation, as a blank line does (spec, Blocks), in a file and in
  the console. A trailing comment continued aligned under itself had
  been refused ("this line is indented, but the line above it opens no
  block"), a comment at the margin inside a block had ended the block,
  and one before a chain's first `then` step left the step indented
  under nothing.
- A string written across lines counts its lines: a stop, an error's
  trace and a refusal after one had named a line short by every line
  break the string held, in both modes (`line 4` for line 6). A refusal
  on the line such a string closes on is placed and underlined there, in
  the terminal and the editor.
- A crowfile holding `cpu` or `gpu`, keys the spec documents, stopped
  every command ("'cpu' is a core builtin — pick another name for this
  variable"): the crowfile's names are keys, not a program's variables.
  A key another key reads is no longer read back as `done`. Both modes
  take both counts, for the entry and for a job (a compiled run had
  taken `cpu` alone), `crow build --ir` and `--rs` print the entry with
  them, and `crow run -i <job>` runs the job interpreted (it had read
  the job's name as a file's).
- In the console and a notebook, a function one cell defines is the next
  cell's to call (an internal error: "calls 'double', which the program
  does not define"), and a cell that defines a function again runs its
  new body.
- `crow new`: `--db` wrote a `table users` block that does not parse and
  listed a `jobs/migrate.co` it never wrote; `--ml`'s job called `train`
  bare, which is undefined. Every file a flavour scaffolds runs now,
  held by a test of each flavour.
- `crow fix --hazard`'s constraint migration is written as the next
  numbered file and applied as `crow db apply` applies it, recorded in
  `_crow_migrations` (the next `crow db apply` had run it again: "index
  … already exists"). With `--db migrations://dir` it goes in that
  directory ("unknown database URL scheme"). The check prints the exact
  command, the `--db` it was handed included, and the exact file name.
  No message names `crow migrate`.
- `crow check` reported a control whose handler only runs `CREATE TABLE`
  (a setup button, as the book writes one) as a dead control: a database
  write is something a control does. Its reach named a writing query
  `__query_write`; it names it `query`. A failed example truth was
  reported "(at n = 0)", naming an input it never read (the book's
  chapter 9 `fee`): a failure names the inputs only where its line reads
  one.
- A program's own `db/schema.co` is imported by its dot path, `use
  db.schema`, as `views/home.co` is: every `use db.…` had been refused
  as a driver's own module (a driver's, `db.postgres`, still is).
- A second derivative is refused before the program runs, as the spec
  says (Automatic Differentiation): `fn df[x]` answering `grad[f][x]`,
  then `grad[df][2.0]`, had answered 0 in both modes — the inner
  gradient ran on the dual numbers' values alone — where the second
  derivative of x³ at 2 is 12, and `grad[grad[f]]` had stopped as it
  ran, interpreted, and not built compiled. `grad` inside a function
  `grad` differentiates, or inside one it reaches, is refused at its
  line (`grad: df calls grad, and a function grad differentiates
  cannot`), and `grad` handed a gradient where it is written; a function
  that computes a derivative and is differentiated by nothing runs as
  before.
- A gradient held in a variable takes what a call takes: `dloss =
  grad[loss]`, then `dloss[w, xs]` with `xs` read by `load`, had been
  refused on its only assignment ("cannot reassign 'dloss'"), where
  `grad[loss][w, xs]` written inline ran. A gradient applied to a value
  that failed answers that failure, the leftmost, in both modes (the
  interpreter had handed it in, and the compiled call had stopped or
  answered it by whether the failure was the first argument), and a
  function value applied twice to a local that can fail builds compiled.
- `y = 1.0`, then `y = log[3]`, was an internal error ("the lowered
  program disagrees with itself"). The answer of a call that answers
  nothing — a builtin whose row declares no answer (`log`, `sleep`,
  `exit`, `seed` …), or a function of the program's whose every return
  hands on nothing — given to a variable, a field, an item or names
  assigned together is refused where it is written, in both modes:
  `log[] answers nothing, which 'y' cannot hold — call it on a line of
  its own`. A new variable given one had held `done`, and a record's
  field had changed kind.
- A line that drops an index by a mask (`df[df.age gt 50]` alone) is
  refused naming the form the program wrote — `` `f[mask]` answers a
  frame, and this line drops it — keep it: df = df[…] `` — where it had
  named the hidden `__pick_frame[]`; `crow fix` and the language server
  apply the keep.
- A call through a standard module of a name it does not have (`use
  math`, then `math.double[5]`) is refused before the program runs, the
  nearest of its names offered where one is a slip away (`math has no
  'meen' — did you mean 'mean'?`): `math`, a name its own import makes
  so that its constants read, had let any `math.f[…]` by, typed as
  nothing known, to stop as the program ran, interpreted, and not build
  compiled.
- A column the program writes on a loaded frame (`df.when = …`) is the
  program's from then on: what it reads of it afterwards asks nothing of
  the file. A column of moments written and compared had stopped the
  load ("the frame has no column 'when'").
- A statement handed a parameter that failed answers the failure, as any
  call does: the interpreter had stopped (`cannot bind done as SQL
  param`), inside `transact` too, where the compiled program answered
  it. A list parameter holding an item that failed builds compiled. In a
  handler a read answers the failure and a write stops the handler, in
  both modes; a read had stopped it in both.
- `insert` answers the database's refusal (rows that break a constraint,
  a column the table lacks) as an error value of kind `'db'`, in both
  modes: the compiled program had unwrapped it and panicked, and the
  interpreter had stopped.
- `describe[conn, sql]` answers what the compiler reads: a computed
  column typed from the statement, a column that can hold NULL marked
  `?` on SQLite and Postgres, `IN ?` described as a list of one. It had
  answered the database's word alone.
- `crow test --docs` reads a call of a name only where `name[` begins a
  name: the coverage check could have counted a builtin as called inside
  a longer name, and the standard library's pages had linked `area[…]`
  to the `a * b` row and `greet[…]` to the moment's `t` rows.
- The task board's committed baseline is the app as it is (`query` where
  it had `exec`, reach by effect kind): it had been accepted before both
  changed, and no suite diffs it.
- The book's site is built again (`docs/book/site.co`): its page view
  read a file for the mark's version and the harness's sample chapter
  was read inside a view, which a view may not do since effects have
  kinds — CI's test job and the Pages workflow had failed at that step
  on every push since 2026-10-20, so the live book stayed at
  2026-10-18's. The version is worked out once and handed to the view,
  the sample's chapter is written in the program, and the gate's list
  in `CLAUDE.md` runs the step as CI does.
- The book's contents list the `chart` and `sys` reference pages: the
  site writes the pages its contents list, and the standard library's
  index had linked two it never wrote.
- Examples that did not run: `examples/dashboard.co` queried inside its
  view, refused since effects have kinds (it reads through a derivation,
  as the book's last chapter does); `examples/multifile` named its own
  module `math`, which `use math` never reaches (it is `arith`, and a
  function is handed `pi` as a parameter, which it does not see
  otherwise); and the parallelism examples described statements run in
  waves and a loop that logs running at once — they show a loop whose
  body only computes, timed at one thread and at eight. Five of the
  book's error blocks were refused for an indented comment rather than
  the mistake they show; each is refused for its own now.
- The interpreter had run two to four times slower since the compiler's
  freeze began (n-body 2.0 s to 8.0 s, k-nucleotide and reverse
  complement twice their time; the compiled program untouched): the
  questions it asks of a builtin's name after every call — whether the
  answer is stamped, whether a list grows in place, whether a rehearsal
  records the call — had become scans of the whole table by name when the
  hand-kept lists became the table's. The table is indexed by name, built
  once, a row's stamp is read by its place in the table, and a rehearsal
  is asked about first; n-body is back to 2.2 s. A test holds the index to
  the scan's answers and to being far faster than one.
- A parameter reassigned in its body from a value that can fail (`words
  = maybe[1]`) keeps the kind its calls hand it: the body had typed an
  untyped parameter as a value that can fail before any call typed it,
  and a call's plain argument — a function's answer, a local, a value
  that can fail — was then handed bare to a `Result` parameter; the
  interpreter ran it and the compiled program did not build (E0308),
  while a literal argument, which types the parameter first, built. The
  type had depended on the order inference met the body and the call.
  `act[…, system = prompt[]]` is such a call: the loop's `system` is
  reassigned from the skills' prompt (parity 278; found building
  examples/agent).
- A part's function is named as the program wrote it wherever `describe`
  names it — the name a model is told, calls back and the steps frame
  records, and the name a refusal uses: it had been the part-qualified
  internal name (`agent_support_order_status`) when an entry imported the
  part and the written name when the part was the entry, so a trace and a
  recorded fixture differed by which file was run (tests/described.rs).
- `crow check`'s snapshot store keeps an answer holding a line break: it
  had written the answer whole, one line per answer, and read it back as
  its first line, so a function answering several lines was reported
  changed by every check after the first. A backslash, a line break and a
  carriage return are escaped on the line now; a stored answer holding a
  backslash is rewritten in that form at the next check, and reads the
  same.
- A program's function in a group handed to a standard module's function
  (`tools = [weather]` to `act`) was never noted as called where the
  module applied it by name, since a program's functions are invisible
  inside a module: the function stayed untyped and the compiled program
  called one it did not hold ("cannot find function `weather`"). The
  note records the signature itself now, and the dispatch's call of such
  a function is lowered as the program's call it is, where the interpreter
  had met a by-name call of a function it does not run (parity 274,
  tests/llm_server.rs).
- A program's own top-level value named with a standard module's prefix
  (`mcp_s = server[…]` under `use mcp`) was printed as a Rust constant,
  by the rule that hoists a user module's values, and the compiled program
  did not build: a standard module hoists no value, so its prefix names
  none now.
- A function copied per signature was scanned without entering its own
  scope: its first assignment to a name was not a local of its own, and
  inside a standard module's copy the program's top-level names were
  visible. The copy of llm's `act` made for a second tool list read the
  program's `one` — the record the first call answered — as the loop's
  local `one`, and refused the module's own push in both modes (parity
  275). A copy's scan enters the function's scope as the function's own
  does.
- `find['a', 'b']` under `use sys` had reached the layer's `find` of
  bytes (the one resolver read the core's runtime rows alone, and the
  core's `find` is a form), and the lowered program disagreed with
  itself: a core form that declares its kinds comes before a module's
  row, as a core row does.
- A derivation indexed (`trades[trades.qty gte min_qty]`, a mask over a
  derived frame) read as a call of the derivation, "'trades' expects 0
  arguments, got 1": a derivation is read like a fact and indexed like
  one. And a call written with its module was read for its effects by
  its bare name, so `ui.input[…]`, the element, counted as the core's
  `input`, which waits on the reader: the module's own row says.
- A function called from a copy of its caller, with an argument whose
  kind settles only once another copy answers (`inner[may[m, false]]`
  inside a function called with text and with a parsed value), was never
  copied for that kind, and the compiled program did not build; a loop's
  variable whose item kind was not yet known kept a binding of the same
  name from outside the loop; a standard module's local named like a
  page's fact made the module's function a handler; a value that can
  fail, converted where a function's parameter takes another form and
  lent to it, was handed by value, so the compiled program did not
  build; and a loop's item from a frame's column whose cells can be
  missing was typed as nothing known by the width pass (the rule for a
  loop's item knew no column), so a function writing the item into text
  did not build compiled, where the interpreter stopped on the missing
  cell (parity 268). Each fixed.
- Three misuses the lowering had let through to a backend that could not
  build them are refused before the program runs, in both modes, each
  naming what it found: a list whose items no type says compared with
  text (two kinds are not compared; bytes compare with bytes alone), a
  function returning two kinds (`return 'a'` beside `return x` of an int:
  `returns an int where the function answers text — a function answers
  one kind`, where it had been an internal error), and a field read of a
  value that has none or that a tensor or a moment lacks (`text has no
  field 'provider'`, `numbers has no field 'id' — its fields are shape,
  dtype, device`; a map's field is its key, read as the program runs).
  And a parameter handed `[]` that the body reads where a builtin takes
  numbers is a list of numbers (`mse[[], m]` stops as the row had: `pred
  and target must be one length`), where the compiled program had held
  the list as text and not built. Found by crow check --self once the
  modules' rows were gone.

## v0.7.0 — 2026-10-18

Crow held to itself: `crow check --self` runs about 100,000 cases from the
compiler's own tables in five configurations, each answer held to the
spec's, and what it found is fixed. Breaking changes first.

### Breaking

- A value that can fail, dropped by its line — `write[path, text]`,
  `query[conn, sql]` or `rm[path]` on a line of its own — is decided on as
  a branch on it is: a function hands the failure to its caller, a
  handler stops, the top level halts with `cannot drop an error value —
  <reason>`. Its failure had gone by unseen in both modes. Assign the
  value to keep a failure on purpose (`kept = rm[tmp]`). To find the lines
  a program has, `CROW_DROP_AUDIT=1 crow build --ir file.co` lists them.
- `slice` counts a negative position from the end, as an index does:
  `slice[xs, 0, -1]` is all but the last (it was empty: a negative end had
  been taken as 0), `slice[xs, -2, len[xs]]` the last two.
- `f.kind` (or `.ok`, `.reason`, `.trace`) of a frame that holds a column
  of that name is refused, naming `rename[f, 'kind', 'another']`: the dot
  is the frame's status, never the column, and it had read the status
  where the column was meant.
- `describe[conn, sql]` answers a frame of `name` and `type`; its `kind`
  column had been unreachable as `d.kind`.
- `m[i] = x` on a matrix is refused (one position of a matrix is its row):
  it had written the flat number `i` while `m[i]` read the row. A number
  of a matrix is written `m[i][j] = x`, a row `m[i] = row`.
- `filter` is removed: a mask picks a frame's rows as it picks a list's
  items, `f[f.age gt 30]` (`filter[a, b]` is `a[b]`). A mask that compares
  the frame's own columns is still computed in one pass. A frame's index
  takes a mask alone: `f[0]` and `f['name']` stay refused.
- `pad`, `append`, `date`, `elapsed`, `today` and `nodes` are unknown
  names like any other: the messages that named their replacements are
  gone, and a program may give the names to functions of its own.
- `-x ^ 2` is `-(x ^ 2)`: the power binds before the sign (`-3 ^ 2` is
  -9; it had been 9).
- A matrix's items are its rows: `slice`, `reverse` and `pop` of a matrix
  take rows (they had counted its numbers), and `sort`, `x in m` and
  `join` of a matrix are refused.
- A route's handler answers a response, and a function answers on every
  path: a handler that answers none is refused in a program that serves
  (compiled it had been an empty 200), and a function that can end
  without its value is refused at the line (it had answered `done`
  interpreted and not built compiled).
- A response's body is text and its status HTTP's: `text`, `html`,
  `redirect` and `respond` of another kind are refused (compiled they had
  not built), and `respond`'s status is 100 to 999 — refused where it is
  written outside, a stop where it is computed (it had answered 599
  interpreted and 200 compiled).
- `later[f, a, b]` in a page's handler is refused: a page's `later[f]`
  runs `f` on the page's state, which keeps what `f` needs (compiled, the
  arguments had been dropped and the page had not built). A script's or a
  server's `later[f, a, b]` is as it was.
- A select bound to a fact takes only a value it offers (a radio's and a
  tab's too): a forged event's value is refused, unless a free input binds
  the same fact.
- Refused where they are written, in both modes (one had stopped
  interpreted and not built compiled, or answered differently): a write
  into text, a frame, a column or a parsed value; a mask as a write's
  position; an item, a map's value or a record's field of another kind; a
  function value nothing types, wherever it is handed; a function that
  takes a function called at two kinds; a field asked of a list that can
  fail (`csv[…].b`), as one of a list is; `encode` of bytes (it had
  answered null); handles compared; `tensor / tensor`.
- A trailing window over times that go back stops, saying where (it had
  summed rows by no reading of "trailing"); `onehot`'s classes are 1 or
  more; `moment[seconds]` past 100,000,000 days of 1970 stops (it had
  clamped to the year 292278994).

### Added

- A matrix's row is written in place, `m[i] = [7.0, 8.0]`, as a list of
  numbers or a tensor of the row's own length; a part of a tensor of more
  axes the same way (`c[i] = matrix`). A value of another shape is refused
  before the program runs, or stops where its shape shows only as it runs.
  It had been refused, the matrix built anew.
- A frame's column of bools is the mask it makes where a mask is taken:
  `f[f.active]`, `where[f.active, a, b]`, `names[f.active]`, and negated
  or joined, `not f.active`, `f.active and f.age gt 30` (negated it had
  been one bool, and joined it had not built compiled).
- A frame's column is picked by a mask, `f.name[f.age gt 30]`: a column of
  the same kind with its missing cells kept, as `f[mask].name` reads it.
- A list of lists of numbers is a matrix where a tensor is taken:
  `tensor[rows]` and `stack[rows]` (both had stopped interpreted and not
  built compiled).
- `crow upgrade` replaces the binary the install line put in
  `~/.crow/bin` with the latest release, held to its published hash as
  the install line holds it; `--version X` installs that release (a
  downgrade too), and `--check` says whether a newer one is out. A crow
  installed another way — a checkout's build, `install.sh --dev` or
  `--source` — is left as it is, and the command says how it upgrades.
- `crow check --self` holds Crow to itself, mechanically: about 100,000
  cases written from the compiler's own tables — every builtin with the
  kinds it takes and their edges (one parameter at a time and two at
  once), every collection at every kind of index, every operator over
  every pair of operands, the shapes and the paths a value takes — and
  families for served routes, views drawn as pages, pages driven by their
  events, files, random draws, gradients, the GPU, the standard library's
  own layer, both database drivers and a WebSocket server, a mail server
  and a language model of the check's own, with the standard modules' and
  the packages' own tests. Each runs in five configurations that must
  agree (interpreted, walked, unfused, unmoved, compiled), and each answer
  is held to the spec's by a reference written in Python
  (tests/self/reference.py: libm, numpy, scipy, pyarrow, hashlib, hmac,
  re, Python's HTML parser). Any finding fails the gate, and the lists of
  builtins no case calls, no reference answers and forms no case reaches
  only shrink (docs/crow-check.md).
- `json` of a parsed value in a program that serves answers it as it is:
  `json[json[req.body]]` echoes a body.
- A view called outside views answers its markup, in both modes: logged,
  held in a variable, measured.
- A tool only handed to a model is typed by what its body reads: a
  parameter handed to `upper` is text, and its schema says so (only a
  call had typed one).

### Changed

- A call evaluates every argument, left to right, before the leftmost
  failure is its value, in both modes (the compiled program had skipped
  the arguments after a failed one).
- A tensor divided by a number divides each number (it had multiplied by
  the reciprocal and rounded twice: `[3.0] / 2.5` was 1.2000000000000002).
- The runtime bundle carries only what macOS arm64 and Linux x86_64
  build: every other vendored package (Windows', WebAssembly's and a few
  more) is a stub of its manifest, which cargo still reads, and the
  download goes from 84 MB to 43 MB.
- Arrow and Parquet are at 60, which reads Parquet's metadata without the
  Thrift crate.

### Fixed

- The Linux binary needed the system's OpenSSL 3 (`libssl.so.3`), which
  a slim container or an older distribution lacks: the live pages' HTTP
  client uses rustls now, as the rest of Crow does, and CI and the release
  check that the binary needs only the system's own libraries.
- A recursive function that adds its own answers into a local and returns
  it (a backtracking search's count) is held in 64 bits compiled; it had
  been a growing int, every addition paying for it.
- A function called with arguments of different types is checked at each
  signature it is compiled for: `set[cols, …]` where one call hands a list
  had run interpreted only to stop and not built compiled; it is refused
  before either mode runs.
- `stack` of a list of lists whose numbers can fail is refused in words;
  it had reached the compiler's own check as an internal error.
- `max`, `min` or `sum` of a tensor that can fail is a number that can
  fail; it had been typed as nothing known, and the compiled program did
  not build (found by the fuzzer).
- `m[i][j] = x` writes a matrix's number where it lies, in both modes; the
  interpreter had stopped on the chained index and the compiled program
  had not built. A matrix held in a record's field, a list's item or a
  map's value is written the same way (`g.cells[i][j] = x`), where both
  modes had failed too.
- `xs[mask]` of a list read from a value that can fail (a file's text
  split into lines) builds compiled; it had run interpreted only.
- `crow stop` stops a program the interpreter serves (`crow run -i`) and
  `crow dev` with what it serves; it had said nothing was running. A
  process a stale pidfile names is signalled only when it runs in the
  project's directory.
- The console runs a paste of several lines as one cell (it had waited
  behind its continuation prompt), reads a `view`, `test` or `derive`
  block whole and an `else` with its `if`, and tells a program that
  serves pages to run from its file.
- Dependencies with security advisories updated: rustls and its
  certificate checker, rand, and (through Parquet) Thrift; OpenSSL is gone.
- `follow` and `next` lost a piece of a model's reply whenever one of its
  events straddled two reads of the connection, and `recv` of a socket
  lost a message when a wait ended inside it: what a reader held is put
  back before it reads more.
- With the GPU on (`gpu[]`), a view of a tensor placed there — a
  transpose, a slice — is its own numbers: it had been read, multiplied
  and taken `exp` of as the storage under it (`transpose[m]` was `m`,
  `slice[v, -2, 3]` the first two). A number taken from or divided by a
  tensor there is one operation in both modes (they had shown different
  digits, and `sigmoid` and `softmax` with them), and a small number
  reaches the device as itself (`t * 1e-12` had multiplied by 0).
- A name two imported modules give, called with a kind neither module's
  row takes — `load` of a tensor under `use frame, math` — is refused as
  one module's call is; it had stopped interpreted and not built compiled.
- `grad` of a function through `floor`, `ceil`, `round` or `abs` runs in
  both modes (the interpreter had not run the whole number's form on a
  dual number, and the compiled program had not built).
- `factorial` and `gamma` of a whole number are exact (`factorial[3]` had
  been 6.000000000000007).
- A compiled page draws a child of any kind — a tensor, a map, a record, a
  moment, a parsed value, a frame, a column — as the text `log` writes for
  it (it had not built); the interpreter takes as attributes only the map
  written first in an element (it had taken every map child so), draws a
  missing cell as missing (it had drawn 0), and both escape a quote alike;
  `class = ''` writes no class in both.
- A record fact's field written in a handler is stored (interpreted it
  had stopped, compiled it had changed nothing the page saw).
- `stream[feed, 2 ^ 100]` and `later[f, 2 ^ 70]` hand the whole number on
  (they had stopped at 64 bits); `msgpack` of a decimal builds; `json` of a
  map with int keys answers (it had been a 500 interpreted); a tensor
  holding 2^63 is encoded as 2^63.
- `linspace` ends at its end; `norm` of nothing is 0 (it had been -0);
  `conv` of nothing stops with its reason (it had overflowed); `floor`,
  `ceil` and `round` of an int past 64 bits is itself; `push` or `set` of
  one onto 64-bit ints holds it; a wait past what a clock holds is none
  (it had panicked).
- Programs that had not built compiled now do: an int literal past 64
  bits, `.^` with a number on its left, a list or a map of ints written a
  float, a write into a map with int keys, a response logged, a tensor
  beside a failed number, `describe` inside a function, and a function
  handed the answer of another called at two kinds.
- One mistake is one message: a function value nothing types, and a rank
  the unknown value reached, are no longer said beside the misspelling
  that caused them.

## v0.6.1 — 2026-10-14

### Added

- The mark: the `(o.o)` every command prints, as `assets/crow.svg`, on
  the README, the book's pages (favicon, touch icon and top bar, their
  addresses versioned by the mark's own bytes) and the editor extension.
- The editor extension is published with each release: to Open VSX
  (Cursor, VSCodium, Gitpod) now, and to the Visual Studio Marketplace by
  trusted publishing once the Marketplace accepts it; the release page
  keeps the `.vsix` for VS Code meanwhile. Its version is Crow's.

### Fixed

- The book site made its directory before writing a page, so a fresh
  checkout published every chapter; the Pages site had answered 404 for
  all but the front page and the specification.

## v0.6.0 — 2026-10-13

The first release of the alpha: the one-line install, and everything
since v0.5.0 (2026-09-06).

### Breaking

- `contains[xs, x]` and `has[m, k]` are written `x in xs`; `gather[…]`
  is `concat[…]`; `pad` and `append` are gone.
- One moment kind: `now`, `moment`, `fields` and `fmt[style = …]` replace
  `calendar`, `parse_time`, `datetime`, `tz` and `time[]`.
- `then` chains are written as lines.
- `nodes[n]` is retired: `crow launch n` starts the processes and
  `env['CROW_RANK']` names each.
- Map literals are `map[k = v]`; a map holds one kind of value; a map's
  keys have a kind (text or int).

### Added

- One line installs Crow: a prebuilt binary for macOS arm64 and Linux
  x86_64 from the release, verified; the interpreter, the tests and the
  servers need nothing else. `crow setup` (or the first `crow build`)
  fetches the pinned Rust toolchain and the runtime into `~/.crow`; with
  no compiler present, `crow run` says so and interprets.
- `use llm`: a language model as a connection (`reach`, `ask`, `chat`,
  `follow`, `embed`), tools as functions, over Anthropic's and the
  OpenAI-compatible wire formats.
- Request fixtures: `crow check --record-fixtures` keeps what a request
  answered, `crow test --fixtures` replays it in both modes with no server
  and no model.
- The HTTP client, the readers, S3, mail, WebSockets, `math.stats` and
  the ClickHouse and Postgres drivers written in Crow on the `sys` layer.
- Ints that grow past 64 bits, decimals and fractions; `use bits`.
- Packages held to a reference: `symbolic` (PySR), `models/boost`
  (scikit-learn), `graph` (networkx), `optimize` (scipy), `retrieval`
  (scikit-learn, rank_bm25), `gpt` (nanoGPT), `vectors` (faiss); the frame
  verbs held to pandas.
- The `rag` reference app; the error-message overhaul and `crow fix`; the
  rehearsal, the app baseline and the scenarios in `crow check`; routes
  reading the app's derivations.

### Performance

- Compiled programs at Rust's speed on the benchmarks game set; the
  interpreter on an instruction form with typed registers (fannkuch from
  77 s to 1.3 s); frames faster than before by 2 to 10 times and ahead of
  polars on load, sort, join and unique; TechEmpower plaintext at Rust's
  speed.

## v0.5.0 — 2026-09-06

The version the alpha starts from.
