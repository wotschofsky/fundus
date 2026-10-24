#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
mkdir -p .cache/codec-tests
if command -v brew >/dev/null; then
 flac_root="$(brew --prefix flac)"
 cc -std=c11 -D_DARWIN_C_SOURCE -Inative/audio/include -I"$flac_root/include" native/audio/vault_audio.c native/audio/test_codec.c -L"$flac_root/lib" -lFLAC -o .cache/codec-tests/codec
else
 cc -std=c11 -D_POSIX_C_SOURCE=200809L -Inative/audio/include native/audio/vault_audio.c native/audio/test_codec.c -lFLAC -o .cache/codec-tests/codec
fi
python3 - <<'PY'
import pathlib, struct, subprocess, hashlib
root=pathlib.Path('.cache/codec-tests')
for bits,floating in [(8,False),(16,False),(24,False),(32,False),(32,True)]:
 raw=bytearray()
 for i in range(16000):
  for channel in range(2):
   if floating: raw.extend(struct.pack('<f',(i%97-48)/31))
   elif bits==8: raw.append(i%256)
   else:
    n=(-1 if channel else 1)*(i*379%2**(bits-1)); raw.extend(n.to_bytes(bits//8,'little',signed=True))
 header=b'RIFF'+struct.pack('<I',36+len(raw))+b'WAVEfmt '+struct.pack('<IHHIIHH',16,3 if floating else 1,2,16000,16000*2*bits//8,2*bits//8,bits)+b'data'+struct.pack('<I',len(raw))
 source=root/f'{bits}-{floating}.wav'; destination=root/f'{bits}-{floating}.prepared'
 source.write_bytes(header+raw); destination.unlink(missing_ok=True)
 subprocess.run([str(root/'codec'),str(source),str(destination)],check=True)
 if floating or bits==32: assert source.read_bytes()==destination.read_bytes()
 else:
  decode=lambda path:subprocess.check_output(['ffmpeg','-v','error','-i',str(path),'-f','s32le','-'])
  assert decode(source)==decode(destination), (bits,floating)
 assert source.read_bytes()==header+raw
print('Native codec: 8/16/24-bit PCM exact; float/int32 byte exact; originals unchanged.')
PY
