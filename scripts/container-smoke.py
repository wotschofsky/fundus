#!/usr/bin/env python3
"""Exercise the shipped image on an internal Docker network using synthetic data.

Usage: python3 scripts/container-smoke.py fundus:local
Only the fixture script is mounted. No account data or real API keys are used.
The same file serves the mock provider and runs the HTTP client in Python containers.
"""

import base64
import hashlib
import http.cookiejar
import http.server
import io
import json
import math
import pathlib
import struct
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
import wave

PASSWORD = "synthetic-container-password"
BASE = "http://vault:3000"
FIXTURE_IMAGE = "python:3.12-slim-bookworm"
# Proxy mode: the mock stands in for the Cloudflare Access team, and the client signs its tokens
# with this throwaway RSA key (made for the tests; it signs nothing real).
ACCESS_TEAM = "http://provider:3119"
ACCESS_AUD = "smoke-audience"
ACCESS_E = 65537
ACCESS_N = int(
    "da4ddae11b5c1b41559163e0b86ae6e93f5f512081bd30b0e3cbd7ff53aa4c841e40571a92e5c6d7cd17b87c"
    "2f5f83b5006a653ff8cd7c62c7a7b6838b4e5d22d8c78ca131c7304ef7d374441ebaf79cc9ff63c7d8b83942"
    "fcd61a082eb0cb5e489d09d3b83bbc838fd919875621648707c6ec265854ca29c6cead471a9f129a0bd251d5"
    "07cf874b3215fe8cdec68c242c2b08b9085dbe09bfbbe917b9f80fb71745bfed31f2d1ff244c76c5fe8fbafc"
    "9a5f68d0762161cd9854d2bdba558e4d3d37c6ebecea18654263b399bc9f17d662402aec52f6992e0a8eb4c2"
    "c706ebab56a83f2f6547b66ccd916232cd9280a19269a13ced09c2be14d19e6f189c4985",
    16,
)
ACCESS_D = int(
    "21ba4044372debd2bef2fa3365f55e3cfb303175e1959148431438b2ecace0557e8c57937623fa0202b5d4d2"
    "f0496ed7dad0955964df4530731853a279e0b43db0b063e723ac5aa7ae07b016e0dde769049713461fbd96b0"
    "9024f95b4f9b66503d77c5ba3d8f3d238bbdf1abc480343fddd110a5d14fdda1716322725c87e71f8c912292"
    "83f7180629f8358288406aa3f6bd6db2d65ba578461a7b1a92ee23865eab4290e7228e91ade5c2091193bb4e"
    "23b70e5fca2b512a2f85d5bf97e8b3465a2cc749ced2c7d22717c8275e73b48f1c2173d5ab5e823e6e442b43"
    "c8d4b5b245464f637b105665aff0abc3222d383f158e7cfe7c8020b40ac9bc23bfec71a1",
    16,
)


def b64url(data):
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode()


def access_token():
    """A token as Cloudflare Access signs one in (RS256, PKCS#1 v1.5)."""
    now = int(time.time())
    signed = ".".join(b64url(json.dumps(part).encode()) for part in (
        {"alg": "RS256", "kid": "smoke", "typ": "JWT"},
        {"iss": ACCESS_TEAM, "aud": [ACCESS_AUD], "iat": now, "nbf": now, "exp": now + 600,
         "email": "owner@example.com", "type": "app"}))
    digest = bytes.fromhex("3031300d060960864801650304020105000420") + hashlib.sha256(
        signed.encode()).digest()
    size = (ACCESS_N.bit_length() + 7) // 8
    padded = b"\x00\x01" + b"\xff" * (size - len(digest) - 3) + b"\x00" + digest
    signature = pow(int.from_bytes(padded, "big"), ACCESS_D, ACCESS_N).to_bytes(size, "big")
    return signed + "." + b64url(signature)


