#!/usr/bin/env python3
"""Package a Linux binary and its ldd closure as an OCI archive, without a daemon.

Equivalent artifact format to Darude's pack-oci.sh. No host system packages or
Darude checkout are needed. Inputs must be locally built, trusted executables.
"""
import hashlib
import io
import json
import os
from pathlib import Path
import platform
import re
import subprocess
import sys
import tarfile

binary = Path(sys.argv[1]).resolve()
out = Path(sys.argv[2]).resolve()
architecture = {'x86_64': 'amd64', 'aarch64': 'arm64'}.get(platform.machine())
if not architecture or platform.system() != 'Linux':
    raise SystemExit('Grain packaging requires Linux amd64 or arm64')
files = {'stufftracker': binary}
ldd = subprocess.run(['ldd', str(binary)], capture_output=True, text=True)
if ldd.returncode and 'not a dynamic executable' not in ldd.stderr + ldd.stdout:
    raise SystemExit(ldd.stderr + ldd.stdout)
if 'not found' in ldd.stdout:
    raise SystemExit('Missing runtime library: ' + ldd.stdout)
for library in re.findall(r'/[^\s()]+', ldd.stdout):
    files[library.lstrip('/')] = Path(library)
layer_io = io.BytesIO()
with tarfile.open(fileobj=layer_io, mode='w', format=tarfile.PAX_FORMAT) as layer:
    directories = {'tmp', 'run', 'state'}
    for destination in files:
        directories.update(str(p) for p in Path(destination).parents if str(p) != '.')
    for name in sorted(directories, key=lambda p: (p.count('/'), p)):
        info = tarfile.TarInfo(name)
        info.type = tarfile.DIRTYPE
        info.mode = 0o1777 if name == 'tmp' else 0o755
        layer.addfile(info)
    for destination, source in sorted(files.items()):
        data = source.read_bytes()
        info = tarfile.TarInfo(destination)
        info.size = len(data)
        info.mode = 0o755
        layer.addfile(info, io.BytesIO(data))
layer = layer_io.getvalue()
blobs = {}
def blob(data, media_type):
    digest = hashlib.sha256(data).hexdigest()
    blobs[f'blobs/sha256/{digest}'] = data
    return {'mediaType': media_type, 'digest': f'sha256:{digest}', 'size': len(data)}
def encode(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':')).encode()
layer_descriptor = blob(layer, 'application/vnd.oci.image.layer.v1.tar')
config = blob(encode({'architecture': architecture, 'os': 'linux', 'config': {'Entrypoint': ['/stufftracker'], 'WorkingDir': '/'}, 'rootfs': {'type': 'layers', 'diff_ids': [layer_descriptor['digest']]}}), 'application/vnd.oci.image.config.v1+json')
manifest = blob(encode({'schemaVersion': 2, 'mediaType': 'application/vnd.oci.image.manifest.v1+json', 'config': config, 'layers': [layer_descriptor]}), 'application/vnd.oci.image.manifest.v1+json')
manifest['annotations'] = {'org.opencontainers.image.ref.name': 'darude.local/stufftracker'}
index = encode({'schemaVersion': 2, 'manifests': [manifest]})
with tarfile.open(out, 'w') as archive:
    for name, data in {'oci-layout': encode({'imageLayoutVersion': '1.0.0'}), 'index.json': index, **blobs}.items():
        info = tarfile.TarInfo(name)
        info.size = len(data)
        info.mode = 0o644
        archive.addfile(info, io.BytesIO(data))
print(f'{out}: {manifest["digest"]} ({out.stat().st_size // 1024} KiB; {len(files)} executables/libraries)')
