# finch's replay rules: Next setup, Auto Next, brackets, auto-trade

What the Python finch replay does, read end to end from its sources on
2026-09-25, so the Crow port of rows 9–11 (finch-sim-design.md §15) has
its rules and constants in one place. Paths are under
`~/code/python_projects/finch/trading_app/`:

- **API** = `api/scanner_vwap_replay.py`
- **SVC** = `runtime/vwap_scanner_replay_service.py`
- **EE** = `runtime/exit_engine.py`
- **BLD** = `runtime/vwap_scanner_replay_builder.py`
- **B2E** = `patterns/b2e.py`
- **VP** = `patterns/volume_profile.py`
- UI files are under `ui/src/components/simulation/`: PM = `PracticeMode.tsx`,
  SRM = `ScannerReplayMode.tsx`, TP = `TradePanel.tsx`,
  COC = `ComplexOrderCard.tsx`, URS = `useReplaySession.ts`,
  TH = `TradeHistory.tsx`, PC = `patternChips.tsx`, RCB = `ReplayControlBar.tsx`

## 0. How the replay runs

1. **Clock.** Session state is in memory only. Virtual time = anchor +
   wall time × speed. While playing it is capped at `window_end_ms`, but
   `playing` never turns itself off (SVC:345-351). `seek` clamps to the
   window and keeps the play/pause state (SVC:353-358).
2. **Speed.** The UI creates every session at speed 1 and has no speed
   control (PM:168, SRM:262). `setSpeed` exists but nothing in the UI
   calls it.
3. **When orders are evaluated.** Only on the SSE heartbeat, once per
   wall-second per connected stream (API:1118, 1133-1148), in this
   order: brackets, auto-arms, resting limit orders.
   - `GET /auto-arms` also evaluates arms (API:2441).
   - `POST /limit-sell` also evaluates resting orders (API:1756).
   - With no stream connected, nothing is evaluated.
4. **Walking bars.** Each evaluator walks the 1-second bars in
   (last_eval, head], so fast-forwarding never skips a level. Afterwards
   it sets last_eval = head, even when the head moved backwards
   (API:1900, 2340, 2564).

## A. Next setup, Auto Next, the closing window

**A1. Next setup** (the "⏭ Setup" button, both tabs; RCB:160-164)

1. **Which patterns.** The codes are the patterns enabled in
   `GET /trading/api/patterns` and not hidden by the chips
   (PM:309-311, SRM:433-435). An empty list omits the parameter, which
   means every enabled detector.
2. **Where the search starts.** after = max(head, lastSetupTsRef)
   (PM:314, SRM:437).
   - Practice sends `symbol=` the focused symbol.
   - Scanner sends `exclude=` the skipped symbols.
3. **The server's search** (API:778-836).
   - For each symbol, in session order and skipping excluded ones, it
     walks that symbol's setup events in ascending time.
   - It takes the first event with ts > after whose bar at-or-before ts
     exists and is visible (`bar_visible`).
   - The earliest event across symbols wins. Ties keep the first symbol
     (strict `<`).
4. **What a setup event is** (BLD:605-653).
   - Every enabled detector (intersected with the codes) runs over the
     pre-window and window 1-second bars, on 60-second candles
     (`SETUP_BAR_SECONDS=60`).
   - `max_signal_instances` is raised to 100000.
   - News context: all of the day's catalysts.
   - Only instances with `status=="confirmed"` count.
   - The event's ts is the minimum of the instance's `marker.x` and
     `segment.x0` values: its earliest drawn point, on a one-minute
     candle. For b2e that is the start of the signal's minute (B2E:467).
   - Results are cached per symbol and code for the session's life.
5. **When found.** lastSetupTsRef = ts; the scanner also focuses the
   symbol. Then pause, and seek to ts − 60 000 ms (`SETUP_JUMP_LEAD_MS`;
   PM:76, 326-328; SRM:98, 448-451). The replay stays paused.
6. **When nothing is found.** An alert says no later setup was found;
   nothing else changes.
7. Next setup is manual only. Neither Auto Next uses it.

**A2. Practice "Auto Next ⏭"** (PM:335-459)

1. **Starting.** `startAuto` sets awaiting = true and autoMode = true,
   then advances once (PM:395-400).
2. **Advance, first choice: the same symbol.** `next-in-gate` for the
   focused symbol after max(head, lastInGateTsRef) finds the first bar
   with ts > after that is in gate and visible, where the bar before is
   not (API:839-881). If found:
   - lastInGateTsRef = ts;
   - seek to ts − gateLead, with gateLead = 5000 + floor(random × 25000)
     ms (PM:80);
   - the play state is left as it is.
