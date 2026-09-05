# Final Project: A Live Dashboard

Everything in one program: a trading dashboard that reads a real SQLite
database, computes stats with frames and math, renders with views, and
updates live in the browser. About sixty lines of Crow, one binary, no
JavaScript, no ORM, no framework configuration.

We build it in four steps: the data, the database layer, the live views,
and shipping it. The finished program is `examples/dashboard.crow` in the
Crow repository — every step here is the path to exactly that file.
