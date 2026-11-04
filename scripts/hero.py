"""Generates docs/hero.svg: Otto's real UI on a Mac desktop, in five looping scenes.

Run from the repo root: python3 scripts/hero.py
"""
import base64
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
gmail = base64.b64encode((ROOT / "src/logos/apps/gmail.png").read_bytes()).decode()

BODY = "M50 12 C74 10 88 26 86 48 C84 68 68 78 48 78 C26 78 12 64 14 44 C16 24 30 13 50 12 Z"
TIE = '<path d="M38 76 L48 81 L38 86 Z M62 76 L52 81 L62 86 Z" fill="#8CC9B2" stroke="#8CC9B2" stroke-width="6" stroke-linejoin="round"/>'
RUNNING = f'<path d="{BODY}" fill="#CFE6F3"/><circle class="blink" cx="76" cy="44" r="5.5" fill="#2F6F62"/><path d="M58 73 L66 78 L58 83 Z" fill="#8CC9B2" stroke="#8CC9B2" stroke-width="6" stroke-linejoin="round"/>'
NEEDS = f'<path d="M44 -12 Q44 -19 50.5 -19 Q57 -19 57 -13 Q57 -9 51 -6 V-2" fill="none" stroke="#2B5677" stroke-width="5" stroke-linecap="round" stroke-linejoin="round"/><circle cx="51" cy="5" r="3" fill="#2B5677"/><path d="{BODY}" fill="#CFE6F3"/><circle cx="39" cy="44" r="6.5" fill="#1F3E5A"/><circle cx="61" cy="44" r="6.5" fill="#1F3E5A"/><path d="M38 76 L48 81 L38 86 Z M62 76 L52 81 L62 86 Z" fill="#7FAED6" stroke="#7FAED6" stroke-width="6" stroke-linejoin="round"/>'
DONE = f'<path d="{BODY}" fill="#BFE5D2"/><path d="M33 47 Q39 39 45 47 M55 47 Q61 39 67 47" fill="none" stroke="#2F6F62" stroke-width="5" stroke-linecap="round"/>{TIE}'
LISTEN = f'<path d="{BODY}" fill="#CFE6F3"/><circle cx="39" cy="44" r="6" fill="#2F6F62"/><circle cx="61" cy="44" r="6" fill="#2F6F62"/>{TIE}'

# Glass, as close as a static SVG gets: bright translucent fill and a light rim.
GLASS = 'fill="#fff" fill-opacity=".8" stroke="#fff" stroke-opacity=".95"'


def mascot(x, y, size, inner, cls="bob"):
    return f'<svg x="{x}" y="{y}" width="{size}" height="{size}" viewBox="-10 -20 120 120"><g class="{cls}">{inner}</g></svg>'


def badge(cx, cy, cls):
    return (f'<g class="{cls}"><circle cx="{cx}" cy="{cy}" r="11" fill="#fff" filter="url(#bsh)"/>'
            f'<image href="data:image/png;base64,{gmail}" x="{cx - 7}" y="{cy - 7}" width="14" height="14"/></g>')


# Scene windows on a 16s loop, in percent.
SCENES = {"s1": (0, 18), "s2": (20, 36), "s3": (38, 55), "s4": (57, 80), "s5": (82, 98)}


def fade(name, a, b):
    lo, hi = max(a - 1.5, 0), min(b + 1.5, 100)
    if a == 0:
        return f"@keyframes {name} {{ 0%, {b}% {{ opacity: 1 }} {hi}%, 100% {{ opacity: 0 }} }}"
    return f"@keyframes {name} {{ 0%, {lo}% {{ opacity: 0 }} {a}%, {b}% {{ opacity: 1 }} {hi}%, 100% {{ opacity: 0 }} }}"


def drip(name, at, sx, sy):
    """A drop out of the notch: falls and stretches, lands and spreads, settles (about 0.7s)."""
    t = lambda d: round(at + d, 2)
    start = f"translateY(-100%) scale({sx}, {sy})"
    return (f"@keyframes {name} {{ 0%, {t(0)}% {{ transform: {start} }} {t(1.4)}% {{ transform: translateY(5%) scale({sx * 1.4:.2f}, 1.18) }} "
            f"{t(2.4)}% {{ transform: scale(1.06, .9) }} {t(3.3)}% {{ transform: scale(.985, 1.025) }} {t(4.2)}%, 100% {{ transform: none }} }}")


