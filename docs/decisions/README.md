# Orion architecture decisions

| Decision | Status | Summary |
| --- | --- | --- |
| [0001](0001-native-rust-runtime.md) | Accepted | Use one ROS-independent Rust runtime for physical and MuJoCo control |
| [0002](0002-semantic-hardware-boundary.md) | Accepted | Keep raw device authority inside `oriond` |
| [0003](0003-studio-primary-voice.md) | Accepted | Use Studio as the primary high-quality voice host |
| [0004](0004-pi-fallback-voice.md) | Accepted | Retain the Pi-local voice stack as a fallback and diagnostic path |
