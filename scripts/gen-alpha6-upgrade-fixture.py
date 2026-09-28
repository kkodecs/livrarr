#!/usr/bin/env python3
"""Generate the synthetic alpha6 library used by the identity-upgrade tests.

The fixture is a real alpha6 (migration 073) database. The alpha6 binary itself
creates the schema, its runtime state and the admin account; this script adds
synthetic rows only (no real user data), then starts alpha6 once more so its own
startup code computes the work keys and installs its runtime `idx_works_identity`
index over those rows. The result is dumped as SQL next to its cover files and
the SHA-384 manifest of alpha6's migration files.

Build the alpha6 binary from the tag first, for example:

    git archive v0.1.0-alpha6 | tar -x -C /tmp/alpha6-src
    (cd /tmp/alpha6-src && cargo build -p livrarr-server --bin livrarr --locked)

Then run from the repository root:

    scripts/gen-alpha6-upgrade-fixture.py --alpha6-bin /tmp/alpha6-src/target/debug/livrarr

Outputs (all under crates/livrarr-server/tests/fixtures/):
    alpha6_library_073/livrarr.sql        the database, as `sqlite3 .dump`
    alpha6_library_073/covers/<user>/...  cover files referenced by the rows
    alpha6_migrations_073.sha384          SHA-384 of alpha6 migrations 001-073
"""

import argparse
import hashlib
import json
import os
import shutil
import signal
import socket
import sqlite3
import subprocess
import sys
import tempfile
import time
import urllib.request

from PIL import Image

ALPHA6_TAG = "v0.1.0-alpha6"
ADMIN_USER = "alpha6-admin"
ADMIN_PASSWORD = "alpha6-fixture-password"
NBSP = " "
IDEOGRAPHIC_SPACE = "　"
ADDED = "2026-06-01T12:00:00+00:00"


def isbn13(prefix12):
    total = sum(int(d) * (1 if i % 2 == 0 else 3) for i, d in enumerate(prefix12))
    return prefix12 + str((10 - total % 10) % 10)


def ol(n):
    return f"/works/OL{n}W"


def ids(n):
    """Five canonical alpha6 identifiers for one synthetic book."""
    return {
        "ol_key": ol(n),
        "hc_key": f"upgrade-fixture-{n}",
        "gr_key": str(n),
        "isbn_13": isbn13(f"978{n:09d}"),
        "asin": f"B0UPG{n % 100000:05d}",
    }


ANCHOR_TYPES = {
    "ol_key": "ol_work",
    "hc_key": "hc_work",
    "gr_key": "gr_work",
    "isbn_13": "isbn_13",
    "asin": "asin",
}


class Alpha6:
    def __init__(self, binary, data, log_path):
        self.binary = binary
        self.data = data
        sock = socket.socket()
        sock.bind(("127.0.0.1", 0))
        self.port = sock.getsockname()[1]
        sock.close()
        with open(os.path.join(data, "config.toml"), "w") as config:
            config.write(f"[server]\nbind_address = '127.0.0.1'\nport = {self.port}\n")
        self.log = open(log_path, "a")
        # A refusing local proxy: alpha6 background jobs cannot reach providers.
        env = {"HTTP_PROXY": "http://127.0.0.1:9", "HTTPS_PROXY": "http://127.0.0.1:9"}
        self.proc = subprocess.Popen(
            [binary, "--data", data], env=env, stdout=self.log, stderr=self.log
        )
        self.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        self.base = f"http://127.0.0.1:{self.port}/api/v1"
        deadline = time.time() + 60
        while True:
            if self.proc.poll() is not None:
                sys.exit(f"alpha6 exited before readiness; see {log_path}")
            try:
                if self.opener.open(self.base + "/health", timeout=1).status == 200:
                    break
            except Exception:
                pass
            if time.time() > deadline:
                sys.exit(f"alpha6 readiness timeout; see {log_path}")
            time.sleep(0.1)

    def post(self, path, body):
        request = urllib.request.Request(
            self.base + path,
            data=json.dumps(body).encode(),
            headers={"content-type": "application/json"},
            method="POST",
        )
        return json.loads(self.opener.open(request, timeout=10).read())

    def stop(self):
        self.proc.send_signal(signal.SIGTERM)
        self.proc.wait(timeout=30)
        self.log.close()


