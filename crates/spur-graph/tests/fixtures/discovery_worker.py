#!/usr/bin/python3
"""Private command/cache discovery regression fixture; no native runtime."""
import json, os, struct, sys, threading

args = sys.argv[1:]
case = os.environ['SPUR_M3_CASE']
with open(os.environ['SPUR_M3_LOG'], 'a') as log:
    log.write(json.dumps({'argv': args, 'cache': os.environ.get('SPUR_ORT_CACHE'), 'pid': os.getpid()}) + '\n')

if args == ['--compatibility']:
    if case in ('cancel', 'timeout'):
        threading.Event().wait()
    if case == 'exit': sys.exit(3)
    if case == 'abort': os.abort()
    if case == 'oversized':
        sys.stdout.write(' ' * (33554432 + 1)); sys.stdout.flush()
    elif case == 'malformed': print('{')
    else:
        meta = {'version': os.environ['SPUR_M3_VERSION'], 'protocol': 2,
                'os': os.environ['SPUR_M3_OS'], 'arch': os.environ['SPUR_M3_ARCH']}
        if case in ('version', 'os', 'arch'): meta[case] = 'wrong'
        if case == 'protocol': meta['protocol'] = 1
        if case == 'unknown': meta['unexpected'] = True
        raw = json.dumps(meta)
        if case == 'duplicate': raw = raw[:-1] + ',"protocol":2}'
        print(raw)
    sys.exit(0)

if not os.environ.get('SPUR_ORT_CACHE'):
    sys.exit('missing child cache')

def send(generation, request_id, reply):
    raw = json.dumps({'version': 2, 'generation': generation, 'request_id': request_id, 'reply': reply}).encode()
    sys.stdout.buffer.write(struct.pack('>I', len(raw)) + raw); sys.stdout.buffer.flush()

if args[0] == '--describe-model':
    identity = {'model': args[1], 'dimensions': 768, 'model_fingerprint': 'test-model',
                'tokenizer_fingerprint': 'test-tokenizer', 'pooling': 'mean', 'normalization': 'unit', 'prefix': 'legacy_preformatted'}
    send(args[3], int(args[4]), {'type': 'Ready', 'identity': identity})
else:
    while True:
        header = sys.stdin.buffer.read(4)
        if not header: break
        envelope = json.loads(sys.stdin.buffer.read(struct.unpack('>I', header)[0]))
        request = envelope['request']; kind = request['type']
        if kind == 'Hello': reply = {'type': 'Ready', 'identity': request['identity']}
        elif kind == 'Shutdown': reply = {'type': 'Closed'}
        else: reply = {'type': 'Embeddings', 'identity': request['identity'], 'rows': [[1.0] * 768 for _ in request['texts']]}
        send(envelope['generation'], envelope['request_id'], reply)
        if kind == 'Shutdown': break
