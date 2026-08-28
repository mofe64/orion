#ifndef ORION_RUNTIME__JOINT_TRAJECTORY_HPP_
#define ORION_RUNTIME__JOINT_TRAJECTORY_HPP_

#include <string>

#include "orion_runtime/pose_library.hpp"

namespace orion_runtime
{

class JointTrajectory
{
public:
  JointTrajectory(
    std::string name, JointPositions start, JointPositions target,
    double duration_seconds);

  JointPositions sample(double elapsed_seconds) const;
  double progress(double elapsed_seconds) const;
  bool complete(double elapsed_seconds) const;
  const std::string & name() const noexcept;
  double duration_seconds() const noexcept;

private:
  std::string name_;
  JointPositions start_;
  JointPositions target_;
  double duration_seconds_ = 0.0;
};

}  // namespace orion_runtime

#endif  // ORION_RUNTIME__JOINT_TRAJECTORY_HPP_