def jpeg(path, colour):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    Image.new("RGB", (500, 500), colour).save(path, "JPEG", quality=80)


def seed(db, covers):
    admin_hash = db.execute("SELECT password_hash FROM users WHERE id = 1").fetchone()[0]
    db.execute(
        "INSERT INTO users (id, username, password_hash, role, api_key_hash, created_at, updated_at) "
        "VALUES (2, 'second-reader', ?, 'user', ?, ?, ?)",
        (admin_hash, hashlib.sha256(b"second-reader-api-key").hexdigest(), ADDED, ADDED),
    )
    authors = [
        (1, 1, "Ursula K. Le Guin"),
        (2, 1, "Frank Herbert"),
        (3, 1, "Ann Leckie"),
        (4, 1, "Octavia E. Butler"),
        (5, 1, "Iain M. Banks"),
        (6, 1, "N. K. Jemisin"),
        (7, 1, "Becky Chambers"),
        (8, 1, "James S. A. Corey"),
        (9, 1, "Dan Simmons"),
        (10, 1, "Emily St. John Mandel"),
        (11, 1, "William Gibson"),
        (12, 1, "Adrian Tchaikovsky"),
        (20, 2, "Frank Herbert"),
    ]
    author_names = {author_id: name for author_id, _, name in authors}
    for author_id, user_id, name in authors:
        db.execute(
            "INSERT INTO authors (id, user_id, name, sort_name, monitored, added_at) "
            "VALUES (?, ?, ?, ?, 0, ?)",
            (author_id, user_id, name, name, ADDED),
        )
    db.execute(
        "INSERT INTO series (id, user_id, author_id, name, gr_key, work_count, added_at) "
        "VALUES (1, 1, 1, 'Earthsea Cycle', '40909', 1, ?)",
        (ADDED,),
    )

    def work(work_id, user_id, title, author_id, author_name, identity_status, **fields):
        row = {
            "id": work_id,
            "user_id": user_id,
            "title": title,
            "author_name": author_name,
            "author_id": author_id,
            "enrichment_status": "enriched",
            "enriched_at": ADDED,
            "enrichment_source": "hardcover",
            "identity_status": identity_status,
            "added_at": ADDED,
            "next_convergence_at": "2099-01-01T00:00:00+00:00",
            "language": "en",
        }
        row.update(fields)
        columns = ", ".join(row)
        marks = ", ".join("?" for _ in row)
        db.execute(f"INSERT INTO works ({columns}) VALUES ({marks})", list(row.values()))

    def anchor(work_id, user_id, column, value, confidence, setter):
        db.execute(
            "INSERT INTO work_identity_anchors "
            "(work_id, anchor_type, anchor_value, confidence, setter, set_at, user_id) "
            "VALUES (?, ?, ?, ?, ?, ?, ?)",
            (work_id, ANCHOR_TYPES[column], value, confidence, setter, ADDED, user_id),
        )

    # 1-4: one book per anchor kind of setter, all five identifier kinds each:
    # the user's picks, an import, automatic matches, and pending guesses.
    for work_id, title, author_id, status, confidence, setter in [
        (1, "A Wizard of Earthsea", 1, "confirmed", "confirmed", "user"),
        (2, "Dune", 2, "confirmed", "confirmed", "import"),
        (3, "Ancillary Justice", 3, "confirmed", "confirmed", "auto_search"),
        (4, "Kindred", 4, "pending", "pending", "auto_search"),
    ]:
        values = ids(900000 + work_id)
        extra = {}
        if work_id == 1:
            extra = {
                "subtitle": "The First Book of Earthsea",
                "series_id": 1,
                "series_name": "Earthsea Cycle",
                "series_position": 1.5,
                "monitor_ebook": 1,
                "monitor_audiobook": 1,
                "cover_url": "https://covers.openlibrary.org/b/id/900001-L.jpg",
                "cover_source": "openlibrary",
                "cover_trust": "user",
                "cover_manual": 1,
                "cover_width": 500,
                "cover_height": 500,
                "audiobook_cover_url": "https://m.media-amazon.com/images/I/900001.jpg",
                "audiobook_cover_source": "audible",
                "audiobook_cover_trust": "validated",
                "audiobook_cover_width": 500,
                "audiobook_cover_height": 500,
            }
            jpeg(os.path.join(covers, "1", "1.jpg"), (40, 90, 160))
            jpeg(os.path.join(covers, "1", "1_audio.jpg"), (160, 90, 40))
        if work_id == 2:
            extra = {"monitor_ebook": 0, "monitor_audiobook": 1}
        work(work_id, 1, title, author_id, author_names[author_id], status, **values, **extra)
        for column, value in values.items():
            anchor_setter = setter
            if work_id == 3 and column in ("isbn_13", "asin"):
                anchor_setter = "auto_isbn"
            anchor(work_id, 1, column, value, confidence, anchor_setter)

    # 5: no identifiers; the user's empty "still pending" marker (alpha6 writes
    # it as an empty OpenLibrary anchor) and two machine guesses that alpha6
    # keeps in the anchor ledger only, never in the identifier columns.
    work(5, 1, "The Player of Games", 5, "Iain M. Banks", "pending")
    anchor(5, 1, "ol_key", "", "pending", "user")
    anchor(5, 1, "ol_key", ol(900055), "pending", "auto_search")
    anchor(5, 1, "gr_key", "900055", "pending", "auto_search")
    # 6: alpha6 "needs review" with an identifier and no anchor.
    work(6, 1, "The Fifth Season", 6, "N. K. Jemisin", "needs_review", ol_key=ol(900006))

    # 7: user-chosen Goodreads audiobook cover (trust 'user'); 8: machine control.
    for work_id, title, trust, colour in [
        (7, "The Long Way to a Small, Angry Planet", "user", (20, 140, 60)),
        (8, "Record of a Spaceborn Few", "validated", (140, 20, 60)),
    ]:
        work(
            work_id, 1, title, 7, "Becky Chambers", "confirmed",
            gr_key=str(900000 + work_id),
            audiobook_cover_url=f"https://images.gr-assets.com/books/{900000 + work_id}l.jpg",
            audiobook_cover_source="goodreads",
            audiobook_cover_trust=trust,
            audiobook_cover_width=500,
            audiobook_cover_height=500,
        )
        anchor(work_id, 1, "gr_key", str(900000 + work_id), "confirmed", "import")
        jpeg(os.path.join(covers, "1", f"{work_id}_audio.jpg"), colour)

    # 9: one book whose identifiers came from different setters: the user
    # picked the OpenLibrary work and the ISBN; the rest came from an import
    # and an automatic match.
    mixed = ids(900009)
    work(9, 1, "Children of Time", 12, "Adrian Tchaikovsky", "confirmed", **mixed)
    for column, setter in [
        ("ol_key", "user"),
        ("hc_key", "import"),
        ("gr_key", "auto_search"),
        ("isbn_13", "user"),
        ("asin", "import"),
    ]:
        anchor(9, 1, column, mixed[column], "confirmed", setter)

    # 10-12: one title credited three ways, all linked to one author row.
    for work_id, credit in [(10, "James S. A. Corey"), (11, "Daniel Abraham"), (12, "Ty Franck")]:
        work(work_id, 1, "Leviathan Wakes", 8, credit, "pending")

    # 13-14: one Goodreads id on two different books.
    work(13, 1, "Hyperion", 9, "Dan Simmons", "confirmed", gr_key="900013")
    anchor(13, 1, "gr_key", "900013", "confirmed", "user")
    work(14, 1, "The Fall of Hyperion", 9, "Dan Simmons", "confirmed", gr_key="900013")

    # 15-17: padded identifiers (legal: alpha6 checks OpenLibrary values only for non-empty).
    padded_owner = NBSP + ol(900015)
    work(15, 1, "Station Eleven", 10, "Emily St. John Mandel", "confirmed", ol_key=padded_owner)
    anchor(15, 1, "ol_key", padded_owner, "confirmed", "user")
    padded_other = "\t" + ol(900015) + "\n"
    work(16, 1, "Sea of Tranquility", 10, "Emily St. John Mandel", "confirmed", ol_key=padded_other)
    anchor(16, 1, "ol_key", padded_other, "confirmed", "import")
    work(
        17, 1, "The Glass Hotel", 10, "Emily St. John Mandel", "pending",
        ol_key=NBSP + " \t", asin=IDEOGRAPHIC_SPACE,
    )

    # 18: one open legacy match question; 19: two closed ones.
    work(18, 1, "Neuromancer", 11, "William Gibson", "conflict", ol_key=ol(900018))
    anchor(18, 1, "ol_key", ol(900018), "confirmed", "user")
    work(19, 1, "Count Zero", 11, "William Gibson", "confirmed", ol_key=ol(900019))
    anchor(19, 1, "ol_key", ol(900019), "confirmed", "user")

    def payload(ol_key, title):
        return json.dumps(
            {
                "ol_key": ol_key,
                "gr_key": None,
                "hc_key": None,
                "isbn_13": None,
                "asin": None,
                "title": title,
                "author_name": "William Gibson",
                "year": 1984,
                "cover_url": None,
                "top_candidates": [],
            },
            separators=(",", ":"),
        )

    conflicts = [
        (1, 18, payload(ol(900098), "Neuromancer"), "open", None, None, None),
        (2, 19, payload(ol(900097), "Count Zero"), "resolved", "2026-06-02T08:00:00+00:00",
         "keep_existing", "kept my match"),
        (3, 19, payload(ol(900096), "Count Zero"), "dismissed", "2026-06-03T08:00:00+00:00",
         None, None),
    ]
    for conflict_id, work_id, body, status, resolved_at, action, notes in conflicts:
        db.execute(
            "INSERT INTO work_identity_conflicts (id, user_id, existing_work_id, kind, "
            "incoming_payload_json, raised_at, raised_by, status, resolved_at, "
            "resolution_action, resolution_notes) "
            "VALUES (?, 1, ?, 'incoming_different_ol_key', ?, ?, 'refresh', ?, ?, ?, ?)",
            (conflict_id, work_id, body, ADDED, status, resolved_at, action, notes),
        )

    # 20: the second user's copy of Dune carries the same OpenLibrary id as work 2.
    work(20, 2, "Dune", 20, "Frank Herbert", "confirmed", ol_key=ol(900002))
    anchor(20, 2, "ol_key", ol(900002), "confirmed", "import")

    for work_id, field, source, setter in [
        (1, "title", None, "user"),
        (1, "description", "hardcover", "provider"),
        (2, "year", "openlibrary", "provider"),
    ]:
        db.execute(
            "INSERT INTO work_metadata_provenance (user_id, work_id, field, source, set_at, setter) "
            "VALUES (1, ?, ?, ?, ?, ?)",
            (work_id, field, source, ADDED, setter),
        )


