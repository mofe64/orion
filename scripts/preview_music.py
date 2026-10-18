#!/usr/bin/env python3
"""Render calm and upbeat music through the real PCM tracker and character controller."""
import argparse
import json
from pathlib import Path
import subprocess

import movement_preview_runtime as adapter

ROOT = Path(__file__).resolve().parents[1]
RUST = r'''
    fn music(root: &Path, bpm: f64, level: f64) -> Result<Value> {
        use crate::expression::{character::CharacterCoordinator, music::BeatTracker};
        let poses=PoseLibrary::load(root.join("motion/config/v2/poses.yaml"), &ORION_JOINT_NAMES)?;
        let motions=MotionLibrary::load(root.join("motion/motions/v2"), &poses)?;
        let anchor=poses.pose("home")?.clone();
        let limits=HardwareVersion::V2.load_calibration(root.join("hardware/profiles/v2-joints.json"))?
            .iter().map(|c| {let (lower_rad,upper_rad)=c.safe_range_radians(); JointLimit{name:c.name.clone(),lower_rad,upper_rad}}).collect();
        let mut core=RuntimeCore::new(Driver{positions:anchor,limits},poses,motions)?;
        core.configure_comfort_box(HardwareVersion::V2.profile().comfort_box.clone().unwrap())?;
        let mut character=CharacterCoordinator::new(7);
        character.set_act_settings(crate::expression::settings::ActSettings{wake_sleep_acts:false,posture_drift:false,..Default::default()});
        character.start(0.,&mut core)?;
        let mut tracker=BeatTracker::default();
        let mut samples=vec![];
        for tick in 1..=1500 {
            let now=tick as f64*0.02;
            let mut pulse=crate::expression::music::MusicPulse::default();
            for half in 0..2 {
                let pcm: Vec<_>=(0..480).map(|index| {
                    let t=((tick-1)*960+half*480+index) as f64/48000.;
                    let phase=t%(60./bpm);
                    let tone=level*((std::f64::consts::TAU*220.*t).sin()+0.3*(std::f64::consts::TAU*330.*t).sin());
                    let kick=if phase<0.07 {level*2.*(-phase*50.).exp()*(std::f64::consts::TAU*80.*t).sin()} else {0.};
                    [if now>27. {0.} else {(tone+kick) as f32};2]
                }).collect();
                pulse=tracker.process(&pcm);
            }
            pulse.streaming=true;
            character.set_music_input(Some(pulse),now>=8.);
            core.tick(now)?;
            character.tick(now,&mut core,false,None,false,None,None)?;
            if now>=8. {
                let positions:JointPositions=core.snapshot().joints.iter().map(|s|(s.name.clone(),s.position)).collect();
                samples.push(json!({"time":now-8.,"positions":positions,"bpm":pulse.bpm,"beats":pulse.beats,"energy":pulse.energy,"audible":pulse.audible,"clip":character.status().active_clip}));
            }
        }
        Ok(json!({"bpm":bpm,"level":level,"samples":samples}))
    }
    pub fn run()->Result<()> {
        let root=std::env::args().nth(1).unwrap();
        println!("{}",json!([music(Path::new(&root),75.,0.035)?,music(Path::new(&root),140.,0.25)?]));
        Ok(())
    }
}
pub fn movement_review_main()->Result<()> {movement_review::run()}
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    adapter.RUST = adapter.RUST.split('    fn item(')[0] + RUST
    binary = adapter.build_adapter(ROOT)
    examples = json.loads(subprocess.check_output([str(binary), str(ROOT)], cwd=ROOT))
    (args.output / 'samples.json').write_text(json.dumps(examples))
    import mujoco
    import numpy as np
    from PIL import Image, ImageDraw, ImageFont
    model = mujoco.MjModel.from_xml_path(str(ROOT / 'simulation/mujoco/v2/scene.xml'))
    data = mujoco.MjData(model)
    font = ImageFont.load_default(size=18)
    cameras = []
    for azimuth in (0, 90):
        camera = mujoco.MjvCamera()
        camera.lookat[:] = [0, 0, 0.24]
        camera.distance = 0.86
        camera.azimuth = azimuth
        camera.elevation = -12
        cameras.append(camera)
    for name, example in zip(('calm', 'upbeat'), examples):
        output = args.output / f'{name}.mp4'
        encoder = subprocess.Popen(['ffmpeg','-loglevel','error','-y','-f','rawvideo','-pix_fmt','rgb24',
            '-s','960x540','-r','25','-i','-','-an','-c:v','libx264','-preset','fast','-crf','20',
            '-pix_fmt','yuv420p','-movflags','+faststart',str(output)],stdin=subprocess.PIPE)
        try:
            with mujoco.Renderer(model,height=480,width=480) as renderer:
                for index, sample in enumerate(example['samples']):
                    if index%2: continue
                    for joint,value in sample['positions'].items():
                        data.qpos[model.joint(joint).qposadr[0]]=value
                    mujoco.mj_forward(model,data)
                    frame=Image.new('RGB',(960,540),'#edf1ee')
                    for column,camera in enumerate(cameras):
                        renderer.update_scene(data,camera=camera)
                        frame.paste(Image.fromarray(renderer.render()),(column*480,60))
                    pen=ImageDraw.Draw(frame)
                    state='Return to still' if sample['time']>=19. else ('Move to music' if sample['clip']=='music_performance' else 'Wait for the bar')
                    pen.text((16,9),f'{name.capitalize()} · {example["bpm"]:.0f} BPM · {state}',font=font,fill='#17251b')
                    pen.text((16,34),f'{sample["time"]:.1f} s    Energy {sample["energy"]:.2f}    Beat {int(sample["beats"])%4+1}',font=font,fill='#17251b')
                    encoder.stdin.write(frame.tobytes())
        finally:
            encoder.stdin.close()
            if encoder.wait(): raise RuntimeError(f'Encoding failed: {output}')
        active=[s for s in example['samples'] if s['clip']=='music_performance']
        if not active: raise RuntimeError(f'No music movement in {name} example')
        print(f'{output}: {len(active)} music samples; final clip {example["samples"][-1]["clip"]}')


if __name__ == '__main__':
    main()
