"""Generate the tiny ONNX identity fixture without third-party Python packages."""
from pathlib import Path

def varint(n):
    out = bytearray()
    while n > 127:
        out.append((n & 127) | 128)
        n >>= 7
    out.append(n)
    return bytes(out)

def integer(field, n):
    return varint(field << 3) + varint(n)

def message(field, value):
    if isinstance(value, str):
        value = value.encode()
    return varint((field << 3) | 2) + varint(len(value)) + value

def info(name):
    dims = b''.join(message(1, integer(1, n) if isinstance(n, int) else message(2, n)) for n in (1, 3, 'height', 'width'))
    tensor = integer(1, 1) + message(2, dims)
    return message(1, name) + message(2, message(1, tensor))

node = message(1, 'input') + message(2, 'output') + message(4, 'Identity')
graph = message(1, node) + message(2, 'NoirView-identity') + message(11, info('input')) + message(12, info('output'))
model = integer(1, 10) + message(2, 'NoirView tests') + message(7, graph) + message(8, integer(2, 17))
Path(__file__).with_name('fixtures').joinpath('identity.onnx').write_bytes(model)
