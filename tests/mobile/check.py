#!/usr/bin/env python3
"""The mobile theme's check: pages loaded in headless Chrome as a phone.

Each page is opened at 375x812 with touch emulated (a coarse pointer,
which is what chooses the mobile theme) and measured, with nothing
pressed:

  overflow   how far the document scrolls sideways (must be 0)
  wide       elements reaching past the viewport that no box of their own
             scrolls (the outermost of each, must be none)
  targets    controls whose hit area is under 44x44 CSS px: buttons, links
             that are not words in a sentence (WCAG 2.5.8's inline
             exception), fields, selects, a checkbox's or radio's label,
             tabs, summaries, anything with a click (must be none)
  text       text under 16px that no caption rule allows (must be none).
             Captions may go to 12px: `th` (column heads), `h2` (the
             section label), `.card h3` (a card's label), `size = 'xs'`
             (the type step written font-size:12px) and `small`, `sup`,
             `sub`; code in `pre`/`code` keeps 0.9em of its line, so it
             may go to 14px
  layers     every modal, drawer and toast forced open, and the runtime's
             own confirm dialog and notice added, then held inside the
             viewport, their controls and text measured as above; the
             offline banner's text size read

At 1280x800 (no touch) a sample of elements' computed styles is written
with --record and held against with --compare: the desktop look before a
change and after it, property by property. Recording runs every page twice
and leaves out what differs between the two runs (live numbers, spinners).

Pages: by default the served test apps and the layout conformance views
of this repository, each copied to a scratch folder, its port changed, and
run interpreted with the crow binary given (CROW=..., default
target/release/crow); or an app already running (--at URL with --sweep
FILE, a list of pages as Lory's tests/drawn/sweep_*.json write them; a page
with steps is skipped, since this check presses nothing).

  python3 tests/mobile/check.py                       # the repo's apps, as a phone
  python3 tests/mobile/check.py --record before.json  # ... and the desktop sample
  python3 tests/mobile/check.py --compare before.json # ... held to it
  python3 tests/mobile/check.py --at http://127.0.0.1:3960 --sweep pages.json

Exit status 1 when a page fails (--report-only: always 0). Needs python3
with `websockets` and Chrome (CHROME=... to point at one).
"""
import argparse, asyncio, json, os, re, shutil, signal, subprocess, sys, tempfile, time, urllib.request
from pathlib import Path

import websockets

ROOT = Path(__file__).resolve().parents[2]
CHROME = os.environ.get("CHROME", "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")
PHONE = {"width": 375, "height": 812, "deviceScaleFactor": 3, "mobile": True}
DESKTOP = {"width": 1280, "height": 800, "deviceScaleFactor": 1, "mobile": False}
UA = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1"

# The repository's apps: (source, the views' paths[, more of serve[]'s
# arguments]). A source with no serve[] gets one.
APPS = [
    ("tests/layout/conformance.co", ["/", "/prose", "/primitives"]),
    ("tests/ui_primitives.co", ["/", "/past"]),
    ("tests/ui_primitives.co", ["/"], "density = 'compact', wide = true"),
    ("tests/served/04_writes.co", ["/"]),
    ("tests/served/06_sign_in.co", ["/"]),
    ("tests/served/07_paging.co", ["/"]),
    ("tests/served/15_pages.co", ["/", "/card/7"]),
    ("tests/served/17_keys.co", ["/"]),
    ("tests/served/18_sort.co", ["/"]),
    ("tests/served/19_accessibility.co", ["/"]),
    ("tests/served/48_a_date_field_written_is_set.co", ["/"]),
    ("examples/dashboard.co", ["/"]),
    ("examples/layout_dashboard.co", ["/"]),
]
FIRST_PORT = 3980

