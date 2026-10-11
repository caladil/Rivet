"""Prove a Rivet's HTTPS pages and passkeys with a THROWAWAY browser.

ADR-063 section 9 step 5's proof, run by an agent rather than on anybody's
daily-use devices: nothing here installs a certificate anywhere, touches a
real browser profile, or uses a real authenticator.

What it does, in order:

1. **TLS, verified against the fleet's root and nothing else.** For every
   name given, Python's OpenSSL connects to the machine's address, verifies
   the chain against the root file ONLY (no system store) and checks the
   name, which applies the root's name constraint as any verifier does.
   This is `curl --cacert` without depending on which TLS backend a curl
   was built with.
2. **A headless Chromium-family browser in a temporary profile** that
   trusts this machine's certificate for this process only
   (`--ignore-certificate-errors-spki-list` with the leaf's public-key hash,
   which step 1 read off the verified chain), resolves the names to the
   given address (`--host-resolver-rules`), and carries a DevTools WebAuthn
   **virtual authenticator** (internal transport, resident keys, user
   verification). The profile directory is deleted afterwards.
3. `probe`: open `/.rivet/login` at each name, and report whether the page
   is a secure context and whether WebAuthn may run there.
   `enrol`: with a one-time code (`cargo rivet run <machine> -- apply
   passkey-code confirm`), enroll the virtual authenticator's passkey at the
   first name, then sign out of the session (a fresh page) and sign in with
   it — the whole ceremony, twice over.

It needs no third-party package: the DevTools websocket is spoken with the
standard library. Run it with `uv run tools/proof/passkey-browser.py --help`.

An enrolled virtual passkey is a real line in the machine's `passkeys`
file, and it outlives this run's authenticator. `enrol` says which line so it
can be removed.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
import shutil
import socket
import ssl
import struct
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path

BROWSERS = [
    r"C:\Program Files\Google\Chrome\Application\chrome.exe",
    r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
    r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/usr/bin/chromium",
    "/usr/bin/google-chrome",
]


# --------------------------------------------------------------------------
# DER, just enough to find a certificate's SubjectPublicKeyInfo
# --------------------------------------------------------------------------


def _tlv(data: bytes, at: int) -> tuple[int, int, int]:
    """(tag, start of value, end of value) of the TLV at `at`."""
    tag = data[at]
    length = data[at + 1]
    at += 2
    if length & 0x80:
        count = length & 0x7F
        length = int.from_bytes(data[at : at + count], "big")
        at += count
    return tag, at, at + length


def spki_of(der: bytes) -> bytes:
    """The SubjectPublicKeyInfo TLV of an X.509 certificate."""
    _, cert, _ = _tlv(der, 0)
    _, tbs, _ = _tlv(der, cert)
    at = tbs
    tag, _, end = _tlv(der, at)
    if tag == 0xA0:  # [0] version
        at = end
    # serial, signature, issuer, validity, subject
    for _ in range(5):
        _, _, at = _tlv(der, at)
    _, _, end = _tlv(der, at)
    return der[at:end]


def spki_hash(der: bytes) -> str:
    return base64.b64encode(hashlib.sha256(spki_of(der)).digest()).decode()


def root_file(path: Path) -> Path:
    """A PEM file Python's ssl will read, from a PEM or DER root."""
    data = path.read_bytes()
    if b"BEGIN CERTIFICATE" in data:
        return path
    pem = ssl.DER_cert_to_PEM_cert(data)
    out = Path(tempfile.mkstemp(suffix=".pem")[1])
    out.write_text(pem)
    return out


# --------------------------------------------------------------------------
# Step 1: TLS against the root alone
# --------------------------------------------------------------------------


def verify_tls(root: Path, name: str, address: str, port: int) -> bytes:
    """Verify `name` at `address` under `root` alone; the leaf's DER."""
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
    context.load_verify_locations(cafile=str(root))
    context.check_hostname = True
    context.verify_mode = ssl.CERT_REQUIRED
    with socket.create_connection((address, port), timeout=10) as raw:
        with context.wrap_socket(raw, server_hostname=name) as tls:
            return tls.getpeercert(binary_form=True)


# --------------------------------------------------------------------------
# A DevTools websocket, standard library only
# --------------------------------------------------------------------------


