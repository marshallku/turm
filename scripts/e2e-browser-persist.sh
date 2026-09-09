#!/usr/bin/env bash
# e2e for the Browser Workbench persistence + focus claims (macOS).
#
#   1. A browser pane persists a tab snapshot whose id SURVIVES A RESTART, and a
#      snapshot
#      taken before the page finishes loading keeps its URL instead of writing
#      "" and erasing the pane. That erasure was a live bug: a session.json in
#      the wild held `{"kind":"webview","url":""}` for a pane the user had open.
#      An unroutable address is used for the second case so WKWebView never
#      produces a URL at all.
#   2. Driving the browser steals neither the frontmost application nor the
#      mouse cursor — the owner's hard requirement that
#      agent automation never fights the human for the pointer.
#
# Isolated by $HOME, so it cannot touch the user's live session.json. Note the
# Swift side reads config through `homeDirectoryForCurrentUser`, which ignores
# $HOME — config is therefore the real one (read-only here), while everything
# Rust writes lands in the temp home.
#
# Usage: scripts/e2e-browser-persist.sh
#
# Build the bundle first, but note `copad-macos/run.sh` ends with
# `pkill -x Copad` + `open -n` — it KILLS a running installed Copad and takes
# over. To build without disturbing a live session:
#
#   cargo build --release -p copad-ffi -p copad-term
#   cd copad-macos && swift build
#   # then lay out .build/debug/Copad.app as run.sh does, minus the last two lines
set -uo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
APP="$REPO/copad-macos/.build/debug/Copad.app/Contents/MacOS/Copad"
COCTL="$REPO/target/release/coctl"; [ -x "$COCTL" ] || COCTL="$REPO/target/debug/coctl"
# Every CLI invocation below runs after `export COPAD_SOCKET=` for the instance
# under test. Without that a `coctl` call inherits the ambient socket and opens
# a tab in the user's REAL copad — which is not a test failure, it is a test
# that reached out and touched something it had no business touching.
[ -x "$APP" ] || { echo "build the bundle first — see the header"; exit 2; }

# Raw JSON-RPC over the per-instance GUI socket.
#
# Not `coctl` for the interaction commands: every `coctl webview <verb>`
# requires `--id <panel-uuid>`, and no CLI verb exposes panel ids (`session
# list`/`session info` report tabs, not panes). macOS's dispatcher DOES accept
# an omitted `id` and falls back to the active webview, so the socket reaches
# what the CLI cannot address. That gap is itself worth recording: it is why
# `coctl` subcommands land with each feature in B3/B4/B5 rather than up front.
rpc() {
    python3 - "$COPAD_SOCKET" "$1" "${2:-{\}}" <<'PY'
import json, socket, sys
sock, method, params = sys.argv[1], sys.argv[2], json.loads(sys.argv[3])
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM); s.settimeout(15)
s.connect(sock)
s.sendall((json.dumps({"id": "e2e", "method": method, "params": params}) + "\n").encode())
buf = b""
while not buf.endswith(b"\n"):
    chunk = s.recv(65536)
    if not chunk:
        break
    buf += chunk
s.close()
print(buf.decode().strip())
PY
}

RC=0