def dark(name, at):
    """The drop starts notch-black and clears to glass as it lands."""
    return f"@keyframes {name} {{ 0%, {at + 1.2}% {{ opacity: .92 }} {at + 2.6}%, 100% {{ opacity: 0 }} }}"


frames = "\n    ".join(fade(k, *v) for k, v in SCENES.items())
frames += "\n    " + fade("s3a", 38, 46) + "\n    " + fade("s3b", 47.5, 55)
frames += "\n    " + drip("drip1", 0, 0.35, 0.6) + "\n    " + dark("dark1", 0)
frames += "\n    " + drip("drip3", 38, 0.12, 0.7) + "\n    " + dark("dark3", 38)

css = f"""
    text {{ font-family: -apple-system, BlinkMacSystemFont, "SF Pro Text", "Segoe UI", Helvetica, Arial, sans-serif; }}
    .cap {{ font-size: 15px; font-weight: 600; fill: #24594a; }}
    .num {{ font-size: 12px; font-weight: 700; fill: #fff; }}
    .menu {{ font-size: 12px; fill: #1c2622; }}
    .h {{ font-size: 17px; font-weight: 700; fill: #1c2622; }}
    .p {{ font-size: 13px; fill: #3a4a44; }}
    .t {{ font-size: 13.5px; font-weight: 600; fill: #1c2622; }}
    .s {{ font-size: 12.5px; fill: #5b6b65; }}
    .a {{ font-size: 12.5px; font-weight: 500; fill: #5b6b65; }}
    .m {{ font-size: 11.5px; fill: #24394c; font-family: ui-monospace, SFMono-Regular, Menlo, monospace; }}
    .q {{ font-size: 14px; font-style: italic; fill: #24594a; }}
    .sc {{ opacity: 0; animation: 16s linear infinite; }}
    .s1 {{ animation-name: s1; }} .s2 {{ animation-name: s2; }} .s3 {{ animation-name: s3; }}
    .s4 {{ animation-name: s4; }} .s5 {{ animation-name: s5; }} .s3a {{ animation-name: s3a; }} .s3b {{ animation-name: s3b; }}
    {frames}
    .drip1, .drip3, .handoff, .drop, .hop {{ transform-box: fill-box; transform-origin: 50% 0; animation: 16s cubic-bezier(.3, .7, .4, 1) infinite; }}
    .drip1 {{ animation-name: drip1; }} .drip3 {{ animation-name: drip3; }}
    .dark1, .dark3 {{ animation: 16s linear infinite; }} .dark1 {{ animation-name: dark1; }} .dark3 {{ animation-name: dark3; }}
    .handoff {{ animation-name: handoff; transform-origin: 50% 50%; }}
    @keyframes handoff {{ 0%, 33.5% {{ transform: none }} 36%, 100% {{ transform: scale(3, .92) }} }}
    .hop {{ animation-name: hop; transform-origin: 50% 100%; }}
    @keyframes hop {{ 0%, 21% {{ transform: none }} 22% {{ transform: translateY(-4px) scale(.97, 1.04) }} 23%, 100% {{ transform: none }} }}
    .content3 {{ animation: content3 16s ease-out infinite; }}
    @keyframes content3 {{ 0%, 40.6% {{ opacity: 0 }} 42.2%, 100% {{ opacity: 1 }} }}
    .drop {{ animation-name: drop; }}
    @keyframes drop {{ 0%, 57% {{ transform: translateY(-110%) scaleX(.55) }} 58.6% {{ transform: translateY(3%) }} 59.6%, 100% {{ transform: none }} }}
    .ring {{ transform-box: fill-box; transform-origin: center; animation: spin 1s linear infinite; }}
    @keyframes spin {{ to {{ transform: rotate(360deg) }} }}
    .level {{ transform-box: fill-box; transform-origin: center; animation: level .5s ease-in-out infinite alternate; }}
    @keyframes level {{ from {{ transform: scale(1.04) }} to {{ transform: scale(1.22) }} }}
    .pop3, .pop4 {{ transform-box: fill-box; transform-origin: center; animation: 16s ease-out infinite; }}
    .pop3 {{ animation-name: pop3; }} .pop4 {{ animation-name: pop4; }}
    @keyframes pop3 {{ 0%, 41% {{ transform: scale(0) }} 43% {{ transform: scale(1.25) }} 44.5%, 100% {{ transform: scale(1) }} }}
    @keyframes pop4 {{ 0%, 59% {{ transform: scale(0) }} 61% {{ transform: scale(1.25) }} 62.5%, 100% {{ transform: scale(1) }} }}
    .cursor {{ animation: cursor 16s ease-in-out infinite; }}
    @keyframes cursor {{ 0%, 66% {{ transform: translate(90px, 70px) }} 72%, 100% {{ transform: none }} }}
    .press {{ animation: press 16s linear infinite; }}
    @keyframes press {{ 0%, 73% {{ fill: #2b5677 }} 74%, 76% {{ fill: #1b3a52 }} 77%, 100% {{ fill: #2b5677 }} }}
    .bob {{ animation: bob 1.4s ease-in-out infinite alternate; }}
    @keyframes bob {{ from {{ transform: translateY(0) }} to {{ transform: translateY(-4px) }} }}
    .blink {{ transform-box: fill-box; transform-origin: center; animation: blink 3s infinite; }}
    @keyframes blink {{ 0%, 92%, 100% {{ transform: scaleY(1) }} 95% {{ transform: scaleY(.1) }} }}
    .type {{ animation: type 16s linear infinite; }}
    @keyframes type {{ 0%, 20% {{ width: 0 }} 90%, 100% {{ width: 262px }} }}
    .typed {{ animation: typed 16s linear infinite; }}
    @keyframes typed {{ 0%, 20% {{ transform: translateX(0) }} 90%, 100% {{ transform: translateX(262px) }} }}
    .key {{ animation: key 16s linear infinite; }}
    @keyframes key {{ 0%, 1% {{ transform: translateY(0) }} 2%, 17% {{ transform: translateY(2px) }} 18%, 100% {{ transform: translateY(0) }} }}
    @media (prefers-reduced-motion: reduce) {{
      .sc, .drip1, .drip3, .dark1, .dark3, .handoff, .hop, .content3, .drop, .ring, .level, .pop3, .pop4, .cursor, .press, .bob, .blink, .type, .typed, .key {{ animation: none; }}
      .s5, .content3 {{ opacity: 1; }}
      .dark1, .dark3 {{ opacity: 0; }}
    }}
"""