class DevTools:
    def __init__(self, url: str) -> None:
        rest = url.split("://", 1)[1]
        hostport, path = rest.split("/", 1)
        host, port = hostport.split(":")
        self.sock = socket.create_connection((host, int(port)), timeout=30)
        key = base64.b64encode(os.urandom(16)).decode()
        self.sock.sendall(
            (
                f"GET /{path} HTTP/1.1\r\nHost: {hostport}\r\nUpgrade: websocket\r\n"
                f"Connection: Upgrade\r\nSec-WebSocket-Key: {key}\r\n"
                "Sec-WebSocket-Version: 13\r\n\r\n"
            ).encode()
        )
        head = b""
        while b"\r\n\r\n" not in head:
            head += self.sock.recv(1)
        if b" 101 " not in head.split(b"\r\n", 1)[0]:
            raise RuntimeError(f"the browser refused the websocket: {head!r}")
        self.next_id = 0
        self.events: list[dict] = []

    def _send(self, text: str) -> None:
        payload = text.encode()
        mask = os.urandom(4)
        header = bytes([0x81])
        n = len(payload)
        if n < 126:
            header += bytes([0x80 | n])
        elif n < 65536:
            header += bytes([0x80 | 126]) + struct.pack(">H", n)
        else:
            header += bytes([0x80 | 127]) + struct.pack(">Q", n)
        masked = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
        self.sock.sendall(header + mask + masked)

    def _exact(self, n: int) -> bytes:
        out = b""
        while len(out) < n:
            chunk = self.sock.recv(n - len(out))
            if not chunk:
                raise RuntimeError("the browser closed the websocket")
            out += chunk
        return out

    def _receive(self) -> str:
        message = b""
        while True:
            first, second = self._exact(2)
            n = second & 0x7F
            if n == 126:
                n = struct.unpack(">H", self._exact(2))[0]
            elif n == 127:
                n = struct.unpack(">Q", self._exact(8))[0]
            message += self._exact(n)
            if first & 0x80:
                return message.decode()

    def call(self, method: str, params: dict | None = None, session: str | None = None) -> dict:
        self.next_id += 1
        mine = self.next_id
        body = {"id": mine, "method": method, "params": params or {}}
        if session:
            body["sessionId"] = session
        self._send(json.dumps(body))
        while True:
            reply = json.loads(self._receive())
            if reply.get("id") == mine:
                if "error" in reply:
                    raise RuntimeError(f"{method}: {reply['error']}")
                return reply.get("result", {})
            self.events.append(reply)


