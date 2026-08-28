#include "orion_runtime/state_snapshot.hpp"

#include <chrono>
#include <cmath>
#include <iomanip>
#include <sstream>
#include <stdexcept>
#include <utility>

namespace orion_runtime
{
namespace
{

std::string json_string(const std::string & value)
{
  std::ostringstream output;
  output << '"';
  for (const char character : value)
  {
    switch (character)
    {
      case '"': output << "\\\""; break;
      case '\\': output << "\\\\"; break;
      case '\b': output << "\\b"; break;
      case '\f': output << "\\f"; break;
      case '\n': output << "\\n"; break;
      case '\r': output << "\\r"; break;
      case '\t': output << "\\t"; break;
      default: output << character; break;
    }
  }
  output << '"';
  return output.str();
}

void require_finite(double value, const std::string & field)
{
  if (!std::isfinite(value))
  {
    throw std::runtime_error("Cannot serialize non-finite Orion state field: " + field);
  }
}

}  // namespace

StateSnapshot make_state_snapshot(
  RuntimeMode mode, std::uint64_t sequence, double update_hz,
  std::vector<orion_hardware::JointState> joints,
  std::string active_motion, double motion_progress)
{
  const auto now = std::chrono::system_clock::now().time_since_epoch();
  return StateSnapshot{
    mode,
    sequence,
    std::chrono::duration_cast<std::chrono::nanoseconds>(now).count(),
    update_hz,
    std::move(joints),
    std::move(active_motion),
    motion_progress,
  };
}

const char * runtime_mode_name(RuntimeMode mode)
{
  switch (mode)
  {
    case RuntimeMode::OBSERVE: return "observe";
    case RuntimeMode::CONFIGURED: return "configured";
    case RuntimeMode::HOLDING: return "holding";
    case RuntimeMode::MOVING: return "moving";
  }
  throw std::logic_error("Unknown Orion runtime mode.");
}

bool profile_is_applied(RuntimeMode mode)
{
  return mode != RuntimeMode::OBSERVE;
}

bool torque_is_enabled(RuntimeMode mode)
{
  return mode == RuntimeMode::HOLDING || mode == RuntimeMode::MOVING;
}

std::string state_snapshot_to_json(const StateSnapshot & snapshot)
{
  require_finite(snapshot.update_hz, "update_hz");
  require_finite(snapshot.motion_progress, "motion_progress");
  std::ostringstream output;
  output << std::setprecision(15)
         << "{\"schema_version\":" << kStateSchemaVersion
         << ",\"robot\":\"orion\""
         << ",\"mode\":" << json_string(runtime_mode_name(snapshot.mode))
         << ",\"profile_applied\":" << (profile_is_applied(snapshot.mode) ? "true" : "false")
         << ",\"torque_enabled\":" << (torque_is_enabled(snapshot.mode) ? "true" : "false")
         << ",\"sequence\":" << snapshot.sequence
         << ",\"sampled_at_unix_ns\":" << snapshot.sampled_at_unix_ns
         << ",\"update_hz\":" << snapshot.update_hz;
  if (snapshot.active_motion.empty())
  {
    output << ",\"motion\":null";
  }
  else
  {
    output << ",\"motion\":{\"name\":" << json_string(snapshot.active_motion)
           << ",\"progress\":" << snapshot.motion_progress << '}';
  }
  output << ",\"joints\":[";

  for (std::size_t index = 0; index < snapshot.joints.size(); ++index)
  {
    const auto & joint = snapshot.joints[index];
    require_finite(joint.position, joint.name + ".position");
    require_finite(joint.velocity, joint.name + ".velocity");
    require_finite(joint.current_ma, joint.name + ".current_ma");
    require_finite(joint.voltage_v, joint.name + ".voltage_v");
    require_finite(joint.temperature_c, joint.name + ".temperature_c");
    if (index != 0)
    {
      output << ',';
    }
    output << "{\"name\":" << json_string(joint.name)
           << ",\"position_rad\":" << joint.position
           << ",\"velocity_rad_s\":" << joint.velocity
           << ",\"current_ma\":" << joint.current_ma
           << ",\"voltage_v\":" << joint.voltage_v
           << ",\"temperature_c\":" << joint.temperature_c
           << ",\"status\":" << joint.status << '}';
  }
  output << "]}";
  return output.str();
}

}  // namespace orion_runtime
