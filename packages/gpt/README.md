# gpt

nanoGPT written in Crow over `grad`'s operations and held to it in
float64 (docs/gpt-design.md): a character tokenizer as `shakespeare_char`
prepares its data, a model of two layers and two heads, 32 wide, over
blocks of 16 tokens — token and position embeddings, a layer norm, causal
self-attention with each head through its own slice of the output
projection, a GELU MLP, the logits through the tied embedding — its loss
differentiated by `grad` as any function is, and plain gradient descent
over the blocks of a text. `held.co` holds the package to
`reference/reference.py` (PyTorch, float64, no dropout, no biases): the
reference's own initial weights loaded, the same tokens, the logits and
the loss at those weights, the loss before each of twenty steps, and the
greedy continuation at the final weights — numbers to 1e-9 relative,
tokens and characters exactly. Each answers the same in `crow run -i` and
`crow run`.

```
deps = map[gpt = 'github.com/agncrow/crow/packages/gpt@main']
```

A program imports the parts it uses:

```
use gpt.chars, gpt.model, gpt.train
```

## The parts

- `chars.co` — `vocab[text]`, `tokens[text, vocabulary]`, `text_of[ids, vocabulary]`.
- `model.co` — `init[vocabulary_size, seed]` (the weights as a list, in the
  order its header states), `logits_of[ids, vocabulary_size, ws]`,
  `loss_of[ids, targets, vocabulary_size, ws]`, `gradients[…]`,
  `greedy[ids, n, vocabulary_size, ws]`.
- `train.co` — `step[ids, targets, vocabulary_size, ws, lr]`,
  `fit[ids, vocabulary_size, ws, steps, lr]` answering a `Fitted` record:
  the losses and the weights.
- `example.co` — two thousand steps on a slice of tiny Shakespeare, then a
  hundred characters continued.
- `reference/` — the reference and what it wrote: the weights
  (`data/w00_wte.csv` …), the text slice and its vocabulary, and under
  `expected/` the tokens, the logits and loss at init, twenty losses, the
  loss after them and the greedy continuation. To write them again:
  `cd packages/gpt/reference && python reference.py` (PyTorch in float64).

The weights are the loss's parameters, spelled out: `grad` takes tensors
one parameter each, and the model's depth is fixed by the code. A `grad`
over a list of tensors is what a model of any depth asks of the language
(docs/gpt-design.md).