def launch(browser: str, profile: Path, rules: str, spkis: list[str]) -> subprocess.Popen:
    args = [
        browser,
        "--headless=new",
        f"--user-data-dir={profile}",
        "--remote-debugging-port=0",
        "--no-first-run",
        "--no-default-browser-check",
        f"--host-resolver-rules={rules}",
        f"--ignore-certificate-errors-spki-list={','.join(spkis)}",
        "about:blank",
    ]
    return subprocess.Popen(args, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def devtools_url(profile: Path) -> str:
    marker = profile / "DevToolsActivePort"
    for _ in range(150):
        if marker.exists():
            lines = marker.read_text().split()
            if len(lines) >= 2:
                return f"ws://127.0.0.1:{lines[0]}{lines[1]}"
        time.sleep(0.1)
    raise RuntimeError("the browser never opened its DevTools port")


def evaluate(tools: DevTools, session: str, expression: str) -> object:
    result = tools.call(
        "Runtime.evaluate",
        {"expression": expression, "awaitPromise": True, "returnByValue": True},
        session,
    )
    if "exceptionDetails" in result:
        raise RuntimeError(json.dumps(result["exceptionDetails"])[:400])
    return result.get("result", {}).get("value")


def open_page(tools: DevTools, session: str, url: str) -> None:
    # Navigated from inside the page rather than with Page.navigate, which can
    # hold its answer until the load settles; the load is polled below.
    evaluate(tools, session, f"location.href={json.dumps(url)};'navigating'")
    state = ""
    for _ in range(150):
        time.sleep(0.2)
        try:
            state = evaluate(tools, session, "document.readyState + ' ' + location.href")
        except RuntimeError:
            continue
        if isinstance(state, str) and state.startswith("complete") and url.split("?")[0] in state:
            return
    raise RuntimeError(f"{url} did not finish loading (last: {state})")


def said(tools: DevTools, session: str, until: str, seconds: float = 20.0) -> str:
    deadline = time.time() + seconds
    text = ""
    while time.time() < deadline:
        text = evaluate(
            tools,
            session,
            "(document.getElementById('said')||{textContent:''}).textContent + ' @ ' + location.pathname",
        ) or ""
        if until in text or " @ /" == text[-4:] or text.endswith("@ /"):
            return text
        if "bad" == evaluate(tools, session, "(document.getElementById('said')||{}).className||''"):
            return text
        time.sleep(0.25)
    return text


# --------------------------------------------------------------------------


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("mode", choices=["probe", "enroll"])
    parser.add_argument("--root", required=True, type=Path, help="the fleet's root, PEM or DER")
    parser.add_argument("--address", required=True, help="the machine's IP address")
    parser.add_argument("--name", action="append", required=True, help="a name on its certificate; repeat")
    parser.add_argument("--port", type=int, default=443)
    parser.add_argument("--code", help="enroll: the one-time code from apply passkey-code")
    parser.add_argument("--browser", help="a Chromium-family browser; found if omitted")
    parser.add_argument("--keep-profile", action="store_true")
    args = parser.parse_args()

    root = root_file(args.root)
    failures = 0
    spkis: list[str] = []
    print("-- TLS, verified against the root alone")
    for name in args.name:
        try:
            leaf = verify_tls(root, name, args.address, args.port)
            spkis.append(spki_hash(leaf))
            print(f"  VERIFIED  {name}  (leaf SHA-256 {hashlib.sha256(leaf).hexdigest()})")
        except Exception as e:  # noqa: BLE001 - every failure is reported, none hidden
            failures += 1
            print(f"  REFUSED   {name}: {e}")
    if not spkis:
        print("nothing verified; the browser was not started")
        return 1

    browser = args.browser or next((b for b in BROWSERS if Path(b).exists()), None)
    if not browser:
        print("no Chromium-family browser found; pass --browser")
        return 1
    profile = Path(tempfile.mkdtemp(prefix="rivet-passkey-proof-"))
    rules = ",".join(f"MAP {n} {args.address}" for n in args.name)
    process = launch(browser, profile, rules, sorted(set(spkis)))
    try:
        tools = DevTools(devtools_url(profile))
        target = tools.call("Target.createTarget", {"url": "about:blank"})["targetId"]
        session = tools.call("Target.attachToTarget", {"targetId": target, "flatten": True})["sessionId"]
        tools.call("Page.enable", {}, session)
        tools.call("Runtime.enable", {}, session)
        tools.call("WebAuthn.enable", {"enableUI": False}, session)
        authenticator = tools.call(
            "WebAuthn.addVirtualAuthenticator",
            {
                "options": {
                    "protocol": "ctap2",
                    "transport": "internal",
                    "hasResidentKey": True,
                    "hasUserVerification": True,
                    "isUserVerified": True,
                    "automaticPresenceSimulation": True,
                }
            },
            session,
        )["authenticatorId"]
        print(f"-- browser {Path(browser).name}, throwaway profile, virtual authenticator {authenticator}")

        if args.mode == "probe":
            for name in args.name:
                open_page(tools, session, f"https://{name}/.rivet/login")
                secure = evaluate(tools, session, "window.isSecureContext")
                webauthn = evaluate(tools, session, "typeof PublicKeyCredential")
                attempt = evaluate(
                    tools,
                    session,
                    "navigator.credentials.get({publicKey:{challenge:new Uint8Array(32),"
                    "rpId:location.hostname,timeout:3000,userVerification:'required'}})"
                    ".then(()=>'answered').catch(e=>e.name)",
                )
                ok = secure is True and webauthn == "function" and attempt != "SecurityError"
                failures += 0 if ok else 1
                print(
                    f"  {'OK ' if ok else 'BAD'}  {name}: secure context {secure}, WebAuthn {webauthn}, "
                    f"a get() with no credential ended {attempt}"
                )
        else:
            if not args.code:
                print("enroll needs --code")
                return 1
            name = args.name[0]
            open_page(tools, session, f"https://{name}/.rivet/enrol")
            evaluate(
                tools,
                session,
                f"document.getElementById('code').value={json.dumps(args.code)};"
                "document.getElementById('go').click();'clicked'",
            )
            text = said(tools, session, "Enrolled")
            enrolled = "Enrolled" in text or text.endswith("@ /")
            print(f"  {'OK ' if enrolled else 'BAD'}  enroll at {name}: {text}")
            credentials = tools.call("WebAuthn.getCredentials", {"authenticatorId": authenticator}, session)
            ids = [c.get("credentialId") for c in credentials.get("credentials", [])]
            print(f"  the virtual authenticator holds {len(ids)} credential(s): {ids}")
            failures += 0 if enrolled else 1

            # A fresh session: clear the cookie, then sign in with the passkey.
            tools.call("Network.enable", {}, session)
            tools.call("Network.clearBrowserCookies", {}, session)
            open_page(tools, session, f"https://{name}/.rivet/login")
            evaluate(tools, session, "document.getElementById('go').click();'clicked'")
            text = said(tools, session, "Signed in")
            signed = text.endswith("@ /") or "Signed in" in text
            print(f"  {'OK ' if signed else 'BAD'}  sign in at {name}: {text}")
            failures += 0 if signed else 1
            if ids:
                print(
                    "  NOTE: the passkey line for this virtual credential now rests in the machine's "
                    "passkeys file; remove it when the proof is no longer wanted."
                )
    except TimeoutError:
        failures += 1
        print(
            "  STALLED: the browser stopped answering while a page loaded. The cause measured on "
            "the Windows workstation: the proxy asks for a client certificate, and when this "
            "operating system's certificate store holds one its root issued, Chromium waits for a "
            "selection a headless browser cannot make. A machine whose root no installed client "
            "certificate chains to does not ask that question of this store."
        )
    finally:
        process.terminate()
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
        if not args.keep_profile:
            shutil.rmtree(profile, ignore_errors=True)
    print("PROVEN" if failures == 0 else f"{failures} CHECK(S) FAILED")
    return 0 if failures == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