3. **Advance, when the symbol has no later episode: a random
   symbol/date.**
   - `GET /replay/random-practice?days=N`, N from the page's selector
     (default 180; `SimulationPage.tsx:26,193`).
   - A seek to first_in_gate_ts − gateLead is queued, and pending
     autoplay is set from the form's Autoplay box.
   - A new practice session is created with the form's window, speed 1,
     autoplay off.
   - When the chart's bars have loaded, the queued seek runs, then Play
     if autoplay was pending (PM:179-195, 363-385).
4. **The random pick** (API:2884-2964).
   - The pool is the eligible dates in the last N days × each day's
     universe under fixed filters: change ≥ 10, float ≤ 50M,
     rel-vol ≥ 5.
   - It is shuffled, and at most 80 candidates are tried.
   - The first candidate with an in-gate bar inside the default
     06:55–09:30 window is returned.
   - Otherwise 404: the UI alerts "Auto Next failed", and Auto Next
     stays on but idle.
5. **Out of the gate.** It advances when all hold: autoMode, not in
   gate, not awaiting, not busy, and playing (PM:432-445). It sets
   awaiting first, and awaiting clears only when in-gate is seen true,
   so there is one advance per gate exit. In-gate comes from the 1 Hz
   stream.
6. **At the window's end.** It advances when autoMode, not awaiting,
   not busy, and head ≥ window_end_ms, playing or not (PM:451-459).
7. **Stopping.** The form's Stop cancels the queued seek and autoplay
   and turns Auto Next off. The bar's Stop also ends the session
   (PM:402-417).
8. It never pauses, never changes speed, and never looks at open
   positions.

**A3. Scanner "Auto Next" and the closing window** (SRM:528-618)

1. **Active** (SRM:536-546).
   - With "RSI > 70 charts" on: active = RSI(3m) > 70, or in gate when
     "In-gate only" is on too.
   - Otherwise: active = in gate.
   - A missing or filtered-out row is inactive.
2. **Turning it on** advances at once when the focused symbol is not
   active (SRM:610-618).
3. **The closing window opens** when the focused symbol goes from active
   to inactive: the previous sample active, this one not, the same
   symbol. It holds {symbol, until = head + 10 000 virtual ms}
   (`AUTO_GRACE_MS`; SRM:34, 566-587). The card gets an amber border and
   "⏳ Closing m:ss", counted up to whole seconds (SRM:858-901).
4. **The closing window ends** (SRM:592-606).
   - It is cancelled when Auto is turned off, the focus changes, or the
     symbol is active again.
   - When head ≥ until, it advances exactly once.
   - While paused it never runs out.
   - Nothing closes positions.
5. **Advancing.**
   - In-gate: `next-in-gate-any` after max(head, lastInGateTsRef),
     excluding skipped symbols (API:884-947); focus that symbol; seek
     to ts − gateLead (SRM:402-422).
   - RSI: `next-rsi-70-any` after head: a bar strictly above 70 whose
     previous bar was at most 70 or had no RSI (API:1006-1071). Focus
     that symbol; seek to ts, with no lead (SRM:460-477).
   - When nothing is found: an alert only. The scanner never rolls
     onto a new session.

## B. Orders

The Practice tab has every order type below. The Scanner tab has manual
BUY and SELL ALL, and the manual arm.

**Rules every order shares**

1. **NBBO fill** (`runtime/nbbo_fills.py:29, 43-54, 120-148`).
   - The quote is the first found at the second of ts, then −1 s, +1 s,
     −2 s, +2 s.
   - Buys fill at the ask, sells at the bid.
   - With no quote, the fill is the close of the bar at or before ts,
     `fill_source = "close_fallback"`.
