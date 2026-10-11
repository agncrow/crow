# -*- coding: utf-8 -*-
# nanoGPT's model (github.com/karpathy/nanoGPT, model.py) in float64, no
# dropout, no biases: two layers, two heads, 32 wide, a block of 16, over
# the characters of a slice of tiny Shakespeare. Writes what the Crow
# package is held to: the weights as Crow holds them (one CSV each, in the
# orientation `h * w` takes), the first block's tokens and targets, its
# logits and loss at the initial weights, the loss before each of twenty
# steps of plain gradient descent (lr 0.1, one block a step, the blocks in
# order), and the greedy continuation of the first block at the final
# weights. Run from the package's reference/ directory:
#   python reference.py
import csv, math, os
import torch
import torch.nn as nn
import torch.nn.functional as F

torch.manual_seed(1337)
torch.set_default_dtype(torch.float64)
L, H, D, T = 2, 2, 32, 16
HS = D // H
STEPS, LR, GENERATED = 20, 0.1, 50

text = open('data/shakespeare_50k.txt', encoding='utf-8').read()
chars = sorted(set(text))
V = len(chars)
stoi = {c: i for i, c in enumerate(chars)}
data = torch.tensor([stoi[c] for c in text], dtype=torch.long)


class LayerNorm(nn.Module):
    def __init__(self, n):
        super().__init__()
        self.weight = nn.Parameter(torch.ones(n))

    def forward(self, x):
        return F.layer_norm(x, self.weight.shape, self.weight, None, 1e-5)


class CausalSelfAttention(nn.Module):
    def __init__(self):
        super().__init__()
        self.c_attn = nn.Linear(D, 3 * D, bias=False)
        self.c_proj = nn.Linear(D, D, bias=False)
        self.register_buffer('bias', torch.tril(torch.ones(T, T)).view(1, 1, T, T))

    def forward(self, x):
        B, Tn, C = x.size()
        q, k, v = self.c_attn(x).split(D, dim=2)
        k = k.view(B, Tn, H, HS).transpose(1, 2)
        q = q.view(B, Tn, H, HS).transpose(1, 2)
        v = v.view(B, Tn, H, HS).transpose(1, 2)
        att = (q @ k.transpose(-2, -1)) * (1.0 / math.sqrt(k.size(-1)))
        att = att.masked_fill(self.bias[:, :, :Tn, :Tn] == 0, float('-inf'))
        att = F.softmax(att, dim=-1)
        y = att @ v
        y = y.transpose(1, 2).contiguous().view(B, Tn, C)
        return self.c_proj(y)


class MLP(nn.Module):
    def __init__(self):
        super().__init__()
        self.c_fc = nn.Linear(D, 4 * D, bias=False)
        self.gelu = nn.GELU()
        self.c_proj = nn.Linear(4 * D, D, bias=False)

    def forward(self, x):
        return self.c_proj(self.gelu(self.c_fc(x)))


class Block(nn.Module):
    def __init__(self):
        super().__init__()
        self.ln_1 = LayerNorm(D)
        self.attn = CausalSelfAttention()
        self.ln_2 = LayerNorm(D)
        self.mlp = MLP()

    def forward(self, x):
        x = x + self.attn(self.ln_1(x))
        x = x + self.mlp(self.ln_2(x))
        return x


class GPT(nn.Module):
    def __init__(self):
        super().__init__()
        self.wte = nn.Embedding(V, D)
        self.wpe = nn.Embedding(T, D)
        self.h = nn.ModuleList([Block() for _ in range(L)])
        self.ln_f = LayerNorm(D)
        self.lm_head = nn.Linear(D, V, bias=False)
        self.lm_head.weight = self.wte.weight
        self.apply(self._init_weights)
        for pn, p in self.named_parameters():
            if pn.endswith('c_proj.weight'):
                torch.nn.init.normal_(p, mean=0.0, std=0.02 / math.sqrt(2 * L))

    def _init_weights(self, module):
        if isinstance(module, nn.Linear):
            torch.nn.init.normal_(module.weight, mean=0.0, std=0.02)
        elif isinstance(module, nn.Embedding):
            torch.nn.init.normal_(module.weight, mean=0.0, std=0.02)

    def forward(self, idx, targets=None):
        b, t = idx.size()
        pos = torch.arange(0, t, dtype=torch.long)
        x = self.wte(idx) + self.wpe(pos)
        for block in self.h:
            x = block(x)
        x = self.ln_f(x)
        logits = self.lm_head(x)
        loss = None
        if targets is not None:
            loss = F.cross_entropy(logits.view(-1, logits.size(-1)), targets.view(-1))
        return logits, loss


