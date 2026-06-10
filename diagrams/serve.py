#!/usr/bin/env python3
"""Tiny static server for the livrarr diagrams folder.

Serves http://oasis:8791/diagrams — a dynamically-generated index of every
*.html diagram in this folder — and http://oasis:8791/diagrams/<file> for each
one. Only this folder is exposed. Drop a new .html in here and it shows up on
the index automatically (label taken from its <title>).
"""
import html
import os
import re
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

DIR = os.path.dirname(os.path.abspath(__file__))
PORT = 8791

INDEX_CSS = """
  body{margin:0;background:#0f1117;color:#e7e9ee;
    font:15px/1.6 -apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,Helvetica,Arial,sans-serif}
  .wrap{max-width:860px;margin:0 auto;padding:56px 28px}
  h1{font-size:24px;margin:0 0 6px}
  .sub{color:#9aa3b2;margin-bottom:32px}
  ul{list-style:none;padding:0;margin:0}
  li{border:1px solid #2b3040;background:#171a23;border-radius:12px;margin-bottom:14px;
    transition:border-color .15s,background .15s}
  li:hover{border-color:#5aa9ff;background:#1d2130}
  a{display:block;padding:18px 22px;text-decoration:none;color:#e7e9ee}
  a .t{font-size:16px;font-weight:600}
  a .f{display:block;color:#6f7a8c;font-family:ui-monospace,Menlo,monospace;font-size:12px;margin-top:4px}
  .empty{color:#9aa3b2}
"""


def diagram_title(path):
    try:
        with open(path, "r", encoding="utf-8") as f:
            head = f.read(4000)
        m = re.search(r"<title>(.*?)</title>", head, re.I | re.S)
        if m:
            return html.unescape(m.group(1).strip())
    except OSError:
        pass
    return os.path.splitext(os.path.basename(path))[0].replace("-", " ").replace("_", " ").title()


def list_diagrams():
    return sorted(
        f for f in os.listdir(DIR)
        if f.endswith(".html") and f != "index.html"
    )


def render_index():
    files = list_diagrams()
    if files:
        items = "\n".join(
            f'<li><a href="/diagrams/{html.escape(urllib.parse.quote(f))}">'
            f'<span class="t">{html.escape(diagram_title(os.path.join(DIR, f)))}</span>'
            f'<span class="f">{html.escape(f)}</span></a></li>'
            for f in files
        )
        body = f"<ul>{items}</ul>"
    else:
        body = '<p class="empty">No diagrams yet — drop an .html file in this folder.</p>'
    return (
        "<!DOCTYPE html><html lang='en'><head><meta charset='utf-8'>"
        "<meta name='viewport' content='width=device-width, initial-scale=1'>"
        f"<title>livrarr diagrams</title><style>{INDEX_CSS}</style></head><body>"
        f"<div class='wrap'><h1>livrarr diagrams</h1>"
        f"<div class='sub'>{len(files)} diagram(s) in <code>/mnt/opt/livrarr/diagrams</code></div>"
        f"{body}</div></body></html>"
    )


class Handler(BaseHTTPRequestHandler):
    def _send(self, code, body, ctype="text/html; charset=utf-8"):
        data = body.encode("utf-8") if isinstance(body, str) else body
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(data)

    def do_GET(self):
        path = urllib.parse.urlparse(self.path).path
        if path in ("/", "/diagrams", "/diagrams/", "/index.html"):
            return self._send(200, render_index())
        if path.startswith("/diagrams/"):
            name = os.path.basename(urllib.parse.unquote(path[len("/diagrams/"):]))
            full = os.path.join(DIR, name)
            if name.endswith(".html") and os.path.isfile(full):
                with open(full, "rb") as f:
                    return self._send(200, f.read())
        return self._send(404, "<h1>404</h1><p><a href='/diagrams'>back to diagrams</a></p>")

    do_HEAD = do_GET

    def log_message(self, *a):  # quiet
        pass


if __name__ == "__main__":
    srv = ThreadingHTTPServer(("0.0.0.0", PORT), Handler)
    print(f"diagrams server on http://oasis:{PORT}/diagrams  (serving {DIR})")
    srv.serve_forever()