# Every instance this run launched, so the reaper below can get them all.
#
# This script starts SIX app instances and kills each one inline when its case
# is done. That is not enough on its own: a failing assertion `exit`s, and Ctrl-C
# never reaches the inline kill at all — which is how a run on 2026-09-09 left a
# debug Copad holding a temp $HOME for seventeen hours. The inline kills stay
# (they keep six GUIs from piling up during a pass); this is the backstop.
#
# A FILE, not an array: `launch` is called inside `$(...)`, a subshell, so an
# array appended there would not survive back into the parent. The record is
# written before the caller ever reads the pid, so an interrupt between the spawn
# and the read is still reapable.
LAUNCHED=$(mktemp /private/tmp/copad-e2e-pids-XXXXXX)
cleanup() {
    # Tolerates a missing record so it is safe to run more than once and safe to
    # run before the record exists at all.
    [ -n "${LAUNCHED:-}" ] && [ -f "$LAUNCHED" ] || return 0
    while read -r pid home sock; do
        [ -n "$pid" ] && kill -9 "$pid" 2>/dev/null
        [ -n "$sock" ] && rm -f "$sock"
        [ -n "$home" ] && rm -rf "$home"
    done <"$LAUNCHED"
    rm -f "$LAUNCHED"
}
# Cleanup runs on EXIT only. `trap cleanup EXIT INT TERM` looks equivalent and is
# not: bash runs a signal handler and then RESUMES the script, so the cleanup
# would delete the temp $HOME the very next command reads from and the run would
# carry on asserting against wiped state. INT/TERM therefore just `exit` with the
# conventional status, which fires the EXIT trap exactly once (codex C1).
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
SESSION_REL="Library/Application Support/copad/session.json"

# The Swift side resolves config through `homeDirectoryForCurrentUser`, which
# ignores $HOME — so this harness cannot isolate config, and the app under test
# reads the REAL `[browser] restore`. Derive expectations from it rather than
# assuming origin-only, or a user who has opted into `url`/`full` sees these
# assertions fail for behaving correctly.
RESTORE=$(python3 - "$HOME/.config/copad/config.toml" <<'PY'
import re, sys
try:
    text = open(sys.argv[1]).read()
except OSError:
    print("origin"); raise SystemExit
section = re.split(r"^\[", text, flags=re.M)
for part in section:
    if part.startswith("browser]"):
        m = re.search(r'^\s*restore\s*=\s*"([^"]*)"', part, flags=re.M)
        if m and m.group(1).strip().lower() in ("origin", "url", "full"):
            print(m.group(1).strip().lower()); raise SystemExit
print("origin")
PY
)
echo "restore policy in effect: $RESTORE"
# Under origin-only the path is dropped; under url/full it survives.
expected_url() {
    if [ "$RESTORE" = "origin" ]; then echo "$1"; else echo "$1$2"; fi
}

# A probe that fails must never compare equal to another failed probe — two
# empty strings would otherwise "match" and report PASS without measuring
# anything.
mouse() {
    local v
    v=$(osascript -l JavaScript -e 'ObjC.import("Cocoa"); var p=$.NSEvent.mouseLocation; Math.round(p.x)+","+Math.round(p.y)' 2>/dev/null)
    [[ "$v" =~ ^-?[0-9]+,-?[0-9]+$ ]] || { echo "FAIL: could not read the cursor position" >&2; return 1; }
    echo "$v"
}
front() {
    local v
    v=$(osascript -e 'tell application "System Events" to name of first application process whose frontmost is true' 2>/dev/null)
    [ -n "$v" ] || { echo "FAIL: could not read the frontmost application" >&2; return 1; }
    echo "$v"
}

# Launch an isolated instance and echo its pid + socket.
launch() {
    local home="$1"
    mkdir -p "$home/Library/Application Support/copad"
    HOME="$home" "$APP" >"$home/app.log" 2>&1 &
    local pid=$!
    local sock="/tmp/copad-$pid.sock"
    # Recorded before the socket wait, so an instance that never came up is
    # reaped too rather than surviving as an orphan with no socket to find it by.
    echo "$pid $home $sock" >>"$LAUNCHED"
    for _ in $(seq 1 40); do [ -S "$sock" ] && break; sleep 0.25; done
    [ -S "$sock" ] || { echo "FAIL: no gui socket at $sock"; tail -5 "$home/app.log"; kill -9 "$pid" 2>/dev/null; return 1; }
    echo "$pid $sock"
}

