#!/usr/bin/env python3
"""Verify OCI descriptors and required rootfs contents in a built archive."""
import hashlib
import io
import json
import sys
import tarfile

with tarfile.open(sys.argv[1]) as archive:
    def read(name):
        return archive.extractfile(name).read()
    def check(descriptor):
        data = read('blobs/sha256/' + descriptor['digest'].split(':', 1)[1])
        assert len(data) == descriptor['size']
        assert 'sha256:' + hashlib.sha256(data).hexdigest() == descriptor['digest']
        return data
    assert json.loads(read('oci-layout'))['imageLayoutVersion'] == '1.0.0'
    index = json.loads(read('index.json'))
    manifest = json.loads(check(index['manifests'][0]))
    config = json.loads(check(manifest['config']))
    assert config['os'] == 'linux'
    assert config['config']['Entrypoint'] == ['/stufftracker']
    layer = check(manifest['layers'][0])
    assert config['rootfs']['diff_ids'] == [manifest['layers'][0]['digest']]
    with tarfile.open(fileobj=io.BytesIO(layer)) as rootfs:
        assert rootfs.getmember('stufftracker').mode & 0o111
        assert rootfs.extractfile('stufftracker').read(4) == b'\x7fELF'
        assert rootfs.getmember('tmp').mode == 0o1777
        assert not any(name.startswith('/') or '..' in name.split('/') for name in rootfs.getnames())
print('OCI archive verified: manifest/config/layer hashes, Linux entrypoint, ELF binary and rootfs paths.')
