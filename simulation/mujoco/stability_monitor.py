"""Measure Orion base stability during one native MuJoCo motion."""

from __future__ import annotations

import math
from dataclasses import dataclass
from typing import Any

import mujoco
import numpy as np


class StabilityPolicyError(ValueError):
    """Raised when the stability configuration is incomplete or invalid."""


@dataclass(frozen=True)
class StabilityPolicy:
    position_tolerance: float
    velocity_tolerance: float
    settle_duration: float
    settle_timeout: float
    base_body_name: str
    maximum_translation: float
    maximum_tilt: float
    maximum_height_change: float
    base_geometry_name: str
    floor_geometry_name: str
    maximum_contact_loss_duration: float


@dataclass(frozen=True)
class StabilitySnapshot:
    maximum_translation: float
    maximum_tilt: float
    maximum_height_change: float
    longest_contact_loss: float
    base_in_contact: bool
    unsafe_reasons: tuple[str, ...]

    @property
    def safe(self) -> bool:
        return not self.unsafe_reasons


def stability_policy_from_data(data: Any) -> StabilityPolicy:
    """Validate and build the native-simulation stability policy."""

    if not isinstance(data, dict):
        raise StabilityPolicyError("stability limits must be a mapping")
    if type(data.get("format_version")) is not int or data["format_version"] != 1:
        raise StabilityPolicyError("stability_limits.format_version must be 1")
    if data.get("applicability") != "provisional_simulation_only":
        raise StabilityPolicyError(
            "stability_limits.applicability must be provisional_simulation_only"
        )

    completion = data.get("completion")
    base = data.get("base")
    contact = data.get("contact")
    if not all(isinstance(section, dict) for section in (completion, base, contact)):
        raise StabilityPolicyError(
            "stability limits require completion, base, and contact mappings"
        )

    numeric_values = {
        "completion.position_tolerance": completion.get("position_tolerance"),
        "completion.velocity_tolerance": completion.get("velocity_tolerance"),
        "completion.settle_duration": completion.get("settle_duration"),
        "completion.settle_timeout": completion.get("settle_timeout"),
        "base.maximum_translation": base.get("maximum_translation"),
        "base.maximum_tilt": base.get("maximum_tilt"),
        "base.maximum_height_change": base.get("maximum_height_change"),
        "contact.maximum_loss_duration": contact.get("maximum_loss_duration"),
    }
    for name, value in numeric_values.items():
        if (
            isinstance(value, bool)
            or not isinstance(value, (int, float))
            or not math.isfinite(value)
            or value <= 0
        ):
            raise StabilityPolicyError(f"{name} must be finite and positive")

    text_values = {
        "base.body_name": base.get("body_name"),
        "contact.base_geometry": contact.get("base_geometry"),
        "contact.floor_geometry": contact.get("floor_geometry"),
    }
    for name, value in text_values.items():
        if not isinstance(value, str) or not value.strip():
            raise StabilityPolicyError(f"{name} must be a non-empty string")

    return StabilityPolicy(
        position_tolerance=float(completion["position_tolerance"]),
        velocity_tolerance=float(completion["velocity_tolerance"]),
        settle_duration=float(completion["settle_duration"]),
        settle_timeout=float(completion["settle_timeout"]),
        base_body_name=base["body_name"],
        maximum_translation=float(base["maximum_translation"]),
        maximum_tilt=float(base["maximum_tilt"]),
        maximum_height_change=float(base["maximum_height_change"]),
        base_geometry_name=contact["base_geometry"],
        floor_geometry_name=contact["floor_geometry"],
        maximum_contact_loss_duration=float(contact["maximum_loss_duration"]),
    )


def _required_id(
    model: mujoco.MjModel,
    object_type: Any,
    name: str,
    kind: str,
) -> int:
    object_id = mujoco.mj_name2id(model, object_type, name)
    if object_id < 0:
        raise ValueError(f"MuJoCo model has no {kind} named '{name}'")
    return object_id


class StabilityMonitor:
    """Accumulate base movement and support-contact evidence."""

    def __init__(
        self,
        model: mujoco.MjModel,
        data: mujoco.MjData,
        policy: StabilityPolicy,
    ) -> None:
        self._data = data
        self._policy = policy
        self._body_id = _required_id(
            model,
            mujoco.mjtObj.mjOBJ_BODY,
            policy.base_body_name,
            "body",
        )
        self._base_geom_id = _required_id(
            model,
            mujoco.mjtObj.mjOBJ_GEOM,
            policy.base_geometry_name,
            "geometry",
        )
        self._floor_geom_id = _required_id(
            model,
            mujoco.mjtObj.mjOBJ_GEOM,
            policy.floor_geometry_name,
            "geometry",
        )
        self._reference_position = data.xpos[self._body_id].copy()
        self._reference_up = data.xmat[self._body_id].reshape(3, 3)[:, 2].copy()
        self._last_time = float(data.time)
        self._current_contact_loss = 0.0
        self._maximum_translation = 0.0
        self._maximum_tilt = 0.0
        self._maximum_height_change = 0.0
        self._longest_contact_loss = 0.0
        self._unsafe_reasons: set[str] = set()

    def update(self) -> StabilitySnapshot:
        position = self._data.xpos[self._body_id]
        current_up = self._data.xmat[self._body_id].reshape(3, 3)[:, 2]
        translation = float(np.linalg.norm(position - self._reference_position))
        height_change = abs(float(position[2] - self._reference_position[2]))
        up_dot = float(np.clip(np.dot(self._reference_up, current_up), -1.0, 1.0))
        tilt = math.acos(up_dot)
        base_in_contact = any(
            {
                int(self._data.contact[index].geom1),
                int(self._data.contact[index].geom2),
            }
            == {self._base_geom_id, self._floor_geom_id}
            for index in range(self._data.ncon)
        )

        elapsed = max(0.0, float(self._data.time) - self._last_time)
        self._last_time = float(self._data.time)
        if base_in_contact:
            self._current_contact_loss = 0.0
        else:
            self._current_contact_loss += elapsed

        self._maximum_translation = max(self._maximum_translation, translation)
        self._maximum_tilt = max(self._maximum_tilt, tilt)
        self._maximum_height_change = max(
            self._maximum_height_change,
            height_change,
        )
        self._longest_contact_loss = max(
            self._longest_contact_loss,
            self._current_contact_loss,
        )

        if translation > self._policy.maximum_translation:
            self._unsafe_reasons.add("base translation exceeded its limit")
        if tilt > self._policy.maximum_tilt:
            self._unsafe_reasons.add("base tilt exceeded its limit")
        if height_change > self._policy.maximum_height_change:
            self._unsafe_reasons.add("base height change exceeded its limit")
        if self._current_contact_loss > self._policy.maximum_contact_loss_duration:
            self._unsafe_reasons.add("base lost floor contact for too long")

        return self.snapshot(base_in_contact=base_in_contact)

    def snapshot(self, *, base_in_contact: bool) -> StabilitySnapshot:
        return StabilitySnapshot(
            maximum_translation=self._maximum_translation,
            maximum_tilt=self._maximum_tilt,
            maximum_height_change=self._maximum_height_change,
            longest_contact_loss=self._longest_contact_loss,
            base_in_contact=base_in_contact,
            unsafe_reasons=tuple(sorted(self._unsafe_reasons)),
        )
