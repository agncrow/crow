# Reactive State and Events

Top-level variables in a ui program are **reactive state**, and every
reader's page holds its own copy. Views read them; event handlers change
them; every change re-renders that reader's page. Here is the whole loop:

```crow,norun
use web, ui

count = 0

fn inc[]
    count = count + 1

fn dec[]
    count = count - 1

view render[]
    return div[
        h1['Count: `count`'],
        button['+', click = inc],
        button['-', click = dec]
    ]

serve[3000]
page['/', render]
```

Click `+`: `inc` runs *on the server*, `count` changes, and the page
shows the new value. A second tab has a `count` of its own — each page is
its own session, and what pages share is the database (the two-users
example in the testing chapter shows both). You wrote no JavaScript, no
fetch calls, no socket code.

Events bind by **name** — `click = inc` — the functions-as-values idea
from chapter 4, never a lambda. And notice what `count` is: the one
sanctioned exception to scope isolation. Ordinary functions still can't
touch it; views and handlers can, because the runtime owns it,
synchronizes it, and knows when it changed. It's not ambient mutation —
it's a managed channel.

Data sources compose the same way: `prices = poll['/api/prices', 5]`
declares state that refreshes every five seconds and re-renders whatever
reads it.

State gets its value once, where it is declared: `count = 0` is what
every session starts from, and handlers move it from there. The top
level does not change it afterwards — a second `count = 1`, or a loop at
the top level that pushes into a list of state, is refused and told why,
since no session and no test would see the change. Give the state its
whole value where it is declared (a function can loop to build it), or
make it a `derive`. A script that imports a ui part but not ui itself
keeps its own top-level variables plain, and runs top to bottom.

## The page and its connection

The browser holds a socket to the server, and the server holds your
session. Several things follow.

**A dropped connection says so, and your session waits for you.** When
the socket closes — the laptop slept, the network dropped — the page
dims, shows *reconnecting…* and takes no clicks until it is back. The
server keeps your session for ten minutes: when the tab reconnects (or
you reload it) you are back where you were, a replay still running, sent
the page's render first so you never click on a page the server no
longer holds. A second tab is a session of its own. `serve[…, keep =
true]` keeps sessions across a restart of the program too — what a
rebuild would otherwise throw away.

**A render sends what changed.** After the first page, the server sends
the page only what a render changed — a text, an attribute, a row added
or gone — not the page again. A chart whose play-head moves is sent the
play-head, a few bytes, not the drawing; a list whose items carry an `id`
gains one row when one item is added at its top. The server sends the
whole page when that is smaller, and when your markup is one the browser
would have to repair — a `div` inside a `p`, a row outside a table — and
it says so in the log, once, so you can fix the markup. A field you are
typing in keeps what you typed while the page around it changes.

**A click shows it was taken.** The element you press is marked busy
(`data-crow-busy`) the instant you press it, and unmarked when the server's
answer to that click arrives — over a slow network the page still shows
the click in the same frame.

**A page can send only what it was offered.** The server accepts a click
only when a render it sent carried that handler with exactly those
arguments — `button['10x', click = set_speed[10.0]]` offers `set_speed`
with `10.0` and nothing else — and a bound value only for an input it
drew. A handler no button names, a helper, a variable no input binds, and
the button of a closed modal are refused and logged, so a confirmation's
second step cannot be sent without its first. The socket opens only from
the app's own page, and a served app listens on this machine only; set
`CROW_HOST=0.0.0.0` when a server must be reachable from the network.

## A click is one transaction

A handler's writes land together when it ends, or not at all. It works on
its own copy of the session's facts — a render never sees it halfway — and
its database writes are one transaction, committed with it. When an error
reaches a decision, a fact or a database call, the handler **stops**:
nothing it did lands, and the reader is told why through the session's
*notice*:

```crow
use web, ui

saved = 0

fn save[]
    saved = saved + 1
    notice['saved `saved`']

fn broken[]
    saved = saved + 100
    px = float['no quote']
    if px gt 1.0
        saved = 0

view panel[]
    return stack[
        p['saved `saved`'],
        p['`notice.text`'],
        button['Save', click = save],
        button['Broken', click = broken]
    ]

test one_transaction[]
    a = app[panel]
    click[a, save]
    assert[a.saved eq 1]
    assert[contains[html[a], 'saved 1']]
    r = click[a, broken]
    assert[not r.ok]
    assert[a.saved eq 1]
    assert[contains[html[a], 'broken stopped: cannot convert']]

page['/', panel]
log['panel defined']
```

```text
panel defined
```

