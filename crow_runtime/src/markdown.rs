//! Markdown for documentation — the kernel behind `use doc`.
//!
//! Chapters in, HTML out, one implementation for both renderers: the
//! interpreter calls these functions directly and compiled programs link
//! them, so a page built by `crow run -i` is byte-identical to one built
//! by the compiled binary. Crow code blocks are highlighted here with the
//! language's own token rules, not a generic grammar.

use pulldown_cmark::{html, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::frame::{Column, CrowFrame};

/// One entry of a book's table of contents (`SUMMARY.md`).
#[derive(Debug, Clone, PartialEq)]
pub struct Chapter {
    pub title: String,
    pub path: String,
    /// 0 for prefix chapters, 1 for top-level list items, 2 for nested ...
    pub depth: i64,
    /// The `# Part` heading the chapter sits under ('' before the first one).
    pub part: String,
}

/// Parse an mdBook-style summary: `# Part` headings, `[Title](path)` prefix
/// chapters, and `- [Title](path)` list items nested by indentation.
pub fn chapters(summary: &str) -> Vec<Chapter> {
    let mut part = String::new();
    let mut out = Vec::new();
    for line in summary.lines() {
        let t = line.trim_end();
        let trimmed = t.trim_start();
        if let Some(h) = trimmed.strip_prefix("# ") {
            part = h.trim().to_string();
            continue;
        }
        let indent = t.len() - trimmed.len();
        let (is_item, rest) = match trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")) {
            Some(r) => (true, r.trim_start()),
            None => (false, trimmed),
        };
        if let Some((title, path)) = parse_link(rest) {
            if path.is_empty() {
                continue;
            }
            let depth = if is_item { (indent / 2) as i64 + 1 } else { 0 };
            out.push(Chapter { title, path, depth, part: part.clone() });
        }
    }
    out
}

fn parse_link(s: &str) -> Option<(String, String)> {
    let s = s.strip_prefix('[')?;
    let close = s.find("](")?;
    let title = s[..close].to_string();
    let rest = &s[close + 2..];
    let end = rest.find(')')?;
    Some((title, rest[..end].to_string()))
}

/// The table of contents as a frame: columns title, path, depth, part.
pub fn chapters_frame(summary: &str) -> CrowFrame {
    let cs = chapters(summary);
    let n = cs.len();
    CrowFrame::new(
        vec!["title".into(), "path".into(), "depth".into(), "part".into()],
        vec![
            Column::Str(cs.iter().map(|c| c.title.clone()).collect()),
            Column::Str(cs.iter().map(|c| c.path.clone()).collect()),
            Column::Int(cs.iter().map(|c| c.depth).collect()),
            Column::Str(cs.iter().map(|c| c.part.clone()).collect()),
        ],
        vec![vec![true; n]; 4],
    )
}

/// Read chapters back out of a frame shaped like `chapters_frame` makes.
pub fn chapters_from_frame(f: &CrowFrame) -> Vec<Chapter> {
    let col = |name: &str| f.columns.iter().position(|c| c == name).map(|i| f.data[i].clone());
    let strs = |c: Option<std::sync::Arc<Column>>| -> Vec<String> {
        match c.as_deref() {
            Some(Column::Str(v)) => v.to_vec(),
            Some(other) => (0..other.len()).map(|_| String::new()).collect(),
            None => Vec::new(),
        }
    };
    let titles = strs(col("title"));
    let paths = strs(col("path"));
    let parts = strs(col("part"));
    let depths: Vec<i64> = match col("depth").as_deref() {
        Some(Column::Int(v)) => v.clone(),
        Some(Column::Float(v)) => v.iter().map(|x| *x as i64).collect(),
        _ => vec![1; titles.len()],
    };
    (0..titles.len())
        .map(|i| Chapter {
            title: titles[i].clone(),
            path: paths.get(i).cloned().unwrap_or_default(),
            depth: depths.get(i).copied().unwrap_or(1),
            part: parts.get(i).cloned().unwrap_or_default(),
        })
        .collect()
}

/// `.md` → `.html`, keeping any `#fragment`.
pub fn html_path(path: &str) -> String {
    let (p, frag) = match path.find('#') {
        Some(i) => (&path[..i], &path[i..]),
        None => (path, ""),
    };
    if let Some(stem) = p.strip_suffix(".md") {
        format!("{}.html{}", stem, frag)
    } else {
        path.to_string()
    }
}

/// The prefix that takes a page at `current` back to the site root.
fn root_prefix(current: &str) -> String {
    "../".repeat(current.matches('/').count())
}