class Provider(http.server.BaseHTTPRequestHandler):
    """AssemblyAI and OpenAI (chat completions, embeddings) stand-in. Jobs complete on their second poll."""
    polls = {}

    def log_message(self, *_args):
        pass

    def reply(self, result):
        response = json.dumps(result).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(response)))
        self.end_headers()
        self.wfile.write(response)

    def do_GET(self):
        if self.path == "/cdn-cgi/access/certs":
            size = (ACCESS_N.bit_length() + 7) // 8
            self.reply({"keys": [{"kid": "smoke", "kty": "RSA", "alg": "RS256", "use": "sig",
                                  "n": b64url(ACCESS_N.to_bytes(size, "big")),
                                  "e": b64url(ACCESS_E.to_bytes(3, "big"))}]})
            return
        if not self.path.startswith("/v2/transcript/"):
            self.send_error(404)
            return
        job = self.path.rsplit("/", 1)[1]
        Provider.polls[job] = Provider.polls.get(job, 0) + 1
        if Provider.polls[job] < 2:
            self.reply({"id": job, "status": "processing"})
            return
        self.reply({"id": job, "status": "completed", "utterances": [
            {"speaker": "A", "start": 0, "end": 5000,
             "text": "I'm Morgan. The cedar launch is on Tuesday.", "confidence": 0.9},
            {"speaker": "A", "start": 6000, "end": 12000,
             "text": "We agreed to send the proposal tomorrow.", "confidence": 0.9},
        ]})

    def do_POST(self):
        body = self.rfile.read(int(self.headers.get("Content-Length", "0")))
        if self.path == "/v2/upload":
            assert body, "empty upload"
            self.reply({"upload_url": "https://cdn.example/synthetic"})
        elif self.path == "/v2/transcript":
            assert json.loads(body)["speaker_labels"] is True
            self.reply({"id": uuid.uuid4().hex, "status": "queued"})
        elif self.path.endswith("/embeddings"):
            inputs = json.loads(body)["input"]
            count = len(inputs) if isinstance(inputs, list) else 1
            self.reply({"data": [{"index": i, "embedding": [1.0, 0.0, 0.0]} for i in range(count)]})
        elif self.path.endswith("/chat/completions"):
            self.reply({"choices": [{"message": {"content": json.dumps({
                "title": "Synthetic integration recording",
                "summary": "Synthetic fixture for container verification.", "names": [],
            })}}]})
        else:
            self.send_error(404)


