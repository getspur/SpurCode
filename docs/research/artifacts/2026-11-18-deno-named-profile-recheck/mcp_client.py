import socket,struct,json,sys,os
SOCKET=os.environ.get('SPUR_RECHECK_SOCKET','/Users/kevintruong/.spur/notebooks/sessions/ws-7e89fab08670f2a778853d55.sock')
def recv(s,n):
    b=b''
    while len(b)<n:
        c=s.recv(n-len(b))
        if not c: raise OSError('MCP closed frame')
        b+=c
    return b
def send(s,v):
    b=json.dumps(v).encode();s.sendall(struct.pack('>I',len(b))+b)
def response(s,i):
    while True:
        n=struct.unpack('>I',recv(s,4))[0]
        if n>16777216:raise ValueError('Oversized response')
        r=json.loads(recv(s,n))
        if r.get('id')==i:return r

def request(method,params):
    with socket.socket(socket.AF_UNIX) as s:
        s.settimeout(45);s.connect(SOCKET)
        send(s,{'jsonrpc':'2.0','id':1,'method':'initialize','params':{'protocolVersion':'2024-11-05','capabilities':{},'clientInfo':{'name':'deno-config-recheck','version':'1'}}})
        response(s,1);send(s,{'jsonrpc':'2.0','method':'notifications/initialized'})
        send(s,{'jsonrpc':'2.0','id':2,'method':method,'params':params})
        return response(s,2)
if __name__=='__main__':
    if sys.argv[1]=='list':
        r=request('tools/list',{})
        wanted=set(sys.argv[2:]);r['result']['tools']=[t for t in r['result']['tools'] if t['name'] in wanted]
    else:
        params=json.load(open(sys.argv[2]));r=request('tools/call',{'name':sys.argv[1],'arguments':params})
    print(json.dumps(r))