2. **When a reader's order fills.** At `observed_ts`, clamped to
   [window_start, the server's head] (API:1282-1294). `observed_ts` is
   the chart's smoothed head: the stream's head plus wall time × speed,
   capped at 1.5 s, and only while playing at speed ≤ 5
   (`VwapMiniChart.tsx:242, 245, 862-885`). Otherwise it is the stream's
   head.
3. **Slippage guard.** If |fill − observed_price| / observed_price × 100
   > 5.0, the order is refused with 409 (API:117, 1297-1317).
4. **The position.** Net = buys − sells with virtual ts ≤ the fill's
   (API:344-364).
5. **Size cap.** A buy is refused if net + qty > 5000 (`MAX_NET_LONG_QTY`;
   API:109, 385-405; also checked client-side at COC:21, TP:9).
6. **Long only.** Every sell is clamped to the net at its time
   (API:367-382).

**B1. The managed bracket** ("Complex Order": a market buy with a
trailing stop and a target, long only, no trigger)

1. **Its state** (SVC:217-243): symbol, qty, avg_price, peak,
   trail_value/unit and target_value/unit (`pct` or `cents`),
   stop_price, target_price, status (`active` | `filled` |
   `flattened`), last_eval, entry and exit ts, exit_price, realized_pnl.
2. **Inputs.**
   - The card's defaults: Qty 100 (min 1), Trail 5 %, Target 10 %
     (COC:39-43).
   - On mount they are replaced by `GET /replay/bracket-defaults`: the
     trail and target of the most recently created bracket row from
     any session, else 5 % and 10 % (API:1801-1819).
   - The server refuses qty, trail or target ≤ 0 with 422
     (API:205-222).
3. **Buying** (API:1569-1622).
   - The size cap, then a buy at the ask.
   - With no active bracket on the symbol, a new one: peak = the fill,
     entry_ts = last_eval = the fill's ts.
   - With an active one, the shares are added:
     avg = (avg·q + px·add) / (q + add), peak = max(peak, px).
   - The request's trail and target replace the old ones, the status
     is active, and the levels are recomputed.
4. **Levels** (SVC:245-267).
   - With qty ≤ 0, there are none.
   - stop = peak − trail/100 (¢) or peak × (1 − trail/100) (%); the
     stop only rises.
   - target = avg + target/100 (¢) or avg × (1 + target/100) (%),
     recomputed every time, so it can fall.
5. **Per bar** (API:1822-1903), from the bar after last_eval (so never
   the entry's bar) to the head:
   - A high (or the close if missing) above the peak raises the peak,
     then the levels are recomputed. So one bar can raise the stop and
     hit it.
   - A hit is low ≤ stop or high ≥ target; the two are treated alike.
   - sell_qty = min(qty, net at the bar). If that is 0, the status is
     `flattened` and there is no trade.
   - Otherwise it sells at that bar's bid (or its close), not at the
     level. Status `filled`, qty = sell_qty, exit_price = the fill,
     realized_pnl = (fill − avg) × sell_qty.
   - No reason is recorded, and there are no partial exits.
6. **Flatten** ("SELL ALL"; API:1625-1660): sells min(qty, net) at the
   server head's bid, with no slippage guard and no catch-up evaluation
   first. Status `flattened`; with net 0, no trade.
7. **Modify** (PATCH; API:1663-1684) replaces the settings and
   recomputes, never loosening the stop. Nothing in the UI calls it.
8. **What the card shows.**
   - Unrealised P&L = (last − avg) × qty; % = (last/avg − 1) × 100;
     last = the stream row's `last_bar.c` or price (COC:99-107).
   - The row shows Avg Buy, Current, Stop, Limit (the target price)
     and uPnL, only while active with qty > 0 (COC:73, 253-290;
     PM:206-211). A finished bracket is hidden.
9. **What is stored.** Each fill writes a trade row. The bracket row is
   inserted on the first buy and updated on add, modify, every
   heartbeat, and exit (API:1475-1520; the table is in
   `migrations/versions/20260604_c9d0e1f2a3b4_add_replay_brackets_table.py`).

**B2. The manual arm** (TradePanel "Manual arm (advanced)": trigger,
stop, qty, lock 2R; TP:285-480)

1. **The box and the button.**
   - Ticking the box POSTs `/auto-trade {symbol, enabled, qty, lock_2r}`
     (TP:325-337): auto-trading on this symbol's b2e signals (C).
   - "Arm" POSTs `/auto-arms {symbol, qty (100), trigger_price,
     stop_price, lock_2r}` (TP:390-405, 473). It shows only when there
     is no live arm, the setup is not ready or broken out, and the
     signal was not chased.
   - Trigger and stop are prefilled from the setup's `entry_trigger`
     and `stop_ref` to 2 decimals, unless chased: trigger < last × 0.99
     (TP:313-319, 338-341).
2. **The server** (API:2417-2433) refuses stop ≥ trigger with 400.
   R = trigger − stop; last_eval = head; pos_index 1; no probation; no
   cap.
3. **Its life** is an auto arm's (C4, C5): it fills only when a later
   bar's high ≥ trigger while the best setup at that bar is ready or
   broken out, and is cancelled otherwise.
4. **Lock 2R, the "+2R stop floor"** (API:2304-2305): once `ladder_ref`
   (the highest close of a completed 10-second bucket) ≥ entry + 2R,
   the stop is at least entry + 2R. R is from the trigger, entry is the
   fill.
5. `DELETE /auto-arms/{id}` cancels only an armed (unfilled) arm
   (API:2448-2456). The UI polls every 1.5 s and shows the last arm as
   "STATUS · trig · stop · in · out" (TP:368-389, 452-457).

**B3. The resting limit sell** (Practice; +5/10/15/20/25¢ buttons;
TP:5, 209-266)

1. **Placing** (API:1698-1761).
   - At the server's head. Refused with 400 when net ≤ 0.
   - limit = round(bid + offset/100, 4), the bid within ±2 s or the
     bar's close.
   - qty = the net at placement, fixed from then on.
   - It replaces any order on the symbol and is evaluated at once.
2. **Filling** (API:2496-2565).
   - The first evaluation includes the bar it was placed on.
   - The first bar with high ≥ limit sells min(qty, net at that bar)
     at the limit, `fill_source = "limit"`. With net 0 there, the order
     is cancelled.
   - "Cancel All" marks the orders cancelled (API:1764-1786). Orders
     live in memory only.
3. A limit fill leaves the bracket active until its next hit, where it
   flattens with no trade.

## C. Session-wide auto-trade

**C1. Turning it on** (PC:176-206; API:2369-2402).
- The chip row's box POSTs `{symbol:"*", enabled, qty:100}`. `lock_2r`
  is not sent, so it is false.
- The setting is per session and in memory; a new session starts off.
- Turning it off removes the per-symbol configs it made and cancels
  their armed arms. Filled arms are still managed.

**C2. The signals it trades** (API:1949-2020, on every evaluation)
1. Every symbol gets `{qty, lock_2r, auto:true}` by setdefault; a
   symbol's own config from the manual arm's box wins.
2. A symbol with an armed or filled arm is skipped: one live arm per
   symbol, no limit across symbols, no overall cap; auto buys bypass
   the 5000-share cap.
3. **Only the enabled `b2e` detector**, with its `pattern_configs`
   parameters, cached on the session once found. The chips and the
   other detectors do not matter; with b2e disabled nothing trades.
   - It runs `_scan` on the pre-window and window bars up to the head.
   - A signal: trailing-60 s volume ≥ 200k, RSI(14, 15 s) > 65, MACD
     histogram > 0 and rising, within 5 % of the session high or the
     high set ≤ 900 s ago, 60-s return ≥ 0.5 %, price 1–15, and at
     least 180 s since the last signal (`cooldown_s=180`; B2E:68-193,
     359-448).
4. **Taking a signal.**
   - Candidates have both an entry and a stop; x = the signal's second
     × 1000 (when its gates completed).
   - ptr = max(cfg.last_sig_ms, the latest exit ts of any arm on the
     symbol). The earliest signal with x > ptr is taken, and
     last_sig_ms = x.
   - At most one new arm per symbol per evaluation.
   - No age limit: switching on mid-session takes signals from the
     day's first.
5. **Spacing** is the detector's own 180 s, plus re-entry needs a signal
   after the last exit. There is no other cooldown.

**C3. The arm** (API:2025-2133)
1. **Sequence.** prior = the symbol's finished arms by exit time;
   pos_index = len(prior) + 1. Probation: the last trade exited at or
   below its entry (EE:254-264).
2. **Levels.**
   - trigger = the signal's entry, the signal bar's close.
   - stop = entry × (1 − risk%/100), risk% = clamp(0.50 × (the 120-s
     high − low range)/px × 100, 1, 5) (B2E:151-153, 425-430).
   - On probation: stop = trigger − 0.6 × (trigger − stop)
     (EE:66, 267-269).
   - R = trigger − stop, after probation.
   - No target and no trail (b2e's 2R target is not used).
   - `cap_price` is always None (`CAP_POSITION=0`; EE:74, 272-278).
3. **Size** = max(int(round(base × sm × crt × pvt × prs × vp)), 1),
   round half to even, base = cfg.qty (100):
   - **sm**, the extended-RSI tier (B2E:124-127, 346-351): 1.0 if the
     partial RSI15 < 75; else 0.25 if entry ≤ the day's high as of
     600 s before, else 0.5.
   - **crt** (API:2055-2071): crv = the cumulative volume up to x ÷
     `adv20`. If crv ≥ 5 and sm == 1 and float < 20M and
     2 ≤ entry ≤ 20: 1.5; else if crv < 1: 0.5; else 1.0. Missing data
     gives 1.0.
   - **pvt** (API:2072-2086): pivots P = (H+L+C)/3 and levels (P,
     2P−L, P+(H−L), H+2(H−L)). The next level above entry within
     0.5 × plan R: 0.25; else entry < P+(H−L): 0.5; else 1.0.
   - **prs** (API:2090): 1.5 if the last finished arm exited above its
     entry, else 1.0.
   - **vp** (VP:40-42, 144-170), from the day's volume profile of bars
     with t < x: a prior-day HVN within 1R above entry: 0.5; no
     prior-day HVN above, a current-day HVN above, and sm == 1: 1.5;
     else 1.0.
   - There is no `sizing_json` in replay; the live-only tiers are
     absent.

**C4. Entry** (API:2143-2193)
- The walk starts at the first bar after x − 1000: the signal's bar.
- On the first bar with high ≥ trigger, `_causal_setup(i)` (cached per
  10-second bucket; BLD:548-602) must report a best status of ready or
  breakout. Otherwise the arm is **cancelled** and the signal stays
  taken.
- entry = round(max(trigger × 1.005, open, close), 4): a buy with
  `fill_source "auto_arm"`, no bid or ask.
- max_seen = the bar's high; the ladder starts. Exits begin on the next
  bar.

**C5. Exits, per bar in this order; the first exit wins**
(API:2194-2339)

Each exit sells min(qty, net then) at the stated price, and the status
becomes `done` even when nothing is sold (API:1908-1931).

0. **Bookkeeping.**
   - max_seen = max(max_seen, high).
   - When the 10-second bucket changes, ladder_ref = max(ladder_ref,
     the last close of the bucket before).
   - r = R if R > 0, else entry × 0.02.
   - hit1 = max_seen ≥ entry + 0.75r (`PROTECT_GATE_R`, EE:51).
1. **Fast fail on probation** (EE:194-228). During the first full
   one-minute candle after the entry's candle, a close below the entry
   candle's close exits at close × 0.999. After that candle, the check
   is off.
2. **A pending exit** (EE:85-93). If one is pending: close ≤ p0 − 0.5r
   exits; close ≥ p0 + 0.5r clears it; more than 60 s since it was set
   exits. Exits are at close × 0.999.
3. **Confluence** (EE:153-191), with hit1 and nothing pending: if the
   close is under the 60-s volume-weighted average of closes and the
   120-s tick-rule CVD < 0, then with RSI(14, 15 s partial) > 70 a
   pending exit at p0 = close × 0.999, otherwise an exit at
   close × 0.999.
4. **Channel** (EE:311-347). The same pending-or-exit rule, reason
   "channel". The line is a least-squares fit through the lows of the
   last 3 completed one-minute candles, with a minimum slope of
   0.01 × entry / 60 per second. It fires when the close is under the
   line.
5. **The trailing window.** T = 180 s, or 30 s if any bar within 120 s
   of entry reached entry + 2r (EE:115-141). W = the bars with t in
   (t − T, t], the current bar included.
6. **The stop** is the highest of:
   - the previous stop;
   - the lowest low in W;
   - entry, once ladder_ref ≥ entry + r;
   - entry + r, once ladder_ref ≥ entry + 2r;
   - entry + 2r, with lock 2R, once ladder_ref ≥ entry + 2r;
   - round(max_seen × 0.99, 4), when entry ≥ 8 and
     max_seen ≥ 1.08 × entry (EE:104-106, 377-387).
7. **Stall**, with hit1. A high ≥ the highest high in W records a new
   high. Otherwise, when there is a last new high more than 120 s ago
   and nothing is pending: pending or exit, reason "stall".
8. **The stop hit.** low ≤ stop exits at stop × 0.999, reason
   "bigwin_trail" if the 1 % trail set the stop, else "stop". Since W
   includes the current bar, a bar that sets or ties the window's low
   exits.

**C6. What is recorded.** Only trade rows and the session's totals.
- Arms (status, exit reason, size) live in memory, seen through
  `GET /auto-arms`.
- Nothing is written to `auto_trade_arms` or `auto_trade_mode`; those
  are live-trading tables.
- `background/auto_trade_monitor.py` is not used by replay; only EE is
  shared.
- Live-only rules absent from replay: `FIXED_TP_R=3.0` (EE:63), the +1R
  bank (EE:509-534), entry cutoffs, and the deto lanes.

**C7. How they meet.**
1. An Auto Next roll-over creates a new session, where auto-trade is off.
2. A forward seek makes arms, brackets and limit orders catch up bar by
   bar. Signals a seek jumped over are taken later, with fills dated in
   the past, about one new arm per symbol per heartbeat.
3. Changing the focused symbol re-renders the manual-arm panel, which
   POSTs `{symbol, enabled:false}`. That removes the symbol's config
   (including one the session-wide switch made, resetting last_sig_ms)
   and cancels its armed arms (TP:325-337, API:2397-2401).
4. Neither the closing window nor Auto Next exits positions. Server-side
   exits go on for every symbol, focused or not.

## D. Trades, P&L, the end of a session

1. **A trade.** `ReplayTrade {id, symbol, side, qty, price,
   fill_source ∈ nbbo | close_fallback | limit | auto_arm, bid, ask,
   virtual_ts_ms}`, appended to the session's trades in wall-clock
   order (SVC:132-144).
2. **What is stored.** Every fill inserts a `replay_trades` row at once
   and recomputes the session's realized P&L, trade count, wins and
   losses, NBBO and fallback counts (API:1392-1429). With no parent
   session row, this is silently skipped.
3. **The server's P&L** is FIFO over the trades in append order, long
   and short alike; each matched lot is a win (≥ 0) or a loss
   (API:430-479). The history's % is pnl / cost × 100, over rows in
   virtual-ts order (API:2655-2687).
4. **Manual trades.** BUY takes the qty input (100) and fills at the
   ask. SELL ALL sends the client's net over all trades; the server
   refuses it when that exceeds the net at the fill's time
   (API:1350-1362).
5. **The UI's history** (TH:33-178) groups fills into round trips by
   virtual ts. Unrealised = realised partial closes + (mark − average
   entry) × open qty. A round trip with pnl ≥ 0 is a win.
6. **Stop / End** (API:2588-2625; URS:469-481) refreshes the totals,
   sets `stopped_at`, and drops the session from memory.
   - There is no last evaluation and no flatten: positions stay open,
     unrealised P&L is never saved, bracket rows stay `active`, and
     limit orders and arms are gone.
   - The same happens when the page unmounts and before a new session
     is created (URS:279-286, 343-348).
   - Reaching the window's end does not end the session.

## E. What the code leaves open

1. **Comments that disagree with the code:**
   - the protect gate is 0.75R, though comments say +0.5R or +1R;
   - the chase window at SVC:302-305 is not on the server;
   - TP's ATR stop fallback is not implemented (the `atr` prop is
     unused);
   - `models/replay.py` says trades are saved at stop; they are saved
     per fill;
   - the bracket's docstring says "stop first", but stop and target are
     alike in code.
2. **Timing.** Whether a b2e signal exists depends on where the head is
   inside its 15-second bucket (`causal_gate=0`), so results depend on
   the heartbeat's cadence. The fill's quality check's 10-second cache
   can answer with another second's state.
3. **Backward seeks** move last_eval back, so bars are walked again: a
   bracket can "hit" on a bar before its entry and flatten with no
   trade.
4. **Separate evaluators.** When one heartbeat spans several bars, a
   fill with an earlier ts handled later can oversell (net below 0).
5. **Two P&L orders.** Append-order FIFO and virtual-ts order diverge
   when fills are dated back. The server counts wins per lot and the UI
   per round trip.
6. **Refs that are never reset.** lastInGateTsRef and lastSetupTsRef
   survive a session change, so a roll-over to an earlier date can roll
   again at once.
7. **Missed gates.** Auto Next samples in-gate at 1 Hz. A gate shorter
   than a sample leaves awaiting set, and Auto Next stalls.
8. **Pre-window signals** can fill at the window's first bar if any
   detector is ready or broken out there.
9. **Optimistic and future-looking fills.** A stop fills at
   stop × 0.999 even through a gap down. The NBBO lookup can use a
   quote up to 2 s ahead.
10. **The manual arm tends to cancel.** Arm shows only when no setup is
    ready or broken out, yet its fill needs ready or broken out at the
    crossing.
11. **Loose ends.** No lock between concurrent evaluations;
    `prune_idle` is never called; the first arm to appear does not
    refresh the trade log (TP:378-383).