/// The table of contents as navigation markup: the first prefix chapter is
/// the brand link, parts are headings, a chapter with sub-chapters is a
/// collapsible group (`details`, open when the current page is inside it),
/// and the current page is marked. Links are relative to the current page
/// so the site works from a folder as well as a server.
pub fn menu(chapters: &[Chapter], current: &str) -> String {
    let up = root_prefix(current);
    let link = |c: &Chapter| {
        let cls = if c.path == current { format!("d{} current", c.depth.max(1)) } else { format!("d{}", c.depth.max(1)) };
        format!("<a class=\"{}\" href=\"{}{}\">{}</a>", cls, up, escape(&html_path(&c.path)), escape(&c.title))
    };
    let mut out = String::from("<nav class=\"doc-nav\">");
    // the brand: the summary's title (its first heading) linking to the
    // first page; a summary with no title heading uses that page's title
    let brand = chapters.iter().find(|c| c.depth == 0).or(chapters.first());
    let brand_text: Option<String> = brand.map(|bc| if bc.part.is_empty() { bc.title.clone() } else { bc.part.clone() });
    if let (Some(bc), Some(text)) = (brand, &brand_text) {
        out.push_str(&format!("<a class=\"brand\" href=\"{}{}\">{}</a>", up, escape(&html_path(&bc.path)), escape(text)));
    }
    let mut part = String::new();
    let mut open: Vec<i64> = Vec::new();
    for (i, c) in chapters.iter().enumerate() {
        if Some(c) == brand && c.depth == 0 {
            continue;
        }
        // leave every group this chapter is not inside
        while open.last().map_or(false, |&d| d >= c.depth) {
            out.push_str("</details>");
            open.pop();
        }
        if c.part != part && !c.part.is_empty() && brand_text.as_deref() != Some(c.part.as_str()) {
            while open.pop().is_some() {
                out.push_str("</details>");
            }
            part = c.part.clone();
            out.push_str(&format!("<p class=\"part\">{}</p>", escape(&part)));
        }
        let has_children = c.depth >= 1 && chapters.get(i + 1).map_or(false, |n| n.depth > c.depth);
        if has_children {
            let inside = chapters[i + 1..].iter().take_while(|n| n.depth > c.depth).any(|n| n.path == current) || c.path == current;
            out.push_str(&format!("<details class=\"group\"{}><summary>{}</summary>", if inside { " open=\"open\"" } else { "" }, link(c)));
            open.push(c.depth);
        } else {
            out.push_str(&link(c));
        }
    }
    while open.pop().is_some() {
        out.push_str("</details>");
    }
    out.push_str("</nav>");
    out
}

/// `menu` over a frame from `chapters_frame`.
pub fn menu_from_frame(f: &CrowFrame, current: &str) -> String {
    menu(&chapters_from_frame(f), current)
}

/// Previous and next chapter links for the page at `current`, in summary
/// order, relative to the current page.
pub fn pager(chapters: &[Chapter], current: &str) -> String {
    let up = root_prefix(current);
    let i = match chapters.iter().position(|c| c.path == current) {
        Some(i) => i,
        None => return String::new(),
    };
    let mut out = String::from("<nav class=\"doc-pager\">");
    if i > 0 {
        let c = &chapters[i - 1];
        out.push_str(&format!("<a class=\"prev\" href=\"{}{}\" rel=\"prev\"><span>Previous</span>{}</a>", up, escape(&html_path(&c.path)), escape(&c.title)));
    } else {
        out.push_str("<span></span>");
    }
    if i + 1 < chapters.len() {
        let c = &chapters[i + 1];
        out.push_str(&format!("<a class=\"next\" href=\"{}{}\" rel=\"next\"><span>Next</span>{}</a>", up, escape(&html_path(&c.path)), escape(&c.title)));
    }
    out.push_str("</nav>");
    out
}

/// `pager` over a frame from `chapters_frame`.
pub fn pager_from_frame(f: &CrowFrame, current: &str) -> String {
    pager(&chapters_from_frame(f), current)
}