# check_pane <session.json> <expected-url> [expected-tab-id]
# Prints the tab id it found on the last line so the caller can compare it
# across a restart.
check_pane() {
    python3 - "$1" "$2" "${3:-}" <<'PY'
import json, sys
path, expect = sys.argv[1], sys.argv[2]
expect_id = sys.argv[3] if len(sys.argv) > 3 else ""
try:
    doc = json.load(open(path))
except Exception as e:
    print("FAIL: no readable session.json:", e); sys.exit(1)
def leaves(n):
    if n.get("type") == "leaf":
        yield n["content"]
    else:
        yield from leaves(n["first"]); yield from leaves(n["second"])
wvs = [c for t in doc["tabs"] for c in leaves(t["root"]) if c["kind"] == "webview"]
if not wvs:
    print("FAIL: no webview pane persisted"); sys.exit(1)
w = wvs[0]
print("  url  =", repr(w["url"]))
print("  pane =", json.dumps(w.get("pane")))
ok = True
if w["url"] != expect:
    print(f"  FAIL: expected {expect!r}, got {w['url']!r}"); ok = False
pane = w.get("pane")
if pane is None:
    print("  FAIL: no browser pane snapshot"); ok = False
else:
    tabs = pane["tabs"]
    if len(tabs) != 1:
        print("  FAIL: expected 1 tab, got", len(tabs)); ok = False
    elif not tabs[0].get("id"):
        print("  FAIL: tab has no stable id"); ok = False
    elif tabs[0]["url"] != w["url"]:
        print("  FAIL: pane url and tab url disagree"); ok = False
    elif expect_id and tabs[0]["id"] != expect_id:
        print(f"  FAIL: tab id changed across restart: {expect_id} -> {tabs[0]['id']}"); ok = False
    else:
        note = " (unchanged across restart)" if expect_id else ""
        print("  PASS: tab id =", tabs[0]["id"], " profile =", pane["profile"], note, sep="")
        print("TABID=" + tabs[0]["id"])
sys.exit(0 if ok else 1)
PY
}

# Last TABID= line emitted by check_pane, or empty.
tabid_of() { grep '^TABID=' <<<"$1" | tail -1 | cut -d= -f2; }

# --- 1. a page that loads normally, and its id survives a restart ----------
echo "[1/5] a browser pane persists a tab snapshot whose id survives a restart"
H1=$(mktemp -d /private/tmp/copad-e2e-XXXXXX)
read -r PID1 SOCK1 <<<"$(launch "$H1")" || exit 1
export COPAD_SOCKET="$SOCK1"; sleep 1
BASE_FRONT=$(front) || RC=1
BASE_MOUSE=$(mouse) || RC=1
"$COCTL" webview open "https://example.com" --mode tab >/dev/null 2>&1 \
    || { echo "  FAIL: webview open failed"; RC=1; }
sleep 2
# WebKit normalizes a bare origin to include the root path, so under `url`/`full`
# the persisted value carries a trailing slash.
OUT1=$(check_pane "$H1/$SESSION_REL" "$(expected_url https://example.com /)") || RC=1
echo "$OUT1" | grep -v '^TABID='
ID1=$(tabid_of "$OUT1")

# --- 2. focus + cursor are not taken ---------------------------------------
echo "[2/5] agent-driven commands take neither focus nor the cursor"
DROVE=0
drive() {
    local out
    out=$(rpc "$1" "${2:-{\}}" 2>/dev/null)
    if grep -q '"ok":true' <<<"$out"; then
        DROVE=$((DROVE + 1))
    else
        echo "  FAIL: $1 did not succeed — a command that never ran proves nothing: $out"; RC=1
    fi
}
drive webview.page_info
drive webview.query '{"selector":"h1"}'
# NOT `a`: example.com's only link goes to iana.org, and now that a navigation
# schedules a save, clicking it would race the restart assertion below into
# persisting the wrong origin. `h1` exercises the same click path and stays put.
drive webview.click '{"selector":"h1"}'
drive webview.screenshot
NOW_FRONT=$(front) || RC=1
NOW_MOUSE=$(mouse) || RC=1
if [ "$DROVE" -eq 4 ] && [ -n "$BASE_FRONT" ] && [ -n "$BASE_MOUSE" ] \
   && [ "$BASE_FRONT" = "$NOW_FRONT" ] && [ "$BASE_MOUSE" = "$NOW_MOUSE" ]; then
    echo "  PASS: frontmost=$NOW_FRONT cursor=$NOW_MOUSE unchanged across $DROVE commands"