`notice[text]` waits for the commit, so a handler that stops never says it
succeeded. `notice.text` and `notice.at` read the latest one; a page that
shows no notice of its own gets the runtime's toast, which fades by itself.
While a handler runs, the button that started it is disabled and marked
busy, and `running[save]` is true — the page shows the work without a flag
in your program. Inside one click `now[]` is one moment, however many
times it is read.

## The address and the questions

Two things every app ends up building by hand come with the page. The
address can carry facts — `page['/', render, url = ['day']]` starts a
session from `/?day=2026-09-04` and keeps the address in step as `day`
changes, so a reload or a shared link opens the same view. And a click
can ask first:

```crow
use web, ui

ended = 0

fn end_session[]
    ended = ended + 1

view panel[]
    return stack[
        p['ended `ended`'],
        button['End session', click = end_session, confirm = 'End this session?']
    ]

test the_button_asks_first[]
    a = app[panel]
    b = elements[a, 'button'][0]
    assert[b['data-crow-click'] eq '__ask']
    click[a, end_session]
    assert[a.ended eq 1]

page['/', panel, url = ['ended']]
log['panel defined']
```

```text
panel defined
```

The button offers the question, not the handler: the runtime shows the
dialog, and only its Confirm button reaches `end_session`, so the step
cannot be skipped by sending the click directly. On a table the questions
sit beside the actions: `actions = map[Delete = delete_session], confirm =
map[Delete = 'Delete this session?']`. In a test `click[a, end_session]`
runs the handler itself — the question is the page's, not the program's.

## Keys and forms

A key presses a button. `shortcut` names the keys, and the button keeps
its one handler:

```crow
use web, ui

saved = 0
added = 0
helping = false
note = ''

fn save[]
    saved = saved + 1

fn show[]
    helping = true

fn hide[]
    helping = false

fn add[]
    added = added + 1

view desk[]
    return stack[
        p['saved `saved` added `added`'],
        button['Save', click = save, shortcut = 'ctrl+s meta+s'],
        button['Help', click = show, shortcut = '?'],
        modal[stack[p['Press ? for this help.'], button['Close', click = hide, shortcut = 'escape']], open = helping],
        form[field['Note', note, type = 'text'], button['Add'], submit = add]
    ]

test the_keys_are_the_buttons[]
    a = app[desk]
    b = elements[a, 'button'][0]
    assert[b['aria-keyshortcuts'] eq 'Control+S Meta+S']
    click[a, add]
    assert[a.added eq 1]

page['/', desk]
log['desk defined']
```

```text
desk defined
```

Ctrl+S, or Command+S on a Mac, presses Save as a click would; `?` opens
the help and Escape closes it. A key works only while its button is on
the page and can be pressed, so it reaches nothing the reader could not
click. While the reader types in the Note field the keys are theirs: a
`?` typed there is a question mark. Enter in the field presses the
form's first button, and the form's `submit = add` runs `add` once; a
form never reloads the page. A shortcut that names no key, or sits on
something a press does nothing to, is refused before the program runs.

## Pages, their addresses and their titles

An app serves every `page` it registers, in both modes, and each page is
a session of its own. A `:name` segment of a page's path is a fact in its
address, the way `url = […]` puts facts in the query string:

```crow
use web, ui

card = 1

fn next_card[]
    card = card + 1

view board[]
    return stack[
        title['Cards'],
        nav[a['Card 1', href = '/card/1'], a['Card 2', href = '/card/2']]
    ]

view card_page[]
    return stack[
        title['Card `card`'],
        meta[name = 'description', content = 'The card numbered `card`'],
        p['card `card`'],
        button['Next', click = next_card]
    ]

test a_card_names_itself[]
    a = app[card_page]
    click[a, next_card]
    assert[elements[a, 'title'][0].text eq 'Card 2']

page['/', board]
page['/card/:card', card_page]
log['two pages']
```

```text
two pages
```

`/card/7` opens a session with `card` set to `7`, read in the fact's shape,
and when Next makes it `8` the address becomes `/card/8`, so a reload or a
shared link opens that card. A value the fact cannot hold, `/card/x`, is no
page: a 404, as is an address no page or route answers. The path says
which thing the page shows; the query string (`url = […]`) tunes how, and
a page can have both. A `:name` must name a fact, or the program is refused
before it runs.

A link is an `a` with an `href`, alone or in a `nav`. Following one opens
the session of the address it names, even to the page you are on: a link
to `/?day=tue` from `/?day=mon` starts from Tuesday. Reloading the tab, or
coming back with the browser's back button, resumes the tab's own session.

