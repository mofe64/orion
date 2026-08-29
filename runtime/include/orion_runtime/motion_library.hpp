#ifndef ORION_RUNTIME__MOTION_LIBRARY_HPP_
#define ORION_RUNTIME__MOTION_LIBRARY_HPP_

#include <map>
#include <string>
#include <vector>

#include "orion_runtime/pose_library.hpp"

namespace orion_runtime
{

struct MotionKeyframe
{
  std::string pose_name;
  JointPositions target;
  double duration_seconds = 0.0;
  double hold_seconds = 0.0;
};

struct MotionDefinition
{
  std::string name;
  std::string description;
  std::vector<MotionKeyframe> keyframes;
};

class MotionLibrary
{
public:
  MotionLibrary(const std::string & directory, const PoseLibrary & poses);

  const MotionDefinition & motion(const std::string & name) const;
  std::vector<std::string> names() const;

private:
  std::map<std::string, MotionDefinition> motions_;
};

}  // namespace orion_runtime

#endif  // ORION_RUNTIME__MOTION_LIBRARY_HPP_