/// The words of a chapter, markup removed and whitespace collapsed — what
/// a search index holds.
pub fn plain_text(md: &str) -> String {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    let mut out = String::new();
    for ev in Parser::new_ext(md, opts) {
        match ev {
            Event::Text(t) | Event::Code(t) => {
                out.push_str(&t);
                out.push(' ');
            }
            Event::SoftBreak | Event::HardBreak => out.push(' '),
            Event::End(_) => out.push(' '),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Markdown to HTML. Headings get anchor ids, links to `.md` files point at
/// their `.html` pages, and fenced `crow` blocks are highlighted with Crow's
/// own token rules and tagged with their mode (`crow,error` → error) so a
/// page runner can find them.
pub fn to_html(md: &str) -> String {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_FOOTNOTES);
    opts.insert(Options::ENABLE_TASKLISTS);
    let parser = Parser::new_ext(md, opts);
    let mut out = String::new();
    let mut buf: Vec<Event> = Vec::new();
    let mut ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut events = parser.into_iter().peekable();
    while let Some(ev) = events.next() {
        match ev {
            Event::Start(Tag::CodeBlock(kind)) => {
                html::push_html(&mut out, buf.drain(..));
                let info = match &kind {
                    CodeBlockKind::Fenced(s) => s.to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                let mut code = String::new();
                for inner in events.by_ref() {
                    match inner {
                        Event::End(TagEnd::CodeBlock) => break,
                        Event::Text(t) => code.push_str(&t),
                        _ => {}
                    }
                }
                // A ```text block directly after a runnable crow block is its
                // expected output: the runner shows it and compares against it.
                let mut expected: Option<String> = None;
                if is_runnable(&info) {
                    if let Some(Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(next_info)))) = events.peek() {
                        let ni = next_info.trim().to_string();
                        if ni == "text" || ni.is_empty() {
                            events.next();
                            let mut exp = String::new();
                            for inner in events.by_ref() {
                                match inner {
                                    Event::End(TagEnd::CodeBlock) => break,
                                    Event::Text(t) => exp.push_str(&t),
                                    _ => {}
                                }
                            }
                            expected = Some(exp);
                        }
                    }
                }
                out.push_str(&code_block(&info, &code, expected.as_deref()));
            }
            Event::Start(Tag::Heading { level, .. }) => {
                html::push_html(&mut out, buf.drain(..));
                let mut inner: Vec<Event> = Vec::new();
                let mut text = String::new();
                for e in events.by_ref() {
                    match e {
                        Event::End(TagEnd::Heading(_)) => break,
                        Event::Text(ref t) | Event::Code(ref t) => {
                            text.push_str(t);
                            inner.push(e);
                        }
                        other => inner.push(other),
                    }
                }
                let mut id = slug(&text);
                let base = id.clone();
                let mut n = 1;
                while !ids.insert(id.clone()) {
                    id = format!("{}-{}", base, n);
                    n += 1;
                }
                let tag = heading_tag(level);
                out.push_str(&format!("<{} id=\"{}\"><a class=\"anchor\" href=\"#{}\">", tag, id, id));
                html::push_html(&mut out, inner.into_iter());
                out.push_str(&format!("</a></{}>\n", tag));
            }
            Event::Start(Tag::Link { link_type, dest_url, title, id }) => {
                let dest = if dest_url.starts_with("http://") || dest_url.starts_with("https://") {
                    dest_url.to_string()
                } else {
                    html_path(&dest_url)
                };
                buf.push(Event::Start(Tag::Link { link_type, dest_url: dest.into(), title, id }));
            }
            other => buf.push(other),
        }
    }
    html::push_html(&mut out, buf.drain(..));
    // markup, not text: a view passes a string that starts and ends with
    // a tag straight through, so the page ends on its last tag
    out.trim_end().to_string()
}

fn heading_tag(level: HeadingLevel) -> &'static str {
    match level {
        HeadingLevel::H1 => "h1",
        HeadingLevel::H2 => "h2",
        HeadingLevel::H3 => "h3",
        HeadingLevel::H4 => "h4",
        HeadingLevel::H5 => "h5",
        HeadingLevel::H6 => "h6",
    }
}

/// A heading's anchor: lowercase words joined by dashes.
pub fn slug(text: &str) -> String {
    let mut s = String::new();
    let mut dash = false;
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            s.extend(ch.to_lowercase());
            dash = false;
        } else if !dash && !s.is_empty() {
            s.push('-');
            dash = true;
        }
    }
    s.trim_end_matches('-').to_string()
}

fn split_info(info: &str) -> (&str, &str) {
    let info = info.trim();
    match info.find(',') {
        Some(i) => (&info[..i], info[i + 1..].trim()),
        None => (info, ""),
    }
}

/// A crow block the reader can run: plain or `norun` (serves). `error` and
/// `ignore` blocks are shown, not run.
fn is_runnable(info: &str) -> bool {
    let (lang, flags) = split_info(info);
    lang == "crow" && (flags.is_empty() || flags == "norun")
}

fn code_block(info: &str, code: &str, expected: Option<&str>) -> String {
    let (lang, flags) = split_info(info);
    match lang {
        "crow" if is_runnable(info) => runner(flags, code, expected),
        "crow" => {
            let mode = if flags.is_empty() { "run" } else { flags };
            let cls = if flags.is_empty() { "crow".to_string() } else { format!("crow crow-{}", flags.replace(',', " crow-")) };
            format!(
                "<pre class=\"{}\" data-mode=\"{}\"><code>{}</code></pre>\n",
                cls,
                escape(mode),
                highlight(code)
            )
        }
        "text" | "" => format!("<pre class=\"output\"><code>{}</code></pre>\n", escape(code)),
        other => format!("<pre class=\"lang-{}\"><code>{}</code></pre>\n", escape(other), escape(code)),
    }
}

