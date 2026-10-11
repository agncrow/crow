# A GPT in Crow, held to nanoGPT

The second piece of the LLM domain (docs/llm-design.md is the first): the
model itself, written in Crow over `grad`'s list of operations and held
to a reference — nanoGPT in float64 — as the packages are held to
scikit-learn, scipy and pandas: the same numbers, the compiler fixes the
port finds, and a measure of where Crow stands. This note is the design;
`packages/gpt` is the code, `docs/roadmap.md` (P8) the status.

## What exists, and what the port asks of the language

A transformer block over four tokens — token and position embeddings,
layer norm, one causal self-attention head, a GELU MLP, the loss
`crossentropy` — differentiates in both modes today (2026-10-11, a probe
against the installed crow): `grad` takes it with only the operations
its list names — `*` as the matrix product, `transpose`, `exp`, `sqrt`,
`mean` and `max` and `sum` along an axis, `reshape` to a column so a
reduction broadcasts, `.*` and `./`, `+` and `-` broadcasting a row, and
`gelu` and `crossentropy` from `math.nn` — and a step of descent lowers
the loss, the backward gradient agreeing with a central difference to
the sixth digit. Three facts shape the design:

- **A function sees no top-level variable**: the inputs (the tokens as
  one-hot rows, the position rows, the causal mask, the targets) are
  parameters of the loss, their gradients zeros.
- **`grad` takes tensors one parameter each**, never a list or a record
  of them, so a model's weights are the loss's parameters, spelled out.
  Attention over several heads needs no `concat` (not in `grad`'s list):
  each head's output goes through its own slice of the output projection
  and the slices are summed — `Σ_h (softmax(q_h k_hᵀ / √d) v_h) · Wo_h`,
  the same numbers as `concat(heads) · Wo`.
- **A reduction along an axis answers a vector**, so `reshape[mean[x, 1],
  rows, 1]` makes the column that broadcasts over `x`'s rows.

## The package: `packages/gpt`

| File | What it holds |
|---|---|
| `chars.co` | A character tokenizer as nanoGPT's `shakespeare_char` prepares its data: `vocab[text]` (the characters, sorted), `encode[text, vocab]` (ints), `decode[ids, vocab]` |
| `model.co` | The model: `init[vocab, block, dim, seed]` (the weights, drawn as nanoGPT draws them: normal 0.02 for the matrices, ones and zeros for the norms), `logits[…]`, `loss[…]` (differentiable), `greedy[…, n]` (generation by the largest logit) |
| `train.co` | `step[…, lr]`: one step of plain gradient descent over a batch of `block` tokens; `fit[text, steps, lr, seed]`: the loop, the loss logged every ten steps |
| `example.co` | Fifty steps on a slice of tiny Shakespeare, then a hundred characters generated |
| `held.co` | The package held to the reference: the same weights loaded, the same tokens, the logits at init, the loss at init and after each of twenty steps, the greedy continuation |
| `reference/reference.py` | nanoGPT's model in PyTorch, float64, dropout 0, writing what Crow is held to under `reference/data` and `reference/expected` (CSV, tracked by a `.gitignore` negation) |

The configuration is fixed by the code, since the weights are the loss's
parameters: **two layers, two heads, 32 wide, a block of 16, the 65
characters of tiny Shakespeare** — per layer two norms (4 tensors), two
heads of `q`, `k`, `v` and their slice of `Wo` (8), the MLP (4); with the
embeddings and the final norm, 36 weight tensors and 4 inputs. That is a
long signature, and it is honest: every number the model holds is named.

## Held to

- **Discrete answers exact**: the tokens `encode` writes, the characters
  `decode` reads, the greedy continuation's characters.
- **Numbers in float64 to 1e-9 relative**: the logits of the first
  block at init, the loss at init, the loss after each of twenty steps of
  descent at lr 0.1 from the reference's own initial weights. Not "to the
  last digit": a matrix product's accumulation order is the BLAS's, which
  neither `faer` nor PyTorch states, so the digit where they part is
  theirs; the frames work matched pandas to the digit only where the
  kernel's order could be reproduced (CLAUDE.md, FMA).
- **Both modes alike**: `crow test --packages` runs the package's tests
  and its example interpreted and compiled.

## Design notes

**Plain descent, not AdamW, first.** nanoGPT trains with AdamW; the
first held comparison uses plain gradient descent so that one formula
is held at a time — the loss and its gradient. AdamW is element-wise
arithmetic over the gradients and two running moments, written next in
`train.co` and held the same way.

**Characters first, bytes next.** GPT-2's tokenizer is a byte-level
BPE: it needs the bytes of a text, which a package cannot reach (`bytes`
is `sys`'s). Holding BPE to tiktoken (`r50k_base`) and reading GPT-2's
weights for inference is the next piece, and it asks the language for
one of: a core row answering a text's UTF-8 as ints, or the tokenizer
written as a standard module on `sys`. To decide with the spec's "Text
Is Text" note in view.

**The weights as parameters, and what the port asks.** Spelling 36
tensors out is what the language allows today. A `grad` over a list of
tensors — `loss[inputs, weights]` with `weights` a list, its gradient a
list of the same shapes — is the feature a model of any depth asks for;
it touches the tape in both backends and the refusal "a parameter that
is a record or a list". Proposed after this round, with Vijay.

**Sampling is greedy where it is held.** A draw from the softmax depends
on the random stream, which the reference and Crow do not share;
`greedy` is held exactly, and a sampled generation is shown but not held.

## What the port found

- **A traced copy took its parameter immutable** (fixed, parity 245): a
  helper a differentiated function calls may assign its parameter (`x = x
  + …`, the stream of a block); the traced copy the emitter writes had
  not marked it `mut`, and the compiled program did not build.
- **A function no test block calls poisons `grad`'s typing** (open):
  with `fit` uncalled in a file whose test block calls `step`, the list
  of weights reaching `gradients` stays `List(Unknown)` and `grad` refuses
  the loss's weights as "a value of a kind grad does not differentiate";
  calling `fit` from the test, or removing it, cures it. The small
  reproduction is in the roadmap (P8). The package keeps `train.co`'s
  tests in one block meanwhile.
- **A list holding two kinds is not refused** (open): `[losses, weights]`
  grown by `push` — a list of floats beside a list of tensors — passed the
  lowering and the verifier caught the mistyped clone; the spec's rule
  (a list holds one kind) wants the refusal in words. `fit` answers a
  record.
- **A literal list of tensors is a matrix**, so three tensors of three
  shapes stop at `[X, P, mask]` ("all inner arrays must be the same
  length"); a list of them is grown with `push`. A list of ints handed
  to `crossentropy` reaches a row `grad` cannot take, so the targets go
  as a tensor. `halt` in a guard's branch does not narrow the value as
  `return` does. `encode`, `decode`, `all`, `text` and `input` are
  builtins' names.
- The interpreter trains the model fast enough to hold: twenty steps of
  the held comparison in 0.7 s, two thousand steps of the example in
  2.4 s.

## The pieces

- `packages/gpt/{chars,model,train,example,held}.co`, `crowfile`,
  `README.md`; `reference/reference.py` with its data and expectations;
  `.gitignore` negations for the CSV and the text slice.
- `docs/roadmap.md` P8: the GPT item moves as it lands; the spec gains no
  row — the package is Crow over existing builtins (design rule 20).
- Compiler fixes the port finds, each with a parity program.
