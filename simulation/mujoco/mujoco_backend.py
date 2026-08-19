"""Shared MuJoCo mapping helpers for Orion simulator development tools."""

from dataclasses import dataclass
from typing import Sequence

import mujoco
import numpy as np


DEFAULT_BASE_BODY_NAME = "scs215_v5"


@dataclass(frozen=True)
class MuJoCoJointMapping:
    """MuJoCo indices aligned with Orion's canonical joint order."""

    joint_names: tuple[str, ...]
    qpos_addresses: tuple[int, ...]
    actuator_ids: tuple[int, ...]


def resolve_joint_mapping(
    model: mujoco.MjModel, joint_names: Sequence[str]
) -> MuJoCoJointMapping:
    """Resolve scalar MuJoCo joints and actuators in canonical order."""

    ordered_names = tuple(joint_names)
    qpos_addresses: list[int] = []
    actuator_ids: list[int] = []

    for name in ordered_names:
        joint_id = mujoco.mj_name2id(model, mujoco.mjtObj.mjOBJ_JOINT, name)
        if joint_id < 0:
            raise ValueError(f"MuJoCo model has no joint named '{name}'")
        if model.jnt_type[joint_id] not in (
            mujoco.mjtJoint.mjJNT_HINGE,
            mujoco.mjtJoint.mjJNT_SLIDE,
        ):
            raise ValueError(f"Joint '{name}' is not a scalar joint")

        actuator_id = mujoco.mj_name2id(
            model, mujoco.mjtObj.mjOBJ_ACTUATOR, name
        )
        if actuator_id < 0:
            raise ValueError(f"MuJoCo model has no actuator named '{name}'")

        qpos_addresses.append(int(model.jnt_qposadr[joint_id]))
        actuator_ids.append(actuator_id)

    return MuJoCoJointMapping(
        joint_names=ordered_names,
        qpos_addresses=tuple(qpos_addresses),
        actuator_ids=tuple(actuator_ids),
    )


def _require_matching_positions(
    mapping: MuJoCoJointMapping, positions: Sequence[float]
) -> tuple[float, ...]:
    values = tuple(float(value) for value in positions)
    if len(values) != len(mapping.joint_names):
        raise ValueError(
            f"Expected {len(mapping.joint_names)} joint positions, got {len(values)}"
        )
    return values


def read_joint_positions(
    data: mujoco.MjData, mapping: MuJoCoJointMapping
) -> tuple[float, ...]:
    """Read Orion's scalar joint positions in canonical order."""

    return tuple(float(data.qpos[address]) for address in mapping.qpos_addresses)


def set_actuator_targets(
    data: mujoco.MjData,
    mapping: MuJoCoJointMapping,
    positions: Sequence[float],
) -> None:
    """Set position-actuator targets in canonical order."""

    values = _require_matching_positions(mapping, positions)
    for actuator_id, value in zip(mapping.actuator_ids, values, strict=True):
        data.ctrl[actuator_id] = value


def set_joint_state(
    model: mujoco.MjModel,
    data: mujoco.MjData,
    mapping: MuJoCoJointMapping,
    positions: Sequence[float],
    *,
    anchor_body_name: str | None = DEFAULT_BASE_BODY_NAME,
) -> None:
    """Set joint state while preserving the grounded base body's world pose.

    Orion's current native MuJoCo tree is rooted in the articulated assembly,
    with the physical base below it in the kinematic tree. Directly changing
    joint coordinates would therefore move the base instead of repositioning
    the free root. The compensating transform below keeps the base fixed.
    """

    values = _require_matching_positions(mapping, positions)

    anchor_body_id = -1
    anchor_position_before: np.ndarray | None = None
    anchor_rotation_before: np.ndarray | None = None
    free_joint_id = -1

    if anchor_body_name is not None:
        anchor_body_id = mujoco.mj_name2id(
            model, mujoco.mjtObj.mjOBJ_BODY, anchor_body_name
        )
        if anchor_body_id < 0:
            raise ValueError(
                f"MuJoCo model has no anchor body named '{anchor_body_name}'"
            )

        free_joint_ids = [
            joint_id
            for joint_id in range(model.njnt)
            if model.jnt_type[joint_id] == mujoco.mjtJoint.mjJNT_FREE
        ]
        if len(free_joint_ids) != 1:
            raise ValueError(
                "Base-preserving initialization requires exactly one free joint"
            )
        free_joint_id = free_joint_ids[0]

        mujoco.mj_forward(model, data)
        anchor_position_before = data.xpos[anchor_body_id].copy()
        anchor_rotation_before = data.xmat[anchor_body_id].reshape(3, 3).copy()

    data.qvel[:] = 0.0
    for address, value in zip(mapping.qpos_addresses, values, strict=True):
        data.qpos[address] = value
    set_actuator_targets(data, mapping, values)
    mujoco.mj_forward(model, data)

    if anchor_body_name is None:
        return

    anchor_position_after = data.xpos[anchor_body_id].copy()
    anchor_rotation_after = data.xmat[anchor_body_id].reshape(3, 3).copy()
    correction_rotation = anchor_rotation_before @ anchor_rotation_after.T
    correction_translation = (
        anchor_position_before - correction_rotation @ anchor_position_after
    )

    free_qpos_address = int(model.jnt_qposadr[free_joint_id])
    root_position = data.qpos[free_qpos_address : free_qpos_address + 3].copy()
    root_quaternion = data.qpos[
        free_qpos_address + 3 : free_qpos_address + 7
    ].copy()
    root_rotation_flat = np.empty(9)
    mujoco.mju_quat2Mat(root_rotation_flat, root_quaternion)
    root_rotation = root_rotation_flat.reshape(3, 3)

    corrected_root_position = (
        correction_rotation @ root_position + correction_translation
    )
    corrected_root_rotation = correction_rotation @ root_rotation
    corrected_root_quaternion = np.empty(4)
    mujoco.mju_mat2Quat(
        corrected_root_quaternion,
        corrected_root_rotation.reshape(9),
    )

    data.qpos[
        free_qpos_address : free_qpos_address + 3
    ] = corrected_root_position
    data.qpos[
        free_qpos_address + 3 : free_qpos_address + 7
    ] = corrected_root_quaternion
    mujoco.mj_forward(model, data)