/// The in-page runner as markup: the block is the editor (with a line
/// gutter), a toolbar row below it, then the output panel and the book's
/// expected output. All of it is laid out by the kernel's own containers,
/// so `box[]` and `fits[]` see it; the script only carries text to the
/// `crow book` server and back.
fn runner(flags: &str, code: &str, expected: Option<&str>) -> String {
    let code = code.trim_end_matches('\n');
    let serves = flags == "norun" || code.lines().any(|l| l.trim_start().starts_with("serve["));
    let has_tests = code.lines().any(|l| l.starts_with("test ") || l.starts_with("test["));
    let n = code.lines().count().max(1);
    let gutter: Vec<String> = (1..=n).map(|i| i.to_string()).collect();
    let mode = if serves { "norun" } else { "run" };
    let outer = crate::layout::render("stack", &[("gap".into(), "s".into())]);
    let editor = crate::layout::render("row", &[("align".into(), "stretch".into())]);
    let toolbar = crate::layout::render("row", &[("gap".into(), "s".into()), ("wrap".into(), "true".into())]);
    let mut out = String::new();
    out.push_str(&format!("<div class=\"{} crow-run\" data-mode=\"{}\" style=\"{}\">", outer.class, mode, outer.style));
    out.push_str(&format!("<div class=\"{} crow-editor\" style=\"{}\">", editor.class, editor.style));
    out.push_str(&format!("<pre class=\"gutter\">{}</pre>", gutter.join("\n")));
    // the block fills the row and scrolls inside it: a long line never
    // widens the page, and the engine models the same rule from the attribute
    let block = crate::layout::render("pre", &[("fill".into(), "true".into())]);
    out.push_str(&format!(
        "<pre class=\"crow\" data-mode=\"{}\" style=\"{}\"><code contenteditable=\"plaintext-only\" spellcheck=\"false\" title=\"editable — Tab indents, Ctrl/Cmd-Enter runs\">{}</code></pre>",
        mode,
        block.style,
        highlight(code)
    ));
    out.push_str("</div>");
    out.push_str(&format!("<div class=\"{} crow-toolbar\" style=\"{}\">", toolbar.class, toolbar.style));
    if serves {
        out.push_str("<button data-action=\"compile\" title=\"crow run — compile, then serve; stop it with ■\">▶ Serve</button>");
    } else {
        out.push_str("<button data-action=\"interp\" title=\"crow run -i — interpret instantly\">▶ Run</button>");
        out.push_str("<button data-action=\"compile\" title=\"crow run — transpile to Rust and run the native binary\">⚙ Compile</button>");
    }
    if has_tests || !serves {
        out.push_str("<button data-action=\"test\" title=\"crow test — run this block's test blocks\">✓ Test</button>");
    }
    out.push_str("<button data-action=\"reset\" title=\"restore the book's text\">↺ Reset</button>");
    out.push_str("<button data-action=\"stop\" class=\"stop\">■ Stop</button>");
    out.push_str("</div>");
    out.push_str("<p class=\"crow-caption\" hidden=\"hidden\"></p><pre class=\"output crow-result\" hidden=\"hidden\"></pre>");
    if let Some(exp) = expected {
        out.push_str("<p class=\"crow-caption expected\">expected output — from the book, verified by crow test --book</p>");
        out.push_str(&format!("<pre class=\"output expected\"><code>{}</code></pre>", escape(exp.trim_end_matches('\n'))));
    }
    out.push_str("</div>\n");
    out
}

/// The sidebar's behaviour: the toggle button opens or closes it (a
/// collapse on wide screens, an overlay on narrow ones), the choice is
/// remembered, and the current chapter is scrolled into view.
pub const NAV_JS: &str = r#"(function(){
var b=document.body,key='crow-doc-nav';
try{if(localStorage.getItem(key)==='closed')b.classList.add('nav-closed');}catch(e){}
document.querySelectorAll('.nav-toggle').forEach(function(t){t.onclick=function(){
  if(window.innerWidth<1024){b.classList.toggle('nav-open');}
  else{b.classList.toggle('nav-closed');try{localStorage.setItem(key,b.classList.contains('nav-closed')?'closed':'open');}catch(e){}}
};});
document.addEventListener('click',function(e){if(b.classList.contains('nav-open')&&!e.target.closest('.doc-side')&&!e.target.closest('.nav-toggle'))b.classList.remove('nav-open');});
var cur=document.querySelector('.doc-nav .current');if(cur&&cur.scrollIntoView)cur.scrollIntoView({block:'center'});
var box=document.querySelector('.doc-search'),res=document.querySelector('.doc-results');
if(box&&res){
  var title=document.querySelector('.doc-top .title'),up=title?title.getAttribute('href').replace('index.html',''):'',index=null;
  function load(cb){if(index)return cb();fetch(up+'search.json').then(function(r){return r.json();}).then(function(j){index=j;cb();}).catch(function(){index=[];cb();});}
  function esc(t){return t.replace(/[&<>]/g,function(c){return {'&':'&amp;','<':'&lt;','>':'&gt;'}[c];});}
  function search(q){
    var words=q.toLowerCase().split(/\s+/).filter(Boolean);if(!words.length){res.hidden=true;res.innerHTML='';return;}
    var hits=[];
    index.forEach(function(e){var t=e.t.toLowerCase(),x=e.x.toLowerCase(),score=0,pos=-1;
      words.forEach(function(w){if(t.indexOf(w)>=0)score+=10;var i=x.indexOf(w);if(i>=0){score+=1;if(pos<0)pos=i;}else if(t.indexOf(w)<0)score-=100;});
      if(score>0)hits.push({e:e,score:score,pos:pos});});
    hits.sort(function(a,b){return b.score-a.score;});
    hits=hits.slice(0,8);
    if(!hits.length){res.innerHTML='<p class="none">No matches</p>';res.hidden=false;return;}
    res.innerHTML=hits.map(function(h){var x=h.e.x,p=h.pos<0?0:Math.max(0,h.pos-60),snip=(p>0?'… ':'')+x.slice(p,p+140)+(p+140<x.length?' …':'');
      return '<a href="'+up+h.e.p+'"><b>'+esc(h.e.t)+'</b><span>'+esc(snip)+'</span></a>';}).join('');
    res.hidden=false;
  }
  box.addEventListener('input',function(){load(function(){search(box.value);});});
  box.addEventListener('focus',function(){load(function(){});});
  box.addEventListener('keydown',function(e){if(e.key==='Escape'){box.value='';res.hidden=true;box.blur();}if(e.key==='Enter'){var a=res.querySelector('a');if(a)a.click();}});
  document.addEventListener('keydown',function(e){if(e.key==='/'&&document.activeElement!==box&&!(document.activeElement&&document.activeElement.isContentEditable)){e.preventDefault();box.focus();}});
  document.addEventListener('click',function(e){if(!e.target.closest('.doc-results')&&e.target!==box)res.hidden=true;});
}
})();"#;

