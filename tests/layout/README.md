# Layout conformance

`conformance.crow` holds two views built from the layout vocabulary. Chrome
measured them once (`measure.py`), and the CSV files under `expected/`
record where every element landed at 1280px and 390px. `crow test
tests/layout/conformance.crow` runs the same views through the engine and
fails if any box drifts beyond the tolerance written in the file. No
browser is needed to run the check; one is needed only to re-measure after
a change to the kernel stylesheet or the engine.