def exercise(restarted=False, proxy=False):
    client = urllib.request.build_opener(
        urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))

    token = {"Cf-Access-Jwt-Assertion": access_token()} if proxy else {}

    def call(path, method="GET", body=None, headers=None):
        merged = {"Origin": BASE, **token, **(headers or {})}
        merged = {name: value for name, value in merged.items() if value is not None}
        if isinstance(body, (dict, list)):
            body = json.dumps(body).encode()
            merged["Content-Type"] = "application/json"
        with client.open(urllib.request.Request(
                BASE + path, body, merged, method=method), timeout=30) as response:
            data = response.read()
            if data and response.headers.get("Content-Type", "").startswith("application/json"):
                data = json.loads(data)
            return response, data

    for attempt in range(60):
        try:
            call("/healthz")
            break
        except (OSError, urllib.error.URLError):
            if attempt == 59:
                raise
            time.sleep(1)
    _, page = call("/")
    assert b'<div id="root">' in page, "Web bundle is missing"
    if proxy:
        _, me = call("/api/v1/me")
        assert me["auth_mode"] == "proxy" and me["scope"] == "admin"
        for path, method, body, headers, expected in [
            # What reaches the server without Access's token for it is not the owner's.
            ("/api/v1/me", "GET", None, {"Cf-Access-Jwt-Assertion": None}, 401),
            ("/api/login", "POST", {"password": PASSWORD}, {}, 403),
            ("/api/v1/uploads", "OPTIONS", None, {}, 401),
            ("/mcp", "POST", {}, {}, 401),
            ("/api/v1/tokens", "POST", {"name": "blocked", "scope": "read"},
             {"Origin": "https://other.example"}, 403),
        ]:
            try:
                call(path, method, body, headers)
            except urllib.error.HTTPError as error:
                assert error.code == expected, (path, error.code)
            else:
                raise AssertionError("Authentication check unexpectedly passed: " + path)
    else:
        call("/api/login", "POST", {"password": PASSWORD})

    if not restarted:
        _, credential = call("/api/v1/tokens", "POST", {"name": "Fixture", "scope": "upload"})
        auth = {"Authorization": "Bearer " + credential["token"]}
        _, batch = call("/api/v1/imports", "POST", {"recorder_timezone": "UTC"}, auth)
        buffer = io.BytesIO()
        with wave.open(buffer, "wb") as audio_file:
            audio_file.setnchannels(1)
            audio_file.setsampwidth(2)
            audio_file.setframerate(16000)
            audio_file.writeframes(b"".join(struct.pack("<h", int(math.sin(i * .073) * 16000))
                                             for i in range(16000 * 12)))
        audio = buffer.getvalue()
        metadata = {"import_id": batch["id"], "filename": "synthetic.wav",
                    "original_name": "synthetic.wav"}
        response, _ = call("/api/v1/uploads", "POST", headers={
            **auth, "Tus-Resumable": "1.0.0", "Upload-Length": str(len(audio)),
            "Upload-Metadata": ",".join(k + " " + base64.b64encode(v.encode()).decode()
                                        for k, v in metadata.items()),
        })
        location = response.headers["Location"]
        middle = len(audio) // 2
        for offset, data in [(0, audio[:middle]), (middle, audio[middle:])]:
            head, _ = call(location, "HEAD", headers={**auth, "Tus-Resumable": "1.0.0"})
            assert int(head.headers["Upload-Offset"]) == offset
            call(location, "PATCH", data, {**auth, "Tus-Resumable": "1.0.0",
                 "Upload-Offset": str(offset), "Content-Type": "application/offset+octet-stream"})
        call("/api/v1/imports/" + batch["id"] + "/complete", "POST", headers=auth)

    entry = None
    for _ in range(120):
        _, entries = call("/api/v1/entries")
        if entries["entries"]:
            entry = entries["entries"][0]
            if entry["stage"] in ["complete", "failed"]:
                break
        time.sleep(1)
    assert entry and entry["stage"] == "complete", entry
    assert not entry["warnings"], entry["warnings"]
    if not restarted:
        _, detail = call("/api/v1/entries/" + entry["id"])
        assert len(detail["speakers"]) == 1
        _, person = call("/api/v1/people", "POST", {"name": "Integration fixture"})
        label = urllib.parse.quote(detail["speakers"][0]["label"], safe="")
        call("/api/v1/entries/" + entry["id"] + "/speakers/" + label,
             "PATCH", {"person_id": person["id"]})
    _, people = call("/api/v1/people")
    assert people["people"][0]["samples"] == 1, people
    _, result = call("/api/v1/search?q=cedar&limit=2")
    assert result["mode"] == "hybrid" and result["results"][0]["entry_id"] == entry["id"], result
    response, _ = call("/api/v1/entries/" + entry["id"] + "/audio", headers={"Range": "bytes=0-31"})
    assert response.status == 206
    print("Passed: " + ("persistent archive after restart" if restarted else
          "web bundle, tus offsets, packaged FFmpeg/ONNX, enrollment, hybrid search, audio Range"))