/// Transport only: the markup above is the whole runner; this carries a
/// block's text to `crow book` (POST /run, /stop) and puts the reply in
/// the output panel. Without the server the panel says how to start one.
pub const RUN_JS: &str = r#"(function(){
var running={};
function setup(run){
  var code=run.querySelector('code'),gutter=run.querySelector('.gutter'),cap=run.querySelector('.crow-caption'),out=run.querySelector('.crow-result');
  var expCap=run.querySelector('.crow-caption.expected'),expPre=run.querySelector('pre.expected');
  var original=code.textContent,serves=run.getAttribute('data-mode')==='norun';
  var expected=expPre?expPre.textContent.replace(/\s+$/,''):null;
  var id=Math.random().toString(36).slice(2);
  function renumber(){var n=code.textContent.replace(/\n$/,'').split('\n').length,a=[];for(var i=1;i<=n;i++)a.push(i);gutter.textContent=a.join('\n');}
  function edited(){return code.textContent!==original;}
  function show(text,kind,note){out.hidden=false;cap.hidden=false;out.className='output crow-result '+(kind||'');out.textContent=text;cap.textContent=note||'';if(expPre){expPre.hidden=true;expCap.hidden=true;}}
  function go(mode){
    var src=code.textContent;serves=serves||/^\s*serve\[/m.test(src);
    if(serves){var m=src.match(/serve\[\s*(\d+)/);show('crow run … compiling, then serving. When it is up, open http://localhost:'+(m?m[1]:'3000')+'/ in a new tab. Press ■ Stop to end it; its output appears here afterwards.','busy');}
    else show('crow '+(mode==='test'?'test':mode==='compile'?'run':'run -i')+' … ('+(mode==='compile'?'compiling to Rust, ~20s the first time':'starting')+')','busy');
    running[id]=true;
    fetch('/run',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({id:id,code:src,mode:mode})})
    .then(function(r){return r.json();}).then(function(res){
      delete running[id];var text=res.output||'';if(res.timeout)text+='\n[stopped: '+res.timeout+']';if(serves)text='server stopped\n'+text;
      var kind='',note='';
      if(expected!==null&&!serves&&!edited()&&mode!=='test'){var same=text.replace(/\s+$/,'')===expected;note=same?'output — matches the book ✓':'output — differs from the book (expected: '+expected.split('\n').join(' ⏎ ')+')';if(!same)kind='fail';}
      else note=mode==='test'?'crow test':serves?'server output':edited()?'output — of your edited version':'output';
      show(text.length?text:'(no output)',kind||(res.ok||serves?'ok':'fail'),note);
    }).catch(function(){delete running[id];show('No runner on this page. Start one from the repository:\n\n    crow book\n\nthen reload — every block gets a real ▶ Run button.','fail');});
  }
  function stop(){fetch('/stop',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({id:id})}).then(function(){show('stopped','fail');}).catch(function(){});}
  code.addEventListener('input',renumber);
  code.addEventListener('keydown',function(e){
    if(e.key==='Tab'){e.preventDefault();document.execCommand('insertText',false,'    ');}
    if(e.key==='Enter'&&(e.metaKey||e.ctrlKey)){e.preventDefault();go(serves?'compile':'interp');}
  });
  run.querySelectorAll('button[data-action]').forEach(function(b){
    var a=b.getAttribute('data-action');
    b.onclick=function(){if(a==='reset'){code.textContent=original;renumber();}else if(a==='stop')stop();else go(a);};
  });
}
document.querySelectorAll('.crow-run').forEach(setup);
})();"#;

const KEYWORDS: &[&str] = &[
    "fn", "view", "test", "use", "struct", "if", "else", "for", "in", "while", "return", "and", "or", "not",
    "eq", "ne", "gt", "lt", "gte", "lte", "true", "false", "then", "break", "continue", "as",
];