def manifest(repo):
    names = subprocess.run(
        ["git", "-C", repo, "ls-tree", "--name-only", f"{ALPHA6_TAG}:crates/livrarr-db/migrations"],
        check=True, capture_output=True, text=True,
    ).stdout.split()
    lines = []
    for name in sorted(names):
        if name.endswith(".sql"):
            body = subprocess.run(
                ["git", "-C", repo, "show", f"{ALPHA6_TAG}:crates/livrarr-db/migrations/{name}"],
                check=True, capture_output=True,
            ).stdout
            lines.append(f"{hashlib.sha384(body).hexdigest()}  {name}")
    return "\n".join(lines) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--alpha6-bin", required=True)
    parser.add_argument("--repo", default=os.getcwd())
    args = parser.parse_args()
    out = os.path.join(args.repo, "crates/livrarr-server/tests/fixtures")
    library_out = os.path.join(out, "alpha6_library_073")

    work_dir = tempfile.mkdtemp(prefix="alpha6-fixture-")
    data = os.path.join(work_dir, "data")
    os.makedirs(data)
    log_path = os.path.join(work_dir, "alpha6.log")

    first = Alpha6(args.alpha6_bin, data, log_path)
    first.post("/setup", {"username": ADMIN_USER, "password": ADMIN_PASSWORD})
    first.stop()

    db_path = os.path.join(data, "livrarr.db")
    db = sqlite3.connect(db_path)
    db.execute("PRAGMA foreign_keys = ON")
    assert db.execute("SELECT MAX(version) FROM _sqlx_migrations").fetchone()[0] == 73
    # alpha6 derives work keys at startup from '__UNMIGRATED__' rows and then
    # (re)installs its runtime unique index; let it do both over the seed.
    db.execute("DROP INDEX idx_works_identity")
    seed(db, os.path.join(data, "covers"))
    db.commit()
    db.close()

    second = Alpha6(args.alpha6_bin, data, log_path)
    time.sleep(2)  # let alpha6's startup passes finish before stopping it
    second.stop()

    db = sqlite3.connect(db_path)
    checks = {
        "works": db.execute("SELECT COUNT(*) FROM works").fetchone()[0],
        "unmigrated": db.execute(
            "SELECT COUNT(*) FROM works WHERE normalized_title = '__UNMIGRATED__'"
        ).fetchone()[0],
        "index": db.execute(
            "SELECT sql FROM sqlite_master WHERE name = 'idx_works_identity'"
        ).fetchone(),
        "padded": db.execute("SELECT ol_key FROM works WHERE id = 16").fetchone()[0],
        "open": db.execute(
            "SELECT COUNT(*) FROM work_identity_conflicts WHERE status = 'open'"
        ).fetchone()[0],
        "user_cover": db.execute(
            "SELECT audiobook_cover_trust FROM works WHERE id = 7"
        ).fetchone()[0],
        "anchors": db.execute("SELECT COUNT(*) FROM work_identity_anchors").fetchone()[0],
        "series": db.execute(
            "SELECT subtitle, series_id, series_position FROM works WHERE id = 1"
        ).fetchone(),
        "guess_columns": db.execute(
            "SELECT ol_key, gr_key FROM works WHERE id = 5"
        ).fetchone(),
    }
    expected = {
        "works": 20,
        "unmigrated": 0,
        "padded": "\t" + ol(900015) + "\n",
        "open": 1,
        "user_cover": "user",
        "anchors": 36,
        "series": ("The First Book of Earthsea", 1, 1.5),
        "guess_columns": (None, None),
    }
    for key, value in expected.items():
        if checks[key] != value:
            sys.exit(f"alpha6 changed the seed: {key} = {checks[key]!r}, expected {value!r}")
    if checks["index"] is None:
        sys.exit("alpha6 did not install idx_works_identity")
    # Sanitize: drop login sessions; fold the WAL into the main file.
    db.execute("DELETE FROM sessions")
    db.commit()
    db.execute("PRAGMA wal_checkpoint(TRUNCATE)")
    db.close()

    dump = subprocess.run(
        ["sqlite3", db_path, ".dump"], check=True, capture_output=True, text=True
    ).stdout
    shutil.rmtree(library_out, ignore_errors=True)
    os.makedirs(library_out)
    with open(os.path.join(library_out, "livrarr.sql"), "w") as out_sql:
        out_sql.write(dump)
    shutil.copytree(os.path.join(data, "covers"), os.path.join(library_out, "covers"))
    with open(os.path.join(out, "alpha6_migrations_073.sha384"), "w") as out_manifest:
        out_manifest.write(manifest(args.repo))
    shutil.rmtree(work_dir)
    print(f"wrote {library_out} and alpha6_migrations_073.sha384")


if __name__ == "__main__":
    main()
