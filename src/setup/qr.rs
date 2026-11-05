//! QR codes as pages the owner opens and scans with their phone. A page reloads itself every
//! few seconds, because both providers replace their codes while waiting for a scan.

use std::fmt::Write as _;

use qrcode::{Color, QrCode};

/// Rows of modules; `true` is dark.
pub type Matrix = Vec<Vec<bool>>;

/// Encodes `text` as a QR code.
pub fn encode(text: &str) -> Option<Matrix> {
    let code = QrCode::new(text.as_bytes()).ok()?;
    let width = code.width();
    let colors = code.to_colors();
    Some(
        colors
            .chunks(width)
            .map(|row| row.iter().map(|color| *color == Color::Dark).collect())
            .collect(),
    )
}

/// Draws the modules as an SVG, with a light border four modules wide.
pub fn svg(matrix: &Matrix) -> String {
    const MODULE: usize = 8;
    const BORDER: usize = 4;
    let width = matrix.iter().map(Vec::len).max().unwrap_or(0) + 2 * BORDER;
    let height = matrix.len() + 2 * BORDER;
    let mut out = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {} {}\" width=\"{}\" \
         height=\"{}\" shape-rendering=\"crispEdges\"><rect width=\"100%\" height=\"100%\" \
         fill=\"#fff\"/>",
        width * MODULE,
        height * MODULE,
        width * MODULE,
        height * MODULE
    );
    for (y, row) in matrix.iter().enumerate() {
        for (x, dark) in row.iter().enumerate() {
            if *dark {
                let _ = write!(
                    out,
                    "<rect x=\"{}\" y=\"{}\" width=\"{MODULE}\" height=\"{MODULE}\"/>",
                    (x + BORDER) * MODULE,
                    (y + BORDER) * MODULE
                );
            }
        }
    }
    out.push_str("</svg>");
    out
}

/// How a final page reads: delivered (green ticks), a notice (amber) or a stop (red).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Success,
    Notice,
    Failure,
}

/// The tab icon: a green chat bubble with the double tick of a delivered message, inline so the
/// page needs no other file.
const FAVICON: &str = "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' \
viewBox='0 0 64 64'%3E%3Cpath d='M14 6h36a10 10 0 0 1 10 10v24a10 10 0 0 1-10 10H30l-14 \
10V50h-2A10 10 0 0 1 4 40V16A10 10 0 0 1 14 6z' fill='%2322c55e'/%3E%3Cpath d='M17 29l7 7 \
13-14M31 34l2 2 13-14' fill='none' stroke='%23fff' stroke-width='5' stroke-linecap='round' \
stroke-linejoin='round'/%3E%3C/svg%3E";

/// The chat bubble every page is built around, in a 120 x 104 box.
const BUBBLE: &str = "M22 8h76a14 14 0 0 1 14 14v44a14 14 0 0 1-14 14H54L32 98V80H22A14 14 0 0 1 \
8 66V22A14 14 0 0 1 22 8z";

