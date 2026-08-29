#ifndef ORION_RUNTIME__STATE_SNAPSHOT_HPP_
#define ORION_RUNTIME__STATE_SNAPSHOT_HPP_

#include <cstddef>
#include <cstdint>
#include <string>
#include <vector>

#include "orion_hardware/sts3215_driver.hpp"

namespace orion_runtime
{

inline constexpr int kStateSchemaVersion = 1;

enum class RuntimeMode
{
  OBSERVE,
  CONFIGURED,
  HOLDING,
  MOVING,
};

struct StateSnapshot
{
  RuntimeMode mode = RuntimeMode::OBSERVE;
  std::uint64_t sequence = 0;
  std::int64_t sampled_at_unix_ns = 0;
  double update_hz = 0.0;
  std::vector<orion_hardware::JointState> joints;
  std::string active_motion;
  double motion_progress = 0.0;
  std::string active_keyframe;
  std::size_t keyframe_index = 0;
  std::size_t keyframe_count = 0;
};

StateSnapshot make_state_snapshot(
  RuntimeMode mode, std::uint64_t sequence, double update_hz,
  std::vector<orion_hardware::JointState> joints,
  std::string active_motion = "", double motion_progress = 0.0,
  std::string active_keyframe = "", std::size_t keyframe_index = 0,
  std::size_t keyframe_count = 0);

const char * runtime_mode_name(RuntimeMode mode);
bool profile_is_applied(RuntimeMode mode);
bool torque_is_enabled(RuntimeMode mode);
std::string state_snapshot_to_json(const StateSnapshot & snapshot);

}  // namespace orion_runtime

#endif  // ORION_RUNTIME__STATE_SNAPSHOT_HPP_
