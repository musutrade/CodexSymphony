"""Versioned streaming AES-256-GCM envelope. Keys are separate recovery custody."""
import os
from pathlib import Path
from cryptography.hazmat.primitives.ciphers import Cipher, algorithms, modes
from common import private, require

MAGIC = b'SYMPHONY-BACKUP-v1\n'
CHUNK = 1024 * 1024


def key(path):
    value = private(path).read_bytes()
    require(len(value) == 32, '32-byte encryption key required')
    return value


def encrypt(source, target, secret):
    nonce = os.urandom(12)
    cipher = Cipher(algorithms.AES(secret), modes.GCM(nonce)).encryptor()
    cipher.authenticate_additional_data(MAGIC)
    with Path(source).open('rb') as src, Path(target).open('xb') as dst:
        dst.write(MAGIC + nonce)
        while block := src.read(CHUNK):
            dst.write(cipher.update(block))
        dst.write(cipher.finalize())
        dst.write(cipher.tag)
        dst.flush()
        os.fsync(dst.fileno())


def decrypt(source, target, secret):
    size = Path(source).stat().st_size
    require(size >= len(MAGIC) + 28, 'truncated encrypted backup')
    with private(source).open('rb') as src, Path(target).open('xb') as dst:
        require(src.read(len(MAGIC)) == MAGIC, 'unsupported backup envelope')
        nonce = src.read(12)
        src.seek(-16, 2)
        tag = src.read(16)
        src.seek(len(MAGIC) + 12)
        cipher = Cipher(algorithms.AES(secret), modes.GCM(nonce, tag)).decryptor()
        cipher.authenticate_additional_data(MAGIC)
        remaining = size - len(MAGIC) - 28
        while remaining:
            block = src.read(min(CHUNK, remaining))
            remaining -= len(block)
            dst.write(cipher.update(block))
        dst.write(cipher.finalize())