model = GPT()


def write(path, rows):
    # (a header row: Crow's loaders read the first row as the names)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    rows = list(rows)
    with open(path, 'w', newline='') as f:
        w = csv.writer(f)
        w.writerow(['c%d' % i for i in range(len(rows[0]))])
        for r in rows:
            w.writerow(['%.17g' % x if isinstance(x, float) else x for x in r])


def matrix(t):
    return [[float(x) for x in row] for row in t.detach().tolist()]


def row(t):
    return [[float(x) for x in t.detach().tolist()]]


# the weights as Crow holds them: `h * w` with the rows the tokens, so a
# Linear's (out, in) weight is transposed; a head's q, k and v are its
# columns of the transposed c_attn, its slice of the output projection the
# rows of the transposed c_proj
def crow_weights(m):
    ws = [('wte', matrix(m.wte.weight)), ('wpe', matrix(m.wpe.weight))]
    for l, blk in enumerate(m.h):
        ws.append(('g1_%d' % l, row(blk.ln_1.weight)))
        wt = blk.attn.c_attn.weight.t()  # (D, 3D): q columns, k columns, v columns
        ot = blk.attn.c_proj.weight.t()  # (D, D)
        for h in range(H):
            ws.append(('q%d_%d' % (h, l), matrix(wt[:, h * HS:(h + 1) * HS])))
            ws.append(('k%d_%d' % (h, l), matrix(wt[:, D + h * HS:D + (h + 1) * HS])))
            ws.append(('v%d_%d' % (h, l), matrix(wt[:, 2 * D + h * HS:2 * D + (h + 1) * HS])))
            ws.append(('o%d_%d' % (h, l), matrix(ot[h * HS:(h + 1) * HS, :])))
        ws.append(('g2_%d' % l, row(blk.ln_2.weight)))
        ws.append(('w1_%d' % l, matrix(blk.mlp.c_fc.weight.t())))
        ws.append(('w2_%d' % l, matrix(blk.mlp.c_proj.weight.t())))
    ws.append(('gf', row(m.ln_f.weight)))
    return ws


for i, (name, rows) in enumerate(crow_weights(model)):
    write('data/w%02d_%s.csv' % (i, name), rows)
print('weights:', len(crow_weights(model)))

# (the vocabulary as one text: the characters in order, the newline first)
open('expected/vocab.txt', 'w', encoding='utf-8').write(''.join(chars))
x0 = data[0:T].unsqueeze(0)
y0 = data[1:T + 1].unsqueeze(0)
write('expected/tokens.csv', [[int(t) for t in data[0:T + 1].tolist()]])
with torch.no_grad():
    logits0, loss0 = model(x0, y0)
write('expected/logits0.csv', matrix(logits0[0]))
write('expected/loss0.csv', [[float(loss0)]])

losses = []
for step in range(STEPS):
    xb = data[step * T:(step + 1) * T].unsqueeze(0)
    yb = data[step * T + 1:(step + 1) * T + 1].unsqueeze(0)
    model.zero_grad()
    logits, loss = model(xb, yb)
    losses.append(float(loss))
    loss.backward()
    with torch.no_grad():
        for p in model.parameters():
            p -= LR * p.grad
write('expected/losses.csv', [[l] for l in losses])
with torch.no_grad():
    _, loss_after = model(data[0:T].unsqueeze(0), data[1:T + 1].unsqueeze(0))
write('expected/loss_after.csv', [[float(loss_after)]])

# the greedy continuation of the first block at the final weights: each
# step feeds the last T tokens and takes the largest logit of the last
idx = data[0:T].tolist()
with torch.no_grad():
    for _ in range(GENERATED):
        ctx = torch.tensor(idx[-T:]).unsqueeze(0)
        logits, _ = model(ctx)
        idx.append(int(torch.argmax(logits[0, -1])))
write('expected/greedy.csv', [idx[T:]])
print('vocab', V, 'loss0 %.12f' % float(loss0), 'losses', ' '.join('%.6f' % l for l in losses[:5]), '...', 'after %.12f' % float(loss_after))
print('greedy:', repr(''.join(chars[i] for i in idx[T:])))