MEASURE_JS = r"""
(() => {
  const vw = window.innerWidth, vh = window.innerHeight;
  const CAPTION = 'th, h2, .card h3, small, sup, sub, [style*="font-size:12px"]';
  const CODE = 'pre, code, kbd, samp';
  const TARGET = 'button, a[href], input:not([type=hidden]), select, textarea, summary, [role=button], [role=tab], [data-crow-click], [tabindex]:not([tabindex="-1"])';
  const path = (el) => {
    const out = [];
    for (let e = el; e && e.nodeType === 1 && out.length < 4; e = e.parentElement) {
      let s = e.tagName.toLowerCase();
      if (e.id) s += '#' + e.id;
      const c = (e.getAttribute('class') || '').trim().split(/\s+/).filter(Boolean).slice(0, 2);
      if (c.length) s += '.' + c.join('.');
      out.unshift(s);
      if (e.id) break;
    }
    return out.join(' > ');
  };
  const label = (el) => (el.getAttribute('aria-label') || el.textContent || el.value || el.getAttribute('title') || '').trim().replace(/\s+/g, ' ').slice(0, 40);
  const shown = (el) => {
    if (!el.getClientRects().length) return false;
    const r = el.getBoundingClientRect();
    if (r.width === 0 && r.height === 0) return false;
    if (getComputedStyle(el).visibility === 'hidden') return false;
    for (let e = el; e; e = e.parentElement) {
      if (e.classList && e.classList.contains('crow-sr')) return false;
      if (getComputedStyle(e).opacity === '0') return false;
    }
    return true;
  };
  const scrolls = (e) => { const o = getComputedStyle(e).overflowX; return o === 'auto' || o === 'scroll' || o === 'hidden' || o === 'clip'; };
  const within = (root) => root ? Array.from(root.querySelectorAll('*')) : Array.from(document.body.querySelectorAll('*'));

  function targets(root) {
    const seen = new Set(), small = [], exempt = [];
    for (const el of (root || document.body).querySelectorAll(TARGET)) {
      let hit = el;
      if (el.tagName === 'INPUT' && (el.type === 'checkbox' || el.type === 'radio') && el.closest('label')) hit = el.closest('label');
      if (seen.has(hit)) continue;
      seen.add(hit);
      if (!shown(hit)) continue;
      if (hit.closest('svg')) continue;
      const r = hit.getBoundingClientRect();
      if (hit.tagName === 'A' && getComputedStyle(hit).display === 'inline') {
        const block = hit.parentElement && hit.parentElement.closest('p, li, dd, blockquote, td, figcaption, .doc');
        if (block && block.textContent.trim().length > hit.textContent.trim().length) { exempt.push(path(hit)); continue; }
      }
      if (r.width < 43.5 || r.height < 43.5) small.push({ el: path(hit), text: label(hit), w: Math.round(r.width * 10) / 10, h: Math.round(r.height * 10) / 10 });
    }
    return { small, exempt: exempt.length, count: seen.size };
  }

  function texts(root) {
    const small = [], captions = {}, code = {};
    let count = 0;
    const walk = document.createTreeWalker(root || document.body, NodeFilter.SHOW_TEXT);
    const done = new Set();
    for (let n = walk.nextNode(); n; n = walk.nextNode()) {
      if (!n.nodeValue.trim()) continue;
      const el = n.parentElement;
      if (!el || done.has(el)) continue;
      done.add(el);
      if (el.closest('script, style, noscript, template, svg, canvas, title')) continue;
      if (!shown(el)) continue;
      count++;
      const fs = parseFloat(getComputedStyle(el).fontSize);
      if (fs >= 15.95) continue;
      const cap = el.closest(CAPTION), mono = el.closest(CODE);
      if (cap && fs >= 11.95) { const k = cap.matches('th') ? 'th' : cap.matches('h2') ? 'h2' : cap.matches('.card h3') ? '.card h3' : cap.matches('[style*="font-size:12px"]') ? "size = 'xs'" : cap.tagName.toLowerCase(); captions[k] = (captions[k] || 0) + 1; continue; }
      if (mono && fs >= 13.95) { code[mono.tagName.toLowerCase()] = (code[mono.tagName.toLowerCase()] || 0) + 1; continue; }
      small.push({ el: path(el), text: n.nodeValue.trim().slice(0, 30), px: Math.round(fs * 10) / 10 });
    }
    return { small, captions, code, count };
  }

  // (anything laid out counts, seen or not: a hidden box still widens the
  // page; a fixed layer put away hidden does not)
  const away = (el) => { for (let e = el; e; e = e.parentElement) { const cs = getComputedStyle(e); if (cs.position === 'fixed' && cs.visibility === 'hidden') return true; } return false; };
  function wide(root) {
    const bad = new Set(), out = [];
    for (const el of within(root)) {
      if (el.closest('svg') && el.tagName !== 'svg') continue;
      if (!el.getClientRects().length || away(el)) continue;
      const r = el.getBoundingClientRect();
      if (r.right <= vw + 0.5 && r.left >= -0.5) continue;
      let held = false;
      for (let a = el.parentElement; a && a !== document.documentElement; a = a.parentElement) {
        if (a === document.body) break;
        if (scrolls(a)) { const ar = a.getBoundingClientRect(); if (ar.right <= vw + 0.5 && ar.left >= -0.5) { held = true; break; } }
      }
      if (!held) bad.add(el);
    }
    for (const el of bad) {
      if (el.parentElement && bad.has(el.parentElement)) continue;
      const r = el.getBoundingClientRect();
      out.push({ el: path(el), left: Math.round(r.left), right: Math.round(r.right), w: Math.round(r.width) });
    }
    return out;
  }

  // the first column of a table that scrolls in its own box: is it in view?
  function tables() {
    const out = [];
    for (const t of document.querySelectorAll('table')) {
      if (!shown(t)) continue;
      const box = scrolls(t) ? t : null;
      if (!box || box.scrollWidth <= box.clientWidth + 1) continue;
      const cell = t.querySelector('tbody tr td, tr td, tr th');
      const r = cell ? cell.getBoundingClientRect() : null;
      out.push({ el: path(t), scroll: box.scrollWidth - box.clientWidth, first: r ? Math.round(r.width) : 0, sticky: cell ? getComputedStyle(cell).position === 'sticky' : false });
    }
    return out;
  }

  // what reaches farthest right, for a page that scrolls sideways with no
  // element found wide (a box held in a scroller that itself does not fit)
  function widest() {
    let best = null, right = vw;
    for (const el of document.body.querySelectorAll('*')) {
      if (!el.getClientRects().length || away(el)) continue;
      const r = el.getBoundingClientRect();
      if (r.right > right + 0.5) { right = r.right; best = el; }
    }
    return best ? { el: path(best), right: Math.round(right) } : null;
  }
  const page = {
    overflow: Math.max(document.documentElement.scrollWidth, document.body.scrollWidth) - document.documentElement.clientWidth,
    wide: wide(), targets: targets(), text: texts(), tables: tables(),
    widest: widest(),
    surface: getComputedStyle(document.documentElement).getPropertyValue('--crow-surface-name').trim() || 'desktop',
  };

  // the layers, forced open, and the runtime's own
  const opened = [];
  for (const el of document.querySelectorAll('.modal, .drawer, .toast')) { opened.push([el, el.getAttribute('data-open')]); el.setAttribute('data-open', 'true'); el.style.animation = 'none'; }
  const added = document.createElement('div');
  added.innerHTML = '<div class="crow-notice" role="status" style="animation:none">A notice the runtime shows when a handler stops: the stop named, as long as it is</div>'
    + '<div aria-modal="true" class="crow-confirm" role="dialog"><div class="crow-confirm-box"><p>Cancel every open order for the account? This cannot be undone.</p><div class="crow-confirm-actions"><button aria-keyshortcuts="Escape">Cancel</button><button class="primary">Confirm</button></div></div></div>';
  document.body.appendChild(added);
  const layers = [];
  const kinds = [['.modal', (e) => e.firstElementChild], ['.drawer', (e) => e], ['.toast', (e) => e], ['.crow-confirm', (e) => e.querySelector('.crow-confirm-box')], ['.crow-notice', (e) => e]];
  for (const [sel, box] of kinds) {
    for (const el of document.querySelectorAll(sel)) {
      const b = box(el);
      if (!b || !shown(b)) continue;
      const r = b.getBoundingClientRect();
      const t = targets(el), x = texts(el);
      layers.push({ el: path(b), fits: r.left >= -0.5 && r.top >= -0.5 && r.right <= vw + 0.5 && r.bottom <= vh + 0.5, box: [Math.round(r.left), Math.round(r.top), Math.round(r.right), Math.round(r.bottom)], small: t.small, smalltext: x.small });
    }
  }
  added.remove();
  for (const [el, was] of opened) { if (was === null) el.removeAttribute('data-open'); else el.setAttribute('data-open', was); el.style.animation = ''; }
  document.documentElement.setAttribute('data-crow-offline', '');
  const after = getComputedStyle(document.documentElement, '::after');
  page.offline = { px: parseFloat(after.fontSize), top: after.top, right: after.right };
  document.documentElement.removeAttribute('data-crow-offline');
  page.layers = layers;
  return page;
})()
"""

