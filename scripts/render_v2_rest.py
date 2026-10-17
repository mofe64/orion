#!/usr/bin/env python3
"""Render the builder's all-zero V2 rest reference; optional comparisons remain private."""
import argparse
from pathlib import Path
import mujoco
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[1]
VIEWS = {'side': (0, -7), 'front': (90, -16)}


def render(output, photos=None):
    model = mujoco.MjModel.from_xml_path(str(ROOT / 'simulation/mujoco/v2/scene.xml'))
    data = mujoco.MjData(model); data.qpos[:] = 0; mujoco.mj_forward(model, data)
    output.mkdir(parents=True, exist_ok=True)
    with mujoco.Renderer(model, height=740, width=650) as renderer:
        for name, (azimuth, elevation) in VIEWS.items():
            camera = mujoco.MjvCamera(); camera.lookat[:] = [0, 0, .145]
            camera.distance = .63; camera.azimuth = azimuth; camera.elevation = elevation
            renderer.update_scene(data, camera=camera)
            picture = Image.fromarray(renderer.render()); picture.save(output / f'v2-rest-{name}.png')
            if photos:
                photo = Image.open(photos / f'v2-rest-{name}.jpg').convert('RGB'); photo.thumbnail((650, 740))
                comparison = Image.new('RGB', (1300, 790), 'white'); draw = ImageDraw.Draw(comparison)
                comparison.paste(photo, ((650 - photo.width) // 2, 50)); comparison.paste(picture, (650, 50))
                draw.text((20, 15), 'Physical mechanical rest (private reference)', fill='black')
                draw.text((670, 15), 'MuJoCo all joints = 0 radians', fill='black')
                comparison.save(photos / f'v2-rest-{name}-comparison.png')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=ROOT / 'docs/images')
    parser.add_argument('--private-photos', type=Path)
    args = parser.parse_args(); render(args.output, args.private_photos)
