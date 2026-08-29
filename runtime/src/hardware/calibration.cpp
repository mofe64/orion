#include "orion_hardware/sts3215_driver.hpp"

#include <set>
#include <stdexcept>
#include <string>
#include <vector>

#include "yaml-cpp/yaml.h"

namespace orion_hardware
{
namespace
{

int required_int(const YAML::Node & node, const std::string & key, const std::string & path)
{
  if (!node[key])
  {
    throw std::runtime_error("Missing calibration value: " + path + "." + key);
  }
  return node[key].as<int>();
}

}  // namespace

std::vector<JointCalibration> load_calibration_file(
  const std::string & path, const std::vector<std::string> & expected_joint_names)
{
  try
  {
    const YAML::Node root = YAML::LoadFile(path);
    if (!root.IsMap() || root["schema_version"].as<int>(0) != 1)
    {
      throw std::runtime_error("Calibration must use schema_version 1.");
    }
    if (root["robot"].as<std::string>("") != "orion" ||
      root["servo_model"].as<std::string>("") != "sts3215")
    {
      throw std::runtime_error("Calibration is not for Orion STS3215 hardware.");
    }
    if (root["encoder_resolution"].as<int>(0) != kEncoderResolution)
    {
      throw std::runtime_error("Calibration must use the STS3215 4096-count encoder.");
    }
    if (!root["writes_servo_eeprom"] || root["writes_servo_eeprom"].as<bool>())
    {
      throw std::runtime_error("Calibration must retain software-only EEPROM provenance.");
    }

    const YAML::Node joints = root["joints"];
    if (!joints.IsMap() || joints.size() != expected_joint_names.size())
    {
      throw std::runtime_error("Calibration must contain exactly the configured Orion joints.");
    }

    std::set<std::string> expected(expected_joint_names.begin(), expected_joint_names.end());
    std::set<std::string> present;
    for (const auto & entry : joints)
    {
      present.insert(entry.first.as<std::string>());
    }
    if (present != expected)
    {
      throw std::runtime_error("Calibration joint names do not match Orion's configured joints.");
    }

    std::set<int> servo_ids;
    std::vector<JointCalibration> result;
    result.reserve(expected_joint_names.size());
    for (const auto & name : expected_joint_names)
    {
      const YAML::Node joint = joints[name];
      const int servo_id = required_int(joint, "servo_id", name);
      const int neutral_raw = required_int(joint, "neutral_raw", name);
      const int direction = required_int(joint, "encoder_direction", name);
      const int safe_min = required_int(joint, "safe_min_delta_raw", name);
      const int safe_max = required_int(joint, "safe_max_delta_raw", name);

      if (servo_id < 1 || servo_id > 252 || !servo_ids.insert(servo_id).second)
      {
        throw std::runtime_error(name + " has an invalid or duplicate servo ID.");
      }
      if (neutral_raw < 0 || neutral_raw >= kEncoderResolution)
      {
        throw std::runtime_error(name + " neutral_raw is outside 0..4095.");
      }
      if (direction != -1 && direction != 1)
      {
        throw std::runtime_error(name + " encoder_direction must be -1 or +1.");
      }
      if (safe_min >= 0 || safe_max <= 0 || safe_min <= -2048 || safe_max >= 2048)
      {
        throw std::runtime_error(
                name + " safe range must contain zero and stay inside one half-turn.");
      }

      result.push_back(JointCalibration{
        name,
        static_cast<std::uint8_t>(servo_id),
        neutral_raw,
        direction,
        safe_min,
        safe_max,
      });
    }
    return result;
  }
  catch (const YAML::Exception & error)
  {
    throw std::runtime_error("Could not parse calibration '" + path + "': " + error.what());
  }
}

}  // namespace orion_hardware