/// Crow source to highlighted HTML: `k` keywords, `s` strings, `v` the
/// interpolated spans inside them, `c` comments, `n` numbers, `f` a name
/// being called.
pub fn highlight(code: &str) -> String {
    let chars: Vec<char> = code.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        // --- comment to end of line
        if c == '-' && chars.get(i + 1) == Some(&'-') && chars.get(i + 2) == Some(&'-') {
            let start = i;
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            out.push_str(&format!("<span class=\"c\">{}</span>", escape(&chars[start..i].iter().collect::<String>())));
            continue;
        }
        if c == '\'' {
            let start = i;
            i += 1;
            while i < chars.len() && chars[i] != '\'' {
                if chars[i] == '\\' {
                    i += 1;
                }
                i += 1;
            }
            i = (i + 1).min(chars.len());
            let s: String = chars[start..i].iter().collect();
            out.push_str("<span class=\"s\">");
            // interpolation spans between backticks
            let mut rest = s.as_str();
            while let Some(a) = rest.find('`') {
                if let Some(b) = rest[a + 1..].find('`') {
                    out.push_str(&escape(&rest[..a]));
                    out.push_str(&format!("<span class=\"v\">{}</span>", escape(&rest[a..a + b + 2])));
                    rest = &rest[a + b + 2..];
                } else {
                    break;
                }
            }
            out.push_str(&escape(rest));
            out.push_str("</span>");
            continue;
        }
        if c.is_ascii_digit() {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '.' || chars[i] == '_') {
                i += 1;
            }
            out.push_str(&format!("<span class=\"n\">{}</span>", escape(&chars[start..i].iter().collect::<String>())));
            continue;
        }
        if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let mut j = i;
            while j < chars.len() && chars[j] == ' ' {
                j += 1;
            }
            let called = chars.get(j) == Some(&'[') && j == i;
            if KEYWORDS.contains(&word.as_str()) {
                out.push_str(&format!("<span class=\"k\">{}</span>", word));
            } else if called {
                out.push_str(&format!("<span class=\"f\">{}</span>", escape(&word)));
            } else {
                out.push_str(&escape(&word));
            }
            continue;
        }
        out.push_str(&escape(&c.to_string()));
        i += 1;
    }
    out
}

pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

/// A complete page: the kernel stylesheet (tokens, elements, responsive
/// rules) plus the documentation rules, then the body as given.
pub fn document(title: &str, body: &str) -> String {
    format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>{}</title>\n<style>{}\n{}\n{}</style>\n</head>\n<body>\n{}\n<script>{}</script>\n</body>\n</html>\n",
        escape(title),
        crate::layout::DEFAULT_CSS,
        crate::layout::RESPONSIVE_CSS,
        DOC_CSS,
        body,
        format!("{}\n{}", NAV_JS, RUN_JS)
    )
}

