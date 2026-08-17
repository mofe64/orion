# Joint Structure for Orion

## `base_yaw_joint`

This connects `shoulder_mount_link` and `base_link`.

Our `base_link` consists of the lamp base, lamp base cover, and base servo components. The joint's axis is exactly the base's vertical Z axis, and it has approximately 360° of travel. Therefore, the joint rotates around the vertical Z axis of the base.

## `shoulder_pitch_joint`

This connects `shoulder_mount_link` and `upper_arm_link`.

`shoulder_mount_link` is the small servo/bracket immediately above the base, while `upper_arm_link` is the first long arm segment. The joint's axis is horizontal relative to the base, and its total range is 180°. This joint is the shoulder hinge that raises and lowers the first arm.

## `elbow_pitch_joint`

This connects `upper_arm_link` and `forearm_link`.

The joint's axis is horizontal, and the joint has 180° of travel. It bends and straightens the two major arm segments.

## `head_roll_joint`

This connects `forearm_link` and `head_roll_link`.

It is located at the end of the second arm, before the lamp-head bracket. Its axis runs approximately along the end of the arm at the exported pose, and it allows 360° of travel.

## `head_pitch_joint`

This connects `head_roll_link` and `lamp_head_link`.

The `lamp_head_link` contains both diffuser and lamp-head components. The joint directly moves the complete lamp head relative to its supporting bracket. It has 180° of travel and acts as the final hinge.
