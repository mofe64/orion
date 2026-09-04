import sys,json
from pathlib import Path
root=Path(__file__).resolve().parents[2]
sys.path[:0]=[str(root/'simulation/mujoco'),str(root/'motion')]
import mujoco
from motion_player import load_playback_data
from mujoco_backend import resolve_joint_mapping,set_joint_state,set_actuator_targets
from orion_motion.compiled_trajectory import sample_trajectory
import numpy as np
import struct,zlib,subprocess
out=root/'.scratch/voice-attention';out.mkdir(parents=True,exist_ok=True)
for name in ['attention_left','attention_right']:
    _,trajectory,start=load_playback_data(name,'home')
    model=mujoco.MjModel.from_xml_path(str(root/'simulation/mujoco/scene.xml'))
    data=mujoco.MjData(model);mapping=resolve_joint_mapping(model,trajectory.joint_names)
    set_joint_state(model,data,mapping,start)
    renderer=mujoco.Renderer(model,height=480,width=640)
    camera=mujoco.MjvCamera();camera.lookat[:]=[.02,0,.26];camera.distance=.95;camera.azimuth=90;camera.elevation=-10
    frames=[]
    duration=trajectory.duration_seconds
    for index in range(int((duration+1)*25)):
        t=index/25
        while data.time<t:
            point,_=sample_trajectory(trajectory,min(data.time,duration))
            set_actuator_targets(data,mapping,point.positions)
            mujoco.mj_step(model,data)
        renderer.update_scene(data,camera=camera)
        frames.append(renderer.render().copy())
    renderer.close()
    subprocess.run(['ffmpeg','-v','error','-y','-f','rawvideo','-pixel_format','rgb24','-video_size','640x480','-framerate','25','-i','-','-c:v','libx264','-pix_fmt','yuv420p','-movflags','+faststart',str(out/f'{name}.mp4')],input=b''.join(f.tobytes() for f in frames),check=True)
    strip=np.concatenate([frames[i] for i in [0,len(frames)//4,len(frames)//2,len(frames)-1]],axis=1)
    def chunk(tag,data):return struct.pack('!I',len(data))+tag+data+struct.pack('!I',zlib.crc32(tag+data))
    png=b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('!2I5B',strip.shape[1],strip.shape[0],8,2,0,0,0))+chunk(b'IDAT',zlib.compress(b''.join(b'\0'+row.tobytes() for row in strip)))+chunk(b'IEND',b'')
    (out/f'{name}.png').write_bytes(png)
    print(name,len(frames),out)