else
    echo "  FAIL: drove=$DROVE/4, frontmost $BASE_FRONT->$NOW_FRONT, cursor $BASE_MOUSE->$NOW_MOUSE"; RC=1
fi

# Restart against the SAME home. A tab id that is reissued on every restore
# would still have passed the nonempty check above; only this catches it.
kill -TERM "$PID1" 2>/dev/null; sleep 1; kill -9 "$PID1" 2>/dev/null; rm -f "$SOCK1"
read -r PID1B SOCK1B <<<"$(launch "$H1")" || exit 1
export COPAD_SOCKET="$SOCK1B"; sleep 3
# Nudge a save so the restored pane is written back.
"$COCTL" webview open "https://example.org" --mode tab >/dev/null 2>&1
sleep 2
if [ -n "$ID1" ]; then
    OUT1B=$(check_pane "$H1/$SESSION_REL" "$(expected_url https://example.com /)" "$ID1") || RC=1
    echo "$OUT1B" | grep -v '^TABID='
else
    echo "  FAIL: no tab id captured before the restart"; RC=1
fi
kill -9 "$PID1B" 2>/dev/null; rm -f "$SOCK1B"; rm -rf "$H1"

# --- 3. a page that never loads --------------------------------------------
echo "[3/5] a pane whose page never loads still persists its URL"
H2=$(mktemp -d /private/tmp/copad-e2e-XXXXXX)
read -r PID2 SOCK2 <<<"$(launch "$H2")" || exit 1
export COPAD_SOCKET="$SOCK2"; sleep 1
# Unroutable: WKWebView never produces a url, so the snapshot must come from
# the pending restore URL or the pane is written out as "" and lost.
"$COCTL" webview open "https://10.255.255.1/never-loads" --mode tab >/dev/null 2>&1 \
    || { echo "  FAIL: webview open failed"; RC=1; }
sleep 2.5
OUT2=$(check_pane "$H2/$SESSION_REL" "$(expected_url https://10.255.255.1 /never-loads)") || RC=1
echo "$OUT2" | grep -v '^TABID='
kill -9 "$PID2" 2>/dev/null; rm -f "$SOCK2"; rm -rf "$H2"

# --- 3b. the same guarantee via navigate, not just via open ----------------
echo "[3b/5] a BLANK pane navigated to an unloadable URL also persists it"
H2B=$(mktemp -d /private/tmp/copad-e2e-XXXXXX)
read -r PID2B SOCK2B <<<"$(launch "$H2B")" || exit 1
export COPAD_SOCKET="$SOCK2B"; sleep 1
# Open with NO url (blank placeholder), then navigate. This reaches the erasure
# by a different route than [3/5]: `pendingURL` is seeded at init there, so only
# this covers `navigate(to:)`.
"$COCTL" webview open "about:blank" --mode tab >/dev/null 2>&1
sleep 1
drive_ok=$(rpc webview.navigate '{"url":"https://10.255.255.2/never-loads"}')
grep -q '"ok":true' <<<"$drive_ok" || { echo "  FAIL: navigate rejected: $drive_ok"; RC=1; }
sleep 2.5
OUT2B=$(check_pane "$H2B/$SESSION_REL" "$(expected_url https://10.255.255.2 /never-loads)") || RC=1
echo "$OUT2B" | grep -v '^TABID='
kill -9 "$PID2B" 2>/dev/null; rm -f "$SOCK2B"; rm -rf "$H2B"

