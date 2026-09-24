import hashlib
from pathlib import Path
import socket
import sys
root = Path(sys.argv[1])
loaded = hashlib.sha256((root / 'settings.json').read_bytes()).hexdigest().encode()
with socket.socket(socket.AF_UNIX) as listener:
    listener.bind(str(root / 'service.sock'))
    listener.listen()
    while True:
        connection, _ = listener.accept()
        with connection:
            connection.sendall(loaded)
