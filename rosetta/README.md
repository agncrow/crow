# Rosetta Code in Crow

Crow solutions to the tasks of Rosetta Code's
[Solutions by Programming Task](https://rosettacode.org/wiki/Category:Solutions_by_Programming_Task)
category — 696 tasks, listed on 2026-10-28 — written to be uploaded to
rosettacode.org, and kept here as a test of Crow: every program runs
interpreted and compiled, and both must print the output recorded beside it.
The category's 62 subcategories (Sorting, Text processing, …) hold tasks of
their own, which this run does not cover.

## Layout

| Path | What it holds |
|---|---|
| `<slug>.co` | a task's solution; its header names the task and links its page |
| `<slug>.out` | what the program prints, the same in both modes: the output an upload shows |
| `<slug>.in` | the standard input of a task that reads one (a game's moves) |
| `data/` | the files programs read by name (`unixdict.txt`): only `data/SOURCES.md` is in the repository, since the files carry licenses of their own (CC BY-SA, the Open Database License, OpenCV's, the GFDL of Rosetta Code's pages); it says where each comes from, with its size and MD5 |
| `tasks.tsv` | every task: its slug, title, status and notes |
| `REVIEW.md` | the tasks not done in full, each with why, for review |
| `QUESTIONS.md` | decisions the run met that are the language's to make |
| `findings/` | what writing the tasks found in the compiler: one small program per finding, and `findings/README.md`, where each stands (fixed with the tests that hold it, or open and why) |
| `run.sh` | the runner |

A slug is the task's title in lower case, accents and punctuation taken out
and words joined by `_` (`Abbreviations, automatic` is
`abbreviations_automatic`, `24 game/Solve` is `24_game_solve`).

## Running

From the repository root, with `target/release/crow` built:

```
rosetta/run.sh              # every program, interpreted and compiled
rosetta/run.sh -i           # interpreted only
rosetta/run.sh 100_doors    # the programs whose names hold a word
```

Programs run in `target/rosetta/`, where the files of `data/` are linked: a
compiled program builds in `.crow_build` under the current directory, and the
first build there takes a minute while later ones take about a second. Runs
in parallel each need a directory of their own: `WORK=target/rosetta-2
rosetta/run.sh a b c`.

## Status

<!-- status -->
| Status | Tasks |
|---|---|
| done | 666 |
| partial | 23 |
| not-possible | 6 |
| skip | 1 |
| all | 696 |
<!-- /status -->

`partial`, `needs-feature`, `not-possible`, `bug` and `skip` are explained
task by task in `REVIEW.md`.
