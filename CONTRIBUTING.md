# Contributing

Crow is in alpha, developed by Vijay Sriram at aggnr. During the alpha it takes no pull
requests: the language still changes between releases, and every line is
kept under one hand so that the licence can still change. A pull request
is closed with this note by a workflow, so nobody waits on one.

What is welcome, in Issues:

- **A bug**, with the program that shows it and what each mode said —
  `crow run -i file.co` and `crow run file.co`. The two modes are held to
  one behaviour, so a difference between them is a bug by definition
  (docs/design-rules.md).
- **A question** the book (`docs/book`) or the specification
  (`docs/language-spec.md`) does not answer. The answer usually becomes a
  line in one of them.
- **What you built with it.**

Working from a checkout: `./install.sh --dev` builds Crow in place;
`docs/design-rules.md` holds the rules the code and the specification
cite by number; `CLAUDE.md` lists the checks every change passes before
it is done, which `.github/workflows/ci.yml` runs on every push.