# --- 4. wired-but-unimplemented methods say so ------------------------------
echo "[4/5] not-yet-built methods answer unsupported_capability, not unknown_method"
H3=$(mktemp -d /private/tmp/copad-e2e-XXXXXX)
read -r PID3 SOCK3 <<<"$(launch "$H3")" || exit 1
export COPAD_SOCKET="$SOCK3"; sleep 1
# The list shrinks as the units land — that is the point of registering them.
# Only the profile surface is still unbuilt.
for m in webview.profile.list webview.profile.clear; do
    OUT=$(rpc "$m")
    if grep -q '"unsupported_capability"' <<<"$OUT"; then
        echo "  PASS: $m -> unsupported_capability"
    else
        echo "  FAIL: $m -> $OUT"; RC=1
    fi
done
# And the case that motivated answering "not built" BEFORE the policy gate:
# without that ordering `browser.secret.fill` would have said
# `requires_protected` and pointed the caller at `webview.tab.protect`, which
# was itself unimplemented. Now that both are built, the refusal is the
# genuinely useful one.
OUT=$(rpc browser.secret.fill '{"credential_id":"nobody/nothing"}')
if grep -q "requires_protected" <<<"$OUT"; then
    echo "  PASS: browser.secret.fill on an unprotected tab -> requires_protected"
else
    echo "  FAIL: browser.secret.fill -> $OUT"; RC=1
fi
kill -9 "$PID3" 2>/dev/null; rm -f "$SOCK3"; rm -rf "$H3"

# --- 5. a navigation the PAGE starts is persisted too -----------------------
echo "[5/5] a page-initiated navigation updates the persisted URL"
H4=$(mktemp -d /private/tmp/copad-e2e-XXXXXX)
read -r PID4 SOCK4 <<<"$(launch "$H4")" || exit 1
export COPAD_SOCKET="$SOCK4"; sleep 1
"$COCTL" webview open "https://example.com" --mode tab >/dev/null 2>&1
sleep 3
# A navigation the PAGE starts, via `location.href` — it never passes through
# navigate()/goBack()/reload(), so it is the one that exercises
# `didStartProvisionalNavigation` adopting a navigation this controller did not
# initiate. Deliberately NOT a click on example.com's own link: WebKit follows
# that synthetic click only sometimes, which made the assertion depend on the
# browser's mood rather than on the behaviour under test.
GO=$(rpc webview.execute_js '{"code":"location.href = \"https://example.org/\"; \"ok\""}')
grep -q '"ok":true' <<<"$GO" || { echo "  FAIL: execute_js rejected: $GO"; RC=1; }

persisted_url() {
    python3 - "$1" <<'PY'
import json, sys
try:
    doc = json.load(open(sys.argv[1]))
except Exception:
    print(""); raise SystemExit
def leaves(n):
    if n.get("type") == "leaf":
        yield n["content"]
    else:
        yield from leaves(n["first"]); yield from leaves(n["second"])
for c in (c for t in doc["tabs"] for c in leaves(t["root"]) if c["kind"] == "webview"):
    print(c["url"]); break
else:
    print("")
PY
}

# POLL rather than sleep: this assertion depends on a real page load over the
# network, and a fixed wait makes it pass or fail on latency rather than on the
# behaviour under test.
FOUND=""
for _ in $(seq 1 30); do
    FOUND=$(persisted_url "$H4/$SESSION_REL")
    case "$FOUND" in
        ""|https://example.com|https://example.com/) sleep 0.5 ;;
        *) break ;;
    esac
done
case "$FOUND" in
    ""|https://example.com|https://example.com/)
        echo "  FAIL: still persisting ${FOUND:-<nothing>} 15s after a page-initiated navigation"; RC=1 ;;
    *)
        echo "  PASS: followed the page-initiated navigation to $FOUND" ;;
esac
kill -9 "$PID4" 2>/dev/null; rm -f "$SOCK4"; rm -rf "$H4"

[ $RC -eq 0 ] && echo "ALL PASS" || echo "FAILURES (rc=$RC)"
exit $RC