/// Documentation rules on top of the kernel stylesheet: a `.doc-nav`
/// menu, `.doc` prose, code blocks, and the highlight classes.
pub const DOC_CSS: &str = "\
body{max-width:none;padding:0;background:var(--crow-bg)}\
.doc-top{position:sticky;top:0;z-index:5;height:52px;padding:0 1rem;background:var(--crow-surface);border-bottom:1px solid var(--crow-border)}\
.doc-top .title{font-weight:650;letter-spacing:-0.01em;color:var(--crow-text);text-decoration:none}\
.doc-top .links a{color:var(--crow-muted);text-decoration:none;font-size:0.9rem}\
.doc-top .links a:hover{color:var(--crow-accent)}\
.doc-search{width:220px;font-size:0.88rem;padding:0.35rem 0.6rem}\
.doc-results{position:fixed;top:52px;right:1rem;width:min(520px,92vw);max-height:70vh;overflow:auto;z-index:7;background:var(--crow-surface);border:1px solid var(--crow-border);border-radius:6px;box-shadow:0 12px 40px rgba(0,0,0,0.35);padding:0.4rem}\
.doc-results a{display:block;padding:0.5rem 0.7rem;border-radius:4px;text-decoration:none;color:var(--crow-text)}\
.doc-results a:hover{background:color-mix(in srgb,var(--crow-accent) 12%,transparent)}\
.doc-results a b{display:block;font-weight:600;color:var(--crow-accent);margin-bottom:0.15rem}\
.doc-results a span{display:block;font-size:0.84rem;color:var(--crow-muted);line-height:1.4}\
.doc-results .none{margin:0.5rem 0.7rem;color:var(--crow-muted)}\
.doc-pager{display:flex;justify-content:space-between;gap:1rem;margin-top:2.5rem;padding-top:1.2rem;border-top:1px solid var(--crow-border)}\
.doc-pager a{flex:1 1 0;max-width:48%;text-decoration:none;color:var(--crow-text);border:1px solid var(--crow-border);border-radius:6px;padding:0.7rem 0.9rem;background:var(--crow-surface)}\
.doc-pager a:hover{border-color:var(--crow-accent)}\
.doc-pager a span{display:block;font-size:0.72rem;text-transform:uppercase;letter-spacing:0.08em;color:var(--crow-muted);margin-bottom:0.2rem}\
.doc-pager a.next{text-align:right;margin-left:auto}\
.nav-toggle{width:34px;height:34px;padding:0;font-size:1.1rem;line-height:1;background:transparent;border-color:transparent}\
.nav-toggle:hover{background:var(--crow-bg);border-color:var(--crow-border)}\
.doc-side{position:sticky;top:52px;height:calc(100vh - 52px);overflow:auto;background:var(--crow-surface);border-right:1px solid var(--crow-border);padding:1rem 0.75rem 2rem}\
body.nav-closed .doc-side{display:none!important}\
@media (max-width:1023px){body.nav-open .doc-side[data-hidden-below]{display:flex!important;position:fixed;left:0;top:52px;width:280px;max-width:85vw;z-index:6;box-shadow:0 12px 40px rgba(0,0,0,0.35)}}\
.doc-nav{font-size:0.9rem;line-height:1.45}\
.doc-nav .brand{display:block;font-weight:650;color:var(--crow-text);text-decoration:none;padding:0.3rem 0.6rem 0.9rem;letter-spacing:-0.01em}\
.doc-nav .part{margin:1.2rem 0 0.35rem;padding:0 0.6rem;font-size:0.68rem;font-weight:600;text-transform:uppercase;letter-spacing:0.09em;color:var(--crow-muted)}\
.doc-nav a{display:block;color:var(--crow-text);text-decoration:none;padding:0.3rem 0.6rem;border-radius:4px;border-left:2px solid transparent}\
.doc-nav a:hover{background:color-mix(in srgb,var(--crow-text) 6%,transparent)}\
.doc-nav a.current{color:var(--crow-accent);font-weight:600;background:color-mix(in srgb,var(--crow-accent) 12%,transparent);border-left-color:var(--crow-accent);border-radius:0 4px 4px 0}\
.doc-nav a.d2{padding-left:1.6rem;font-size:0.86rem;color:var(--crow-muted)}\
.doc-nav a.d2:hover,.doc-nav a.d2.current{color:var(--crow-accent)}\
.doc-nav a.d2:hover{color:var(--crow-text)}\
.doc-nav a.d3{padding-left:2.4rem;font-size:0.84rem;color:var(--crow-muted)}\
.doc-nav details>summary{list-style:none;display:flex;align-items:center;cursor:pointer;border-radius:4px}\
.doc-nav details>summary::-webkit-details-marker{display:none}\
.doc-nav details>summary>a{flex:1 1 auto}\
.doc-nav details>summary::after{content:'';flex:none;width:0.45rem;height:0.45rem;margin:0 0.7rem 0 0.4rem;border-right:1.5px solid var(--crow-muted);border-bottom:1.5px solid var(--crow-muted);transform:rotate(-45deg);transition:transform 120ms ease}\
.doc-nav details[open]>summary::after{transform:rotate(45deg)}\
.doc{min-width:0;max-width:880px;font-size:1rem;line-height:1.65}\
.doc h1{font-size:1.9rem;margin:0 0 1rem}\
.doc h2{font-size:1.35rem;font-weight:650;text-transform:none;letter-spacing:-0.01em;color:var(--crow-text);margin:2rem 0 0.6rem;padding-top:0.4rem;border-top:1px solid var(--crow-border)}\
.doc h3{font-size:1.1rem;margin:1.5rem 0 0.4rem}\
.doc h1 a.anchor,.doc h2 a.anchor,.doc h3 a.anchor,.doc h4 a.anchor{color:inherit;text-decoration:none}\
.doc p{margin:0.7rem 0}\
.doc pre{background:var(--crow-surface);border:1px solid var(--crow-border);border-radius:6px;padding:0.8rem 1rem;overflow-x:auto;margin:0.8rem 0;font-size:0.88rem;line-height:1.5}\
.doc pre.output{border-left:3px solid var(--crow-muted);color:var(--crow-muted)}\
.doc pre.crow-error{border-left:3px solid var(--crow-down)}\
.doc :not(pre)>code{background:var(--crow-surface);border:1px solid var(--crow-border);border-radius:3px;padding:0.05em 0.3em;font-size:0.88em}\
.doc blockquote{margin:1rem 0;padding:0.2rem 1rem;border-left:3px solid var(--crow-accent);color:var(--crow-muted)}\
.doc table{margin:0.8rem 0;display:block;overflow-x:auto}\
.doc th{text-transform:none;letter-spacing:0;font-size:0.9rem;color:var(--crow-text)}\
.doc td{font-variant-numeric:normal;vertical-align:top}\
.doc ul,.doc ol{padding-left:1.4rem}\
.doc li{margin:0.25rem 0}\
.doc img{max-width:100%}\
.doc hr{border:0;border-top:1px solid var(--crow-border);margin:2rem 0}\
.crow-run{margin:0.8rem 0}\
.crow-run .crow-editor pre{margin:0}\
.crow-run pre.gutter{flex:none;text-align:right;user-select:none;color:var(--crow-muted);padding-right:0.7rem;border-right:0;border-radius:6px 0 0 6px}\
.crow-run pre.co{flex:1 1 0;min-width:0;border-left:0;border-radius:0 6px 6px 0}\
.crow-run code[contenteditable]{display:block;outline:none;cursor:text;min-height:1.4em}\
.crow-run .crow-toolbar button{font-size:0.85rem;padding:0.2rem 0.7rem}\
.crow-run .crow-toolbar button.stop{margin-left:auto}\
.crow-run .crow-caption{margin:0;font-size:0.78rem;color:var(--crow-muted)}\
.crow-run pre.output{margin:0;white-space:pre-wrap}\
.crow-run pre.crow-result.ok{border-left-color:var(--crow-up)}\
.crow-run pre.crow-result.fail{border-left-color:var(--crow-down)}\
.crow-run pre.crow-result.busy{border-left-color:var(--crow-accent);opacity:0.8}\
.k{color:var(--crow-accent)}.s{color:var(--crow-up)}.c{color:var(--crow-muted);font-style:italic}.n{color:var(--crow-down)}.v{font-weight:600}\
.f{color:var(--crow-text)}";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_parses_parts_prefix_and_nesting() {
        let s = "# Book\n\n[Intro](intro.md)\n\n# Part I\n\n- [One](one.md)\n  - [One A](one-a.md)\n- [Two](two.md)\n";
        let cs = chapters(s);
        assert_eq!(cs.len(), 4);
        assert_eq!((cs[0].depth, cs[0].part.as_str()), (0, "Book"));
        assert_eq!((cs[2].title.as_str(), cs[2].depth, cs[2].part.as_str()), ("One A", 2, "Part I"));
        let f = chapters_frame(s);
        assert_eq!(chapters_from_frame(&f), cs);
    }

    #[test]
    fn menu_marks_current_and_rewrites_paths() {
        let cs = chapters("- [One](one.md)\n- [Two](sub/two.md)\n");
        let m = menu(&cs, "sub/two.md");
        assert!(m.contains("href=\"../one.html\""));
        assert!(m.contains("class=\"d1 current\" href=\"../sub/two.html\""));
    }

    #[test]
    fn pager_links_neighbours_and_plain_text_strips_markup() {
        let cs = chapters("- [One](one.md)\n- [Two](sub/two.md)\n- [Three](three.md)\n");
        let p = pager(&cs, "sub/two.md");
        assert!(p.contains("<a class=\"prev\" href=\"../one.html\" rel=\"prev\"><span>Previous</span>One</a>"));
        assert!(p.contains("<a class=\"next\" href=\"../three.html\" rel=\"next\"><span>Next</span>Three</a>"));
        assert!(pager(&cs, "one.md").starts_with("<nav class=\"doc-pager\"><span></span><a class=\"next\""));
        assert!(!pager(&cs, "three.md").contains("class=\"next\""));
        assert_eq!(plain_text("# Hi *there*\n\nSay `log[1]`.\n\n```crow\nx = 1\n```\n"), "Hi there Say log[1] . x = 1");
    }

    #[test]
    fn menu_groups_sub_chapters_and_opens_the_current_group() {
        let s = "# Book\n\n[Book](title.md)\n\n# Part I\n\n- [One](one.md)\n  - [One A](one-a.md)\n  - [One B](one-b.md)\n- [Two](two.md)\n  - [Two A](two-a.md)\n- [Three](three.md)\n";
        let m = menu(&chapters(s), "one-b.md");
        assert!(m.starts_with("<nav class=\"doc-nav\"><a class=\"brand\" href=\"title.html\">Book</a>"));
        assert!(!m.contains("<p class=\"part\">Book</p>"), "the brand is not a part heading");
        assert!(m.contains("<details class=\"group\" open=\"open\"><summary><a class=\"d1\" href=\"one.html\">One</a></summary><a class=\"d2\" href=\"one-a.html\">One A</a><a class=\"d2 current\" href=\"one-b.html\">One B</a></details>"));
        assert!(m.contains("<details class=\"group\"><summary><a class=\"d1\" href=\"two.html\">Two</a></summary><a class=\"d2\" href=\"two-a.html\">Two A</a></details><a class=\"d1\" href=\"three.html\">Three</a></nav>"));
    }

    #[test]
    fn crow_fences_are_highlighted_and_tagged() {
        let h = to_html("# Hi there\n\n```crow\nx = 'a `b` c'   --- note\nlog[x]\n```\n\n```text\nout\n```\n\nSee [next](next.md#top).");
        assert!(h.contains("<h1 id=\"hi-there\">"));
        assert!(h.contains("<pre class=\"crow\" data-mode=\"run\" style=\"flex:1 1 0;min-width:0\">"));
        assert!(h.contains("<span class=\"c\">--- note</span>"));
        assert!(h.contains("<span class=\"v\">`b`</span>"));
        assert!(h.contains("<span class=\"f\">log</span>["));
        assert!(h.contains("class=\"stack crow-run\" data-mode=\"run\""), "runnable blocks become the runner");
        assert!(h.contains("<pre class=\"gutter\">1\n2</pre>"));
        for (tag, attrs) in [("stack", vec![("gap", "s")]), ("row", vec![("align", "stretch")]), ("row", vec![("gap", "s"), ("wrap", "true")])] {
            let a: Vec<(String, String)> = attrs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
            assert!(crate::layout::render(tag, &a).errors.is_empty(), "runner attrs must be valid vocabulary");
        }
        assert!(h.contains("data-action=\"interp\"") && h.contains("data-action=\"test\""));
        assert!(h.contains("<pre class=\"output expected\"><code>out</code></pre>"), "the text block after is the expected output");
        assert!(!h.contains("<pre class=\"output\"><code>out"), "and is not rendered twice");
        assert!(h.contains("href=\"next.html#top\""));
        let s = to_html("```crow,norun\nserve[3000]\n```\n\nafter\n\n```text\nnot expected\n```");
        assert!(s.contains("data-mode=\"norun\"") && s.contains("▶ Serve") && !s.contains("⚙ Compile"));
        assert!(s.contains("<pre class=\"output\"><code>not expected"), "a text block elsewhere stays a plain block");
        let e = to_html("```crow,error\nbad[\n```");
        assert!(e.contains("class=\"crow crow-error\" data-mode=\"error\""));
    }

    #[test]
    fn document_carries_every_stylesheet() {
        let d = document("T", "<p>x</p>");
        assert!(d.contains("--crow-bg") && d.contains("data-below") && d.contains(".doc-nav"));
        assert!(d.contains("<title>T</title>"));
        assert!(d.contains("<script>(function(){") && d.contains("fetch('/run'"));
    }
}
