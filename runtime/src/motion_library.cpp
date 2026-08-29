#include "orion_runtime/motion_library.hpp"

#include <algorithm>
#include <cmath>
#include <filesystem>
#include <stdexcept>
#include <utility>
#include <vector>

#include "yaml-cpp/yaml.h"

namespace orion_runtime
{
namespace
{

MotionDefinition load_motion_file(
  const std::filesystem::path & path, const PoseLibrary & poses)
{
  try
  {
    const YAML::Node root = YAML::LoadFile(path.string());
    if (root["format_version"].as<int>(0) != 1)
    {
      throw std::runtime_error("Motion file must use format_version 1.");
    }
    const YAML::Node motion_node = root["motion"];
    if (!motion_node.IsMap())
    {
      throw std::runtime_error("Motion file must contain a motion mapping.");
    }

    MotionDefinition motion;
    motion.name = motion_node["name"].as<std::string>("");
    motion.description = motion_node["description"].as<std::string>("");
    if (motion.name.empty())
    {
      throw std::runtime_error("Motion name cannot be empty.");
    }
    const YAML::Node keyframes = motion_node["keyframes"];
    if (!keyframes.IsSequence() || keyframes.size() == 0)
    {
      throw std::runtime_error("Motion '" + motion.name + "' must contain keyframes.");
    }

    for (const auto & keyframe_node : keyframes)
    {
      const std::string pose_name = keyframe_node["pose"].as<std::string>("");
      const double duration = keyframe_node["duration"].as<double>();
      const double hold = keyframe_node["hold"].as<double>(0.0);
      if (pose_name.empty() || !std::isfinite(duration) || duration <= 0.0 ||
        !std::isfinite(hold) || hold < 0.0)
      {
        throw std::runtime_error(
                "Motion '" + motion.name +
                "' keyframes require a pose, positive duration, and non-negative hold.");
      }
      motion.keyframes.push_back(MotionKeyframe{
        pose_name,
        poses.pose(pose_name),
        duration,
        hold,
      });
    }
    return motion;
  }
  catch (const YAML::Exception & error)
  {
    throw std::runtime_error(
            "Could not parse motion file '" + path.string() + "': " + error.what());
  }
  catch (const std::invalid_argument & error)
  {
    throw std::runtime_error(
            "Invalid pose reference in motion file '" + path.string() + "': " + error.what());
  }
}

}  // namespace

MotionLibrary::MotionLibrary(const std::string & directory, const PoseLibrary & poses)
{
  namespace fs = std::filesystem;
  try
  {
    if (!fs::is_directory(directory))
    {
      throw std::runtime_error("Motion library is not a directory: " + directory);
    }
    std::vector<fs::path> files;
    for (const auto & entry : fs::recursive_directory_iterator(directory))
    {
      if (entry.is_regular_file() &&
        (entry.path().extension() == ".yaml" || entry.path().extension() == ".yml"))
      {
        files.push_back(entry.path());
      }
    }
    std::sort(files.begin(), files.end());
    if (files.empty())
    {
      throw std::runtime_error("Motion library contains no YAML files: " + directory);
    }

    for (const auto & path : files)
    {
      MotionDefinition motion = load_motion_file(path, poses);
      const std::string name = motion.name;
      if (!motions_.emplace(name, std::move(motion)).second)
      {
        throw std::runtime_error("Duplicate Orion motion name: " + name);
      }
    }
  }
  catch (const fs::filesystem_error & error)
  {
    throw std::runtime_error(
            "Could not read motion library '" + directory + "': " + error.what());
  }
}

const MotionDefinition & MotionLibrary::motion(const std::string & name) const
{
  const auto found = motions_.find(name);
  if (found == motions_.end())
  {
    throw std::invalid_argument("Unknown Orion motion: " + name);
  }
  return found->second;
}

std::vector<std::string> MotionLibrary::names() const
{
  std::vector<std::string> result;
  result.reserve(motions_.size());
  for (const auto & [name, motion] : motions_)
  {
    (void)motion;
    result.push_back(name);
  }
  return result;
}

}  // namespace orion_runtime
