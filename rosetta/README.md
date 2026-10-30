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
| `held_to_python.tsv` | each task's output held to the wiki's Python solutions: its result, what it was held to, and a note where it is not plain agreement |
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

## Held to Python

On 2026-10-29 every recorded output was held to a third result: the Python
solutions on the task's page, each run in a sandbox (Python 3.12, writing
only in a directory of its own, with no network; a Python 2 block converted
by lib2to3 first), its numbers compared in order with the Crow program's,
and read by hand wherever the two did not match at once. Where no Python
ran, the output the page records for Python, or for another language, stood
in. The comparison runs code from the wiki, so it is not part of `run.sh`;
`held_to_python.tsv` keeps what it found:

| Result | Tasks |
|---|---|
| agrees | 588: 541 held to Python, 47 to another language |
| not comparable | 42: random draws, interactive games, another input, a demonstration of the language itself |
| Python wrong | 9: the page's Python is wrong and the Crow answer right, the note says how |
| fixed | 1: Binomial transform's prime flip-flop sequence began at n = 0, the task's at n = 1 |
| no reference | 51: no Python and no recorded output to hold the program to |

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
