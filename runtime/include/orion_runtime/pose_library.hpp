#ifndef ORION_RUNTIME__POSE_LIBRARY_HPP_
#define ORION_RUNTIME__POSE_LIBRARY_HPP_

#include <map>
#include <string>
#include <vector>

namespace orion_runtime
{

using JointPositions = std::map<std::string, double>;

class PoseLibrary
{
public:
  PoseLibrary(const std::string & path, const std::vector<std::string> & joint_names);

  const JointPositions & pose(const std::string & name) const;
  std::vector<std::string> names() const;

private:
  std::map<std::string, JointPositions> poses_;
};

}  // namespace orion_runtime

#endif  // ORION_RUNTIME__POSE_LIBRARY_HPP_