`title` and `meta` are the page's head. They take no room on the page: the
tab shows the title, and a render that changes it changes the tab. The last
title a page places wins, so a layout's default gives way to the page's
own, and a page that places none is titled `Crow App`. In a test they are
elements of the render, as above.

## Who the reader is

A page knows who is reading it through `reader`: a map of strings, empty
until someone signs in. A handler signs the reader in with
`sign_in[record]` and out with `sign_out[]`. Passwords are stored with
`hash[password, 'password']` — salted and deliberately slow — and checked
by hashing the attempt the way the stored one was made:

```crow
use web, ui, db

book = db.connect['sqlite://:memory:']
name = ''
password = ''

fn setup[]
    exec[book, 'CREATE TABLE IF NOT EXISTS people (name TEXT, role TEXT, hash TEXT)']
    exec[book, 'INSERT INTO people VALUES (?, ?, ?)', ['ada', 'admin', hash['lovelace', 'password']]]

fn log_in[]
    found = query[book, 'SELECT role, hash FROM people WHERE name = ?', [name]]
    if len[found] eq 0
        notice['no one called `name`']
        return
    if hash[password, found.hash[0]] ne found.hash[0]
        notice['wrong password']
        return
    sign_in[map[name = name, role = found.role[0]]]

fn log_out[]
    sign_out[]

derive greeting
    if has[reader, 'name']
        return 'hello `reader.name`'
    return 'hello stranger'

view door[]
    return stack[
        p[greeting],
        field['Name', name, type = 'text'],
        field['Password', password, type = 'password'],
        button['Log in', click = log_in],
        button['Log out', click = log_out]
    ]

test ada_signs_in_and_out[]
    a = app[door]
    click[a, setup]
    change[a, 'name', 'ada']
    change[a, 'password', 'lovelace']
    click[a, log_in]
    assert[reader.role eq 'admin']
    assert[elements[a, 'p'][0].text eq 'hello ada']
    click[a, log_out]
    assert[len[reader] eq 0]

page['/', door]
log['door defined']
```

```text
door defined
```

Signing in gives the browser a cookie, so the reader is signed in on
every page and tab they open next, and after a restart, for thirty days.
Signing out clears it and ends that sign-in everywhere: another tab still
open is signed out at once, and the old cookie is refused if it ever
comes back. The program never touches the cookie, and nothing on the page
can write `reader` — no field binds it, no handler assigns it.

Deciding who may do what is ordinary code in the handler: `if reader.role
ne 'admin'`. A signed-out reader has no `role`, and reading it is an
error that stops the handler, so a forgotten check fails closed. In a
test, `change[a, 'reader', map[name = 'bob', role = 'member']]` stands for
someone who signed in, without a password. The cookie is signed with the
app's secret — `CROW_SECRET`, or a key the runtime writes to
`.crow/keys/` the first time it needs one.

A page only signed-in readers may see checks `reader` in its view, and
draws the sign-in form until someone signs in:

```crow
use web, ui

name = ''
refunded = 0

fn log_in[]
    sign_in[map[name = name]]

fn refund[]
    refunded = refunded + 1

view sign_in_form[]
    return stack[
        field['Name', name, type = 'text'],
        button['Log in', click = log_in]
    ]

view orders[]
    if not has[reader, 'name']
        return sign_in_form[]
    return stack[
        p['orders for `reader.name`, `refunded` refunded'],
        button['Refund', click = refund]
    ]

test the_orders_wait_for_a_sign_in[]
    a = app[orders]
    assert[len[elements[a, 'button']] eq 1]
    change[a, 'reader', map[name = 'ada']]
    assert[elements[a, 'p'][0].text eq 'orders for ada, 0 refunded']

page['/orders', orders]
log['orders defined']
```

```text
orders defined
```

Nothing else of the page reaches a signed-out reader. The page answers
only the buttons its render drew, so `refund` can't be pressed, not even
by a message written by hand, and a query the orders view reads never
runs for them. When `log_in` succeeds, the same page, at the same
address, shows the orders.

## Files, and tables too big to show

A file is an input: `field['Trades', upload, type = 'file']` binds
`upload`, declared `upload = map[]`. When the reader picks a file the page
sends it, and `upload` holds its `name`, `type`, `size` and the `path` of
the server's copy — read it like any file. The other way, `download[value,
name]` hands the reader's browser a file when the handler commits: a
frame as CSV, a string as text. And a table that could grow past a screen
pages: `table[frame, page = 50]` draws fifty rows at a time, with the
count and the previous and next pages under it. `sort = true` lets the
reader sort it: each header becomes a button that orders the rows by its
column, ascending and then descending.

```crow
use web, ui, frame