def run_image(image, proxy=False):
    name = "vault-smoke-" + uuid.uuid4().hex[:12]
    network, volume = name + "-net", name + "-data"
    script = str(pathlib.Path(__file__).resolve())
    fixture_mount = ["--mount", "type=bind,src=" + script + ",dst=/fixture.py,readonly"]
    containers = [name + suffix for suffix in ("-mock", "-server", "-client", "-init", "-missing")]

    def docker(*args, **kwargs):
        return subprocess.run(["docker", *args], check=True, timeout=240, **kwargs)

    restrictions = ["--read-only", "--cap-drop=ALL", "--security-opt=no-new-privileges",
                    "--tmpfs", "/tmp:rw,noexec,nosuid,size=256m,uid=10001,gid=10001,mode=0700"]
    try:
        docker("pull", FIXTURE_IMAGE)
        docker("network", "create", "--internal", network, stdout=subprocess.DEVNULL)
        docker("volume", "create", volume, stdout=subprocess.DEVNULL)
        missing = subprocess.run(["docker", "run", "--rm", "--name", name + "-missing",
                                  "--network=none", *restrictions, image, "serve"],
                                 capture_output=True, text=True, timeout=30)
        assert missing.returncode and "ASSEMBLYAI_API_KEY" in missing.stderr, missing.stderr
        # What video.rs needs from the packaged FFmpeg.
        for kind, names in (("encoders", ["libx264", "aac"]), ("decoders", ["h264", "hevc", "vp9"]),
                            ("muxers", ["mp4"]),
                            ("filters", ["scale", "setsar", "transpose", "zscale", "tonemap",
                                         "format", "hflip", "vflip", "trim", "setpts", "asetpts"])):
            listed = subprocess.run(["docker", "run", "--rm", "--network=none", *restrictions,
                                     "--entrypoint", "ffmpeg", image, "-hide_banner", "-" + kind],
                                    capture_output=True, text=True, timeout=30, check=True).stdout
            assert all(name in listed.split() for name in names), (kind, names)
        if not proxy:
            docker("run", "--rm", "-i", "--name", name + "-init", "--network=none",
                   *restrictions, "-v", volume + ":/data", image, "init",
                   input=(PASSWORD + "\n").encode())
        docker("run", "-d", "--name", name + "-mock", "--network", network,
               "--network-alias=provider", "--read-only", "--cap-drop=ALL", *fixture_mount,
               FIXTURE_IMAGE, "python3", "/fixture.py", "mock", stdout=subprocess.DEVNULL)
        docker("run", "-d", "--name", name + "-server", "--network", network,
               "--network-alias=vault", *restrictions, "-v", volume + ":/data",
               *[arg for name in ("ASSEMBLYAI", "OPENAI") for arg in (
                   "-e", name + "_API_KEY=synthetic-only", "-e", name + "_BASE_URL=http://provider:3119")],
               "-e", "VAULT_PUBLIC_URL=" + BASE,
               "-e", "VAULT_AUTH_MODE=" + ("proxy" if proxy else "password"),
               *(["-e", "VAULT_ACCESS_TEAM_DOMAIN=" + ACCESS_TEAM,
                  "-e", "VAULT_ACCESS_AUD=" + ACCESS_AUD] if proxy else []),
               image, stdout=subprocess.DEVNULL)
        for mode in ("client", "restarted"):
            if mode == "restarted":
                docker("restart", "--time=30", name + "-server", stdout=subprocess.DEVNULL)
            docker("run", "--rm", "--name", name + "-client", "--network", network,
                   "--read-only", "--cap-drop=ALL", *fixture_mount,
                   FIXTURE_IMAGE, "python3", "/fixture.py", mode, *( ["--proxy"] if proxy else []))
            docker("exec", name + "-server", "fundus", "healthcheck")
        print("Container smoke passed; no external provider requests were possible.")
    except BaseException:
        subprocess.run(["docker", "logs", name + "-server"], check=False, timeout=15)
        raise
    finally:
        for args in (("rm", "-f", *containers), ("volume", "rm", volume), ("network", "rm", network)):
            subprocess.run(["docker", *args], check=False, timeout=45,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


if __name__ == "__main__":
    mode = sys.argv[1] if len(sys.argv) > 1 else "fundus:local"
    if mode == "mock":
        http.server.ThreadingHTTPServer(("0.0.0.0", 3119), Provider).serve_forever()
    elif mode in ("client", "restarted"):
        exercise(mode == "restarted", "--proxy" in sys.argv[2:])
    else:
        run_image(mode, "--proxy" in sys.argv[2:])