def cap(n, text, cls):
    return (f'<g class="sc {cls}"><circle cx="40" cy="30" r="12" fill="#2f6f62"/>'
            f'<text class="num" x="40" y="34.5" text-anchor="middle">{n}</text><text class="cap" x="60" y="35">{text}</text></g>')


def keycap(x, label, w=34):
    return (f'<g class="key"><rect x="{x}" y="0" width="{w}" height="30" rx="7" fill="#fff" stroke="#cdd8d3"/>'
            f'<rect x="{x}" y="27" width="{w}" height="4" rx="2" fill="#cdd8d3"/>'
            f'<text x="{x + w / 2}" y="20" text-anchor="middle" style="font-size:13px;font-weight:600;fill:#1c2622">{label}</text></g>')


# Geometry: the notch sits at the top center of the menu bar; everything drips out of it.
MID = 440
BAR = 84  # menu bar bottom = notch bottom
IW = 236  # task island width
CW = 340  # card width
CX = MID - CW // 2
TX = CX + 62  # card text column

svg = f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 880 420" width="880" height="420" role="img" aria-label="Otto on a Mac: hold Control Option Space and ask it to reply to Jordan. A small island drips out of the notch while it listens, Otto checks Gmail in the background, waits under the notch for you to allow the send, then shows the result.">
  <style>{css}  </style>
  <defs>
    <linearGradient id="wall" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#cfe0ec"/><stop offset=".55" stop-color="#e2ecef"/><stop offset="1" stop-color="#e4ddf0"/></linearGradient>
    <filter id="sh" x="-20%" y="-20%" width="140%" height="170%"><feDropShadow dx="0" dy="6" stdDeviation="9" flood-color="#1c2622" flood-opacity=".14"/></filter>
    <filter id="bsh" x="-50%" y="-50%" width="200%" height="200%"><feDropShadow dx="0" dy="1" stdDeviation="2" flood-color="#142832" flood-opacity=".28"/></filter>
    <clipPath id="desk"><rect x="20" y="60" width="840" height="344" rx="14"/></clipPath>
    <clipPath id="under"><rect x="20" y="{BAR}" width="840" height="320"/></clipPath>
    <clipPath id="typeclip"><rect class="type" x="72" y="296" height="20" width="0"/></clipPath>
  </defs>

  <rect width="880" height="420" rx="20" fill="#f4f7f6"/>
  {cap(1, "Hold ⌃ ⌥ Space and say what you need", "s1")}
  {cap(2, "Let go. It's transcribed on your Mac", "s2")}
  {cap(3, "Otto drips out of the notch and works in the background", "s3")}
  {cap(4, "Anything that sends or changes waits for you", "s4")}
  {cap(5, "The result lands when it's done", "s5")}

  <g clip-path="url(#desk)">
    <rect x="20" y="60" width="840" height="344" fill="url(#wall)"/>
    <!-- menu bar and notch -->
    <rect x="20" y="60" width="840" height="24" fill="#fff" fill-opacity=".72"/>
    <circle cx="40" cy="72" r="5" fill="#1c2622"/>
    <text class="menu" x="56" y="76" style="font-weight:700">Notes</text>
    <text class="menu" x="104" y="76">File</text><text class="menu" x="136" y="76">Edit</text><text class="menu" x="170" y="76">Format</text><text class="menu" x="222" y="76">View</text>
    <g transform="translate(726 64)"><path d="M8 2 C13 1.6 16 4.6 15.6 8.6 C15.2 12.2 12.4 14 8.6 14 C4.6 14 2 11.6 2.4 8 C2.8 4.4 5.2 2.2 8 2 Z" fill="none" stroke="#1c2622" stroke-width="1.4"/><circle cx="6.6" cy="8" r="1" fill="#1c2622"/><circle cx="10.4" cy="8" r="1" fill="#1c2622"/></g>
    <text class="menu" x="752" y="76">Wed 9:41 AM</text>

    <!-- the work you keep doing -->
    <g filter="url(#sh)"><rect x="48" y="112" width="420" height="262" rx="11" fill="#fff"/></g>
    <rect x="48" y="112" width="420" height="30" rx="11" fill="#f6f8f7"/><rect x="48" y="130" width="420" height="12" fill="#f6f8f7"/>
    <line x1="48" y1="142" x2="468" y2="142" stroke="#e8eeeb"/>
    <circle cx="66" cy="127" r="5.5" fill="#ff5f57"/><circle cx="84" cy="127" r="5.5" fill="#febc2e"/><circle cx="102" cy="127" r="5.5" fill="#28c840"/>
    <text class="s" x="258" y="131" text-anchor="middle" style="font-weight:600">Q3 planning</text>
    <text class="h" x="72" y="174">Q3 planning</text>
    <text class="p" x="72" y="202">• Hire two designers before September</text>
    <text class="p" x="72" y="226">• Move the launch review to Thursday</text>
    <text class="p" x="72" y="250">• Book the offsite venue (40 people)</text>
    <text class="p" x="72" y="274">• Budget draft due on the 14th</text>
    <g clip-path="url(#typeclip)"><text class="p" x="72" y="310">• Send the hiring plan to the team by Friday</text></g>
    <rect class="typed" x="72" y="297" width="1.6" height="17" fill="#2f6f62"><animate attributeName="opacity" values="1;0;1" dur="1s" repeatCount="indefinite"/></rect>

    <!-- 1: hotkey, and the listening island drips out of the notch -->
    <g class="sc s1">
      <g transform="translate(588 300)">{keycap(0, "⌃")}{keycap(40, "⌥")}{keycap(80, "space", 84)}</g>
      <text class="q" x="672" y="360" text-anchor="middle">"Reply to Jordan that</text>
      <text class="q" x="672" y="378" text-anchor="middle">Friday at 3 works"</text>
      <g clip-path="url(#under)"><g class="drip1">
        <circle class="level" cx="{MID}" cy="{BAR + 21}" r="21" fill="none" stroke="#8cc9b2" stroke-opacity=".55" stroke-width="3"/>
        <g filter="url(#sh)"><circle cx="{MID}" cy="{BAR + 21}" r="20" {GLASS}/></g>
        {mascot(MID - 16, BAR + 5, 32, LISTEN, "")}
        <circle class="dark1" cx="{MID}" cy="{BAR + 21}" r="20.5" fill="#000"/>
      </g></g>
    </g>

    <!-- 2: heard; it hops, then stretches into the task island -->
    <g class="sc s2"><g class="handoff">
      <g filter="url(#sh)"><circle cx="{MID}" cy="{BAR + 21}" r="20" fill="#e3f4ea" fill-opacity=".9" stroke="#fff"/></g>
      <g class="hop">{mascot(MID - 16, BAR + 5, 32, DONE, "")}</g>
    </g></g>

    <!-- 3: the task island -->
    <g class="sc s3"><g clip-path="url(#under)"><g class="drip3">
      <g filter="url(#sh)"><rect x="{MID - IW // 2}" y="{BAR}" width="{IW}" height="36" rx="18" {GLASS}/></g>
      <g class="content3">
        {mascot(MID - IW // 2 + 6, BAR + 5, 26, RUNNING)}
        <text class="t" x="{MID - IW // 2 + 40}" y="{BAR + 23}" style="font-size:13px;font-weight:500">Reply to Jordan</text>
        <g class="pop3"><image href="data:image/png;base64,{gmail}" x="{MID + IW // 2 - 52}" y="{BAR + 11}" width="14" height="14"/></g>
        <circle cx="{MID + IW // 2 - 20}" cy="{BAR + 18}" r="6" fill="none" stroke="#2f6f62" stroke-opacity=".18" stroke-width="2"/>
        <circle class="ring" cx="{MID + IW // 2 - 20}" cy="{BAR + 18}" r="6" fill="none" stroke="#2f6f62" stroke-width="2" stroke-linecap="round" stroke-dasharray="11 27"/>
      </g>
      <rect class="dark3" x="{MID - IW // 2}" y="{BAR}" width="{IW}" height="36" rx="18" fill="#000"/>
    </g></g></g>

    <!-- 4: needs you; the card drops out from under the notch -->
    <g class="sc s4"><g clip-path="url(#under)"><g class="drop">
      <g filter="url(#sh)"><rect x="{CX}" y="{BAR + 2}" width="{CW}" height="168" rx="18" fill="#eef5fb" fill-opacity=".9" stroke="#fff"/></g>
      {mascot(CX + 14, BAR + 16, 36, NEEDS)}
      {badge(CX + 47, BAR + 49, "pop4")}
      <text class="t" x="{TX}" y="{BAR + 28}">Reply to Jordan</text>
      <text class="s" x="{TX}" y="{BAR + 48}">Gmail: send message</text>
      <rect x="{TX}" y="{BAR + 60}" width="262" height="56" rx="8" fill="#dfe9f3"/>
      <text class="m" x="{TX + 10}" y="{BAR + 80}">To: jordan@northwind.co</text>
      <text class="m" x="{TX + 10}" y="{BAR + 99}">Friday at 3 works. See you then!</text>
      <rect class="press" x="{TX}" y="{BAR + 128}" width="58" height="28" rx="8" fill="#2b5677"/>
      <text x="{TX + 29}" y="{BAR + 147}" text-anchor="middle" style="font-size:12.5px;font-weight:600;fill:#fff">Allow</text>
      <text class="a" x="{TX + 74}" y="{BAR + 147}">Don't allow</text>
      <text class="a" x="{TX + 158}" y="{BAR + 147}">Cancel</text>
    </g></g>
      <g class="cursor"><path transform="translate({TX + 38} {BAR + 142})" d="M0 0 L0 17 L4.5 13 L7.5 20 L10.5 18.6 L7.6 12 L13 12 Z" fill="#1c2622" stroke="#fff" stroke-width="1.3" stroke-linejoin="round"/></g>
    </g>

    <!-- 5: done -->
    <g class="sc s5">
      <g filter="url(#sh)"><rect x="{CX}" y="{BAR + 2}" width="{CW}" height="104" rx="18" {GLASS}/></g>
      {mascot(CX + 14, BAR + 14, 36, DONE, "")}
      <text class="t" x="{TX}" y="{BAR + 28}">Reply to Jordan</text>
      <text class="s" x="{TX}" y="{BAR + 48}">Replied on the thread: "Friday at 3</text>
      <text class="s" x="{TX}" y="{BAR + 66}">works. See you then!"</text>
      <text x="{TX}" y="{BAR + 92}" style="font-size:12.5px;font-weight:600;fill:#24594a">Open</text>
      <text class="a" x="{TX + 46}" y="{BAR + 92}">Show in Finder</text>
    </g>

    <!-- the notch, drawn last so everything comes out from behind it -->
    <path d="M{MID - 56} 60 H{MID + 56} V{BAR - 8} Q{MID + 56} {BAR} {MID + 48} {BAR} H{MID - 48} Q{MID - 56} {BAR} {MID - 56} {BAR - 8} Z" fill="#000"/>
    <circle cx="{MID + 24}" cy="72" r="3" fill="#1a1d24"/>
  </g>
</svg>
'''
(ROOT / "docs/hero.svg").write_text(svg)
print(f"docs/hero.svg: {len(svg)} bytes")
