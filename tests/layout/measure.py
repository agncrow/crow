#!/usr/bin/env python3
"""Measure the conformance fixtures in a real browser.

Serves tests/layout/conformance.crow with `crow run`, opens each page at
each viewport in Chrome through Playwright, and writes one CSV per page
and width to tests/layout/expected/: selector, index, x, y, width, height
of every element the engine is expected to place. `crow test
tests/layout/conformance.crow` then holds the engine to these numbers with
no browser in the loop.

Needs: a crow binary on PATH (or CROW=...), Python with playwright, and a
Chrome/Chromium the launcher can find (CHROME=... to point at one).
Re-run after any change to the kernel stylesheet or the engine.
"""
import csv, os, subprocess, sys, time, urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CROW = os.environ.get("CROW", str(ROOT / "target" / "release" / "crow"))
CHROME = os.environ.get("CHROME", "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")
PAGES = {
    "dashboard": ("/", "h1,p,.card,.card h3,.card p,.row,.row button,.row .crow-field,.overlay,.scroll"),
    "prose": ("/prose", "h1,h2,h3,p,.row,.row button"),
    "primitives": ("/primitives", "nav,nav a,.crow-tabs,.crow-tabs label,.row,svg.icon,.crow-field,.crow-field input,.crow-field textarea,.crow-check,.crow-radio,.crow-radio label,progress,hr.divider,details,summary,details p.body,.dropdown,.dropdown summary,.row button,p.end"),
}
WIDTHS = (1280, 390)


def main():
    from playwright.sync_api import sync_playwright
    proc = subprocess.Popen([CROW, "run", "tests/layout/conformance.crow"], cwd=ROOT,
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
    try:
        for _ in range(600):
            try:
                urllib.request.urlopen("http://localhost:3000/", timeout=1)
                break
            except Exception:
                time.sleep(1)
        else:
            sys.exit("the fixture server did not come up on :3000")
        with sync_playwright() as p:
            launch = {"executable_path": CHROME} if os.path.exists(CHROME) else {}
            browser = p.chromium.launch(**launch)
            for name, (path, selectors) in PAGES.items():
                for width in WIDTHS:
                    page = browser.new_page(viewport={"width": width, "height": 1000})
                    page.goto(f"http://localhost:3000{path}", wait_until="networkidle")
                    rows = page.evaluate("""(sels) => {
                        const out = [];
                        for (const sel of sels.split(',')) {
                            const els = document.querySelectorAll(sel);
                            els.forEach((e, i) => {
                                const r = e.getBoundingClientRect();
                                out.push([sel, i, r.x, r.y + window.scrollY, r.width, r.height]);
                            });
                        }
                        return out;
                    }""", selectors)
                    target = ROOT / "tests" / "layout" / "expected" / f"{name}_{width}.csv"
                    with open(target, "w", newline="") as f:
                        w = csv.writer(f)
                        w.writerow(["sel", "i", "x", "y", "w", "h"])
                        for sel, i, x, y, wd, h in rows:
                            w.writerow([sel, i, f"{x:.1f}", f"{y:.1f}", f"{wd:.1f}", f"{h:.1f}"])
                    print(f"{target.relative_to(ROOT)}: {len(rows)} boxes")
                    page.close()
            browser.close()
    finally:
        import signal
        os.killpg(os.getpgid(proc.pid), signal.SIGTERM)


if __name__ == "__main__":
    main()
