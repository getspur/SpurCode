import sys,os,json,subprocess,hashlib,datetime,shlex
from pathlib import Path
root=Path(__file__).parent
name=sys.argv[1]
args=sys.argv[2:]
extra={"SPUR_REMOTE":"1","SPUR_NO_LOCAL_FALLBACK":"1"}
if os.environ.get("SPUR_SYNC_FULL"): extra["SPUR_SYNC_FULL"]=os.environ["SPUR_SYNC_FULL"]
if args[1:2]==["fmt"]: extra={"SPUR_REMOTE":"0"}
env=os.environ.copy();env.update(extra)
command=" ".join(k+"="+v for k,v in extra.items())+" "+shlex.join(args)
r={"name":name,"argv":args,"environment":extra,"command":command,"command_sha256":hashlib.sha256(command.encode()).hexdigest(),"cwd":os.getcwd(),"head":subprocess.check_output(["git","rev-parse","HEAD"],text=True).strip(),"source_sha256":{str(p):hashlib.sha256(p.read_bytes()).hexdigest() for p in map(Path,["crates/spur-graph/src/extract/languages.rs","crates/spur-graph/tests/rust_macro_qualified_calls.rs"])},"started_utc":datetime.datetime.now(datetime.timezone.utc).isoformat()}
log=root/(name+".log")
with log.open("w") as f: result=subprocess.run(args,env=env,stdout=f,stderr=subprocess.STDOUT)
r.update(exit_code=result.returncode,finished_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(),log=str(log),log_sha256=hashlib.sha256(log.read_bytes()).hexdigest())
(root/(name+".json")).write_text(json.dumps(r,indent=2)+"\n")
print(json.dumps(r,indent=2));print(log.read_text()[-10000:]);sys.exit(result.returncode)
