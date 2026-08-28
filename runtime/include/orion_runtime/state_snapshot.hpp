#ifndef ORION_RUNTIME__STATE_SNAPSHOT_HPP_
#define ORION_RUNTIME__STATE_SNAPSHOT_HPP_

#include <cstdint>
#include <string>
#include <vector>

#include "orion_hardware/sts3215_driver.hpp"

namespace orion_runtime
{

inline constexpr int kStateSchemaVersion = 1;

struct StateSnapshot
{
  std::uint64_t sequence = 0;
  std::int64_t sampled_at_unix_ns = 0;
  double update_hz = 0.0;
  std::vector<orion_hardware::JointState> joints;
};

StateSnapshot make_state_snapshot(
  std::uint64_t sequence, double update_hz,
  std::vector<orion_hardware::JointState> joints);

std::string state_snapshot_to_json(const StateSnapshot & snapshot);

}  // namespace orion_runtime

#endif  // ORION_RUNTIME__STATE_SNAPSHOT_HPP_