/// Colors, layout and motion shared by every page. Light and dark follow the system; motion stops
/// for people who asked their system to reduce it.
const STYLE: &str = ":root{--bg:#f6f3ec;--dots:#e4ded1;--ink:#1d1b16;--soft:#6f685a;\
--paper:#fffdf8;--line:#e7e1d4;--green:#1fae5b;--amber:#e09b16;--red:#e0483e}\
@media (prefers-color-scheme:dark){:root{--bg:#121411;--dots:#22261f;--ink:#f1efe8;\
--soft:#a3a093;--paper:#1b1e19;--line:#2b2f27;--green:#35d07a;--amber:#f2b33d;--red:#ff6a5f}}\
*{box-sizing:border-box}html{background:var(--bg);background-image:radial-gradient(var(--dots) \
1.2px,transparent 1.2px);background-size:22px 22px}body{margin:0;min-height:100vh;display:grid;\
place-items:center;padding:32px 16px;color:var(--ink);font:17px/1.55 ui-sans-serif,system-ui,\
-apple-system,sans-serif}main{width:100%;max-width:460px;text-align:center}\
h1{margin:28px 0 10px;font:700 34px/1.15 ui-rounded,'SF Pro Rounded',system-ui,sans-serif;\
letter-spacing:-.02em}p{margin:0 auto;max-width:340px;color:var(--soft)}\
.tag{display:table;margin:28px auto 0;padding:4px 12px;border:1px solid var(--line);\
border-radius:999px;font:12px/1.6 ui-monospace,SFMono-Regular,Menlo,monospace;color:var(--soft)}\
.hero{width:150px;margin:0 auto;animation:float 4s 2.4s ease-in-out infinite}\
.hero svg{display:block;width:100%;height:auto;overflow:visible}\
.shape{animation:pop .6s cubic-bezier(.2,1.5,.4,1) both;transform-origin:50% 90%}\
.success .shape{fill:var(--green)}.notice .shape{fill:var(--amber)}.failure .shape{fill:var(--red)}\
.typing circle{fill:#fff;animation:hop .45s ease-in-out 4 alternate both,gone .2s 1.7s forwards}\
.typing circle:nth-child(2){animation-delay:.15s,1.7s}.typing circle:nth-child(3){\
animation-delay:.3s,1.7s}.mark{fill:none;stroke:#fff;stroke-width:8;stroke-linecap:round;\
stroke-linejoin:round;stroke-dasharray:80;stroke-dashoffset:80;animation:draw .35s 1.9s \
ease-out forwards}.mark+.mark{animation-delay:2.15s}\
.scan{position:relative;display:inline-block;margin:6px 0 30px;padding:16px;background:#fff;\
border:1px solid var(--line);border-radius:26px;box-shadow:0 18px 40px -22px rgba(40,30,10,.45)}.scan::after{content:'';position:absolute;\
left:42px;bottom:-11px;width:22px;height:22px;background:#fff;border-right:1px solid var(--line);\
border-bottom:1px solid var(--line);transform:rotate(45deg);border-bottom-right-radius:5px}\
.scan svg{display:block;width:232px;height:auto;border-radius:12px}\
ol{list-style:none;margin:0 auto;padding:0;max-width:320px;text-align:left;counter-reset:step}\
li{counter-increment:step;display:flex;gap:12px;align-items:baseline;margin:10px 0;\
color:var(--soft)}li::before{content:counter(step);flex:none;width:24px;height:24px;\
border-radius:50%;display:grid;place-items:center;background:var(--ink);color:var(--bg);\
font:700 13px/1 ui-rounded,system-ui,sans-serif}li b{color:var(--ink);font-weight:600}\
.waiting{display:inline-flex;gap:10px;align-items:center;margin-top:24px;padding:8px 16px;\
border-radius:999px;background:var(--paper);border:1px solid var(--line);font-size:14px;\
color:var(--soft)}.waiting i{display:inline-flex;gap:4px}.waiting i b{width:6px;height:6px;\
border-radius:50%;background:var(--green);animation:hop .45s ease-in-out infinite alternate}\
.waiting i b:nth-child(2){animation-delay:.15s}.waiting i b:nth-child(3){animation-delay:.3s}\
@keyframes pop{from{opacity:0;transform:scale(.6)}}@keyframes hop{to{transform:translateY(-7px)}}\
@keyframes gone{to{opacity:0}}@keyframes draw{to{stroke-dashoffset:0}}\
@keyframes float{50%{transform:translateY(-8px) rotate(-2deg)}}\
@media (prefers-reduced-motion:reduce){*{animation:none!important}.mark{stroke-dashoffset:0}\
.typing{display:none}}";

fn shell(title: &str, refresh: bool, body: &str) -> String {
    let refresh = if refresh {
        "<meta http-equiv=\"refresh\" content=\"5\">"
    } else {
        ""
    };
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" \
         content=\"width=device-width,initial-scale=1\">{refresh}<title>{title}</title><link \
         rel=\"icon\" href=\"{FAVICON}\"><style>{STYLE}</style></head><body><main>{body}<div \
         class=\"tag\">messaging-mcp</div></main></body></html>"
    )
}

/// A final page with no code and no reload, shown once a login ends, so the tab that kept
/// reloading the code lands on the outcome instead of a missing file. The bubble "types" first,
/// then shows its mark: a double tick, like a delivered message, when it worked.
pub fn status_page(tone: Tone, title: &str, message: &str) -> String {
    let (class, marks) = match tone {
        Tone::Success => (
            "success",
            "<path class=\"mark\" d=\"M30 45l12 12 22-24\"/><path class=\"mark\" \
             d=\"M56 53l4 4 22-24\"/>",
        ),
        Tone::Notice => (
            "notice",
            "<path class=\"mark\" d=\"M60 24v22\"/><path class=\"mark\" d=\"M60 60v.5\"/>",
        ),
        Tone::Failure => (
            "failure",
            "<path class=\"mark\" d=\"M47 31l26 26\"/><path class=\"mark\" d=\"M73 31L47 57\"/>",
        ),
    };
    let body = format!(
        "<div class=\"hero {class}\"><svg viewBox=\"0 0 120 104\" aria-hidden=\"true\"><path \
         class=\"shape\" d=\"{BUBBLE}\"/><g class=\"typing\"><circle cx=\"44\" cy=\"44\" r=\"6\"/>\
         <circle cx=\"60\" cy=\"44\" r=\"6\"/><circle cx=\"76\" cy=\"44\" r=\"6\"/></g>{marks}\
         </svg></div><h1>{title}</h1><p>{message}</p>"
    );
    shell(title, false, &body)
}

/// A page that shows the code `svg` in a chat bubble, the steps to scan it, and reloads every
/// 5 seconds to pick up a new code. Nothing on it animates in, so a reload is invisible until the
/// code itself changes. Steps may use `<b>` for the names of buttons.
pub fn page(title: &str, steps: &[&str], svg: &str) -> String {
    let steps: String = steps
        .iter()
        .map(|step| format!("<li><span>{step}</span></li>"))
        .collect();
    let body = format!(
        "<div class=\"scan\">{svg}</div><h1>{title}</h1><ol>{steps}</ol><div \
         class=\"waiting\"><i><b></b><b></b><b></b></i>Waiting for your scan</div>"
    );
    shell(title, true, &body)
}
