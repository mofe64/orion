from glob import glob
from os.path import join

from setuptools import find_packages, setup

package_name = "orion_motion"

setup(
    name=package_name,
    version="0.1.0",
    packages=find_packages(exclude=["test"]),
    data_files=[
        (
            "share/ament_index/resource_index/packages",
            ["resource/" + package_name],
        ),
        ("share/" + package_name, ["package.xml"]),
        (join("share", package_name, "config"), glob("config/*.yaml")),
        (
            join("share", package_name, "motions", "functional"),
            glob("motions/functional/*.yaml"),
        ),
        (
            join("share", package_name, "motions", "expressive"),
            glob("motions/expressive/*.yaml"),
        ),
    ],
    install_requires=["setuptools"],
    zip_safe=True,
    maintainer="mofe",
    maintainer_email="ogunbiyioladapo33@gmail.com",
    description="Named poses and keyframe motion playback for Orion.",
    license="GPL-3.0-only",
    tests_require=["pytest"],
    entry_points={
        "console_scripts": [
            "go_to_pose = orion_motion.ros_pose_player:main",
            "play_motion = orion_motion.ros_motion_player:main",
        ],
    },
)