SAMPLE = ("html, body, #crow-root, h1, h2, h3, p, a, button, button.primary, input, select, textarea, label, table, th, td, "
          "td.num, tr, nav a, .row, .stack, .grid, .card, .stats, .controls, header, .crow-field, .crow-tabs, .crow-tabs label, "
          "summary, details, .dropdown > summary, .modal, .drawer, .toast, pre, code, img, svg.icon, progress, hr, .crow-check, .crow-radio label")

DESKTOP_JS = r"""
((sels) => {
  const out = {};
  for (const sel of sels.split(',').map(s => s.trim())) {
    const els = document.querySelectorAll(sel);
    for (let i = 0; i < Math.min(els.length, 3); i++) {
      const cs = getComputedStyle(els[i]), props = {};
      for (let j = 0; j < cs.length; j++) { const p = cs[j]; if (!p.startsWith('--')) props[p] = cs.getPropertyValue(p); }
      out[sel + '[' + i + ']'] = props;
    }
  }
  out['document'] = { scrollWidth: String(document.documentElement.scrollWidth), scrollHeight: String(document.documentElement.scrollHeight) };
  return out;
})
"""


class Chrome:
    """One headless Chrome, driven over its DevTools socket."""

    def __init__(self):
        self.dir = tempfile.mkdtemp(prefix="crow-mobile-chrome-")
        self.proc = subprocess.Popen([CHROME, "--headless=new", "--remote-debugging-port=0", f"--user-data-dir={self.dir}",
                                      "--no-first-run", "--no-default-browser-check", "--disable-gpu", "--hide-scrollbars", "about:blank"],
                                     stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
        port_file = Path(self.dir) / "DevToolsActivePort"
        for _ in range(200):
            if port_file.exists() and len(port_file.read_text().splitlines()) >= 2:
                break
            time.sleep(0.05)
        port, path = port_file.read_text().splitlines()[:2]
        self.url = f"ws://127.0.0.1:{port}{path}"
        self.next = 0
        self.waiting = {}
        self.events = {}

    async def start(self):
        self.ws = await websockets.connect(self.url, max_size=None)
        self.reader = asyncio.create_task(self.read())

    async def read(self):
        async for raw in self.ws:
            msg = json.loads(raw)
            if "id" in msg and msg["id"] in self.waiting:
                self.waiting.pop(msg["id"]).set_result(msg)
            elif "method" in msg:
                q = self.events.get(msg.get("sessionId"))
                if q is not None:
                    q.put_nowait(msg)

    async def send(self, method, params=None, session=None):
        self.next += 1
        fut = asyncio.get_running_loop().create_future()
        self.waiting[self.next] = fut
        msg = {"id": self.next, "method": method, "params": params or {}}
        if session:
            msg["sessionId"] = session
        await self.ws.send(json.dumps(msg))
        res = await asyncio.wait_for(fut, 60)
        if "error" in res:
            raise RuntimeError(f"{method}: {res['error']}")
        return res.get("result", {})

    async def page(self, url, metrics, touch, js, arg=None, settle=1.5):
        target = (await self.send("Target.createTarget", {"url": "about:blank"}))["targetId"]
        session = (await self.send("Target.attachToTarget", {"targetId": target, "flatten": True}))["sessionId"]
        self.events[session] = asyncio.Queue()
        try:
            await self.send("Page.enable", session=session)
            await self.send("Emulation.setDeviceMetricsOverride", dict(metrics, screenWidth=metrics["width"], screenHeight=metrics["height"]), session)
            await self.send("Emulation.setTouchEmulationEnabled", {"enabled": touch, "maxTouchPoints": 5 if touch else 1}, session)
            if touch:
                await self.send("Emulation.setUserAgentOverride", {"userAgent": UA, "platform": "iPhone"}, session)
            await self.send("Page.navigate", {"url": url}, session)
            q = self.events[session]
            end = time.time() + 30
            while time.time() < end:
                try:
                    ev = await asyncio.wait_for(q.get(), max(0.1, end - time.time()))
                except asyncio.TimeoutError:
                    break
                if ev["method"] == "Page.loadEventFired":
                    break
            await asyncio.sleep(settle)
            expr = js if arg is None else f"({js})({json.dumps(arg)})"
            res = await self.send("Runtime.evaluate", {"expression": expr, "returnByValue": True, "awaitPromise": True}, session)
            if "exceptionDetails" in res:
                raise RuntimeError(json.dumps(res["exceptionDetails"])[:400])
            return res["result"].get("value")
        finally:
            self.events.pop(session, None)
            await self.send("Target.closeTarget", {"targetId": target})

    async def stop(self):
        self.reader.cancel()
        await self.ws.close()
        os.killpg(os.getpgid(self.proc.pid), signal.SIGTERM)
        self.proc.wait(10)
        shutil.rmtree(self.dir, ignore_errors=True)


def serve_repo_apps(crow):
    """The repository's apps, each in a scratch folder on its own port."""
    work = Path(tempfile.mkdtemp(prefix="crow-mobile-apps-"))
    procs, pages = [], []
    for n, (src, paths, *more) in enumerate(APPS):
        port = FIRST_PORT + n
        text = (ROOT / src).read_text()
        serve = f"serve[{port}" + "".join(f", {m}" for m in more)
        if re.search(r"serve\[\d+", text):
            text = re.sub(r"serve\[\d+", serve, text)
        else:
            text += f"\n{serve}]\n"
        name = Path(src).stem + (f" ({more[0]})" if more else "")
        d = work / f"{n:02}_{Path(src).stem}"
        d.mkdir()
        (d / "main.co").write_text(text)
        fixtures = ROOT / Path(src).parent / "fixtures"
        if fixtures.is_dir():
            shutil.copytree(fixtures, d / "fixtures")
        procs.append(subprocess.Popen([crow, "run", "-i", "main.co"], cwd=d, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True))
        for p in paths:
            pages.append({"name": f"{name} {p}", "url": f"http://127.0.0.1:{port}{p}", "port": port})
    for port in sorted({p["port"] for p in pages}):
        for _ in range(300):
            try:
                urllib.request.urlopen(f"http://127.0.0.1:{port}/", timeout=1)
                break
            except urllib.error.HTTPError:
                break
            except Exception:
                time.sleep(0.2)
        else:
            print(f"warning: nothing answered on {port}", file=sys.stderr)
    return work, procs, pages


def sweep_pages(at, sweep):
    spec = json.loads(Path(sweep).read_text())
    out = []
    for p in spec["pages"]:
        if p.get("steps"):
            continue
        url = at.rstrip("/") + p["path"] + (("?" + p["query"]) if p.get("query") else "")
        out.append({"name": p.get("name", p["path"]), "url": url})
    return out


def failures(m):
    out = []
    if m["overflow"] > 0:
        out.append(f"overflow {m['overflow']}px")
    if m["wide"]:
        out.append(f"{len(m['wide'])} wide")
    elif m["overflow"] > 0 and m.get("widest"):
        out.append(f"farthest right: {m['widest']}")
    if m["targets"]["small"]:
        out.append(f"{len(m['targets']['small'])} small targets")
    if m["text"]["small"]:
        out.append(f"{len(m['text']['small'])} small text")
    for layer in m["layers"]:
        if not layer["fits"]:
            out.append(f"layer {layer['el']} outside the viewport {layer['box']}")
        if layer["small"] or layer["smalltext"]:
            out.append(f"layer {layer['el']}: {len(layer['small'])} small targets, {len(layer['smalltext'])} small text")
    if m["offline"]["px"] < 12:
        out.append(f"offline banner text {m['offline']['px']}px")
    return out


async def run(args):
    work = procs = None
    if args.at:
        pages = sweep_pages(args.at, args.sweep)
    else:
        crow = os.environ.get("CROW", str(ROOT / "target" / "release" / "crow"))
        work, procs, pages = serve_repo_apps(crow)
    if args.only:
        pages = [p for p in pages if re.search(args.only, p["name"])]
    chrome = Chrome()
    await chrome.start()
    report = {"pages": {}, "desktop": {}}
    failed = 0
    try:
        for p in pages:
            m = await chrome.page(p["url"], PHONE, True, MEASURE_JS, settle=args.settle)
            report["pages"][p["name"]] = m
            bad = failures(m)
            if m["surface"] != "mobile":
                bad.insert(0, f"the {m['surface']} theme, not the mobile one")
            failed += bool(bad)
            caps = ", ".join(f"{k} {v}" for k, v in sorted(m["text"]["captions"].items()))
            print(f"{'FAIL' if bad else 'ok  '} {p['name']}: overflow {m['overflow']}px, {len(m['wide'])} wide, "
                  f"{len(m['targets']['small'])}/{m['targets']['count']} small targets ({m['targets']['exempt']} inline links exempt), "
                  f"{len(m['text']['small'])}/{m['text']['count']} small text" + (f"; captions: {caps}" if caps else "")
                  + (f"; code: {sum(m['text']['code'].values())}" if m['text']['code'] else ""))
            if args.verbose:
                for w in m["wide"][:8]:
                    print(f"       wide {w}")
                for t in m["targets"]["small"][:args.verbose]:
                    print(f"       target {t}")
                for t in m["text"]["small"][:args.verbose]:
                    print(f"       text {t}")
                for t in m["tables"]:
                    print(f"       table {t}")
                for l in m["layers"]:
                    print(f"       layer {l['el']} fits={l['fits']} {l['box']} small={l['small'][:3]} smalltext={l['smalltext'][:3]}")
            for b in bad:
                print(f"       {b}")
        if args.record or args.compare:
            for p in pages:
                first = await chrome.page(p["url"], DESKTOP, False, DESKTOP_JS, SAMPLE, settle=args.settle)
                if args.record:
                    second = await chrome.page(p["url"], DESKTOP, False, DESKTOP_JS, SAMPLE, settle=args.settle)
                    unstable = sorted(f"{k} {prop}" for k in first for prop in first[k] if second.get(k, {}).get(prop) != first[k][prop])
                    report["desktop"][p["name"]] = {"styles": first, "unstable": unstable}
                else:
                    report["desktop"][p["name"]] = {"styles": first}
        if args.record:
            Path(args.record).write_text(json.dumps(report["desktop"], indent=0, sort_keys=True))
            n = sum(len(v["styles"]) for v in report["desktop"].values())
            print(f"desktop: {n} elements' computed styles recorded at 1280x800 to {args.record}")
        if args.compare:
            before = json.loads(Path(args.compare).read_text())
            diffs = 0
            for name, now in report["desktop"].items():
                was = before.get(name)
                if was is None:
                    print(f"desktop {name}: not in {args.compare}")
                    continue
                skip = set(was.get("unstable", []))
                page_diffs = []
                for k in sorted(set(was["styles"]) | set(now["styles"])):
                    a, b = was["styles"].get(k), now["styles"].get(k)
                    if a is None or b is None:
                        page_diffs.append(f"{k}: {'gone' if b is None else 'new'}")
                        continue
                    for prop in sorted(set(a) | set(b)):
                        if f"{k} {prop}" in skip or a.get(prop) == b.get(prop):
                            continue
                        page_diffs.append(f"{k} {prop}: {a.get(prop)!r} -> {b.get(prop)!r}")
                diffs += len(page_diffs)
                n = len(now["styles"])
                print(f"{'ok  ' if not page_diffs else 'FAIL'} desktop {name}: {n} elements, {len(page_diffs)} differences ({len(skip)} unstable left out)")
                for d in page_diffs[:12]:
                    print(f"       {d}")
            failed += bool(diffs)
        if args.json:
            Path(args.json).write_text(json.dumps(report["pages"], indent=1))
    finally:
        await chrome.stop()
        for pr in procs or []:
            os.killpg(os.getpgid(pr.pid), signal.SIGTERM)
        if work:
            shutil.rmtree(work, ignore_errors=True)
    print(f"{len(pages)} pages, {failed} failing")
    return 0 if args.report_only or not failed else 1


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--at", help="an app already running, e.g. http://127.0.0.1:3960")
    ap.add_argument("--sweep", help="its pages, as a sweep_*.json lists them")
    ap.add_argument("--only", help="the pages whose names match this")
    ap.add_argument("--record", help="write the desktop sample here")
    ap.add_argument("--compare", help="hold the desktop sample to this recording")
    ap.add_argument("--json", help="write every page's measurements here")
    ap.add_argument("--settle", type=float, default=1.5, help="seconds after load before measuring")
    ap.add_argument("--verbose", "-v", type=int, nargs="?", const=8, default=0, help="list up to N findings per page")
    ap.add_argument("--report-only", action="store_true")
    args = ap.parse_args()
    if args.at and not args.sweep:
        ap.error("--at needs --sweep")
    sys.exit(asyncio.run(run(args)))


if __name__ == "__main__":
    main()
