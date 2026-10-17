# Orion V1 joint structure

V1 has five powered joints in one chain. A joint connects a parent link to a child link. Moving the joint moves its child and all later links.

The V1 chain is:

```text
base_link
  → base_yaw_joint → shoulder_mount_link
  → shoulder_pitch_joint → upper_arm_link
  → elbow_pitch_joint → forearm_link
  → head_roll_joint → head_roll_link
  → head_pitch_joint → lamp_head_link
```

## Joint functions

| Joint | Function |
| --- | --- |
| `base_yaw_joint` | Turns everything above the base |
| `shoulder_pitch_joint` | Moves the upper arm and later links |
| `elbow_pitch_joint` | Moves the forearm and head |
| `head_roll_joint` | Rolls the head support and head |
| `head_pitch_joint` | Tilts the head relative to its support |

Each angle is relative to its parent link. The head's world position depends on every earlier joint. Thus, the same head angle can produce different world directions after an arm movement.

`description/urdf/orion.urdf` defines the V1 chain. Hardware calibration defines its physical command limits. The model's geometric travel does not grant permission to use that range on hardware.

See [URDF concepts](orion_urdf_basics.md) for origins and axes. V2's joint order and coordinates differ; see [hardware versions](../hardware-versions.md#joint-names-and-limits).
