import json,socket,subprocess,tempfile,time
from pathlib import Path
root=Path(__file__).resolve().parents[2]
def request(path,command):
 with socket.socket(socket.AF_UNIX,socket.SOCK_STREAM) as s:
  s.settimeout(2);s.connect(str(path));s.sendall((command+'\n').encode())
  return json.loads(s.makefile('rb').readline())
with tempfile.TemporaryDirectory() as tmp:
 for mode in ['on','off']:
  path=Path(tmp)/f'{mode}.sock'
  args=[str(root/'runtime/target/debug/oriond'),'--serve','--backend','mujoco','--socket',str(path),'--start-pose','home']
  if mode=='off':args+=['--character-on-start','off']
  with (Path(tmp)/f'{mode}.log').open('w+') as log:
   process=subprocess.Popen(args,cwd=root,stdout=log,stderr=log)
   try:
    deadline=time.monotonic()+15
    while True:
     if process.poll() is not None:
      log.seek(0);raise AssertionError(log.read())
     if path.exists():
      state=request(path,'character status')['character']
      if (mode=='on' and state['state']=='home_idle') or (mode=='off' and state['state']=='off'):break
     if time.monotonic()>deadline:raise AssertionError('startup deadline')
     time.sleep(.1)
    print(mode,state['state'],flush=True)
    if mode=='on':
     assert request(path,'character stop')['ok']
     deadline=time.monotonic()+12
     while request(path,'character status')['character']['state']!='off':
      if time.monotonic()>deadline:raise AssertionError('stop deadline')
      time.sleep(.1)
     time.sleep(.5)
     assert not request(path,'character status')['character']['enabled']
     print('Studio-equivalent stop persists in same daemon session',flush=True)
   finally:
    process.terminate()
    try:process.wait(timeout=5)
    except subprocess.TimeoutExpired:process.kill();process.wait()