upload = map[]

derive trades
    if has[upload, 'path']
        return frame.load[upload.path]
    return frame.new[]

fn export[]
    download[trades, 'trades.csv']

view desk[]
    return stack[
        field['Trades', upload, type = 'file'],
        table[trades, page = 50, sort = true],
        button['Export', click = export]
    ]

test a_file_in_and_out[]
    write['desk_trades.csv', 'symbol,qty\nIMRN,100\nFFIE,300\n']
    a = app[desk]
    change[a, 'upload', map[name = 'trades.csv', type = 'text/csv', size = '29', path = 'desk_trades.csv']]
    assert[len[elements[a, 'tbody tr']] eq 2]
    assert[elements[a, 'th button'][0].text eq 'symbol']
    click[a, export]
    assert[downloads[a][0].name eq 'trades.csv']
    assert[contains[downloads[a][0].text, 'IMRN,100']]

page['/', desk]
log['desk defined']
```

```text
desk defined
```

The file travels beside the page's connection, never through it, and a
file bigger than the app allows — 10 MB, or `serve[…, uploads = 50]` in
megabytes — is refused before it is sent, with the limit named. The
server's copy belongs to the session and goes when the session does; a
handler that wants to keep it copies it. A table of more than 1,000 rows
without `page` is refused in place, as an over-budget chart is: the page
says how many rows there are and how to page them, rather than quietly
cutting some off. The session keeps each table's page and its order, and
the frame stays as it was: sorting the program's own data is
`sort[df, 'col']`.

## Mail

`mail[mailer, to, subject, body]` sends a plain-text message through a
mailer that `smtp[host, port, user, pass]` makes once for the whole
program, as a connection is made. Like `notice` and `download` it is an
effect held to the handler's commit, so a handler that stops sends
nothing, and a function a handler calls holds its messages for that
handler. A handler that must know whether the server took a message reads
the answer: `sent = mail[…]` is sent at once, and `sent.ok` says. A test
sends nothing: `mails[a]` is what the session sent.

```crow
use web, ui
use web.mail

mailer = smtp['smtp.example.com', 587, 'desk@example.com', 'app-password']
email = ''

fn subscribe[]
    mail[mailer, email, 'Subscribed', 'You will hear from the desk.']
    notice['subscribed `email`']

view signup[]
    return stack[field['Email', email], button['Subscribe', click = subscribe]]

test a_subscription_sends_one_message[]
    a = app[signup]
    change[a, 'email', 'ada@example.com']
    click[a, subscribe]
    assert[len[mails[a]] eq 1]
    assert[mails[a][0].to eq 'ada@example.com']

page['/', signup]
log['signup defined']
```

```text
signup defined
```

A mailer prints as its host, never its password.

## Sliders, helpers, and bigger state

`slider['Seek %', seek_pct, min = 0, max = 100, step = 1]` binds a range
input to `seek_pct` the way `field` binds a text input; a `Seek` button's
handler then reads it. Bound inputs keep the variable's type: the browser
sends text, and an integer variable receives an integer. When a handler
assigns to a bound variable (a row click that sets `sym`), the field shows
the new value on the next render. A handler may call a helper that assigns module
state — `fn restart[]` calling `fn rewind_to[t]` — and the helper joins the
handler class in both renderers, so its write lands in state. And state
is not only numbers and strings: an array, a map, or a whole frame can be
reactive, kept as JSON between renders. The following block is verified
by the book's tests:

```crow
use web, ui, frame

seek_pct = 50
focus = ''
universe = frame.new[]

fn pick[symbol]
    focus = symbol

fn keep[u]
    universe = u

fn load_universe[]
    u = with[frame.new[], 'symbol', ['IMRN', 'AOUT', 'CHPT']]
    keep[with[u, 'gate', [1, 0, 1]]]

view render[]
    return stack[
        slider['Seek', seek_pct, min = 0, max = 100, step = 5],
        p['focus: `focus`'],
        table[universe, click = pick, key = 'symbol', mark = 'gate'],
        button['Load', click = load_universe],
        gap = 'm'
    ]

test rows[]
    s = app[render, width = 1280]
    click[s, load_universe]
    h = html[s]
    assert[contains[h, 'type="range"']]
    assert[contains[h, 'data-crow-click="pick"']]
    assert[contains[h, 'class="marked"']]
    assert[len[elements[s, 'tr']] eq 4]
    click[s, pick['AOUT']]
    assert[s.focus eq 'AOUT']
    assert[fits[s, 390]]

page['/', render]
log['reactive page defined']
```

```text
reactive page defined
```
