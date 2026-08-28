#include "orion_runtime/pose_library.hpp"

#include <cmath>
#include <set>
#include <stdexcept>

#include "yaml-cpp/yaml.h"

namespace orion_runtime
{

PoseLibrary::PoseLibrary(
  const std::string & path, const std::vector<std::string> & joint_names)
{
  try
  {
    const YAML::Node root = YAML::LoadFile(path);
    if (root["format_version"].as<int>(0) != 1 ||
      root["units"].as<std::string>("") != "radians")
    {
      throw std::runtime_error("Pose library must use format_version 1 and radians.");
    }
    const YAML::Node poses = root["poses"];
    if (!poses.IsMap() || poses.size() == 0)
    {
      throw std::runtime_error("Pose library contains no poses.");
    }

    const std::set<std::string> expected(joint_names.begin(), joint_names.end());
    for (const auto & pose_entry : poses)
    {
      const std::string pose_name = pose_entry.first.as<std::string>();
      const YAML::Node positions = pose_entry.second["positions"];
      if (!positions.IsMap() || positions.size() != joint_names.size())
      {
        throw std::runtime_error("Pose '" + pose_name + "' must contain all Orion joints.");
      }

      std::set<std::string> present;
      JointPositions result;
      for (const auto & position : positions)
      {
        const std::string joint_name = position.first.as<std::string>();
        const double radians = position.second.as<double>();
        if (!std::isfinite(radians))
        {
          throw std::runtime_error("Pose '" + pose_name + "' contains a non-finite target.");
        }
        present.insert(joint_name);
        result.emplace(joint_name, radians);
      }
      if (present != expected)
      {
        throw std::runtime_error("Pose '" + pose_name + "' joint names do not match Orion.");
      }
      poses_.emplace(pose_name, std::move(result));
    }
  }
  catch (const YAML::Exception & error)
  {
    throw std::runtime_error("Could not parse pose library '" + path + "': " + error.what());
  }
}

const JointPositions & PoseLibrary::pose(const std::string & name) const
{
  const auto found = poses_.find(name);
  if (found == poses_.end())
  {
    throw std::invalid_argument("Unknown Orion pose: " + name);
  }
  return found->second;
}

std::vector<std::string> PoseLibrary::names() const
{
  std::vector<std::string> result;
  result.reserve(poses_.size());
  for (const auto & [name, positions] : poses_)
  {
    (void)positions;
    result.push_back(name);
  }
  return result;
}

}  // namespace orion_runtime
